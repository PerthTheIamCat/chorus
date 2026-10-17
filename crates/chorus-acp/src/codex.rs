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

/// The Claude bridge. Renamed upstream from `claude-code-acp`, which is
/// deprecated — kept as the documented fallback because a machine that
/// hasn't updated still has only the old one on disk.
pub const CLAUDE_BRIDGE: &[&str] = &["npx", "-y", "@zed-industries/claude-agent-acp"];
pub const CLAUDE_BRIDGE_LEGACY: &[&str] = &["npx", "-y", "@zed-industries/claude-code-acp"];

fn find_binary(name: &str) -> Option<PathBuf> {
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let candidates = [
            home.join(".local/bin").join(name),
            home.join(".cargo/bin").join(name),
            home.join(".npm-global/bin").join(name),
            PathBuf::from("/opt/homebrew/bin").join(name),
            PathBuf::from("/usr/local/bin").join(name),
        ];
        for c in candidates {
            if c.is_file() {
                return Some(c);
            }
        }
    }
    None
}

/// The ACP command for a given agent name. Every one of these speaks the
/// same protocol (`Agent` + this module handle it); they differ only in
/// which process to spawn. An unknown name falls back to the Codex bridge.
///
/// Each can be overridden by environment variable — `CHORUS_CLAUDE_BRIDGE`,
/// `CHORUS_CODEX_BRIDGE`, `CHORUS_GEMINI_BRIDGE` — so a package rename (which
/// has already happened once for Claude) or a locally installed binary needs
/// no rebuild. The value is a whitespace-separated command line, e.g.
/// `CHORUS_CLAUDE_BRIDGE="npx -y @zed-industries/claude-code-acp"`.
pub fn bridge_for(agent_name: &str) -> Vec<String> {
    let n = agent_name.to_lowercase();
    let (env_key, cmd): (&str, &[&str]) = if n.contains("claude") {
        ("CHORUS_CLAUDE_BRIDGE", CLAUDE_BRIDGE)
    } else if n.contains("antigravity") || n.contains("agy") {
        ("CHORUS_ANTIGRAVITY_BRIDGE", &["antigravity-acp"])
    } else if n.contains("gemini") {
        ("CHORUS_GEMINI_BRIDGE", &["gemini", "--experimental-acp"])
    } else {
        ("CHORUS_CODEX_BRIDGE", DEFAULT_BRIDGE)
    };
    if let Ok(v) = std::env::var(env_key) {
        let parts: Vec<String> = v.split_whitespace().map(|s| s.to_string()).collect();
        if !parts.is_empty() {
            return parts;
        }
    }
    if n.contains("claude") {
        for candidate in ["claude-agent-acp", "claude-code-acp"] {
            if let Some(p) = find_binary(candidate) {
                return vec![p.to_string_lossy().to_string()];
            }
        }
    } else if n.contains("antigravity") || n.contains("agy") {
        if let Some(p) = find_binary("antigravity-acp") {
            return vec![p.to_string_lossy().to_string()];
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let candidate = dir.join("antigravity-acp");
                if candidate.exists() {
                    return vec![candidate.to_string_lossy().to_string()];
                }
            }
        }
        for rel in [
            "target/debug/antigravity-acp",
            "target/release/antigravity-acp",
            "../../target/debug/antigravity-acp",
            "../../target/release/antigravity-acp",
        ] {
            let target_candidate = std::path::Path::new(rel);
            if target_candidate.is_file() {
                if let Ok(p) = target_candidate.canonicalize() {
                    return vec![p.to_string_lossy().to_string()];
                }
            }
        }
    } else if n.contains("codex") {
        if let Some(p) = find_binary("codex-acp") {
            return vec![p.to_string_lossy().to_string()];
        }
    }
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
    /// A child session announced by the bridge, including nested children.
    SubagentSpawned {
        session_id: String,
        parent_session_id: String,
        name: String,
        task: String,
    },
    /// The child session's final state and any message it sent while running.
    SubagentState {
        session_id: String,
        state: String,
        message: String,
    },
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
    /// The model selected and reported for this ACP session, when available.
    pub model: Option<String>,
    pub context_used: Option<i64>,
    pub context_window: Option<i64>,
    /// Dollar cost the bridge reported for this turn (`usage_update.cost`,
    /// USD only). Codex sends none; claude-agent-acp sends the SDK's total.
    pub cost_usd: Option<f64>,
    pub stop_reason: String,
}

/// How to start the bridge.
pub struct CodexConfig {
    pub command: Vec<String>,
    /// Written into `session/new`; empty is valid.
    pub mcp_servers: Value,
    /// The mode id to pin. `read-only` routes approvals to us.
    pub mode: Option<String>,
    /// The model id to select, as the agent itself reports it (see
    /// [`ModelInfo`]). `None` leaves the agent on its own default.
    pub model: Option<String>,
    /// Transcript sink. `None` discards every frame.
    pub log: Option<Log>,
    /// Extra environment for the bridge process (account selection).
    pub env: Vec<(String, String)>,
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            command: DEFAULT_BRIDGE.iter().map(|s| s.to_string()).collect(),
            mcp_servers: json!([]),
            mode: Some("read-only".into()),
            model: None,
            log: None,
            env: Vec::new(),
        }
    }
}

/// A model the agent says it can run, as reported by `session/new`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub is_current: bool,
}

/// Pull `models.availableModels` out of a `session/new` / `session/load` reply.
///
/// The old comment in `agents.rs` said ACP has no way to ask an agent what
/// models it offers. That was true of the *methods* and false of the replies:
/// codex-acp answers `session/new` with the whole list plus `currentModelId`,
/// and a probe against the real bridge showed the hardcoded guesses
/// (`gpt-5.1-codex`) matching nothing it actually runs. Ask, do not guess.
pub fn models_from(res: &Value) -> Vec<ModelInfo> {
    let Some(m) = res.get("models") else {
        return Vec::new();
    };
    let current = m.get("currentModelId").and_then(Value::as_str).unwrap_or("");
    m.get("availableModels")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| {
                    let id = v.get("modelId").or_else(|| v.get("id"))?.as_str()?;
                    let name = v.get("name").and_then(Value::as_str).unwrap_or(id);
                    Some(ModelInfo {
                        id: id.to_string(),
                        name: name.to_string(),
                        is_current: id == current,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What actually happened when we tried to pin the session's mode.
///
/// This is a *report*, not a courtesy: `--read-only` is a safety promise, and
/// the only honest way to keep it is to know whether the agent accepted the
/// restriction. Codex answers the legacy `session/set_mode` with `{}` and
/// leaves the mode untouched, so "the call returned Ok" proves nothing —
/// `effective` is filled in only from a mode the agent itself reported back.
#[derive(Debug, Clone, Default)]
pub struct ModeOutcome {
    pub requested: String,
    /// The mode the agent says is now current. `None` = it would not say.
    pub effective: Option<String>,
    /// Which mechanism answered: "config_option", "set_mode", or "none".
    pub via: &'static str,
}

impl ModeOutcome {
    /// Did the agent confirm the mode we asked for? `false` also covers
    /// "it never told us", which for a safety flag must count as a failure.
    pub fn confirmed(&self) -> bool {
        self.effective.as_deref() == Some(self.requested.as_str())
    }
}

/// A live Codex session over ACP.
pub struct CodexAgent {
    agent: Agent,
    session_id: String,
    cwd: PathBuf,
    mode: ModeOutcome,
    models: Vec<ModelInfo>,
    model: Option<String>,
    image_prompt: bool,
    embedded_context_prompt: bool,
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
        this.models = models_from(&res);
        this.model = current_config(&res, "model");
        this.mode = this.pin_mode(cfg.mode.as_deref());
        this.model = this.pin_model(cfg.model.as_deref());
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
        this.mode = this.pin_mode(cfg.mode.as_deref());
        this.model = this.pin_model(cfg.model.as_deref());
        Ok(this)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// What the agent did with our mode request. Callers that promised the
    /// user a restriction must check this before running a turn.
    pub fn mode(&self) -> &ModeOutcome {
        &self.mode
    }

    /// The models this agent reported when the session opened. Empty when the
    /// bridge does not report any — callers must treat that as "unknown",
    /// never as "none".
    pub fn models(&self) -> &[ModelInfo] {
        &self.models
    }

    /// Capabilities advertised by the active bridge during `initialize`.
    pub fn supports_image_prompt(&self) -> bool {
        self.image_prompt
    }

    pub fn supports_embedded_context(&self) -> bool {
        self.embedded_context_prompt
    }

    /// The model the agent says this session is running, after any selection.
    /// `None` when it would not say.
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// Ask the agent to switch model, and report what it says is current.
    ///
    /// Same mechanism as the mode, and the same reason for checking the reply:
    /// a picker that silently does nothing is worse than no picker. Unlike the
    /// mode, this one has no fallback path — `session/set_mode` is about modes
    /// only — so an agent without config options keeps its own default and
    /// says so by returning `None`.
    fn pin_model(&mut self, model: Option<&str>) -> Option<String> {
        let want = model?;

        // Two namespaces, and picking the wrong one gets "Invalid params".
        //
        // `session/set_model` takes the ids from `models.availableModels` —
        // for Codex those carry a reasoning level, `gpt-5.5[low]`. The `model`
        // *config option* takes bare ids, `gpt-5.5`, with the level as a
        // separate `reasoning_effort` option. We show the user the
        // availableModels ids, because those are the ones that say what will
        // actually run, so try that method first and fall back to the config
        // option with the bracket stripped. (Measured against codex-acp on
        // 12 Sep 2026: set_config_option with a bracketed id is rejected.)
        if let Ok(id) = self.agent.request(
            "session/set_model",
            json!({ "sessionId": self.session_id, "modelId": want }),
        ) {
            if self.pump(id, Duration::from_secs(20)).is_ok() {
                return Some(want.to_string());
            }
        }

        let bare = want.split('[').next().unwrap_or(want);
        let id = self
            .agent
            .request(
                "session/set_config_option",
                json!({ "sessionId": self.session_id, "configId": "model", "value": bare }),
            )
            .ok()?;
        let res = self.pump(id, Duration::from_secs(20)).ok()?;
        current_config(&res, "model")
    }

    /// Run one prompt turn. `decide` is called for every
    /// `session/request_permission`; returning [`Permission::Deny`] cancels
    /// that action (and usually the turn).
    pub fn prompt(
        &mut self,
        text: &str,
        decide: &mut dyn FnMut(&PermissionAsk) -> Permission,
    ) -> Result<TurnOutcome> {
        self.prompt_blocks(vec![json!({ "type": "text", "text": text })], decide)
    }

    /// Send ordered ACP content blocks after checking the bridge's live prompt capabilities.
    pub fn prompt_blocks(
        &mut self,
        prompt: Vec<Value>,
        decide: &mut dyn FnMut(&PermissionAsk) -> Permission,
    ) -> Result<TurnOutcome> {
        for block in &prompt {
            match block.get("type").and_then(Value::as_str) {
                Some("text" | "resource_link") => {}
                Some("image") if self.image_prompt => {}
                Some("resource") if self.embedded_context_prompt => {}
                Some(kind) => return Err(anyhow!("bridge does not accept {kind} prompt content")),
                None => return Err(anyhow!("prompt content is missing its type")),
            }
        }
        let mut acc = Accumulator {
            root_session_id: self.session_id.clone(),
            ..Default::default()
        };
        let cwd = self.cwd.clone();

        let id = self.agent.request(
            "session/prompt",
            json!({
                "sessionId": self.session_id,
                "prompt": prompt,
            }),
        )?;

        let mut handle = |a: &Agent, inbound: Inbound| match inbound {
            Inbound::Notification { method, params } => {
                if method == "session/update" {
                    report_progress(&params);
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
        let mut outcome = acc.finish();
        outcome.model = self.model.clone();
        Ok(outcome)
    }

    // ---- internals ----------------------------------------------------

    fn spawn_and_init(cwd: &Path, cfg: &CodexConfig) -> Result<Self> {
        let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        let log: Log = cfg
            .log
            .clone()
            .unwrap_or_else(|| Arc::new(Mutex::new(Box::new(sink()))));
        let agent = Agent::spawn_env(&cfg.command, &cfg.env, log)
            .with_context(|| format!("spawn codex bridge: {}", cfg.command.join(" ")))?;
        let mut this = Self {
            agent,
            session_id: String::new(),
            cwd,
            mode: ModeOutcome::default(),
            models: Vec::new(),
            model: None,
            image_prompt: false,
            embedded_context_prompt: false,
        };

        let init = this.agent.request(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": { "readTextFile": true, "writeTextFile": true },
                    "terminal": false,
                    "subagents": {}
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
        let prompt_caps = &res["agentCapabilities"]["promptCapabilities"];
        this.image_prompt = prompt_caps["image"].as_bool().unwrap_or(false);
        this.embedded_context_prompt = prompt_caps["embeddedContext"].as_bool().unwrap_or(false);
        Ok(this)
    }

    /// Pin the session's mode, and find out whether it took.
    ///
    /// Two mechanisms, in the order ACP is moving: `session/set_config_option`
    /// first, because the spec requires the agent to answer it with the
    /// complete option list — that reply is the only confirmation the protocol
    /// offers, and without a confirmation a safety flag is decoration. The
    /// legacy `session/set_mode` is the fallback for agents that predate config
    /// options; it answers `{}`, so a success there leaves `effective` unset
    /// and [`ModeOutcome::confirmed`] false, which is the truthful answer.
    ///
    /// Measured against @agentclientprotocol/codex-acp on 12 Sep 2026:
    /// `set_mode` returns `{}` and the mode stays `agent` — the agent then
    /// writes files with no permission request at all. That is the whole
    /// reason this returns a report instead of nothing.
    fn pin_mode(&mut self, mode: Option<&str>) -> ModeOutcome {
        let Some(mode) = mode else {
            return ModeOutcome { requested: String::new(), effective: None, via: "none" };
        };
        let requested = mode.to_string();

        if let Ok(id) = self.agent.request(
            "session/set_config_option",
            json!({ "sessionId": self.session_id, "configId": "mode", "value": mode }),
        ) {
            if let Ok(res) = self.pump(id, Duration::from_secs(15)) {
                if let Some(v) = current_config(&res, "mode") {
                    return ModeOutcome { requested, effective: Some(v), via: "config_option" };
                }
            }
        }

        if let Ok(id) = self.agent.request(
            "session/set_mode",
            json!({ "sessionId": self.session_id, "modeId": mode }),
        ) {
            if self.pump(id, Duration::from_secs(15)).is_ok() {
                return ModeOutcome { requested, effective: None, via: "set_mode" };
            }
        }
        ModeOutcome { requested, effective: None, via: "none" }
    }

    /// Wait for the response to `id`, discarding any `session/update` frames
    /// that arrive alongside it (used for setup calls, not prompt turns).
    fn pump(&self, id: i64, timeout: Duration) -> Result<Value> {
        let mut ignore = |_: &Agent, _: Inbound| {};
        self.agent.wait_for(id, timeout, &mut ignore)
    }
}

/// Dig the current value of the `mode` config option out of an agent reply.
/// Accepts both shapes the bridges use: the flat `configOptions` list from
/// `set_config_option`, and the `modes.currentModeId` that `session/new` and
/// `session/load` return.
fn current_config(res: &Value, id: &str) -> Option<String> {
    if let Some(opts) = res.get("configOptions").and_then(Value::as_array) {
        for o in opts {
            if o.get("id").and_then(Value::as_str) == Some(id) {
                if let Some(v) = o.get("currentValue").and_then(Value::as_str) {
                    return Some(v.to_string());
                }
            }
        }
    }
    match id {
        "mode" => res
            .get("modes")
            .and_then(|m| m.get("currentModeId"))
            .and_then(Value::as_str)
            .map(str::to_string),
        "model" => res
            .get("models")
            .and_then(|m| m.get("currentModelId"))
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
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

/// Forward one `session/update` to the live progress sink, if any.
fn report_progress(params: &Value) {
    let Some(update) = params.get("update") else { return };
    let kind = update.get("sessionUpdate").and_then(Value::as_str).unwrap_or("");
    match kind {
        "agent_thought_chunk" => {
            if let Some(t) = chunk_text(update) {
                crate::emit_progress("thought", &t);
            }
        }
        "agent_message_chunk" => {
            if let Some(t) = chunk_text(update) {
                crate::emit_progress("message", &t);
            }
        }
        "tool_call" => {
            let title = update.get("title").and_then(Value::as_str).unwrap_or("tool");
            crate::emit_progress("tool", title);
        }
        "tool_call_update" => {
            if let Some(st) = update.get("status").and_then(Value::as_str) {
                if st == "completed" || st == "failed" {
                    let title = update.get("title").and_then(Value::as_str).unwrap_or("");
                    crate::emit_progress("tool_done", &format!("{st}\t{title}"));
                }
            }
        }
        _ => {}
    }
}

/// Folds a turn's `session/update` stream into a [`TurnOutcome`].
#[derive(Default)]
struct Accumulator {
    root_session_id: String,
    message: String,
    thoughts: String,
    events: Vec<AgentEvent>,
    child_messages: std::collections::HashMap<String, String>,
    /// `toolCallId` → title, so a `tool_call_update` (which carries only the
    /// id) can name the call it resolves.
    titles: std::collections::HashMap<String, String>,
    context_used: Option<i64>,
    context_window: Option<i64>,
    cost_usd: Option<f64>,
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
        let session_id = params.get("sessionId").and_then(Value::as_str).unwrap_or("");
        match kind {
            "subagent_spawned" => {
                if let Some(child) = update.get("subagentSessionId").and_then(Value::as_str) {
                    self.events.push(AgentEvent::SubagentSpawned {
                        session_id: child.to_string(),
                        parent_session_id: session_id.to_string(),
                        name: update.get("name").and_then(Value::as_str).unwrap_or("Agent").to_string(),
                        task: update.get("task").and_then(Value::as_str).unwrap_or("").to_string(),
                    });
                }
                return;
            }
            "subagent_state_update" => {
                if let (Some(child), Some(state)) = (
                    update.get("subagentSessionId").and_then(Value::as_str),
                    update.get("state").and_then(Value::as_str),
                ) {
                    self.events.push(AgentEvent::SubagentState {
                        session_id: child.to_string(),
                        state: state.to_string(),
                        message: self.child_messages.remove(child).unwrap_or_default(),
                    });
                }
                return;
            }
            _ => {}
        }
        // Child updates share the transport with the parent. Their prose and
        // tools must not become the parent's answer or tool history.
        if !session_id.is_empty() && session_id != self.root_session_id {
            if kind == "agent_message_chunk" {
                if let Some(text) = chunk_text(update) {
                    self.child_messages.entry(session_id.to_string()).or_default().push_str(&text);
                }
            }
            return;
        }
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
                let id = update.get("toolCallId").and_then(Value::as_str).unwrap_or("");
                if !id.is_empty() {
                    self.titles.insert(id.to_string(), title.clone());
                }
                // Subagent Normalizer: Normalize Claude Task tool call into SubagentSpawned
                if title == "Task" || kind == "task" {
                    let task_desc = update.get("rawInput")
                        .and_then(|v| v.get("description").or_else(|| v.get("prompt")))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let child_id = format!("task-{}", if id.is_empty() { "subagent" } else { id });
                    self.events.push(AgentEvent::SubagentSpawned {
                        session_id: child_id,
                        parent_session_id: self.root_session_id.clone(),
                        name: "Claude Task Subagent".to_string(),
                        task: task_desc.to_string(),
                    });
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
                let cost = update.get("cost");
                if cost.and_then(|c| c.get("currency")).and_then(Value::as_str) == Some("USD") {
                    if let Some(amount) = cost.and_then(|c| c.get("amount")).and_then(Value::as_f64) {
                        self.cost_usd = Some(amount);
                    }
                }
                // Live context meter: `used\tsize`, either side may be empty.
                if self.context_used.is_some() || self.context_window.is_some() {
                    let fmt = |v: Option<i64>| v.map(|n| n.to_string()).unwrap_or_default();
                    crate::emit_progress(
                        "usage",
                        &format!("{}\t{}", fmt(self.context_used), fmt(self.context_window)),
                    );
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
            model: None,
            context_used: self.context_used,
            context_window: self.context_window,
            cost_usd: self.cost_usd,
            stop_reason: self.stop_reason,
        }
    }
}

fn chunk_text(update: &Value) -> Option<String> {
    let content = update.get("content")?;
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    if let Some(t) = content.get("text").and_then(Value::as_str) {
        return Some(t.to_string());
    }
    // Some builds send content as an array of typed parts or strings.
    if let Some(arr) = content.as_array() {
        let joined: String = arr
            .iter()
            .filter_map(|p| {
                if let Some(s) = p.as_str() {
                    Some(s)
                } else {
                    p.get("text").and_then(Value::as_str)
                }
            })
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

#[cfg(test)]
mod usage_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn usage_update_keeps_context_and_usd_cost() {
        let mut acc = Accumulator::default();
        // Shape claude-agent-acp sends; Codex omits `cost`.
        acc.absorb(&json!({"update": {"sessionUpdate": "usage_update", "used": 1200, "size": 200000,
                                      "cost": {"amount": 0.042, "currency": "USD"}}}));
        acc.absorb(&json!({"update": {"sessionUpdate": "usage_update", "used": 1500, "size": 200000}}));
        let out = acc.finish();
        assert_eq!(out.context_used, Some(1500));
        assert_eq!(out.context_window, Some(200000));
        assert_eq!(out.cost_usd, Some(0.042));
    }

    #[test]
    fn non_usd_cost_is_ignored() {
        let mut acc = Accumulator::default();
        acc.absorb(&json!({"update": {"sessionUpdate": "usage_update", "used": 10,
                                      "cost": {"amount": 5.0, "currency": "EUR"}}}));
        assert_eq!(acc.finish().cost_usd, None);
    }
}
