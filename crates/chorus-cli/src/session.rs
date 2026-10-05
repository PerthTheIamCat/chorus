//! The Session Engine spine: one app-owned session, one ledger, agents
//! called into it one turn at a time.
//!
//! This is the smallest real version of the orchestrator from the design
//! doc — enough to prove the invariant that matters: **the session is the
//! app's, not the provider's.** Every turn, whoever answers, lands in the
//! same ledger with an `actor`. Kill the agent mid-turn and the ledger is
//! still whole.

use anyhow::{Context, Result};
use chorus_acp::codex::{AgentEvent, CodexAgent, CodexConfig, TurnOutcome};
pub use chorus_acp::codex::{Permission, PermissionAsk};
use chorus_core::checkpoint::Checkpoint;
use chorus_core::attachments::{self, Attachment};
use chorus_core::compaction;
use chorus_core::context::{AssembleInput, Assembler, Budget, TurnItem};
use chorus_core::handoff::{Brief, BriefInput, HandoffMeta, HandoffReason};
use chorus_core::mcp::McpServer;
use chorus_core::projector::{self, AgentFile, Drift, Projection};
use chorus_acp::sandbox;
use chorus_core::mcp_config;
use chorus_core::integration_catalog;
use chorus_core::skills;
use chorus_core::render::ContextItem;
use chorus_core::tokens::{CachedCounter, HeuristicCounter};
use chorus_core::{actor, ledger, memory, Event, EventKind, NewEvent, Store};
use std::path::{Path, PathBuf};

/// How to answer the agent's permission requests for a whole run. The
/// three levels mirror the design's per-agent permission (§7): deny
/// everything the sandbox escalates, auto-allow only reads, or allow all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Deny everything the sandbox escalates (the safe default).
    DenyAll,
    /// Auto-allow read-only tool calls; deny writes, commands, network.
    ReadOnly,
    /// Allow everything — only for throwaway directories.
    AllowAll,
}

impl Policy {
    /// Map a level int (0 deny · 1 read-only · 2 allow) from the FFI/UI.
    pub fn from_level(level: i32) -> Self {
        match level {
            1 => Policy::ReadOnly,
            n if n >= 2 => Policy::AllowAll,
            _ => Policy::DenyAll,
        }
    }

    /// Does this policy promise the user that nothing gets written?
    pub fn restrictive(self) -> bool {
        matches!(self, Policy::DenyAll | Policy::ReadOnly)
    }

    /// The ACP session mode this policy needs the agent to be in.
    ///
    /// Answering `session/request_permission` is not enforcement: an agent
    /// that never asks is never stopped, and Codex does not ask before running
    /// shell commands. The restriction has to live in the agent's own sandbox,
    /// and the mode is how ACP asks for it. `read-only` is Codex's
    /// "ask for approval" preset — it routes actions back to us, which is what
    /// both `DenyAll` and `ReadOnly` need; `agent` is "approve for me".
    /// `agent-full-access` is deliberately never requested: it grants reach to
    /// any file on the machine, which nothing in this CLI promises.
    pub fn mode_id(self) -> &'static str {
        match self {
            Policy::DenyAll | Policy::ReadOnly => "read-only",
            Policy::AllowAll => "agent",
        }
    }

    fn decide(self, ask: &PermissionAsk) -> Permission {
        let allow = match self {
            Policy::DenyAll => false,
            Policy::AllowAll => true,
            Policy::ReadOnly => {
                let k = ask.kind.to_lowercase();
                k == "read" || k.contains("read") || k == "fetch" || k == "search"
            }
        };
        if allow {
            Permission::Allow
        } else {
            Permission::Deny
        }
    }
}

/// What one turn wrote to the ledger.
pub struct TurnReport {
    pub user_seq: i64,
    pub agent_seqs: Vec<i64>,
    pub message: String,
    pub context_used: Option<i64>,
    pub context_window: Option<i64>,
    pub stop_reason: String,
}

/// One link in a relay chain.
#[derive(Debug, Clone)]
pub struct AgentSpec {
    pub name: String,
    pub provider: String,
    /// ACP command; empty = use the session's default bridge.
    pub bridge: Vec<String>,
}

impl AgentSpec {
    pub fn new(name: impl Into<String>, provider: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            provider: provider.into(),
            bridge: Vec::new(),
        }
    }
}

/// `(approved project-scope facts, pending proposals as (id, scope, text))`.
pub type MemoryState = (Vec<String>, Vec<(i64, i64, String)>);

/// One branch of a compare run: the forked session id and what the agent
/// produced there (`Ok((message, stop_reason))` or the error string).
pub struct CompareBranch {
    pub agent: String,
    pub session_id: i64,
    pub result: std::result::Result<(String, String), anyhow::Error>,
}

/// The result of a relay run: who ended up answering, their turn, and every
/// automatic handoff it took to get there.
pub struct RelayReport {
    pub answered_by: String,
    pub turn: TurnReport,
    pub handoffs: Vec<(String, String, HandoffReason)>,
}

/// The outcome of a `handoff`: the brief that was handed over and the
/// incoming agent's first response (its Verify echo-back).
pub struct HandoffResult {
    pub handoff_seq: i64,
    pub from: String,
    pub to: String,
    pub brief: Brief,
    pub echo_back: String,
    pub agent_seqs: Vec<i64>,
    pub stop_reason: String,
}
/// The never-trimmed tier of the context window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskState {
    pub goal: String,
    pub done: Vec<String>,
    pub pending: Vec<String>,
    pub constraints: Vec<String>,
}

impl TaskState {
    /// Compact plain text — the model reads this, so it carries no ids, no
    /// timestamps and no field it would not act on.
    pub fn render(&self) -> String {
        let mut s = String::new();
        if !self.goal.trim().is_empty() {
            s.push_str("goal: ");
            s.push_str(self.goal.trim());
            s.push('\n');
        }
        for (label, items) in [
            ("done", &self.done),
            ("pending", &self.pending),
            ("constraints", &self.constraints),
        ] {
            if items.is_empty() {
                continue;
            }
            s.push_str(label);
            s.push_str(":\n");
            for i in items {
                s.push_str("  - ");
                s.push_str(i.trim());
                s.push('\n');
            }
        }
        s
    }

    pub fn is_empty(&self) -> bool {
        self.goal.trim().is_empty() && self.done.is_empty() && self.pending.is_empty()
    }
}

/// What one compaction pass did.
#[derive(Debug, Clone)]
pub struct CompactReport {
    pub compacted: usize,
    pub remaining: usize,
    /// Which summariser produced these rows, e.g. `ollama:qwen2.5:3b`.
    pub model: String,
    pub took_ms: u64,
    /// True when the pass stopped on the clock rather than finishing its
    /// batch — the signal that the summariser is slow, not that anything failed.
    pub hit_budget: bool,
}

fn clip(text: &str, max_chars: usize) -> String {
    let line = text.trim();
    let first = line.lines().next().unwrap_or("").trim();
    if first.chars().count() <= max_chars {
        return first.to_string();
    }
    let mut out: String = first.chars().take(max_chars).collect();
    out.push('\u{2026}');
    out
}

fn str_list(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Is this tool-result body nothing but "<title> → completed"?
///
/// Deliberately narrow: only the statuses that mean "it worked and there is
/// nothing to report". Anything else — failed, rejected, a status we have not
/// seen — reaches the next agent unchanged, because the cost of dropping a
/// line that mattered is much higher than the tokens it saves.
fn is_bare_success(body: &str) -> bool {
    let Some((_, status)) = body.rsplit_once(" → ") else {
        return false;
    };
    matches!(status.trim(), "completed" | "success" | "ok")
}

/// `CHORUS_ACP_LOG=<path>` writes every JSON-RPC frame, both directions, to a
/// file.
///
/// Off by default and not a flag: it records whatever the config puts in
/// `session/new`, which includes MCP `env` values — that is to say, API keys.
/// It is the only way to answer "did the agent actually receive that server",
/// so it exists; making it an env var the user has to type is the point.
fn acp_log() -> Option<chorus_acp::Log> {
    let path = std::env::var("CHORUS_ACP_LOG").ok()?;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()?;
    Some(std::sync::Arc::new(std::sync::Mutex::new(
        Box::new(f) as Box<dyn std::io::Write + Send>
    )))
}

/// Note, without stopping, that the agent would not confirm the restricted
/// mode we asked for.
///
/// This used to refuse the turn, on the principle that a safety flag we cannot
/// honour should fail closed. Running it against the real bridges showed that
/// was the wrong lever: `claude-agent-acp` answers neither
/// `session/set_config_option` nor `session/set_mode`, so the rule blocked
/// every Claude turn in the app — while Codex, which *does* answer and reports
/// `read-only` back, writes files anyway. Confirmation turned out to predict
/// nothing at all.
///
/// What actually keeps the promise is the snapshot-and-revert guard around the
/// turn, which needs no cooperation from the agent. So the mode request stays
/// (free, and correct the day a bridge honours it), the outcome is reported,
/// and the guard does the work.
fn note_mode(agent_name: &str, outcome: &chorus_acp::codex::ModeOutcome, log: bool) {
    if !log || outcome.requested.is_empty() || outcome.confirmed() {
        return;
    }
    eprintln!(
        "  ⚠  {agent_name}: ไม่ยืนยันโหมด {} (ช่องทาง {}) — Chorus จะเช็คไฟล์หลังจบเทิร์นแทน",
        outcome.requested, outcome.via
    );
}

fn push_unique(v: &mut Vec<String>, item: &str) {
    let item = item.trim();
    if item.is_empty() || v.iter().any(|x| x == item) {
        return;
    }
    v.push(item.to_string());
}

pub struct Session {
    store: Store,
    sess: i64,
    cwd: PathBuf,
    user: i64,
    /// A forced ACP command for every agent, set by `with_bridge` (tests
    /// point it at the in-repo mock). `None` means resolve per agent name
    /// via `chorus_acp::codex::bridge_for`.
    forced_bridge: Option<Vec<String>>,
    /// Echo permission decisions to stderr (CLI only — see `log_permissions`).
    log_permissions: bool,
    /// Shadow git repo for the workspace. `None` when `git` is unavailable
    /// or the workspace can't host one — the session still runs, handoffs
    /// just carry no auto-diff.
    checkpoint: Option<Checkpoint>,
    /// agent name → the model id the user picked for it, if any.
    models: std::collections::HashMap<String, String>,
    /// Sha the handoff brief's diff is measured from — the shadow repo's
    /// HEAD when the session opened. Immutable for the life of the `Session`.
    base_ckpt: String,
    /// The in-process MCP server handed to every agent (exposes
    /// `memory.propose`). `None` when it couldn't bind a port.
    mcp: Option<McpServer>,
    /// Where facts and proposals live when this session belongs to a
    /// project: one store shared by every folder in it. `None` = the
    /// session's own store, as before projects existed.
    memory: Option<Store>,
}

/// Open the shadow repo and pick the base sha a handoff diff is measured
/// from. On a brand-new repo we snapshot the current tree; on an existing
/// one we take `HEAD` and leave the tree alone, so the diff covers whatever
/// agents have changed since — not since this process started.
fn open_checkpoint(cwd: &Path) -> (Option<Checkpoint>, String) {
    match Checkpoint::open_reporting(cwd) {
        Ok((ck, fresh)) => {
            let base = if fresh {
                ck.snapshot("session base").unwrap_or_default()
            } else {
                ck.head().unwrap_or_default()
            };
            (Some(ck), base)
        }
        Err(_) => (None, String::new()),
    }
}

impl Session {
    /// Open (or create) the store and start a new session in it.
    pub fn create(db: &Path, cwd: &Path, title: &str) -> Result<Self> {
        let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        let store = Store::open(db).context("open store")?;
        let user = actor::intern(store.conn(), "user", "human")?;
        let sess = ledger::new_session(store.conn(), title, &cwd.to_string_lossy())?;
        let (checkpoint, base_ckpt) = open_checkpoint(&cwd);
        let mcp = McpServer::start_for_session(&db.to_string_lossy(), sess).ok();
        Ok(Self {
            store,
            sess,
            cwd,
            user,
            forced_bridge: None,
            log_permissions: false,
            checkpoint,
            models: std::collections::HashMap::new(),
            base_ckpt,
            mcp,
            memory: None,
        })
    }

    /// Reattach to an existing session in an existing store.
    pub fn open(db: &Path, session_id: i64) -> Result<Self> {
        let store = Store::open(db).context("open store")?;
        let (cwd, exists): (String, bool) = store
            .conn()
            .query_row(
                "SELECT cwd, 1 FROM session WHERE id = ?1",
                [session_id],
                |r| Ok((r.get(0)?, true)),
            )
            .unwrap_or_default();
        anyhow::ensure!(exists, "no session {session_id} in {}", db.display());
        let user = actor::intern(store.conn(), "user", "human")?;
        let cwd = PathBuf::from(cwd);
        let (checkpoint, base_ckpt) = open_checkpoint(&cwd);
        let mcp = McpServer::start_for_session(&db.to_string_lossy(), session_id).ok();
        Ok(Self {
            store,
            sess: session_id,
            cwd,
            user,
            forced_bridge: None,
            log_permissions: false,
            checkpoint,
            models: std::collections::HashMap::new(),
            base_ckpt,
            mcp,
            memory: None,
        })
    }

    pub fn import_attachment(
        &self,
        name: &str,
        media_type: Option<&str>,
        bytes: &[u8],
        derived_text: Option<&str>,
        normalized_image: Option<(&str, &[u8])>,
    ) -> Result<Attachment> {
        Ok(attachments::import_staged_prepared(
            self.store.conn(),
            self.sess,
            name,
            media_type,
            bytes,
            derived_text,
            normalized_image,
        )?)
    }

    pub fn list_attachments(&self) -> Result<Vec<Attachment>> {
        Ok(attachments::list(self.store.conn(), self.sess)?)
    }

    pub fn delete_attachment(&self, id: i64) -> Result<()> {
        Ok(attachments::delete(self.store.conn(), self.sess, id)?)
    }

    /// Force one ACP command for every agent (tests, or a pinned install).
    pub fn with_bridge(mut self, cmd: Vec<String>) -> Self {
        self.forced_bridge = Some(cmd);
        self
    }

    /// The ACP command to spawn for `agent_name`: an explicit `spec` bridge
    /// wins, then a session-wide `forced_bridge`, else the per-name default.
    fn resolve_bridge(&self, agent_name: &str, spec: &[String]) -> Vec<String> {
        if !spec.is_empty() {
            spec.to_vec()
        } else if let Some(f) = &self.forced_bridge {
            f.clone()
        } else {
            chorus_acp::codex::bridge_for(agent_name)
        }
    }

    pub fn session_id(&self) -> i64 {
        self.sess
    }

    /// The workspace diff since the session's base checkpoint — what the
    /// agents have changed. Empty when there's no shadow repo or no change.
    pub fn working_diff(&self) -> String {
        self.checkpoint
            .as_ref()
            .and_then(|ck| ck.diff_since_user(&self.base_ckpt).ok())
            .unwrap_or_default()
    }

    /// Undo anything the agent changed in the workspace during a restrictive
    /// turn, and say so. Called only when the policy promised read-only.
    ///
    /// This catches writes *inside* the workspace. It cannot catch writes
    /// outside it, or anything else a shell command did — an agent that
    /// ignores the mode can still reach the rest of the machine, and no part of
    /// Chorus can stop it today. That gap is the reason the design calls for a
    /// real Seatbelt profile; until then the flag's help says what it covers.
    fn revert_if_touched(&self, agent_name: &str, base: &str) -> Result<()> {
        let Some(ck) = self.checkpoint.as_ref() else {
            return Ok(());
        };
        let diff = ck.diff_since(base).unwrap_or_default();
        if diff.trim().is_empty() {
            return Ok(());
        }
        let files: Vec<String> = diff
            .lines()
            .filter_map(|l| l.strip_prefix("+++ b/"))
            .map(str::to_string)
            .collect();
        ck.restore(base)?;
        anyhow::bail!(
            "{agent_name} แก้ไฟล์ทั้งที่สั่ง --read-only — ย้อนกลับให้แล้ว ({} ไฟล์: {})
               bridge ของ agent ตัวนี้ไม่เคารพโหมด — ไฟล์นอก workspace เราย้อนกลับให้ไม่ได้",
            files.len(),
            files.join(", "),
        )
    }

    /// Take a manual checkpoint of the current tree. Returns the new sha.
    pub fn snapshot(&self, message: &str) -> Result<String> {
        let ck = self
            .checkpoint
            .as_ref()
            .context("no shadow repo for this workspace")?;
        Ok(ck.snapshot(message)?)
    }

    /// The connection facts and proposals are read from and written to.
    fn mem_store(&self) -> &Store {
        self.memory.as_ref().unwrap_or(&self.store)
    }

    /// Share memory with the rest of a project: facts and the approve queue
    /// move to `path` (created on first use), and the MCP `memory_propose`
    /// tool writes there too. This folder's existing facts are copied in once,
    /// so attaching a folder to a project never loses what it remembered.
    pub fn set_memory_db(&mut self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).context("create project memory dir")?;
        }
        let shared = Store::open(path).context("open project memory")?;
        for f in memory::top(self.store.conn(), 1, 1000)? {
            memory::add(shared.conn(), f.scope, &f.text)?;
        }
        self.mcp = McpServer::start(&path.to_string_lossy()).ok();
        self.memory = Some(shared);
        Ok(())
    }

    /// The memory inspector's view: rendered project-scope facts, and the
    /// pending proposal queue as `(id, scope, text)`.
    pub fn memory_state(&self) -> Result<MemoryState> {
        let c = self.mem_store().conn();
        let facts = memory::top(c, 1, 100)?
            .iter()
            .map(|f| f.text.clone())
            .collect();
        let pending = memory::pending(c)?
            .into_iter()
            .map(|p| (p.id, p.scope, p.text))
            .collect();
        Ok((facts, pending))
    }

    /// Approve or reject a queued proposal.
    pub fn review_proposal(&self, id: i64, approve: bool) -> Result<()> {
        let c = self.mem_store().conn();
        if approve {
            memory::approve(c, id)?;
        } else {
            memory::reject(c, id)?;
        }
        Ok(())
    }

    /// Reject a selection of hunks: reverse-apply `patch` (which contains
    /// only the hunks the user rejected) to the workspace.
    pub fn reject_hunks(&self, patch: &str) -> Result<()> {
        let ck = self
            .checkpoint
            .as_ref()
            .context("no shadow repo for this workspace")?;
        Ok(ck.apply_reverse(patch)?)
    }

    /// Roll the workspace back to `sha` (or the session base when empty).
    /// Destructive — the caller confirms with the user first.
    pub fn restore_to(&self, sha: &str) -> Result<()> {
        let ck = self
            .checkpoint
            .as_ref()
            .context("no shadow repo for this workspace")?;
        let target = if sha.trim().is_empty() {
            self.base_ckpt.as_str()
        } else {
            sha.trim()
        };
        anyhow::ensure!(!target.is_empty(), "no checkpoint to restore to");
        ck.restore(target)?;
        Ok(())
    }

    /// Run one user turn against `agent_name`, persisting the user message
    /// and every structured thing the agent does back into the ledger.
    ///
    /// When the session already has history, that history is assembled from
    /// the ledger (never from a provider's own session) and prepended to the
    /// prompt — only `user_text` is stored as the new event.
    pub fn run_turn(
        &mut self,
        agent_name: &str,
        provider: &str,
        user_text: &str,
        policy: Policy,
    ) -> Result<TurnReport> {
        self.run_turn_with_attachments(agent_name, provider, user_text, policy, &[])
    }

    /// Submit a user turn with staged session attachments, preserving the
    /// text-only entry point for existing callers.
    pub fn run_turn_with_attachments(
        &mut self,
        agent_name: &str,
        provider: &str,
        user_text: &str,
        policy: Policy,
        attachment_ids: &[i64],
    ) -> Result<TurnReport> {
        let agent_actor = actor::intern(self.store.conn(), agent_name, provider)?;
        let has_history = !ledger::history(self.store.conn(), self.sess, 1)?.is_empty();

        // Assemble history from the ledger *before* adding this turn, so the
        // new instruction isn't also fed back as the last thing said.
        let prompt = if has_history {
            let ctx = self.assemble_context(120_000)?;
            format!(
                "{ctx}\n\n---\nThat is the session so far, reconstructed from the shared \
                 ledger. Continue from here.\n\nNew instruction:\n{user_text}"
            )
        } else {
            user_text.to_string()
        };

        let event = NewEvent::new(self.sess, EventKind::Msg, self.user, user_text);
        let user_seq = if attachment_ids.is_empty() {
            ledger::append(self.store.conn(), &event)?
        } else {
            attachments::link_to_event(self.store.conn_mut(), self.sess, &event, attachment_ids)?
        };
        let bridge = self.resolve_bridge(agent_name, &[]);
        let outcome = self.one_turn(agent_name, &bridge, &prompt, policy, attachment_ids)?;
        let agent_seqs = self.persist_outcome(&outcome, agent_actor, user_seq)?;

        Ok(TurnReport {
            user_seq,
            agent_seqs,
            message: outcome.message,
            context_used: outcome.context_used,
            context_window: outcome.context_window,
            stop_reason: outcome.stop_reason,
        })
    }

    /// Hand the live session to another agent. Runs the five-step pipeline:
    /// Freeze (the caller has already stopped the outgoing agent), Repack /
    /// Brief (here), Verify (the incoming agent echoes back its
    /// understanding), Resume (it answers `first_task`, or just the brief).
    ///
    /// A `Handoff` event is written first, so the ledger records the switch
    /// even if the incoming agent never answers.
    #[allow(clippy::too_many_arguments)]
    pub fn handoff(
        &mut self,
        to: &str,
        to_provider: &str,
        reason: HandoffReason,
        goal: &str,
        done: Vec<String>,
        pending: Vec<String>,
        constraints: Vec<String>,
        diff: String,
        policy: Policy,
    ) -> Result<HandoffResult> {
        let from = self.last_agent().unwrap_or_else(|| "unknown".into());
        let events = self.transcript()?;
        let at = events.last().map(|(e, _)| e.t).unwrap_or(0);

        // Fill the diff from the shadow repo when the caller didn't supply one.
        let diff = if diff.trim().is_empty() {
            self.checkpoint
                .as_ref()
                .and_then(|ck| ck.diff_since_user(&self.base_ckpt).ok())
                .unwrap_or_default()
        } else {
            diff
        };

        let brief = Brief::build(BriefInput {
            goal: goal.to_string(),
            done,
            pending,
            constraints,
            events: &events,
            diff,
            handoff: HandoffMeta {
                from: from.clone(),
                to: to.to_string(),
                reason,
                at,
            },
        });

        let c = self.store.conn();
        let to_actor = actor::intern(c, to, to_provider)?;
        let handoff_seq = ledger::append(
            c,
            &NewEvent::new(self.sess, EventKind::Handoff, to_actor, brief.to_json()),
        )?;

        // Verify + Resume: the incoming agent's first turn is the brief.
        let bridge = self.resolve_bridge(to, &[]);
        let outcome = self.one_turn(to, &bridge, &brief.render_markdown(), policy, &[])?;
        let agent_seqs = self.persist_outcome(&outcome, to_actor, handoff_seq)?;

        Ok(HandoffResult {
            handoff_seq,
            from,
            to: to.to_string(),
            brief,
            echo_back: outcome.message,
            agent_seqs,
            stop_reason: outcome.stop_reason,
        })
    }

    /// Compare mode (design §7): fork the session once per agent, run the
    /// same prompt on each fork, and hand back what each produced so the UI
    /// can diff them. The forks are real ledger branches (one row each, no
    /// history copy) — the user can later resume whichever they prefer.
    pub fn compare(
        &mut self,
        agents: &[AgentSpec],
        prompt: &str,
        policy: Policy,
    ) -> Result<Vec<CompareBranch>> {
        let db_path: String = self
            .store
            .conn()
            .query_row("PRAGMA database_list", [], |r| r.get::<_, String>(2))
            .unwrap_or_default();
        anyhow::ensure!(!db_path.is_empty(), "compare needs a file-backed store");

        let head = ledger::history(self.store.conn(), self.sess, 1_000_000)
            .ok()
            .and_then(|h| h.last().map(|e| e.seq))
            .unwrap_or(0);

        let mut out = Vec::new();
        for spec in agents {
            let fork_id = ledger::fork(
                self.store.conn(),
                self.sess,
                head,
                &format!("compare: {}", spec.name),
            )?;
            let mut branch = Session::open(Path::new(&db_path), fork_id)?;
            branch.forced_bridge = Some(self.resolve_bridge(&spec.name, &spec.bridge));
            let turn = branch.run_turn(&spec.name, &spec.provider, prompt, policy);
            out.push(CompareBranch {
                agent: spec.name.clone(),
                session_id: fork_id,
                result: turn.map(|t| (t.message, t.stop_reason)),
            });
        }
        Ok(out)
    }

    /// Run a turn against a relay chain: try the first agent; on a transient,
    /// agent-specific failure (rate limit, quota, context window) hand off to
    /// the next and let it carry on. A real error is not routed around — it
    /// propagates. `handoffs` records every switch it took.
    pub fn run_relay(
        &mut self,
        chain: &[AgentSpec],
        user_text: &str,
        policy: Policy,
    ) -> Result<RelayReport> {
        self.run_relay_with_attachments(chain, user_text, policy, &[])
    }

    pub fn run_relay_with_attachments(
        &mut self,
        chain: &[AgentSpec],
        user_text: &str,
        policy: Policy,
        attachment_ids: &[i64],
    ) -> Result<RelayReport> {
        let first = chain.first().context("relay chain is empty")?;
        chorus_acp::reset_cancel();
        let saved_bridge = self.forced_bridge.clone();
        self.forced_bridge = Some(self.resolve_bridge(&first.name, &first.bridge));

        let mut handoffs = Vec::new();

        let result: Result<RelayReport> =
            match self.run_turn_with_attachments(
                &first.name,
                &first.provider,
                user_text,
                policy,
                attachment_ids,
            ) {
                Ok(turn) if !is_transient_stop(&turn.stop_reason) => {
                    crate::limits::clear(&first.name);
                    Ok(RelayReport {
                        answered_by: first.name.clone(),
                        turn,
                        handoffs,
                    })
                }
                Ok(turn) => {
                    // Answered but stopped short — relay from here.
                    let reason = HandoffReason::ContextExhausted;
                    match self.relay_rest(
                        &chain[1..],
                        &saved_bridge,
                        user_text,
                        policy,
                        &first.name,
                        reason,
                        &mut handoffs,
                    ) {
                        Ok(Some(r)) => Ok(r),
                        Ok(None) => Ok(RelayReport {
                            answered_by: first.name.clone(),
                            turn,
                            handoffs,
                        }),
                        Err(e) => Err(e),
                    }
                }
                Err(_) if chorus_acp::was_cancelled() => {
                    // The user pressed Stop. Not a provider failure — failing
                    // over would just start the same turn on another agent.
                    Err(anyhow::anyhow!("หยุดโดยผู้ใช้"))
                }
                Err(e) => {
                    // `{e:#}` flattens anyhow's context chain so the classifier
                    // sees the underlying "…rate limit…" message.
                    let msg = format!("{e:#}");
                    if matches!(
                        HandoffReason::from_failure(&msg),
                        Some(HandoffReason::RateLimit | HandoffReason::QuotaExhausted)
                    ) {
                        crate::limits::record_hit(&first.name, &msg);
                    }
                    if crate::limits::is_auth_failure(&msg) {
                        crate::limits::record_auth_failure(&first.name);
                    }
                    match HandoffReason::from_failure(&msg) {
                        None => Err(e).with_context(|| first.name.clone()),
                        Some(reason) => match self.relay_rest(
                            &chain[1..],
                            &saved_bridge,
                            user_text,
                            policy,
                            &first.name,
                            reason,
                            &mut handoffs,
                        ) {
                            Ok(Some(r)) => Ok(r),
                            Ok(None) => Err(e).context("relay chain exhausted"),
                            Err(e2) => Err(e2),
                        },
                    }
                }
            };

        self.forced_bridge = saved_bridge;
        result
    }

    /// Try each remaining link via a handoff. `Ok(Some)` on the first that
    /// answers cleanly, `Ok(None)` if the whole tail is exhausted.
    #[allow(clippy::too_many_arguments)]
    fn relay_rest(
        &mut self,
        rest: &[AgentSpec],
        test_bridge: &Option<Vec<String>>,
        user_text: &str,
        policy: Policy,
        from: &str,
        reason: HandoffReason,
        handoffs: &mut Vec<(String, String, HandoffReason)>,
    ) -> Result<Option<RelayReport>> {
        let mut from = from.to_string();
        let mut reason = reason;
        for spec in rest {
            // Resolve independently of the previous link: an explicit spec
            // bridge, then the session's test bridge, then the per-name one.
            self.forced_bridge = Some(if !spec.bridge.is_empty() {
                spec.bridge.clone()
            } else if let Some(t) = test_bridge {
                t.clone()
            } else {
                chorus_acp::codex::bridge_for(&spec.name)
            });
            handoffs.push((from.clone(), spec.name.clone(), reason));

            // The incoming agent must inherit the *session's* objective, not
            // the sentence that happened to be in flight when the outgoing one
            // fell over. Getting this wrong is subtle and expensive: the relay
            // still "works", but the new agent optimises for the wrong thing.
            let ts = self.task_state().unwrap_or_default();
            let goal = if ts.goal.trim().is_empty() {
                user_text.to_string()
            } else {
                ts.goal.clone()
            };
            let mut pending = ts.pending.clone();
            // On the first turn the instruction *is* the goal, and repeating it
            // under `pending` spends the never-trimmed tier saying the same
            // sentence twice — which also reads, to the receiving agent, as two
            // separate things to do.
            if user_text.trim() != goal.trim() {
                push_unique(&mut pending, user_text);
            }

            let h = self.handoff(
                &spec.name,
                &spec.provider,
                reason,
                &goal,
                ts.done.clone(),
                pending,
                ts.constraints.clone(),
                String::new(),
                policy,
            );
            match h {
                Ok(hr) if !is_transient_stop(&hr.stop_reason) => {
                    crate::limits::clear(&spec.name);
                    let turn = TurnReport {
                        user_seq: hr.handoff_seq,
                        agent_seqs: hr.agent_seqs,
                        message: hr.echo_back,
                        context_used: None,
                        context_window: None,
                        stop_reason: hr.stop_reason,
                    };
                    return Ok(Some(RelayReport {
                        answered_by: spec.name.clone(),
                        turn,
                        handoffs: std::mem::take(handoffs),
                    }));
                }
                Ok(_) => {
                    reason = HandoffReason::ContextExhausted;
                    from = spec.name.clone();
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    reason = HandoffReason::from_failure(&msg).unwrap_or(HandoffReason::Failover);
                    if matches!(reason, HandoffReason::RateLimit | HandoffReason::QuotaExhausted) {
                        crate::limits::record_hit(&spec.name, &msg);
                    }
                    if crate::limits::is_auth_failure(&msg) {
                        crate::limits::record_auth_failure(&spec.name);
                    }
                    from = spec.name.clone();
                }
            }
        }
        Ok(None)
    }

    /// Rebuild the hot context from the ledger, sized to `budget` tokens.
    /// This is the "session is the app's" invariant in practice: history for
    /// the next turn comes from here, never from a provider's session state.
    /// What the session is for, what is already settled, and what is still
    /// open. This is the `TaskState` tier — 5% of the window and never
    /// trimmed — so it must not be empty on a session that has said anything.
    ///
    /// Nothing here is invented: the goal is the first thing the human asked
    /// for, `pending`/`constraints` come from the most recent handoff brief,
    /// and `done` is decisions plus the *stored* L2 gists of recent agent
    /// turns. No summariser runs on this path.
    pub fn task_state(&self) -> Result<TaskState> {
        let c = self.store.conn();
        let events = ledger::history(c, self.sess, 400)?;
        let mut ts = TaskState::default();

        for e in &events {
            if e.kind == EventKind::Msg && e.actor == self.user {
                ts.goal = clip(&ledger::body(c, e)?, 240);
                break;
            }
        }

        // The last handoff brief is the only place a human states what is
        // still pending, so it wins over anything we could infer.
        if let Some(e) = events.iter().rev().find(|e| e.kind == EventKind::Handoff) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&ledger::body(c, e)?) {
                if ts.goal.trim().is_empty() {
                    ts.goal = v["goal"].as_str().unwrap_or_default().to_string();
                }
                ts.pending = str_list(&v, "pending");
                ts.constraints = str_list(&v, "constraints");
            }
        }

        for e in &events {
            if e.kind == EventKind::Decision {
                push_unique(&mut ts.done, ledger::body(c, e)?.trim());
            }
        }
        // `done` carries only what actually happened: decisions someone
        // recorded, and the files that changed. Agent chatter ("looking at
        // that file for you") is not an accomplishment — summarising prose
        // into this field is how a brief quietly fills with noise, and the
        // receiving agent then treats filler as settled fact.
        //
        // The file list comes from the workspace, not from `Diff` events.
        // Diff events only exist when the *client* performed the write, and
        // the agents that matter here do not write that way: Codex shells out.
        // Measured on a real two-provider session — Codex created a file and
        // the ledger recorded zero Diff events, so a brief built from them
        // told the next agent nothing had been touched. The shadow repo sees
        // the file either way.
        let touched: Vec<String> = self
            .checkpoint
            .as_ref()
            .and_then(|ck| ck.files_since(&self.base_ckpt).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|f| !projector::AgentFile::is_projected(f))
            .collect();
        if !touched.is_empty() {
            let shown: Vec<&str> = touched.iter().take(6).map(String::as_str).collect();
            let more = touched.len().saturating_sub(shown.len());
            let line = if more > 0 {
                format!("edited {}: {} (+{more} more)", touched.len(), shown.join(", "))
            } else {
                format!("edited {}: {}", touched.len(), shown.join(", "))
            };
            push_unique(&mut ts.done, &line);
        }
        Ok(ts)
    }

    /// Fill in the rungs this session is missing (storage spec: the warm
    /// tier). Deliberately *not* called from `run_turn` — this is where the
    /// summariser is allowed to be slow, so `assemble_context` never is.
    /// `limit` bounds one pass; call it again to catch up a long session.
    /// How long one background compaction pass may take before it stops and
    /// leaves the rest for the next one. Three seconds is chosen from the
    /// other side of the problem: this pass shares the engine's serial queue
    /// with turns, so it is also the longest a user's next message can be made
    /// to wait. With the extractive summariser a pass never gets close; with a
    /// local model 40 events could otherwise run for minutes.
    fn compact_budget() -> std::time::Duration {
        let ms = std::env::var("CHORUS_COMPACT_BUDGET_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(3_000);
        std::time::Duration::from_millis(ms)
    }

    pub fn compact(&self, limit: usize) -> Result<CompactReport> {
        let summarizer = compaction::best();
        self.compact_with(summarizer.as_ref(), limit, Self::compact_budget())
    }

    /// The testable half of [`compact`]: summariser and time budget injected.
    ///
    /// Stops at whichever comes first — `max_events` or `budget` — and always
    /// reports what is left, so the caller can simply run it again. Bounding by
    /// wall clock rather than only by count is what makes this safe to call
    /// after every turn no matter how slow the summariser is.
    pub fn compact_with(
        &self,
        summarizer: &dyn compaction::Summarizer,
        max_events: usize,
        budget: std::time::Duration,
    ) -> Result<CompactReport> {
        let c = self.store.conn();
        let todo = compaction::uncompacted(c, self.sess, max_events)?;
        if todo.is_empty() {
            return Ok(CompactReport {
                compacted: 0,
                remaining: 0,
                model: "nothing to do".into(),
                took_ms: 0,
                hit_budget: false,
            });
        }
        let model = summarizer.label();
        let started = std::time::Instant::now();
        let mut compacted = 0usize;
        let mut hit_budget = false;
        for seq in todo {
            let e = ledger::get(c, seq)?;
            let body = ledger::body(c, &e)?;
            let r = compaction::rungs_for(summarizer, &body);
            compaction::save_rungs(c, seq, &r, &model)?;
            compacted += 1;
            // Checked *after* the write, so a pass always makes progress even
            // when one event alone costs more than the whole budget.
            if started.elapsed() >= budget {
                hit_budget = true;
                break;
            }
        }
        Ok(CompactReport {
            compacted,
            remaining: compaction::uncompacted_count(c, self.sess)? as usize,
            model,
            took_ms: started.elapsed().as_millis() as u64,
            hit_budget,
        })
    }

    /// How many events still have no stored rungs — what the shell shows to
    /// decide whether a background compaction pass is worth running.
    pub fn uncompacted_count(&self) -> Result<i64> {
        Ok(compaction::uncompacted_count(self.store.conn(), self.sess)?)
    }

    pub fn assemble_context(&self, budget: i64) -> Result<String> {
        let c = self.store.conn();
        let events = ledger::history(c, self.sess, 400)?;
        let bodies: Vec<(Event, String)> = events
            .into_iter()
            .map(|e| {
                let b = ledger::body(c, &e)?;
                Ok((e, b))
            })
            .collect::<Result<_>>()?;

        // The warm tier, read in one query. Anything not compacted yet falls
        // back to the model-free `Extractive` — never to `compaction::best()`,
        // which may reach for Ollama and would put a network round trip per
        // event on the critical path of every turn.
        let rungs = compaction::load_session_rungs(c, self.sess)?;

        // A tool result that only says the call finished carries no
        // information the tool call above it did not already carry, and it
        // costs a full line every time. On a real two-provider session that
        // was ~30% of the conversation tier spent on "X → completed".
        // Failures stay: those change what the next agent should do.
        let bodies: Vec<(Event, String)> = bodies
            .into_iter()
            .filter(|(e, body)| {
                e.kind != EventKind::ToolResult || !is_bare_success(body)
            })
            .collect();

        let n = bodies.len();
        let turns: Vec<TurnItem> = bodies
            .iter()
            .enumerate()
            .map(|(i, (e, body))| {
                let who = actor::name(c, e.actor).unwrap_or_else(|_| "?".into());
                let r = match rungs.get(&e.seq) {
                    Some(r) => r.clone(),
                    None => compaction::rungs_for(&compaction::Extractive, body),
                };
                TurnItem {
                    text_l1: Some(format!("{who}: {}", r.l1)),
                    text_l2: Some(format!("{who}: {}", r.l2)),
                    text_l0: format!("{who}: {body}"),
                    age: n - 1 - i,
                    key: e.blob.as_ref().map(|b| b.0.clone()),
                }
            })
            .collect();

        let code: Vec<ContextItem> = bodies
            .iter()
            .filter(|(e, _)| e.kind == EventKind::Diff)
            .map(|(e, body)| ContextItem {
                text: body.clone(),
                key: e.blob.as_ref().map(|b| b.0.clone()),
                label: "diff".into(),
            })
            .collect();

        let facts = memory::top(self.mem_store().conn(), 1, 12)?;
        let memory_items: Vec<ContextItem> = memory::render(&facts)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| ContextItem {
                text: l.to_string(),
                key: None,
                label: "fact".into(),
            })
            .collect();

        let input = AssembleInput {
            system: "You are one of several agents sharing this session. The context below \
                     was assembled by Chorus from the shared ledger."
                .into(),
            task_state: self.task_state()?.render(),
            memory: memory_items,
            turns,
            code,
        };
        let assembler = Assembler::new(Budget::new(budget), CachedCounter::new(HeuristicCounter));
        Ok(assembler.assemble(&input).text)
    }

    // ---- internals --------------------------------------------------

    fn one_turn(
        &self,
        agent_name: &str,
        bridge: &[String],
        text: &str,
        policy: Policy,
        attachment_ids: &[i64],
    ) -> Result<TurnOutcome> {
        let log = self.log_permissions;
        let mut decide = |ask: &PermissionAsk| {
            let d = policy.decide(ask);
            if log {
                eprintln!(
                    "  permission: {} ({}) — {}",
                    ask.tool,
                    ask.kind,
                    if d == Permission::Allow {
                        "ALLOW"
                    } else {
                        "DENY"
                    }
                );
            }
            d
        };
        // Asking for the mode is not the same as getting it. Measured against
        // @agentclientprotocol/codex-acp on 12 Sep 2026: it accepts
        // `set_config_option`, reports `read-only` back as current, then writes
        // files — inside *and* outside the workspace — without one
        // `session/request_permission`. So for a restrictive policy we take our
        // own snapshot first and check the workspace afterwards: not prevention,
        // but the difference between a promise we keep and one we only print.
        // Project memory *before* snapshotting. The projection writes AGENTS.md
        // itself, and a guard taken earlier would blame the agent for our own
        // file. (It did, the first time this ran.)
        self.project_memory(agent_name);
        let guard = policy
            .restrictive()
            .then(|| self.snapshot("read-only guard").ok())
            .flatten();
        let out = self.one_turn_decide(
            agent_name,
            bridge,
            text,
            policy.mode_id(),
            attachment_ids,
            &mut decide,
        );
        if let Some(base) = guard {
            self.revert_if_touched(agent_name, &base)?;
        }
        out
    }

    /// Echo every permission decision to stderr. Off by default: a library
    /// that prints is a library the GUI cannot use quietly. The CLI turns it
    /// on because there a human is watching the stream.
    /// Pin which model an agent should use, by the id the agent itself
    /// reports (`chorus models`). Without this the agent runs whatever its own
    /// config defaults to — which is what the app's model picker did for its
    /// whole life before this existed: it changed a label and nothing else.
    pub fn set_model(&mut self, agent_name: &str, model_id: &str) {
        if model_id.trim().is_empty() {
            self.models.remove(agent_name);
        } else {
            self.models.insert(agent_name.to_string(), model_id.to_string());
        }
    }

    /// The pin for `agent_name`, falling back to its agent without the
    /// account (`claude@work` uses the model picked for `claude`).
    fn model_pin(&self, agent_name: &str) -> Option<String> {
        self.models
            .get(agent_name)
            .or_else(|| self.models.get(crate::accounts::split(agent_name).0))
            .cloned()
    }

    /// What model is pinned for `agent_name`, if any.
    pub fn model_for(&self, agent_name: &str) -> Option<&str> {
        self.models.get(agent_name).map(String::as_str)
    }

    pub fn log_permissions(mut self, on: bool) -> Self {
        self.log_permissions = on;
        self
    }

    fn one_turn_decide(
        &self,
        agent_name: &str,
        bridge: &[String],
        text: &str,
        mode: &str,
        attachment_ids: &[i64],
        decide: &mut dyn FnMut(&PermissionAsk) -> Permission,
    ) -> Result<TurnOutcome> {
        self.project_memory(agent_name);
        // One MCP list for whoever is answering this turn. ACP lets the client
        // decide what an agent can reach, so the servers come from
        // `.chorus/mcp.json` rather than from each CLI's own config — that is
        // the whole point: declare a server once, and Claude and Codex both
        // have it, with no per-agent file to keep in step.
        let catalog_mcp = integration_catalog::project_mcp(&self.cwd)
            .context("load enabled catalog MCP tools")?;
        let composed = mcp_config::compose_catalog(&self.cwd, &catalog_mcp);
        anyhow::ensure!(
            composed.collisions.is_empty(),
            "catalog MCP server name conflicts with project configuration: {}",
            composed.collisions.join(", ")
        );
        let mut servers: Vec<serde_json::Value> = composed.servers.into_iter().map(|s| s.entry).collect();
        if let Some(m) = &self.mcp {
            servers.push(m.mcp_server_entry());
        }
        let mcp_servers = serde_json::Value::Array(servers);
        // Confine the agent's writes with the OS before the turn, not just
        // revert them after it. `mode` already told us how much this policy
        // promised; the sandbox is what makes that promise true for files the
        // shadow repo cannot reach.
        let profile = if mode == "read-only" {
            sandbox::Profile::ReadOnly
        } else {
            sandbox::Profile::Workspace
        };
        let command = match sandbox::wrap(bridge, &self.cwd, profile) {
            Ok(c) => c,
            Err(why) => {
                if self.log_permissions {
                    eprintln!("  ⚠  ไม่ได้เปิด sandbox ({why}) — เหลือแค่ย้อนไฟล์หลังเทิร์น");
                }
                bridge.to_vec()
            }
        };

        let mut env = crate::accounts::env_for(agent_name);
        let cli_paths = integration_catalog::cli_paths_for(&self.cwd)
            .context("load enabled catalog CLI tools")?;
        if !cli_paths.is_empty() {
            let mut paths = cli_paths;
            if let Some(existing) = std::env::var_os("PATH") {
                paths.extend(std::env::split_paths(&existing));
            }
            let joined = std::env::join_paths(paths).context("compose managed tool PATH")?;
            env.retain(|(name, _)| name != "PATH");
            env.push(("PATH".to_string(), joined.to_string_lossy().into_owned()));
        }
        let cfg = CodexConfig {
            command,
            mcp_servers,
            mode: Some(mode.to_string()),
            model: self.model_pin(agent_name),
            log: acp_log(),
            // `codex@work` runs the same CLI with its own CODEX_HOME.
            env,
        };
        let mut agent =
            CodexAgent::start(&self.cwd, cfg).with_context(|| format!("start {agent_name}"))?;
        note_mode(agent_name, agent.mode(), self.log_permissions);
        if self.log_permissions {
            match (self.model_pin(agent_name).as_ref(), agent.model()) {
                // Asked for one and the agent confirmed a different one: the
                // pick did not take. Say so — the alternative is a user who
                // thinks they switched models and is reading output from the
                // old one.
                (Some(want), Some(got)) if want != got => {
                    eprintln!("  ⚠  {agent_name}: ขอ {want} แต่ได้ {got}")
                }
                (_, Some(got)) => eprintln!("  model: {got}"),
                (Some(want), None) => {
                    eprintln!("  ⚠  {agent_name}: ไม่ยืนยันโมเดล {want}")
                }
                (None, None) => {}
            }
        }
        let mut prompt_text = text.to_string();
        let instructions = integration_catalog::project_instructions(&self.cwd)
            .context("load enabled catalog instructions")?;
        if !instructions.is_empty() {
            prompt_text.push_str("\n\nEnabled Chorus tool guidance:\n");
            for instruction in instructions {
                prompt_text.push_str("- ");
                prompt_text.push_str(&instruction);
                prompt_text.push('\n');
            }
        }
        let available: Vec<_> = attachments::list(self.store.conn(), self.sess)?
            .into_iter()
            .filter(|a| a.status == "sent")
            .collect();
        if !available.is_empty() {
            prompt_text.push_str("\n\nSession files available through Chorus MCP tools attachment_list and attachment_read:\n");
            for a in &available {
                let safe_name: String = a.name.chars().filter(|c| !c.is_control()).collect();
                prompt_text.push_str(&format!("- id {}: {} ({}, {} bytes)\n", a.id, safe_name, a.media_type, a.size));
            }
            prompt_text.push_str("Open older files only when needed; a listed name is not the file content.\n");
        }
        let mut blocks = vec![serde_json::json!({ "type": "text", "text": prompt_text })];
        for id in attachment_ids {
            let (meta, bytes, mime) =
                attachments::get_for_agent(self.store.conn(), self.sess, *id)?;
            if mime.starts_with("image/") {
                anyhow::ensure!(
                    agent.supports_image_prompt(),
                    "{agent_name} does not advertise image input for {}",
                    meta.name
                );
                blocks.push(serde_json::json!({
                    "type": "image",
                    "mimeType": mime,
                    "data": attachments::encode_base64(&bytes),
                }));
            } else if mime.starts_with("text/")
                || mime == "application/json"
                || mime.ends_with("+json")
                || mime.ends_with("+xml")
            {
                let content = String::from_utf8(bytes)
                    .with_context(|| format!("attachment {} is not valid UTF-8", meta.name))?;
                if content.chars().count() <= 12_000 && agent.supports_embedded_context() {
                    blocks.push(serde_json::json!({
                        "type": "resource",
                        "resource": {
                            "uri": format!("chorus://attachment/{}", meta.id),
                            "mimeType": mime,
                            "text": content,
                        }
                    }));
                } else {
                    let excerpt: String = content.chars().take(12_000).collect();
                    let suffix = if excerpt.chars().count() < content.chars().count() {
                        "\n[Excerpt only. Use Chorus attachment_read with offset_chars to read the rest.]"
                    } else {
                        ""
                    };
                    blocks.push(serde_json::json!({
                        "type": "text",
                        "text": format!("Attachment {} (id {}):\n{}{}", meta.name, meta.id, excerpt, suffix),
                    }));
                }
            } else {
                anyhow::bail!("attachment {} ({mime}) is not readable by this agent", meta.name);
            }
        }
        agent.prompt_blocks(blocks, decide).context("agent turn")
    }

    /// Like [`run_turn`], but each permission request is put to `decide`
    /// (the shell shows a dialog and blocks until the user answers). The
    /// history/ledger/persistence path is identical.
    pub fn run_turn_decide(
        &mut self,
        agent_name: &str,
        provider: &str,
        user_text: &str,
        decide: &mut dyn FnMut(&PermissionAsk) -> Permission,
    ) -> Result<TurnReport> {
        self.run_turn_decide_with_attachments(agent_name, provider, user_text, &[], decide)
    }

    pub fn run_turn_decide_with_attachments(
        &mut self,
        agent_name: &str,
        provider: &str,
        user_text: &str,
        attachment_ids: &[i64],
        decide: &mut dyn FnMut(&PermissionAsk) -> Permission,
    ) -> Result<TurnReport> {
        let agent_actor = actor::intern(self.store.conn(), agent_name, provider)?;
        let has_history = !ledger::history(self.store.conn(), self.sess, 1)?.is_empty();
        let prompt = if has_history {
            let ctx = self.assemble_context(120_000)?;
            format!(
                "{ctx}\n\n---\nThat is the session so far, reconstructed from the shared \
                 ledger. Continue from here.\n\nNew instruction:\n{user_text}"
            )
        } else {
            user_text.to_string()
        };
        let event = NewEvent::new(self.sess, EventKind::Msg, self.user, user_text);
        let user_seq = if attachment_ids.is_empty() {
            ledger::append(self.store.conn(), &event)?
        } else {
            attachments::link_to_event(self.store.conn_mut(), self.sess, &event, attachment_ids)?
        };
        let bridge = self.resolve_bridge(agent_name, &[]);

        // A dialog the agent never opens protects nothing. Codex writes files
        // without sending a single `session/request_permission`, so "the user
        // pressed Deny" has to mean something on this side too: watch for a
        // deny, and if the workspace changed anyway, put it back.
        //
        // The revert is all-or-nothing for the turn. Undoing an edit the user
        // allowed is the lesser wrong: keeping one they refused is the failure
        // that makes the dialog a lie, and the ledger still has every event.
        let denied = std::cell::Cell::new(false);
        let asked = std::cell::Cell::new(false);
        let mut watch = |ask: &PermissionAsk| {
            asked.set(true);
            let d = decide(ask);
            if d == Permission::Deny {
                denied.set(true);
            }
            d
        };
        self.project_memory(agent_name);
        let guard = self.snapshot("turn guard").ok();
        // An interactive caller shows the user a dialog per action, so it wants
        // the agent to ask about everything — the same mode the restrictive
        // policies use.
        let outcome = self.one_turn_decide(
            agent_name,
            &bridge,
            &prompt,
            Policy::ReadOnly.mode_id(),
            attachment_ids,
            &mut watch,
        )?;
        // Two ways this turn breaks the promise the dialog makes. The user said
        // no and it went ahead — or it never asked at all and wrote anyway,
        // which is what Codex does: the person sat in front of "ask me first"
        // and was never asked. Claude does send requests, so a turn with asks
        // and no denials is left alone.
        //
        // What this cannot catch: an agent that asks about one file and quietly
        // writes another. Closing that needs a real sandbox, not a ledger.
        if denied.get() || !asked.get() {
            if let Some(base) = guard.as_deref() {
                self.revert_if_touched(agent_name, base)?;
            }
        }
        let agent_seqs = self.persist_outcome(&outcome, agent_actor, user_seq)?;
        Ok(TurnReport {
            user_seq,
            agent_seqs,
            message: outcome.message,
            context_used: outcome.context_used,
            context_window: outcome.context_window,
            stop_reason: outcome.stop_reason,
        })
    }

    /// Render the single memory into the file the agent reads on startup
    /// (`CLAUDE.md` / `AGENTS.md` / `GEMINI.md`), unless the user has taken
    /// that file over by hand. Best-effort — a projection failure never
    /// blocks a turn.
    fn project_memory(&self, agent_name: &str) {
        let target = match agent_name.to_lowercase() {
            n if n.contains("claude") => AgentFile::Claude,
            n if n.contains("gemini") => AgentFile::Gemini,
            _ => AgentFile::Codex,
        };
        let path = self.cwd.join(target.filename());

        let existing = std::fs::read_to_string(&path).ok();
        match projector::drift(existing.as_deref()) {
            Drift::Foreign | Drift::Edited => return, // never clobber a human's file
            Drift::Missing | Drift::Untouched => {}
        }

        let project =
            std::fs::read_to_string(self.cwd.join(".chorus/project.md")).unwrap_or_default();
        let facts = memory::top(self.mem_store().conn(), 1, 24)
            .map(|f| memory::render(&f))
            .unwrap_or_default();
        let task = self.task_state().map(|t| t.render()).unwrap_or_default();

        // Skills are projected whichever way this agent can read them: real
        // `.claude/skills/` files for Claude, a one-line-each index in the
        // instruction file for everyone else.
        let found = skills::discover(&self.cwd);
        let skill_index = if target == AgentFile::Claude {
            self.project_claude_skills(&found);
            String::new()
        } else {
            skills::index(&found)
        };

        if project.trim().is_empty()
            && facts.trim().is_empty()
            && task.trim().is_empty()
            && skill_index.trim().is_empty()
        {
            return; // nothing to project yet
        }

        let out = projector::render(
            target,
            &Projection {
                project,
                facts,
                task_state: task,
                skills: skill_index,
            },
        );
        let _ = std::fs::write(&path, out);
    }

    /// Write each skill where Claude Code's loader will find it.
    ///
    /// Same contract as the instruction files: our copies carry the generated
    /// marker, and a skill the user wrote (or edited) by hand under
    /// `.claude/skills/` is left exactly as it is. Overwriting someone's own
    /// skill because a file of ours happens to share its name would be the
    /// worst kind of "one set of skills".
    fn project_claude_skills(&self, found: &[skills::Skill]) {
        for k in found {
            let path = self.cwd.join(k.claude_path());
            let existing = std::fs::read_to_string(&path).ok();
            match projector::drift_raw(existing.as_deref()) {
                Drift::Foreign | Drift::Edited => continue,
                Drift::Missing | Drift::Untouched => {}
            }
            if let Some(dir) = path.parent() {
                if std::fs::create_dir_all(dir).is_err() {
                    continue;
                }
            }
            let _ = std::fs::write(&path, projector::render_raw(&k.body));
        }
    }

    fn persist_outcome(
        &self,
        outcome: &TurnOutcome,
        agent_actor: i64,
        parent: i64,
    ) -> Result<Vec<i64>> {
        let c = self.store.conn();
        let mut seqs = Vec::new();
        for ev in &outcome.events {
            let (kind, body) = match ev {
                AgentEvent::Message(m) => (EventKind::Msg, m.clone()),
                AgentEvent::ToolCall { title, kind } => {
                    (EventKind::ToolCall, format!("{kind}: {title}"))
                }
                AgentEvent::ToolResult { title, status } => {
                    (EventKind::ToolResult, format!("{title} → {status}"))
                }
                AgentEvent::Diff { path, old, new } => {
                    (EventKind::Diff, render_diff(path, old, new))
                }
            };
            let mut ne = NewEvent::new(self.sess, kind, agent_actor, body).parent(parent);
            if matches!(ev, AgentEvent::Message(_)) {
                ne.tok_out = outcome.context_used;
            }
            seqs.push(ledger::append(c, &ne)?);
        }
        Ok(seqs)
    }

    /// The actor name of the most recent non-user event, if any.
    fn last_agent(&self) -> Option<String> {
        let c = self.store.conn();
        let events = ledger::history(c, self.sess, 200).ok()?;
        events
            .iter()
            .rev()
            .filter(|e| e.actor != self.user)
            .find_map(|e| actor::name(c, e.actor).ok())
    }

    /// The whole session, oldest first, as `(seq, kind name, actor name,
    /// body)` — the shape the FFI/UI wants without exposing core types.
    pub fn transcript_labeled(&self) -> Result<Vec<(i64, &'static str, String, String)>> {
        self.events_since(0)
    }

    /// Labeled events with `seq > after` — the delta a UI polls for so it
    /// only ever pulls what it hasn't seen.
    pub fn events_since(&self, after: i64) -> Result<Vec<(i64, &'static str, String, String)>> {
        let c = self.store.conn();
        let mut out = Vec::new();
        for (e, body) in self.transcript()? {
            if e.seq <= after {
                continue;
            }
            let actor = actor::name(c, e.actor).unwrap_or_else(|_| "?".into());
            out.push((e.seq, kind_name(e.kind), actor, body));
        }
        Ok(out)
    }

    /// The whole session so far, oldest first, with bodies resolved.
    pub fn transcript(&self) -> Result<Vec<(chorus_core::Event, String)>> {
        let c = self.store.conn();
        let events = ledger::history(c, self.sess, 10_000)?;
        events
            .into_iter()
            .map(|e| {
                let body = ledger::body(c, &e)?;
                Ok((e, body))
            })
            .collect()
    }

    /// Hand over with no human input at all: goal, done, pending and constraints
    /// come from `task_state()`, the diff from the checkpoint. This is the whole
    /// point of keeping task state in the ledger — the user should never have to
    /// retype what the app already knows.
    ///
    /// Reason is `HandoffReason::Manual` — a deliberate switch with no forced
    /// cause, which is exactly what a one-click handoff is: nobody hit a limit,
    /// the user just wants a different agent on it.
    pub fn handoff_auto(
        &mut self,
        to: &str,
        provider: &str,
        policy: Policy,
    ) -> Result<HandoffResult> {
        let ts = self.task_state()?;
        self.handoff(
            to,
            provider,
            HandoffReason::Manual,
            &ts.goal,
            ts.done,
            ts.pending,
            ts.constraints,
            String::new(),
            policy,
        )
    }
}

/// Stop reasons that mean the agent hit its own limit mid-answer — relay
/// onward. A `cancelled` (a denied permission) is a real outcome, not this.
fn is_transient_stop(stop: &str) -> bool {
    matches!(stop, "max_tokens" | "context_exhausted" | "error")
}

fn kind_name(k: EventKind) -> &'static str {
    match k {
        EventKind::Msg => "msg",
        EventKind::ToolCall => "tool_call",
        EventKind::ToolResult => "tool_result",
        EventKind::Diff => "diff",
        EventKind::Decision => "decision",
        EventKind::Handoff => "handoff",
        EventKind::Summary => "summary",
    }
}

/// A minimal unified diff for the ledger. Not a patch tool — just enough
/// that the change is legible and `handoff::files_touched` can find the path.
fn render_diff(path: &str, old: &str, new: &str) -> String {
    let mut s = format!("--- {path}\n+++ {path}\n");
    for line in old.lines() {
        s.push_str("- ");
        s.push_str(line);
        s.push('\n');
    }
    for line in new.lines() {
        s.push_str("+ ");
        s.push_str(line);
        s.push('\n');
    }
    s.truncate(s.trim_end().len());
    s
}

/// One row of the session picker. Deliberately not `Session::list` — it
/// carries no shadow-git handle, no MCP server, nothing that costs anything
/// to open, because `list_sessions` opens the store *once* for however many
/// rows it returns, not a `Session` per row.
pub struct SessionSummary {
    pub id: i64,
    pub title: String,
    pub cwd: String,
    pub started_ms: i64,
    /// `t` of the newest event (an offset in ms from `started_ms`), or 0
    /// when the session has no events yet.
    pub last_ms: i64,
    pub events: i64,
    /// Distinct actor names that are not "user", in first-seen order —
    /// which agents actually spoke in this session, not which ones a UI
    /// might let the user pick.
    pub agents: Vec<String>,
    /// The first user message, clipped to ~90 chars — enough to tell
    /// sessions apart at a glance in a list. Empty if nobody has said
    /// anything yet.
    pub goal: String,
    pub forked_from: Option<i64>,
}

/// List every session in `db`, newest activity first. Two SQL statements
/// total, regardless of how many sessions or events exist: one aggregate
/// (event count + newest `t` per session) and one ordered scan of `event`
/// joined to `actor` that builds every session's agent list and goal in a
/// single pass. Opening a `Session` — and its shadow-git checkpoint, MCP
/// listener — per row would make a picker screen the slowest thing in the
/// app; this function exists so it never has to.
/// Rename a session in a store, without opening it.
pub fn rename_session(db: &Path, id: i64, title: &str) -> Result<()> {
    let store = Store::open(db).context("open store")?;
    Ok(ledger::rename_session(store.conn(), id, title)?)
}

/// Delete a session from a store (see [`ledger::delete_session`]).
pub fn delete_session(db: &Path, id: i64) -> Result<()> {
    let store = Store::open(db).context("open store")?;
    Ok(ledger::delete_session(store.conn(), id)?)
}

pub fn list_sessions(db: &Path) -> Result<Vec<SessionSummary>> {
    let store = Store::open(db).context("open store")?;
    let c = store.conn();

    struct Base {
        id: i64,
        title: String,
        cwd: String,
        started_ms: i64,
        forked_from: Option<i64>,
        events: i64,
        last_ms: i64,
    }

    let mut bases = Vec::new();
    {
        // `event.t` is an offset in ms from *that session's own*
        // `started_ms` (see ledger::append) — comparing it raw across
        // sessions started at different times would rank an old session
        // with a large offset above a genuinely more recent one. Sort by
        // absolute activity time (`started_ms + last_ms`) instead; the
        // `last_ms` column returned to callers stays the session-relative
        // offset the rest of the codebase expects.
        let mut stmt = c.prepare(
            "SELECT s.id, s.title, s.cwd, s.started_ms, s.forked_from, \
             COUNT(e.seq) AS events, COALESCE(MAX(e.t), 0) AS last_ms \
             FROM session s LEFT JOIN event e ON e.sess = s.id \
             GROUP BY s.id \
             ORDER BY (s.started_ms + COALESCE(MAX(e.t), 0)) DESC, s.started_ms DESC",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            bases.push(Base {
                id: r.get(0)?,
                title: r.get(1)?,
                cwd: r.get(2)?,
                started_ms: r.get(3)?,
                forked_from: r.get(4)?,
                events: r.get(5)?,
                last_ms: r.get(6)?,
            });
        }
    }

    // One pass over every event, ordered so "first-seen" and "first user
    // message" both fall out for free without a second per-session query.
    let mut per_sess: std::collections::HashMap<i64, (Vec<String>, Option<String>)> =
        std::collections::HashMap::new();
    {
        let mut stmt = c.prepare(
            "SELECT e.sess, e.kind, a.name, e.body, e.blob \
             FROM event e JOIN actor a ON a.id = e.actor \
             ORDER BY e.sess, e.seq",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let sess: i64 = row.get(0)?;
            let kind: i64 = row.get(1)?;
            let actor_name: String = row.get(2)?;
            let body: Option<String> = row.get(3)?;
            let blob: Option<String> = row.get(4)?;
            let entry = per_sess.entry(sess).or_default();
            if actor_name != "user" {
                push_unique(&mut entry.0, &actor_name);
            } else if entry.1.is_none() && EventKind::from_i64(kind) == Some(EventKind::Msg) {
                let text = match body {
                    Some(b) => b,
                    None => match blob {
                        Some(id) => chorus_core::blob::get_text(c, &chorus_core::BlobId(id))?,
                        None => String::new(),
                    },
                };
                entry.1 = Some(clip(&text, 90));
            }
        }
    }

    Ok(bases
        .into_iter()
        .map(|b| {
            let (agents, goal) = per_sess.remove(&b.id).unwrap_or_default();
            SessionSummary {
                id: b.id,
                title: b.title,
                cwd: b.cwd,
                started_ms: b.started_ms,
                last_ms: b.last_ms,
                events: b.events,
                agents,
                goal: goal.unwrap_or_default(),
                forked_from: b.forked_from,
            }
        })
        .collect())
}
