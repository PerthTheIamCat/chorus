//! A real adapter over the Codex ACP bridge (`@agentclientprotocol/codex-acp`).
//!
//! Spike 1 settled the shape (see CLAUDE.md §7.6 / §8):
//!   - Codex has no native `codex acp`; we drive the npm bridge.
//!   - `session/load` replays history — we ignore the replay, the ledger is
//!     the source of truth.
//!   - Pin `read-only` mode so the model's `on-request` approvals for
//!     anything the sandbox blocks arrive as `session/request_permission`.
//!   - Updates are structured: `tool_call`, `agent_message_chunk`,
//!     `agent_thought_chunk`, `usage_update` — no prose parsing.
//!
//! This module turns one prompt turn into a [`TurnOutcome`]: the text, the
//! tool calls, the token counts, and a `Vec<AgentEvent>` the caller maps
//! onto ledger events. It never touches `chorus-core` — the crate boundary
//! stays one-way.

use crate::{Agent, Inbound, Log};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::io::sink;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The published bridge, run through `npx`. Override for a pinned install or
/// a local checkout via `CodexConfig::command`.
pub const DEFAULT_BRIDGE: &[&str] = &["npx", "-y", "@agentclientprotocol/codex-acp"];

/// The ACP command for a given agent name. Every one of these speaks the
/// same protocol (`Agent` + this module handle it); they differ only in
/// which process to spawn. An unknown name falls back to the Codex bridge.
pub fn bridge_for(agent_name: &str) -> Vec<String> {
    let n = agent_name.to_lowercase();
    let cmd: &[&str] = if n.contains("claude") {
        &["npx", "-y", "@zed-industries/claude-code-acp"]
    } else if n.contains("gemini") {
        &["gemini", "--experimental-acp"]
    } else {
        DEFAULT_BRIDGE
    };
    cmd.iter().map(|s| s.to_string()).collect()
}

/// One decision from the caller's permission engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Allow,
    Deny,
}

/// What the agent is asking permission to do. `tool` is the ACP tool-call
/// title ("Run command", "Edit files", …); `detail` is any reason string.
#[derive(Debug, Clone)]
pub struct PermissionAsk {
    pub tool: String,
    pub kind: String,
    pub detail: String,
}

/// A structured thing the agent did this turn, ready to become a ledger
/// event. Deliberately small — no ids, no timestamps (the ledger stamps
/// those), no confidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    /// Assistant prose (all `agent_message_chunk`s of the turn, joined).
    Message(String),
    /// A tool the agent invoked.
    ToolCall { title: String, kind: String },
    /// The resolution of a tool call.
    ToolResult { title: String, status: String },
    /// A file edit the agent proposed, as a unified-diff-ready triple.
    Diff {
        path: String,
        old: String,
        new: String,
    },
}

/// The result of one `prompt` turn.
///
/// Codex reports usage as cumulative *context* pressure, not a per-turn
/// in/out split — `context_used` of `context_window` — so that's what we
/// surface. The relay logic watches `context_used / context_window` to
/// decide when a handoff for `ContextExhausted` is due.
#[derive(Debug, Clone, Default)]
pub struct TurnOutcome {
    pub message: String,
    pub thoughts: String,
    pub events: Vec<AgentEvent>,
    pub context_used: Option<i64>,
    pub context_window: Option<i64>,
    pub stop_reason: String,
}

/// How to start the bridge.
pub struct CodexConfig {
    pub command: Vec<String>,
    /// Written into `session/new`; empty is valid.
    pub mcp_servers: Value,
    /// The mode id to pin. `read-only` routes approvals to us.
    pub mode: Option<String>,
    /// Transcript sink. `None` discards every frame.
    pub log: Option<Log>,
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            command: DEFAULT_BRIDGE.iter().map(|s| s.to_string()).collect(),
            mcp_servers: json!([]),
            mode: Some("read-only".into()),
            log: None,
        }
    }
}

/// A live Codex session over ACP.
pub struct CodexAgent {
    agent: Agent,
    session_id: String,
    cwd: PathBuf,
}

impl CodexAgent {
    /// Start a fresh session: spawn → initialize → `session/new` → pin mode.
    pub fn start(cwd: &Path, cfg: CodexConfig) -> Result<Self> {
        let mut this = Self::spawn_and_init(cwd, &cfg)?;
        let new = this.agent.request(
            "session/new",
            json!({ "cwd": this.cwd.to_string_lossy(), "mcpServers": cfg.mcp_servers }),
        )?;
        let res = this.pump(new, Duration::from_secs(60))?;
        this.session_id = res
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("session/new returned no sessionId: {res}"))?
            .to_string();
        this.pin_mode(cfg.mode.as_deref());
        Ok(this)
    }

    /// Reattach to an existing session. The bridge replays history on load;
    /// we drain and discard it — the ledger already holds that history.
    pub fn load(cwd: &Path, session_id: &str, cfg: CodexConfig) -> Result<Self> {
        let mut this = Self::spawn_and_init(cwd, &cfg)?;
        this.session_id = session_id.to_string();
        let load = this.agent.request(
            "session/load",
            json!({
                "sessionId": session_id,
                "cwd": this.cwd.to_string_lossy(),
                "mcpServers": cfg.mcp_servers,
            }),
        )?;
        // Swallow the replayed session/update frames.
        let _ = this.pump(load, Duration::from_secs(120))?;
        this.pin_mode(cfg.mode.as_deref());
        Ok(this)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Run one prompt turn. `decide` is called for every
    /// `session/request_permission`; returning [`Permission::Deny`] cancels
    /// that action (and usually the turn).
    pub fn prompt(
        &mut self,
        text: &str,
        decide: &mut dyn FnMut(&PermissionAsk) -> Permission,
    ) -> Result<TurnOutcome> {
        let mut acc = Accumulator::default();
        let cwd = self.cwd.clone();

        let id = self.agent.request(
            "session/prompt",
            json!({
                "sessionId": self.session_id,
                "prompt": [{ "type": "text", "text": text }],
            }),
        )?;

        let mut handle = |a: &Agent, inbound: Inbound| match inbound {
            Inbound::Notification { method, params } => {
                if method == "session/update" {
                    acc.absorb(&params);
                }
            }
            Inbound::Request { id, method, params } => {
                let reply = handle_request(&cwd, &method, &params, decide);
                let _ = match reply {
                    RequestReply::Result(v) => a.respond(&id, v),
                    RequestReply::Error(code, msg) => a.respond_err(&id, code, &msg),
                };
            }
        };

        let res = self
            .agent
            .wait_for(id, Duration::from_secs(600), &mut handle)
            .context("session/prompt")?;
        self.agent.drain(Duration::from_millis(400), &mut handle);

        acc.stop_reason = res
            .get("stopReason")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok(acc.finish())
    }

    // ---- internals ----------------------------------------------------

    fn spawn_and_init(cwd: &Path, cfg: &CodexConfig) -> Result<Self> {
        let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        let log: Log = cfg
            .log
            .clone()
            .unwrap_or_else(|| Arc::new(Mutex::new(Box::new(sink()))));
        let agent = Agent::spawn(&cfg.command, log)
            .with_context(|| format!("spawn codex bridge: {}", cfg.command.join(" ")))?;
        let this = Self {
            agent,
            session_id: String::new(),
            cwd,
        };

        let init = this.agent.request(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": { "readTextFile": true, "writeTextFile": true },
                    "terminal": false
                },
                "clientInfo": { "name": "chorus", "version": env!("CARGO_PKG_VERSION") }
            }),
        )?;
        let res = this
            .pump(init, Duration::from_secs(30))
            .context("initialize")?;
        let pv = res.get("protocolVersion").and_then(Value::as_i64);
        if pv != Some(1) {
            return Err(anyhow!("bridge spoke protocolVersion {pv:?}, expected 1"));
        }
        Ok(this)
    }

    /// `session/set_mode` is best-effort: the bridge accepts it but spike 1
    /// showed it does not always take. We still send it — on a bridge that
    /// honours it, this is what puts our permission engine in the loop.
    fn pin_mode(&mut self, mode: Option<&str>) {
        let Some(mode) = mode else { return };
        if let Ok(id) = self.agent.request(
            "session/set_mode",
            json!({ "sessionId": self.session_id, "modeId": mode }),
        ) {
            let _ = self.pump(id, Duration::from_secs(15));
        }
    }

    /// Wait for the response to `id`, discarding any `session/update` frames
    /// that arrive alongside it (used for setup calls, not prompt turns).
    fn pump(&self, id: i64, timeout: Duration) -> Result<Value> {
        let mut ignore = |_: &Agent, _: Inbound| {};
        self.agent.wait_for(id, timeout, &mut ignore)
    }
}

enum RequestReply {
    Result(Value),
    Error(i64, String),
}

fn handle_request(
    cwd: &Path,
    method: &str,
    params: &Value,
    decide: &mut dyn FnMut(&PermissionAsk) -> Permission,
) -> RequestReply {
    match method {
        "fs/read_text_file" => match read_text_file(cwd, params) {
            Ok(v) => RequestReply::Result(v),
            Err(e) => RequestReply::Error(-32000, e.to_string()),
        },
        "fs/write_text_file" => {
            let ask = PermissionAsk {
                tool: "write file".into(),
                kind: "edit".into(),
                detail: params
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .into(),
            };
            if decide(&ask) == Permission::Deny {
                return RequestReply::Error(-32001, "denied by permission engine".into());
            }
            match write_text_file(cwd, params) {
                Ok(()) => RequestReply::Result(json!({})),
                Err(e) => RequestReply::Error(-32000, e.to_string()),
            }
        }
        "session/request_permission" => {
            let ask = permission_ask(params);
            let want_kind = match decide(&ask) {
                Permission::Allow => "allow_once",
                Permission::Deny => "reject_once",
            };
            let options = params.get("options").and_then(Value::as_array);
            let chosen = options.and_then(|arr| {
                arr.iter()
                    .find(|o| o.get("kind").and_then(Value::as_str) == Some(want_kind))
                    .or_else(|| arr.first())
                    .and_then(|o| o.get("optionId"))
                    .and_then(Value::as_str)
            });
            match chosen {
                Some(oid) => RequestReply::Result(
                    json!({ "outcome": { "outcome": "selected", "optionId": oid } }),
                ),
                None => RequestReply::Result(json!({ "outcome": { "outcome": "cancelled" } })),
            }
        }
        _ => RequestReply::Error(-32601, format!("chorus does not implement {method}")),
    }
}

fn permission_ask(params: &Value) -> PermissionAsk {
    let tc = params.get("toolCall");
    PermissionAsk {
        tool: tc
            .and_then(|t| t.get("title"))
            .and_then(Value::as_str)
            .unwrap_or("action")
            .to_string(),
        kind: tc
            .and_then(|t| t.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        detail: params
            .get("_meta")
            .and_then(|m| m.get("permission"))
            .and_then(|p| p.get("description"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    }
}

/// Serve a read only from inside `cwd` — the client holds the files.
fn read_text_file(cwd: &Path, params: &Value) -> Result<Value> {
    let path = params
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("no path"))?;
    let canon = resolve_inside(cwd, path)?;
    Ok(json!({ "content": std::fs::read_to_string(&canon)? }))
}

fn write_text_file(cwd: &Path, params: &Value) -> Result<()> {
    let path = params
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("no path"))?;
    let content = params.get("content").and_then(Value::as_str).unwrap_or("");
    // A write may create a new file, so resolve the parent, not the target.
    let target = PathBuf::from(path);
    let parent = target.parent().unwrap_or(cwd);
    let parent = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());
    if !parent.starts_with(cwd) {
        return Err(anyhow!(
            "refused: {} is outside {}",
            target.display(),
            cwd.display()
        ));
    }
    std::fs::write(&target, content)?;
    Ok(())
}

fn resolve_inside(cwd: &Path, path: &str) -> Result<PathBuf> {
    let canon = PathBuf::from(path)
        .canonicalize()
        .map_err(|e| anyhow!("{path}: {e}"))?;
    if !canon.starts_with(cwd) {
        return Err(anyhow!(
            "refused: {} is outside {}",
            canon.display(),
            cwd.display()
        ));
    }
    Ok(canon)
}

/// Folds a turn's `session/update` stream into a [`TurnOutcome`].
#[derive(Default)]
struct Accumulator {
    message: String,
    thoughts: String,
    events: Vec<AgentEvent>,
    /// `toolCallId` → title, so a `tool_call_update` (which carries only the
    /// id) can name the call it resolves.
    titles: std::collections::HashMap<String, String>,
    context_used: Option<i64>,
    context_window: Option<i64>,
    stop_reason: String,
    open_message: bool,
}

impl Accumulator {
    fn absorb(&mut self, params: &Value) {
        let update = match params.get("update") {
            Some(u) => u,
            None => return,
        };
        let kind = update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or("");
        match kind {
            "agent_message_chunk" => {
                if let Some(t) = chunk_text(update) {
                    self.message.push_str(&t);
                    self.open_message = true;
                }
            }
            "agent_thought_chunk" => {
                if let Some(t) = chunk_text(update) {
                    self.thoughts.push_str(&t);
                }
            }
            "tool_call" => {
                self.flush_message();
                let title = update
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string();
                let kind = update
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if let Some(id) = update.get("toolCallId").and_then(Value::as_str) {
                    self.titles.insert(id.to_string(), title.clone());
                }
                self.events.push(AgentEvent::ToolCall { title, kind });
            }
            "tool_call_update" => {
                let status = update
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let title = update
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        update
                            .get("toolCallId")
                            .and_then(Value::as_str)
                            .and_then(|id| self.titles.get(id).cloned())
                    })
                    .unwrap_or_else(|| "tool".to_string());
                if let Some(content) = update.get("content").and_then(Value::as_array) {
                    for c in content {
                        if c.get("type").and_then(Value::as_str) == Some("diff") {
                            self.events.push(AgentEvent::Diff {
                                path: c
                                    .get("path")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string(),
                                old: c
                                    .get("oldText")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string(),
                                new: c
                                    .get("newText")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string(),
                            });
                        }
                    }
                }
                if !status.is_empty() {
                    self.events.push(AgentEvent::ToolResult { title, status });
                }
            }
            "usage_update" => {
                // Bridge sends `{used, size}` — cumulative context pressure.
                // Last one in the turn wins.
                if let Some(used) = update.get("used").and_then(Value::as_i64) {
                    self.context_used = Some(used);
                }
                if let Some(size) = update.get("size").and_then(Value::as_i64) {
                    self.context_window = Some(size);
                }
            }
            _ => {}
        }
    }

    fn flush_message(&mut self) {
        if self.open_message && !self.message.trim().is_empty() {
            self.events
                .push(AgentEvent::Message(self.message.trim().to_string()));
            self.message.clear();
            self.open_message = false;
        }
    }

    fn finish(mut self) -> TurnOutcome {
        self.flush_message();
        // If the whole message came after the last tool call, `message` is
        // still populated — keep it as the outcome's headline text.
        let message = if self.message.trim().is_empty() {
            self.events
                .iter()
                .rev()
                .find_map(|e| match e {
                    AgentEvent::Message(m) => Some(m.clone()),
                    _ => None,
                })
                .unwrap_or_default()
        } else {
            self.message.trim().to_string()
        };
        TurnOutcome {
            message,
            thoughts: self.thoughts.trim().to_string(),
            events: self.events,
            context_used: self.context_used,
            context_window: self.context_window,
            stop_reason: self.stop_reason,
        }
    }
}

fn chunk_text(update: &Value) -> Option<String> {
    let content = update.get("content")?;
    if let Some(t) = content.get("text").and_then(Value::as_str) {
        return Some(t.to_string());
    }
    // Some builds send content as an array of typed parts.
    if let Some(arr) = content.as_array() {
        let joined: String = arr
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect();
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    None
}

/// A file-backed transcript log, for callers that want the frames on disk.
pub fn file_log(path: &Path) -> Result<Log> {
    let f = std::fs::File::create(path)?;
    Ok(Arc::new(Mutex::new(Box::new(f))))
}
