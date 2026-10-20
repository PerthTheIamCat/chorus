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
use chorus_acp::sandbox;
use chorus_core::attachments::{self, Attachment};
use chorus_core::checkpoint::Checkpoint;
use chorus_core::checkpoint_dag::CheckpointDag;
use chorus_core::compaction;
use chorus_core::computer;
use chorus_core::context::{AssembleInput, Assembler, Budget, Tier, TurnItem};
use chorus_core::handoff::{Brief, BriefInput, HandoffMeta, HandoffReason};
use chorus_core::integration_catalog;
use chorus_core::mcp::McpServer;
use chorus_core::mcp_config;
use chorus_core::projector::{self, AgentFile, Drift, Projection};
use chorus_core::render::ContextItem;
use chorus_core::skills;
use chorus_core::tokens::{CachedCounter, HeuristicCounter};
use chorus_core::{actor, ledger, memory, usage, Event, EventKind, NewEvent, Store};
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
                matches!(k.as_str(), "read" | "fetch" | "search")
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

/// What `Session::rewind_before` did.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RewindReport {
    /// The fork the conversation continues in (the original is untouched).
    pub session_id: i64,
    /// The message rewound to, for the composer to offer again.
    pub text: String,
    pub code_restored: bool,
    /// Snapshot of the workspace taken just before the code was restored,
    /// so the rewind itself can be undone with `restore_to`.
    pub backup: Option<String>,
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

#[derive(Clone)]
struct ChildRunnerState {
    db_path: PathBuf,
    /// The orchestrating session's workspace: role files are looked up here,
    /// not in a child's worktree.
    cwd: PathBuf,
    forced_bridge: Option<Vec<String>>,
    /// The user's permission for this session — what children are capped by.
    /// Never the orchestrator's own read-only turn policy.
    parent_policy: Policy,
    route_preset: chorus_core::router::RoutePreset,
    caps: chorus_core::agent_control::OrchestraCaps,
    /// Who may run a child. `None` = ask `agents::list_agents()` at spawn
    /// time; tests set it so routing does not depend on this machine's logins.
    candidates: Option<Vec<chorus_core::router::Candidate>>,
    /// subagent id → the cancel tag its current run is filed under. Tags are
    /// unique in the process; subagent ids are only unique per store.
    tags: std::collections::HashMap<i64, i64>,
}

/// Next process-unique cancel tag (see `chorus_acp::scope`).
fn next_cancel_tag() -> i64 {
    static NEXT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}

struct SessionChildRunner {
    state: std::sync::Arc<std::sync::Mutex<ChildRunnerState>>,
}

fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

type RunnerState = std::sync::Arc<std::sync::Mutex<ChildRunnerState>>;

/// The runner for children `sess` spawns, registered under `sess` only.
fn register_runner(
    store: &Store,
    db: &Path,
    cwd: &Path,
    sess: i64,
) -> (
    RunnerState,
    std::sync::Arc<dyn chorus_core::agent_control::ChildRunner>,
) {
    let state = std::sync::Arc::new(std::sync::Mutex::new(ChildRunnerState {
        db_path: db.to_path_buf(),
        cwd: cwd.to_path_buf(),
        forced_bridge: None,
        parent_policy: Policy::ReadOnly,
        route_preset: Default::default(),
        caps: Default::default(),
        candidates: None,
        tags: Default::default(),
    }));
    let runner: std::sync::Arc<dyn chorus_core::agent_control::ChildRunner> =
        std::sync::Arc::new(SessionChildRunner {
            state: state.clone(),
        });
    chorus_core::agent_control::register_child_runner(store.conn(), sess, runner.clone());
    (state, runner)
}

/// A child's permission: never wider than the parent's, and a child without
/// its own worktree never writes, because it shares the user's tree.
fn child_policy(parent: Policy, has_worktree: bool) -> Policy {
    match parent {
        Policy::AllowAll if has_worktree => Policy::AllowAll,
        Policy::AllowAll | Policy::ReadOnly => Policy::ReadOnly,
        Policy::DenyAll => Policy::DenyAll,
    }
}

/// Paths a child's diff never counts: our shadow repo and the agent files the
/// projector writes into every workspace (see CLAUDE.md 7.11).
const DIFF_EXCLUDES: &[&str] = &[
    ":(exclude).chorus",
    ":(exclude)CLAUDE.md",
    ":(exclude)AGENTS.md",
    ":(exclude)GEMINI.md",
];

/// What a writing child changed in its worktree, untracked files included:
/// `git diff --numstat HEAD` misses every file the child created, which for a
/// builder is most of its work. Untracked files count as all-added lines.
pub fn worktree_diff_stat(dir: &Path) -> (String, Vec<String>) {
    let git = |args: &[&str]| -> Option<String> {
        let out = std::process::Command::new("git")
            .args(args)
            .args(["--"])
            .args(["."])
            .args(DIFF_EXCLUDES)
            .current_dir(dir)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let mut files = Vec::new();
    let (mut ins, mut del) = (0u64, 0u64);
    for line in git(&["diff", "--numstat", "HEAD"])
        .unwrap_or_default()
        .lines()
    {
        let mut f = line.splitn(3, '\t');
        let (a, d, path) = (f.next(), f.next(), f.next());
        if let Some(path) = path {
            ins += a.and_then(|v| v.parse().ok()).unwrap_or(0);
            del += d.and_then(|v| v.parse().ok()).unwrap_or(0);
            files.push(path.to_string());
        }
    }
    let untracked = git(&["ls-files", "--others", "--exclude-standard"]).unwrap_or_default();
    for path in untracked.lines().map(str::trim).filter(|p| !p.is_empty()) {
        if let Ok(text) = std::fs::read(dir.join(path)) {
            ins += text.iter().filter(|b| **b == b'\n').count() as u64;
        }
        if !files.iter().any(|f| f == path) {
            files.push(path.to_string());
        }
    }
    let n = files.len();
    let stat = format!(
        "{n} file{} changed, {ins} insertion{}(+), {del} deletion{}(-)",
        if n == 1 { "" } else { "s" },
        if ins == 1 { "" } else { "s" },
        if del == 1 { "" } else { "s" },
    );
    (stat, files)
}

/// The prompt a child starts from: its role's instructions, then the task.
fn child_prompt(role: Option<&chorus_core::roles::AgentRole>, task: &str) -> String {
    match role
        .map(|r| r.system_prompt.trim())
        .filter(|p| !p.is_empty())
    {
        Some(sp) => format!("{sp}\n\n---\nTask:\n{task}"),
        None => task.to_string(),
    }
}

/// Run one child to the end on the calling thread. Everything it spawns is
/// filed under the subagent id, so stopping it stops only it.
#[allow(clippy::too_many_arguments)]
fn run_child_turn(
    db_path: &Path,
    child_sess_id: i64,
    tag: i64,
    forced_bridge: Option<Vec<String>>,
    preset: chorus_core::router::RoutePreset,
    route: &crate::orchestra::ChildRoute,
    chain: &[AgentSpec],
    prompt: &str,
    policy: Policy,
    has_worktree: bool,
) -> Result<(String, String, Vec<String>)> {
    let _scope = chorus_acp::scope::cancel_scope(tag);
    let mut child = Session::open(db_path, child_sess_id)?;
    if let Some(fb) = forced_bridge {
        child = child.with_bridge(fb);
    }
    child.set_route_preset(preset);
    child.set_model(&route.agent, &route.model);
    let rep = child.run_relay(chain, prompt, policy)?;
    let (stat, files) = if has_worktree {
        worktree_diff_stat(&child.cwd)
    } else {
        ("0 files changed".to_string(), Vec::new())
    };
    Ok((rep.turn.message, stat, files))
}

impl SessionChildRunner {
    fn tag_of(&self, subagent_id: i64) -> Option<i64> {
        self.state
            .lock()
            .ok()
            .and_then(|s| s.tags.get(&subagent_id).copied())
    }
}

impl chorus_core::agent_control::ChildRunner for SessionChildRunner {
    fn run_child(
        &self,
        subagent: &chorus_core::agent_control::Subagent,
        weight: Option<&str>,
    ) -> chorus_core::Result<()> {
        use chorus_core::agent_control::{self as ac, SubagentStatus};
        let Some(child_sess_id) = subagent.child_sess else {
            return Ok(());
        };
        let tag = next_cancel_tag();
        let st = self
            .state
            .lock()
            .map(|mut s| {
                s.tags.insert(subagent.id, tag);
                s.clone()
            })
            .map_err(|_| chorus_core::ChorusError::Invalid("child runner state poisoned".into()))?;
        // Route here, before any thread: a role that pins a model nobody
        // offers, or no ready agent at all, fails the spawn where the
        // orchestrator can see it.
        let candidates = st
            .candidates
            .clone()
            .unwrap_or_else(|| crate::orchestra::candidates_from(&crate::agents::list_agents()));
        let role = chorus_core::roles::find_role(&st.cwd, &subagent.role);
        let route = crate::orchestra::route_child(
            &candidates,
            &subagent.role,
            weight,
            st.route_preset,
            role.as_ref().and_then(|r| r.model.as_deref()),
        )
        .map_err(|e| chorus_core::ChorusError::Invalid(format!("{e:#}")))?;
        let chain = crate::orchestra::child_chain(&route, &candidates);
        let prompt = child_prompt(role.as_ref(), &subagent.task);
        let has_worktree = subagent.worktree.is_some();
        let policy = child_policy(st.parent_policy, has_worktree);
        let subagent_id = subagent.id;
        let parent = subagent.parent_sess.unwrap_or_default();
        let deadline = {
            let store = Store::open(&st.db_path)?;
            ac::wall_deadline_ms(store.conn(), parent, &st.caps)?
        };

        std::thread::spawn(move || {
            // Wall time is a cap on running children too, not only on the
            // next spawn: past the deadline the child is stopped and waits
            // for the user's Continue (which restarts the clock).
            let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            if let Some(deadline) = deadline {
                let (done, db, caps) = (done.clone(), st.db_path.clone(), st.caps);
                std::thread::spawn(move || {
                    use std::sync::atomic::Ordering::SeqCst;
                    while !done.load(SeqCst) {
                        if epoch_ms() >= deadline {
                            if let Ok(store) = Store::open(&db) {
                                if ac::pause_for_wall_time(store.conn(), subagent_id, &caps)
                                    .is_ok_and(|s| s.status == SubagentStatus::Waiting)
                                {
                                    chorus_acp::cancel_tag(tag);
                                }
                            }
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                });
            }
            let res = run_child_turn(
                &st.db_path,
                child_sess_id,
                tag,
                st.forced_bridge.clone(),
                st.route_preset,
                &route,
                &chain,
                &prompt,
                policy,
                has_worktree,
            );
            done.store(true, std::sync::atomic::Ordering::SeqCst);
            let Ok(store) = Store::open(&st.db_path) else {
                return;
            };
            // A child stopped by the user or paused at the wall clock keeps
            // that status; only a child still `running` gets an outcome.
            let still_running = ac::get_subagent(store.conn(), subagent_id)
                .is_ok_and(|s| s.status == SubagentStatus::Running);
            if !still_running {
                return;
            }
            let _ = match res {
                Ok((message, stat, files)) => {
                    ac::complete_agent(store.conn(), subagent_id, &message, Some(&stat), &files)
                }
                Err(e) => ac::fail_agent(store.conn(), subagent_id, &format!("Failed: {e:#}")),
            };
        });

        Ok(())
    }

    fn cancel_child(&self, subagent_id: i64) -> chorus_core::Result<()> {
        if let Some(tag) = self.tag_of(subagent_id) {
            chorus_acp::cancel_tag(tag);
        }
        Ok(())
    }

    fn cancel_all(&self, _parent_sess: i64) -> chorus_core::Result<()> {
        // This runner belongs to one orchestrating session, so every tag it
        // handed out is one of that session's children.
        let tags: Vec<i64> = self
            .state
            .lock()
            .map(|s| s.tags.values().copied().collect())
            .unwrap_or_default();
        for tag in tags {
            chorus_acp::cancel_tag(tag);
        }
        Ok(())
    }

    fn caps(&self) -> chorus_core::agent_control::OrchestraCaps {
        self.state.lock().map(|s| s.caps).unwrap_or_default()
    }
}

/// Files Chorus itself writes into a workspace before a turn: the projected
/// agent files and Claude skills (CLAUDE.md 7.11, 7.16).
fn is_projected_file(path: &str) -> bool {
    matches!(path, "CLAUDE.md" | "AGENTS.md" | "GEMINI.md") || path.starts_with(".claude/skills/")
}

/// One agent event as a ledger row. `children` maps a provider subagent's
/// bridge key to the child session that holds its transcript.
fn event_row(
    ev: &AgentEvent,
    children: &std::collections::HashMap<String, i64>,
) -> (EventKind, String) {
    match ev {
        AgentEvent::Message(m) => (EventKind::Msg, m.clone()),
        AgentEvent::ToolCall { title, kind } => (EventKind::ToolCall, format!("{kind}: {title}")),
        AgentEvent::ToolResult { title, status } => {
            (EventKind::ToolResult, format!("{title} → {status}"))
        }
        AgentEvent::SubagentSpawned {
            session_id,
            parent_session_id,
            name,
            task,
        } => (
            EventKind::ToolCall,
            serde_json::json!({
                "event": "subagent_spawned",
                "sessionId": session_id,
                "parentSessionId": parent_session_id,
                "childSession": children.get(session_id),
                "name": name,
                "task": task,
            })
            .to_string(),
        ),
        AgentEvent::SubagentState {
            session_id,
            state,
            message,
        } => (
            EventKind::ToolResult,
            serde_json::json!({
                "event": "subagent_state_update",
                "sessionId": session_id,
                "childSession": children.get(session_id),
                "state": state,
                "message": message,
            })
            .to_string(),
        ),
        AgentEvent::Diff { path, old, new } => (EventKind::Diff, render_diff(path, old, new)),
    }
}

/// What an orchestra-mode turn did: the relay result, and the chain it led
/// with (`lead[0]` is who was asked first; `relay.handoffs` says who took over).
pub struct OrchestraReport {
    pub relay: RelayReport,
    pub lead: Vec<crate::orchestra::Lead>,
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
    /// How Auto routing leans for this session (composer menu). Not stored
    /// in the ledger: it is a dispatch setting, not something anyone said.
    route_preset: chorus_core::router::RoutePreset,
    /// Sha the handoff brief's diff is measured from — the shadow repo's
    /// HEAD when the session opened, or the last review baseline.
    base_ckpt: String,
    /// Git HEAD at turn end. If this changes between turns, user made a commit.
    turn_end_git_head: Option<String>,
    /// Guard sha before the latest turn started (for Latest turn scope).
    last_turn_guard: Option<String>,
    /// Guard sha when the current agent took over (for Current agent scope).
    agent_turn_start_guard: Option<String>,
    /// The in-process MCP server handed to every agent (exposes
    /// `memory.propose`). `None` when it couldn't bind a port.
    mcp: Option<McpServer>,
    /// Where facts and proposals live when this session belongs to a
    /// project: one store shared by every folder in it. `None` = the
    /// session's own store, as before projects existed.
    memory: Option<Store>,
    runner_state: RunnerState,
    /// This session's entry in the child-runner registry, removed on drop.
    runner: std::sync::Arc<dyn chorus_core::agent_control::ChildRunner>,
    /// Set while the orchestrator's own turn runs: the user's permission,
    /// which caps its children, while the turn itself is read-only.
    lead_policy: Option<Policy>,
    /// Symbol map of the workspace, rebuilt only when its source files change.
    /// Behind a Mutex because `assemble_context` takes `&self`.
    repo_map: std::sync::Mutex<chorus_core::repo_map::RepoMapCache>,
    /// Set while a plan turn runs (`run_plan_with_attachments`): the prompt
    /// carries `PLAN_MODE_GUIDANCE`, the ledger keeps only what the user typed.
    plan_turn: bool,
    /// The user's computer-use switch (Settings › off by default).
    computer_use: bool,
    /// Let Claude use the user's own Chrome through the Claude in Chrome
    /// extension (Settings › off by default).
    claude_chrome: bool,
}

/// Environment that turns on a provider's *own* browser integration for its
/// bridge process. Only Claude has one that works outside its own app:
/// `CLAUDE_CODE_ENABLE_CFC=1` starts Claude Code with Claude in Chrome on
/// (code.claude.com/docs/en/env-vars, checked 20 Oct 2026; read from the
/// process environment — Claude Code ignores it in project settings).
/// Codex's Chrome extension works only in the Codex app, not its CLI.
/// Antigravity (`agy`) has a built-in browser subagent that needs no switch
/// from us; whether it runs under `agy --input-format stream-json` and the
/// Seatbelt profile is not verified yet.
/// Off adds nothing, so the user's own `/chrome` default still applies.
pub fn provider_browser_env(agent_name: &str, claude_chrome: bool) -> Vec<(String, String)> {
    if claude_chrome && agent_name.to_lowercase().contains("claude") {
        vec![("CLAUDE_CODE_ENABLE_CFC".to_string(), "1".to_string())]
    } else {
        Vec::new()
    }
}

/// Rides with the prompt of a plan turn, never stored (rule 2.1). The turn
/// itself runs under `Policy::ReadOnly`, so this is the explanation, not the
/// enforcement: Seatbelt blocks writes and `revert_if_touched` rolls back any
/// that got through.
const PLAN_MODE_GUIDANCE: &str = "\n\n(Chorus plan mode: this turn is read-only — writes are \
     blocked and anything changed is rolled back. Read and search as much as you need, but do \
     not edit files, run commands that change anything, or spawn subagents. Reply with a short \
     numbered plan: what you will change and in which files, the risks, and how you will verify \
     it. The user approves the plan before anything runs.)\n";

/// What the user "says" when they press approve on a plan. Stored in the
/// ledger like any message, so every later agent can see the plan was agreed.
pub const PLAN_APPROVED_PROMPT: &str =
    "Plan approved. Carry out the plan above, then report what you changed and how you verified it.";

/// Ceiling on the repo map, whatever the window: past ~1k tokens the extra
/// symbols are ones the agent would look up itself anyway (Aider's default).
const REPO_MAP_MAX_TOKENS: i64 = 1024;

impl Drop for Session {
    fn drop(&mut self) {
        chorus_core::agent_control::unregister_child_runner(
            self.store.conn(),
            self.sess,
            &self.runner,
        );
    }
}

pub fn real_git_head(cwd: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if out.status.success() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            return Some(s);
        }
    }
    None
}

/// Open the shadow repo and pick the base sha a handoff diff is measured
/// from. On a brand-new repo we snapshot the current tree; on an existing
/// one we take `HEAD` and leave the tree alone, so the diff covers whatever
/// agents have changed since — not since this process started.
fn open_checkpoint(cwd: &Path) -> (Option<Checkpoint>, String) {
    match Checkpoint::open_reporting(cwd) {
        Ok((ck, fresh)) => {
            // A shadow repo can exist without a single commit (opened earlier
            // by something other than a session); treat that like a fresh one,
            // otherwise the session has no base to diff against or restore to.
            let head = ck.head().unwrap_or_default();
            let base = if fresh || head.is_empty() {
                ck.snapshot("session base").unwrap_or_default()
            } else {
                head
            };
            (Some(ck), base)
        }
        Err(_) => (None, String::new()),
    }
}

impl Session {
    /// Open (or create) the store and start a new session in it.
    pub fn create(db: &Path, cwd: &Path, title: &str) -> Result<Self> {
        chorus_acp::env::repair_process_path();
        let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        let store = Store::open(db).context("open store")?;
        let user = actor::intern(store.conn(), "user", "human")?;
        let sess = ledger::new_session(store.conn(), title, &cwd.to_string_lossy())?;
        let (checkpoint, default_base) = open_checkpoint(&cwd);
        let _ = usage::set_review_base(store.conn(), sess, Some(&default_base));
        let base_ckpt = default_base;
        let turn_end_git_head = real_git_head(&cwd);
        let mcp = McpServer::start_for_session(&db.to_string_lossy(), sess).ok();
        let (runner_state, runner) = register_runner(&store, db, &cwd, sess);
        Ok(Self {
            store,
            sess,
            cwd,
            user,
            forced_bridge: None,
            log_permissions: false,
            checkpoint,
            models: std::collections::HashMap::new(),
            route_preset: Default::default(),
            base_ckpt,
            turn_end_git_head,
            last_turn_guard: None,
            agent_turn_start_guard: None,
            mcp,
            memory: None,
            runner_state,
            runner,
            lead_policy: None,
            repo_map: Default::default(),
            plan_turn: false,
            computer_use: false,
            claude_chrome: false,
        })
    }

    /// Reattach to an existing session in an existing store.
    pub fn open(db: &Path, session_id: i64) -> Result<Self> {
        chorus_acp::env::repair_process_path();
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
        let (checkpoint, default_base) = open_checkpoint(&cwd);
        let stored_base = usage::review_base(store.conn(), session_id).ok().flatten();
        let base_ckpt = if let Some(stored) = stored_base {
            stored
        } else {
            let _ = usage::set_review_base(store.conn(), session_id, Some(&default_base));
            default_base
        };
        let turn_end_git_head = real_git_head(&cwd);
        let mcp = McpServer::start_for_session(&db.to_string_lossy(), session_id).ok();
        let (runner_state, runner) = register_runner(&store, db, &cwd, session_id);
        Ok(Self {
            store,
            sess: session_id,
            cwd,
            user,
            forced_bridge: None,
            log_permissions: false,
            checkpoint,
            models: std::collections::HashMap::new(),
            route_preset: Default::default(),
            base_ckpt,
            turn_end_git_head,
            last_turn_guard: None,
            agent_turn_start_guard: None,
            mcp,
            memory: None,
            runner_state,
            runner,
            lead_policy: None,
            repo_map: Default::default(),
            plan_turn: false,
            computer_use: false,
            claude_chrome: false,
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

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Force one ACP command for every agent (tests, or a pinned install).
    pub fn with_bridge(mut self, cmd: Vec<String>) -> Self {
        if let Ok(mut s) = self.runner_state.lock() {
            s.forced_bridge = Some(cmd.clone());
        }
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

    /// The checkpoint sha this session diffs against. The system monitor's
    /// checkpoint prune must never drop it while the session is open.
    pub fn checkpoint_base(&self) -> &str {
        &self.base_ckpt
    }

    pub fn review_base(&self) -> String {
        usage::review_base(self.store.conn(), self.sess)
            .ok()
            .flatten()
            .unwrap_or_else(|| self.base_ckpt.clone())
    }

    pub fn set_review_base(&mut self, sha: Option<&str>) -> Result<()> {
        usage::set_review_base(self.store.conn(), self.sess, sha)?;
        if let Some(s) = sha {
            self.base_ckpt = s.to_string();
        }
        Ok(())
    }

    /// Mark reviewed: snapshots the shadow repo as the new baseline.
    /// Available in git and non-git projects, never touches user's git.
    pub fn mark_reviewed(&mut self) -> Result<String> {
        let ck = self.checkpoint.as_ref().context("no shadow repo")?;
        let new_base = ck.snapshot("mark reviewed")?;
        self.set_review_base(Some(&new_base))?;
        self.turn_end_git_head = real_git_head(&self.cwd);
        Ok(new_base)
    }

    /// Check if user made a commit between turns; if so, advance baseline.
    pub fn check_user_commit(&mut self) -> Result<bool> {
        let Some(last_head) = &self.turn_end_git_head else {
            self.turn_end_git_head = real_git_head(&self.cwd);
            return Ok(false);
        };
        let current_head = real_git_head(&self.cwd);
        if let (Some(cur), true) = (&current_head, current_head.as_ref() != Some(last_head)) {
            if let Some(ck) = &self.checkpoint {
                let new_base = ck.snapshot("baseline moved by user commit")?;
                self.set_review_base(Some(&new_base))?;
                self.turn_end_git_head = Some(cur.clone());
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn resolve_scope_base(&self, scope: &str) -> String {
        match scope.trim().to_lowercase().as_str() {
            "latest_turn" | "latest-turn" | "turn" | "latestturn" => self
                .last_turn_guard
                .clone()
                .unwrap_or_else(|| self.review_base()),
            "current_agent" | "current-agent" | "agent" | "currentagent" => self
                .agent_turn_start_guard
                .clone()
                .unwrap_or_else(|| self.review_base()),
            _ => self.review_base(),
        }
    }

    /// The workspace diff since the session's review baseline or scope.
    pub fn working_diff(&self) -> String {
        self.working_diff_scoped("session", false)
    }

    pub fn working_diff_scoped(&self, scope: &str, include_projected: bool) -> String {
        let base = self.resolve_scope_base(scope);
        self.checkpoint
            .as_ref()
            .and_then(|ck| ck.diff_since_user_opts(&base, include_projected).ok())
            .unwrap_or_default()
    }

    pub fn diff_stats(
        &self,
        scope: &str,
        include_projected: bool,
    ) -> Result<chorus_core::checkpoint::DiffSummaryReport> {
        let base = self.resolve_scope_base(scope);
        let ck = self.checkpoint.as_ref().context("no shadow repo")?;
        Ok(ck.diff_file_stats(&base, include_projected)?)
    }

    pub fn file_diff(&self, scope: &str, path: &str) -> Result<String> {
        let base = self.resolve_scope_base(scope);
        let ck = self.checkpoint.as_ref().context("no shadow repo")?;
        Ok(ck.file_diff(&base, path)?)
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
        // Agent files the projector writes are ours, not the agent's. With
        // orchestra children sharing the tree, a sibling's projection lands
        // in the middle of this turn; blaming the agent for it failed the
        // turn (and the read-only Seatbelt profile already stops the agent
        // itself from writing them).
        if !files.is_empty() && files.iter().all(|f| is_projected_file(f)) {
            return Ok(());
        }
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
        if let Ok(mut s) = self.runner_state.lock() {
            s.parent_policy = self.lead_policy.unwrap_or(policy);
        }
        let _computer = computer::turn(self.store.conn(), self.sess, self.computer_access(policy));
        self.check_user_commit().ok();
        let guard = self.snapshot("turn guard").ok();
        self.last_turn_guard = guard.clone();
        if self.agent_turn_start_guard.is_none() {
            self.agent_turn_start_guard = guard;
        }

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
        self.announce(user_seq);
        self.mark_rewind_point(user_seq, user_text);
        let bridge = self.resolve_bridge(agent_name, &[]);
        let outcome = self.one_turn(agent_name, &bridge, &prompt, policy, attachment_ids)?;
        let agent_seqs = self.persist_outcome(&outcome, agent_actor, user_seq)?;
        self.turn_end_git_head = real_git_head(&self.cwd);

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
        self.announce(handoff_seq);

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

    /// Plan mode: one relay turn under `Policy::ReadOnly` whose prompt asks
    /// for a plan instead of changes. The user approves it by sending
    /// [`PLAN_APPROVED_PROMPT`] with their usual permission.
    pub fn run_plan_with_attachments(
        &mut self,
        chain: &[AgentSpec],
        user_text: &str,
        attachment_ids: &[i64],
    ) -> Result<RelayReport> {
        self.plan_turn = true;
        let r = self.run_relay_with_attachments(chain, user_text, Policy::ReadOnly, attachment_ids);
        self.plan_turn = false;
        r
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

        let result: Result<RelayReport> = match self.run_turn_with_attachments(
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
                } else if HandoffReason::from_failure(&msg).is_none() {
                    crate::limits::record_unclassified_failure(&first.name, &msg);
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
                    if matches!(
                        reason,
                        HandoffReason::RateLimit | HandoffReason::QuotaExhausted
                    ) {
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

    /// The last usage this session reported (`session_usage`), so a reopened
    /// session's context meter starts where it left off instead of empty.
    pub fn usage(&self) -> Result<Option<usage::SessionUsage>> {
        Ok(usage::last_known(self.store.conn(), self.sess)?)
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
                format!(
                    "edited {}: {} (+{more} more)",
                    touched.len(),
                    shown.join(", ")
                )
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
            .filter(|(e, body)| e.kind != EventKind::ToolResult || !is_bare_success(body))
            .collect();

        let n = bodies.len();
        let turns: Vec<TurnItem> = bodies
            .iter()
            .enumerate()
            .map(|(i, (e, body))| {
                // Storage ≠ context (rule 2.1): a handoff is stored as the full
                // brief but costs one line here, on every rung.
                if e.kind == EventKind::Handoff {
                    if let Some(line) = chorus_core::handoff::context_line(body) {
                        return TurnItem {
                            text_l1: Some(line.clone()),
                            text_l2: Some(line.clone()),
                            text_l0: line,
                            age: n - 1 - i,
                            key: None,
                        };
                    }
                }
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

        let mut code = code;
        if let Some(map) = self.repo_map_context(budget) {
            code.insert(
                0,
                ContextItem {
                    text: map,
                    key: None,
                    label: "repo-map".into(),
                },
            );
        }

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

    /// The workspace's symbols, ranked towards the files this session has
    /// touched, in at most a third of the code tier — so the agent starts
    /// from a map instead of spending its first tool calls on `grep`.
    /// `None` without a shadow repo, with nothing to map, or with
    /// `CHORUS_REPO_MAP=off`. Optional context: a failure here leaves the
    /// turn without a map rather than without a turn.
    fn repo_map_context(&self, budget: i64) -> Option<String> {
        if std::env::var("CHORUS_REPO_MAP").is_ok_and(|v| v == "off") {
            return None;
        }
        let ck = self.checkpoint.as_ref()?;
        let files = ck.files().ok()?;
        let seeds = ck.files_since(&self.base_ckpt).unwrap_or_default();
        let cap = (Budget::new(budget).cap(Tier::Code) / 3).min(REPO_MAP_MAX_TOKENS);
        let mut cache = self.repo_map.lock().ok()?;
        let map = cache.render(ck.work_tree(), &files, &seeds, cap, &HeuristicCounter);
        (!map.is_empty()).then_some(map)
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
            self.models
                .insert(agent_name.to_string(), model_id.to_string());
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

    /// Set how Auto routing leans for this session's sends and, once
    /// orchestra mode exists, for every child it routes.
    pub fn set_route_preset(&mut self, preset: chorus_core::router::RoutePreset) {
        self.route_preset = preset;
        if let Ok(mut s) = self.runner_state.lock() {
            s.route_preset = preset;
        }
    }

    /// Cancel the orchestrating session's children and reap their process
    /// groups — only theirs: other sessions' agents keep running.
    pub fn cancel_orchestration(&mut self) -> Result<()> {
        chorus_core::agent_control::cancel_orchestration(self.store.conn(), self.sess)?;
        Ok(())
    }

    /// Agent processes the current run of child `subagent_id` has alive.
    pub fn child_pids(&self, subagent_id: i64) -> Vec<u32> {
        self.runner_state
            .lock()
            .ok()
            .and_then(|s| s.tags.get(&subagent_id).copied())
            .map(chorus_acp::scope::pids)
            .unwrap_or_default()
    }

    /// The caps this session's orchestration runs under (Settings).
    pub fn set_orchestra_caps(&mut self, caps: chorus_core::agent_control::OrchestraCaps) {
        let caps = chorus_core::agent_control::OrchestraCaps {
            max_concurrency: caps.max_concurrency.clamp(1, 8),
            max_children: caps.max_children.max(1),
            max_wall_time_secs: caps.max_wall_time_secs.max(1),
        };
        if let Ok(mut s) = self.runner_state.lock() {
            s.caps = caps;
        }
    }

    /// Fix who may run this session's children and its orchestrator, instead
    /// of asking `agents::list_agents()`. For tests and pinned installs.
    pub fn set_child_candidates(&mut self, candidates: Vec<chorus_core::router::Candidate>) {
        if let Ok(mut s) = self.runner_state.lock() {
            s.candidates = Some(candidates);
        }
    }

    fn ready_candidates(&self) -> Vec<chorus_core::router::Candidate> {
        self.runner_state
            .lock()
            .ok()
            .and_then(|s| s.candidates.clone())
            .unwrap_or_else(|| crate::orchestra::candidates_from(&crate::agents::list_agents()))
    }

    /// One orchestra-mode turn: the orchestrator answers, plans and spawns.
    ///
    /// Everything about *how* is decided here, not in the shell:
    /// - who leads: `primary` from Settings (`""` = Claude Opus, medium), then
    ///   `backup` (`""` = the strongest ready model from another provider), as
    ///   a relay chain, so a limit hands off with the usual brief;
    /// - that it never writes: the turn runs under [`Policy::ReadOnly`], which
    ///   puts the bridge in the read-only Seatbelt profile and reverts anything
    ///   that still lands in the workspace;
    /// - what its children may do: `user_policy`, the session's permission —
    ///   never the orchestrator's read-only one, never wider than the user's.
    pub fn run_orchestrator_turn(
        &mut self,
        user_text: &str,
        attachment_ids: &[i64],
        primary: &str,
        backup: &str,
        user_policy: Policy,
    ) -> Result<OrchestraReport> {
        let lead = crate::orchestra::lead_chain(&self.ready_candidates(), primary, backup);
        anyhow::ensure!(
            !lead.is_empty(),
            "ไม่มีโมเดลที่พร้อมเป็น orchestrator — login อย่างน้อยหนึ่งบัญชีที่ ตั้งค่า → บัญชี"
        );
        let chain: Vec<AgentSpec> = lead
            .iter()
            .map(|l| AgentSpec::new(l.agent.clone(), crate::orchestra::provider_for(&l.agent)))
            .collect();
        let saved_models = self.models.clone();
        for l in &lead {
            self.set_model(&l.agent, &l.model);
        }
        self.lead_policy = Some(user_policy);
        let out =
            self.run_relay_with_attachments(&chain, user_text, Policy::ReadOnly, attachment_ids);
        self.lead_policy = None;
        self.models = saved_models;
        Ok(OrchestraReport { relay: out?, lead })
    }

    pub fn route_preset(&self) -> chorus_core::router::RoutePreset {
        self.route_preset
    }

    /// What model is pinned for `agent_name`, if any.
    pub fn model_for(&self, agent_name: &str) -> Option<&str> {
        self.models.get(agent_name).map(String::as_str)
    }

    pub fn log_permissions(mut self, on: bool) -> Self {
        self.log_permissions = on;
        self
    }

    /// Every streamed token, tool and usage signal of the turn is tagged with
    /// this session, and the shell hears `status` when an agent starts and
    /// stops answering here.
    fn one_turn_decide(
        &self,
        agent_name: &str,
        bridge: &[String],
        text: &str,
        mode: &str,
        attachment_ids: &[i64],
        decide: &mut dyn FnMut(&PermissionAsk) -> Permission,
    ) -> Result<TurnOutcome> {
        let head = self.head_seq();
        let _scope = chorus_acp::push_scope(self.sess, head);
        chorus_acp::push(self.sess, head, "status", "running");
        let out =
            self.one_turn_decide_inner(agent_name, bridge, text, mode, attachment_ids, decide);
        let status = if out.is_ok() { "idle" } else { "failed" };
        chorus_acp::push(self.sess, head, "status", status);
        out
    }

    fn one_turn_decide_inner(
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
        let mut servers: Vec<serde_json::Value> =
            composed.servers.into_iter().map(|s| s.entry).collect();
        if let Some(m) = &self.mcp {
            // Per-agent bearer secret: scopes the caller to this session and
            // attributes its control-surface writes to this agent.
            servers.push(m.mcp_server_entry(agent_name)?);
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
                anyhow::ensure!(
                    mode != "read-only",
                    "Cannot enforce read-only sandbox: {why}"
                );
                if self.log_permissions {
                    eprintln!("  ⚠  ไม่ได้เปิด sandbox ({why}) — เหลือแค่ย้อนไฟล์หลังเทิร์น");
                }
                bridge.to_vec()
            }
        };

        // Codex applies its own Seatbelt profile for every mode except
        // `agent-full-access`, and macOS refuses a sandbox inside a sandbox
        // (`sandbox_apply: Operation not permitted`) — every shell command it
        // ran died. When ours is the wall around the bridge, ask Codex not to
        // build a second one; ours still confines writes.
        let sandboxed = command.as_slice() != bridge;
        let mode = if sandboxed && crate::accounts::split(agent_name).0 == "codex" {
            "agent-full-access"
        } else {
            mode
        };

        let mut env = crate::accounts::env_for(agent_name);
        env.extend(provider_browser_env(agent_name, self.claude_chrome));
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
        // Agents skip a tool they were only told about at connect time and
        // answer "remembered" from their own head, so the rule rides with
        // every turn. Costs one line; the fact then reaches the Memory pane.
        if self.mcp.is_some() {
            prompt_text.push_str(
                "\n\n(Chorus: if the user asks you to remember something, or states a durable \
                 project fact, call the `memory_propose` tool with it. Do not just say you \
                 remembered it.)\n\
                 (Chorus: Multi-agent orchestration is enabled. For complex, multi-step, or exploratory \
                 tasks, you can act as Lead Orchestrator: decompose objectives, spawn subagents via \
                 `chorus_spawn_agent` (roles: investigator, builder, reviewer, or custom roles in \
                 isolated git worktrees), coordinate them with `chorus_wait_agent` and \
                 `chorus_collect_results`, and track your progress with `chorus_update_checklist`.)\n",
            );
        }
        if self.plan_turn {
            prompt_text.push_str(PLAN_MODE_GUIDANCE);
        }
        let available: Vec<_> = attachments::list(self.store.conn(), self.sess)?
            .into_iter()
            .filter(|a| a.status == "sent")
            .collect();
        if !available.is_empty() {
            prompt_text.push_str("\n\nSession files available through Chorus MCP tools attachment_list and attachment_read:\n");
            for a in &available {
                let safe_name: String = a.name.chars().filter(|c| !c.is_control()).collect();
                prompt_text.push_str(&format!(
                    "- id {}: {} ({}, {} bytes)\n",
                    a.id, safe_name, a.media_type, a.size
                ));
            }
            prompt_text.push_str(
                "Open older files only when needed; a listed name is not the file content.\n",
            );
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
                anyhow::bail!(
                    "attachment {} ({mime}) is not readable by this agent",
                    meta.name
                );
            }
        }
        // Persist every `usage_update` as it lands, so a second (read-only)
        // handle sees the meter move mid-turn. A separate connection: this
        // one is not ours to borrow from inside the agent's callback.
        let observer = self
            .store
            .conn()
            .path()
            .filter(|p| !p.is_empty())
            .and_then(|path| {
                let store = Store::open(Path::new(path)).ok()?;
                let _ = store
                    .conn()
                    .busy_timeout(std::time::Duration::from_millis(200));
                let sess = self.sess;
                Some(Box::new(move |used, window| {
                    let _ = usage::record_context(store.conn(), sess, used, window);
                }) as chorus_acp::UsageObserver)
            });
        let previous = chorus_acp::set_usage_observer(observer);
        let out = agent.prompt_blocks(blocks, decide).context("agent turn");
        chorus_acp::set_usage_observer(previous);
        out
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
        // Every action that changes something goes through the same dialog
        // as a file write (the shell registers it with `set_computer_asker`).
        let access = if self.computer_use {
            computer::Access::Ask
        } else {
            computer::Access::Off
        };
        let _computer = computer::turn(self.store.conn(), self.sess, access);
        self.check_user_commit().ok();
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
        self.announce(user_seq);
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
        self.last_turn_guard = guard.clone();
        // After this turn's snapshot, not before: bound earlier, the message
        // got the previous turn's sha and a rewind lost that turn's edits.
        self.mark_rewind_point(user_seq, user_text);
        if self.agent_turn_start_guard.is_none() {
            self.agent_turn_start_guard = guard.clone();
        }
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
        self.turn_end_git_head = real_git_head(&self.cwd);
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
        // Provider subagents first, so the parent's spawn and result events
        // can name the child session their transcript lives in.
        let children = self.persist_provider_children(outcome, agent_actor)?;
        let mut seqs = Vec::new();
        // The bridge's turn cost goes on one event only, or the session total
        // would count it once per message.
        let mut cost = outcome.cost_usd;
        for ev in &outcome.events {
            let (kind, body) = event_row(ev, &children);
            let mut ne = NewEvent::new(self.sess, kind, agent_actor, body).parent(parent);
            if matches!(ev, AgentEvent::Message(_)) {
                ne.tok_out = outcome.context_used;
                ne.model_id = outcome.model.clone();
                ne.cost_usd = cost.take();
            }
            seqs.push(ledger::append(c, &ne)?);
        }
        // Bridges report context pressure, not an in/out split, so the token
        // columns stay at what is known (nothing yet); cost adds to the total.
        usage::record_turn(
            c,
            self.sess,
            outcome.context_used,
            outcome.context_window,
            0,
            0,
            outcome.cost_usd,
        )?;
        if let Some(&last) = seqs.last() {
            self.announce(last);
        }
        Ok(seqs)
    }

    /// Store each provider subagent of the turn as its own `kind=provider`
    /// child session with its whole transcript. The parent keeps only the
    /// spawn and the final result (its [`AgentEvent::SubagentState`]), which
    /// is all its assembled context ever needs; a nested child hangs under
    /// the child that launched it. Returns bridge key → child session id.
    fn persist_provider_children(
        &self,
        outcome: &TurnOutcome,
        agent_actor: i64,
    ) -> Result<std::collections::HashMap<String, i64>> {
        use chorus_core::agent_control::{record_provider_subagent, SubagentStatus};
        let c = self.store.conn();
        let mut sessions = std::collections::HashMap::new();
        // `children` is in spawn order, so a parent is always recorded first.
        for child in &outcome.children {
            let parent_sess = sessions
                .get(&child.parent_session_id)
                .copied()
                .unwrap_or(self.sess);
            let status =
                SubagentStatus::from_str(&child.state).unwrap_or(SubagentStatus::Completed);
            let sub = record_provider_subagent(
                c,
                parent_sess,
                &child.name,
                &child.task,
                status,
                (!child.message.is_empty()).then_some(child.message.as_str()),
            )?;
            let Some(child_sess) = sub.child_sess else {
                continue;
            };
            sessions.insert(child.session_id.clone(), child_sess);
            let mut prev: Option<i64> = None;
            if !child.task.is_empty() {
                // What the child was asked, as the parent agent said it.
                let ne = NewEvent::new(child_sess, EventKind::Msg, agent_actor, child.task.clone());
                prev = Some(ledger::append(c, &ne)?);
            }
            for ev in &child.events {
                let (kind, body) = event_row(ev, &sessions);
                let mut ne = NewEvent::new(child_sess, kind, agent_actor, body);
                if let Some(p) = prev {
                    ne = ne.parent(p);
                }
                prev = Some(ledger::append(c, &ne)?);
            }
            // Claude's answer is the `Task` tool's result, which need not be
            // the child's last chunk of prose; the child's own transcript
            // ends with what it handed back.
            let last_msg = child.events.iter().rev().find_map(|e| match e {
                AgentEvent::Message(m) => Some(m.trim()),
                _ => None,
            });
            if !child.message.trim().is_empty() && last_msg != Some(child.message.trim()) {
                let mut ne = NewEvent::new(
                    child_sess,
                    EventKind::Msg,
                    agent_actor,
                    child.message.clone(),
                );
                if let Some(p) = prev {
                    ne = ne.parent(p);
                }
                ledger::append(c, &ne)?;
            }
        }
        Ok(sessions)
    }

    /// Tell the shell this session's ledger now reaches `seq`. A hint only:
    /// the shell pulls the rows itself on its read-only handle.
    /// Let agents see and drive this Mac through the `computer` MCP tool.
    /// What each turn may then do follows its policy (`computer_access`).
    pub fn set_computer_use(&mut self, on: bool) {
        self.computer_use = on;
        computer::set_enabled(self.store.conn(), self.sess, on);
    }

    /// Claude uses the user's Chrome through its own extension (Settings).
    pub fn set_claude_chrome(&mut self, on: bool) {
        self.claude_chrome = on;
    }

    /// Who answers "may the agent click here?" during an interactive turn.
    pub fn set_computer_asker(&self, asker: Option<computer::Asker>) {
        computer::set_asker(self.store.conn(), self.sess, asker);
    }

    /// A turn that promises not to change anything (plan mode, read-only,
    /// the orchestrator's own turn) may only look; "allow" may act.
    fn computer_access(&self, policy: Policy) -> computer::Access {
        if !self.computer_use {
            computer::Access::Off
        } else if self.plan_turn || self.lead_policy.is_some() || policy.restrictive() {
            computer::Access::Look
        } else {
            computer::Access::Act
        }
    }

    /// Remember the workspace as it was when the user sent `seq` — the turn
    /// guard taken just before it — so the message can later be rewound to.
    /// A failed bind only costs that message its code rewind; never the turn.
    fn mark_rewind_point(&self, seq: i64, text: &str) {
        let Some(sha) = self.last_turn_guard.as_deref().filter(|s| !s.is_empty()) else {
            return;
        };
        let label: String = text.chars().take(80).collect();
        if let Err(e) = CheckpointDag::bind(&self.store, self.sess, seq, sha, &label) {
            eprintln!("chorus: no rewind point for message {seq}: {e}");
        }
    }

    /// Go back to just before the user's message `seq`. With `conversation`,
    /// the chat continues in a fork that ends right before it — nothing is
    /// deleted and the original stays one click away (rule 2.2). With `code`,
    /// the workspace returns to how it was when that message was sent, after
    /// a snapshot of the current tree so the rewind itself can be undone.
    pub fn rewind_before(
        &mut self,
        seq: i64,
        code: bool,
        conversation: bool,
    ) -> Result<RewindReport> {
        anyhow::ensure!(code || conversation, "nothing to rewind");
        let c = self.store.conn();
        let hist = ledger::history(c, self.sess, 1_000_000)?;
        let i = hist
            .iter()
            .position(|e| e.seq == seq)
            .context("that message is not in this session")?;
        let ev = &hist[i];
        anyhow::ensure!(
            ev.kind == EventKind::Msg && ev.actor == self.user,
            "only a message the user sent can be rewound to"
        );
        let text = ledger::body(c, ev)?;

        let (mut backup, mut code_restored) = (None, false);
        if code {
            let ck = self
                .checkpoint
                .as_ref()
                .context("this workspace has no shadow repo, so its files cannot be rewound")?;
            let rec = CheckpointDag::get_turn_checkpoint(&self.store, seq)?.context(
                "no file snapshot for this message (sent before rewind existed) — \
                 rewind the conversation only",
            )?;
            backup = Some(ck.snapshot("before rewind")?);
            ck.restore(&rec.commit_sha)?;
            code_restored = true;
        }

        if !conversation {
            return Ok(RewindReport {
                session_id: self.sess,
                text,
                code_restored,
                backup,
            });
        }
        // Fork right before the message; seqs are global, so the previous
        // event may live in a parent session and the fork still reads it.
        let at = if i > 0 { hist[i - 1].seq } else { seq - 1 };
        let title: String = c
            .query_row(
                "SELECT title FROM session WHERE id = ?1",
                [self.sess],
                |r| r.get(0),
            )
            .unwrap_or_default();
        let fork = ledger::fork(c, self.sess, at, &format!("{title} (ย้อนกลับ)"))?;
        Ok(RewindReport {
            session_id: fork,
            text,
            code_restored,
            backup,
        })
    }

    fn announce(&self, seq: i64) {
        chorus_acp::push(self.sess, seq, "events", "");
    }

    /// Newest seq written in this session (0 = none).
    fn head_seq(&self) -> i64 {
        head_seq(&self.store, self.sess)
    }

    /// A read-only view of this session for the shell's second handle: it
    /// pulls deltas, usage and the live diff while a turn holds this one.
    /// Opening it touches nothing — no snapshot, no migration.
    pub fn reader(&self) -> Result<SessionReader> {
        let path = self
            .store
            .conn()
            .path()
            .filter(|p| !p.is_empty())
            .context("reader needs a file-backed store")?
            .to_string();
        Ok(SessionReader {
            store: Store::open_read_only(Path::new(&path)).context("open read-only store")?,
            sess: self.sess,
            checkpoint: Checkpoint::open(&self.cwd).ok(),
            base_ckpt: self.base_ckpt.clone(),
        })
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
    pub fn transcript_labeled(&self) -> Result<Vec<LabeledEvent>> {
        self.events_since(0)
    }

    /// Labeled events with `seq > after` — the delta a UI polls for so it
    /// only ever pulls what it hasn't seen.
    pub fn events_since(&self, after: i64) -> Result<Vec<LabeledEvent>> {
        labeled_since(&self.store, self.sess, after)
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

/// `(seq, kind name, actor name, body, model id, cost_usd)` — the row shape the FFI
/// and UI want without exposing core types.
pub type LabeledEvent = (
    i64,
    &'static str,
    String,
    String,
    Option<String>,
    Option<f64>,
);

/// Events in `sess`'s history (fork chain included) with `seq > after`.
/// Filters before resolving bodies, so a delta pull never decompresses the
/// blobs of rows the caller already has.
fn labeled_since(store: &Store, sess: i64, after: i64) -> Result<Vec<LabeledEvent>> {
    let c = store.conn();
    let mut out = Vec::new();
    for e in ledger::history(c, sess, 10_000)? {
        if e.seq <= after {
            continue;
        }
        let body = ledger::body(c, &e)?;
        let actor = actor::name(c, e.actor).unwrap_or_else(|_| "?".into());
        out.push((
            e.seq,
            kind_name(e.kind),
            actor,
            body,
            e.model_id,
            e.cost_usd,
        ));
    }
    Ok(out)
}

fn head_seq(store: &Store, sess: i64) -> i64 {
    store
        .conn()
        .query_row(
            "SELECT COALESCE(MAX(seq), 0) FROM event WHERE sess = ?1",
            [sess],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
}

/// The shell's second handle onto one session: read-only SQLite (WAL lets it
/// read while the turn writes on the [`Session`]'s connection) plus the shadow
/// repo for a diff that never writes the shared index. Nothing here writes,
/// so it can live on its own thread beside a running turn (CLAUDE.md 7.7).
pub struct SessionReader {
    store: Store,
    sess: i64,
    checkpoint: Option<Checkpoint>,
    base_ckpt: String,
}

impl SessionReader {
    pub fn session_id(&self) -> i64 {
        self.sess
    }

    /// The delta since `after` — what a push said is waiting.
    pub fn events_since(&self, after: i64) -> Result<Vec<LabeledEvent>> {
        labeled_since(&self.store, self.sess, after)
    }

    /// Newest seq in this session (0 = none).
    pub fn head_seq(&self) -> i64 {
        head_seq(&self.store, self.sess)
    }

    pub fn usage(&self) -> Result<Option<usage::SessionUsage>> {
        Ok(usage::last_known(self.store.conn(), self.sess)?)
    }

    pub fn review_base(&self) -> String {
        usage::review_base(self.store.conn(), self.sess)
            .ok()
            .flatten()
            .unwrap_or_else(|| self.base_ckpt.clone())
    }

    /// Same patch as [`Session::working_diff`], staged in a private index.
    pub fn working_diff(&self) -> String {
        self.working_diff_scoped("session", false)
    }

    pub fn working_diff_scoped(&self, _scope: &str, include_projected: bool) -> String {
        let base = self.review_base();
        self.checkpoint
            .as_ref()
            .and_then(|ck| ck.peek_diff_since_user_opts(&base, include_projected).ok())
            .unwrap_or_default()
    }

    pub fn diff_stats(
        &self,
        _scope: &str,
        include_projected: bool,
    ) -> Result<chorus_core::checkpoint::DiffSummaryReport> {
        let base = self.review_base();
        let ck = self.checkpoint.as_ref().context("no shadow repo")?;
        Ok(ck.peek_diff_file_stats(&base, include_projected)?)
    }

    pub fn file_diff(&self, _scope: &str, path: &str) -> Result<String> {
        let base = self.review_base();
        let ck = self.checkpoint.as_ref().context("no shadow repo")?;
        Ok(ck.peek_file_diff(&base, path)?)
    }
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
    pub cost_usd: Option<f64>,
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
        cost_usd: Option<f64>,
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
             COUNT(e.seq) AS events, COALESCE(MAX(e.t), 0) AS last_ms, \
             u.cost_usd \
             FROM session s LEFT JOIN event e ON e.sess = s.id \
             LEFT JOIN session_usage u ON u.sess = s.id \
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
                cost_usd: r.get(7)?,
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
                cost_usd: b.cost_usd,
            }
        })
        .collect())
}

#[cfg(test)]
mod policy_tests {
    use super::*;

    #[test]
    fn read_only_never_grants_unknown_or_mixed_permission_kinds() {
        for kind in [
            "read/write",
            "read-write",
            "read_and_delete",
            "thread",
            "execute",
            "",
            "other",
        ] {
            let ask = PermissionAsk {
                tool: "fixture".into(),
                kind: kind.into(),
                detail: "synthetic request".into(),
            };
            assert_eq!(
                Policy::ReadOnly.decide(&ask),
                Permission::Deny,
                "kind: {kind}"
            );
        }
        for kind in ["read", "READ", "fetch", "search"] {
            let ask = PermissionAsk {
                tool: "fixture".into(),
                kind: kind.into(),
                detail: "synthetic request".into(),
            };
            assert_eq!(
                Policy::ReadOnly.decide(&ask),
                Permission::Allow,
                "kind: {kind}"
            );
            assert_eq!(Policy::DenyAll.decide(&ask), Permission::Deny);
        }
    }
}

#[cfg(test)]
mod child_tests {
    use super::*;

    #[test]
    fn child_permission_is_never_wider_than_the_parent() {
        assert_eq!(child_policy(Policy::AllowAll, true), Policy::AllowAll);
        assert_eq!(child_policy(Policy::AllowAll, false), Policy::ReadOnly);
        assert_eq!(child_policy(Policy::ReadOnly, true), Policy::ReadOnly);
        assert_eq!(child_policy(Policy::DenyAll, true), Policy::DenyAll);
        assert_eq!(child_policy(Policy::DenyAll, false), Policy::DenyAll);
    }

    #[test]
    fn diff_stat_counts_untracked_files_and_skips_our_own() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap()
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(dir.path().join("a.rs"), "one\ntwo\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);

        std::fs::write(dir.path().join("a.rs"), "one\nTWO\nthree\n").unwrap();
        std::fs::write(dir.path().join("new.rs"), "fn x() {}\n").unwrap();
        std::fs::create_dir_all(dir.path().join(".chorus/ckpt")).unwrap();
        std::fs::write(dir.path().join(".chorus/ckpt/HEAD"), "x\n").unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "projected\n").unwrap();

        let (stat, files) = worktree_diff_stat(dir.path());
        assert_eq!(files, vec!["a.rs".to_string(), "new.rs".to_string()]);
        assert_eq!(stat, "2 files changed, 3 insertions(+), 1 deletion(-)");
    }

    #[test]
    fn child_prompt_leads_with_the_role() {
        let role = chorus_core::roles::AgentRole::parse(
            "---\nname: r\ndescription: \"d\"\n---\nYou review.",
            "r",
        );
        let p = child_prompt(Some(&role), "check auth");
        assert!(p.ends_with("check auth"), "{p}");
        assert!(p.len() > "check auth".len());
        assert_eq!(child_prompt(None, "t"), "t");
    }
}
