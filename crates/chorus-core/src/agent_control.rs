//! Agent control surface: subagent orchestration, task checklist,
//! interactive reviews, and file jump primitives.
//!
//! Exposes the shared engine primitives used by both the Streamable-HTTP MCP
//! server and the CLI (`chorus agent ...`).

use crate::{ChorusError, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Caps enforced in Rust for orchestra mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrchestraCaps {
    /// Concurrency limit (range 1–8, default 3).
    pub max_concurrency: usize,
    /// Maximum children per orchestration (default 10).
    pub max_children: usize,
    /// Maximum wall time in seconds (default 30 min = 1800s).
    pub max_wall_time_secs: u64,
}

impl Default for OrchestraCaps {
    fn default() -> Self {
        Self {
            max_concurrency: 3,
            max_children: 10,
            max_wall_time_secs: 30 * 60,
        }
    }
}

/// Engine interface to run and cancel child sessions.
pub trait ChildRunner: Send + Sync {
    fn run_child(&self, subagent: &Subagent, weight: Option<&str>) -> Result<()>;
    fn cancel_child(&self, subagent_id: i64) -> Result<()>;
    fn cancel_all(&self, parent_sess: i64) -> Result<()>;
    /// The caps this orchestration runs under (the user's Settings). A spawn
    /// that names none uses these.
    fn caps(&self) -> OrchestraCaps {
        OrchestraCaps::default()
    }
}

/// One runner per orchestrating session, keyed by database file *and*
/// session id: every project's store numbers its sessions from 1. A single
/// global slot used to be overwritten by whichever session opened last —
/// including each child's own session — so a spawn ran under a stranger's
/// policy, preset and database.
type RunnerKey = (String, i64);
static RUNNERS: std::sync::Mutex<Option<HashMap<RunnerKey, Arc<dyn ChildRunner>>>> =
    std::sync::Mutex::new(None);

/// Which store `conn` is, as a stable string. SQLite reports the main
/// database's file; canonicalised so two handles on one file agree.
pub(crate) fn db_key(conn: &Connection) -> String {
    match conn.path().filter(|p| !p.is_empty()) {
        Some(p) => std::fs::canonicalize(p)
            .map(|c| c.to_string_lossy().into_owned())
            .unwrap_or_else(|_| p.to_string()),
        None => format!("memory:{:p}", conn as *const Connection),
    }
}

/// Make `runner` the one that runs children spawned by `sess` in `conn`'s store.
pub fn register_child_runner(conn: &Connection, sess: i64, runner: Arc<dyn ChildRunner>) {
    if let Ok(mut r) = RUNNERS.lock() {
        r.get_or_insert_with(HashMap::new)
            .insert((db_key(conn), sess), runner);
    }
}

/// Forget `sess`'s runner, but only if it is still `runner`: a reopened
/// session registers a new one before the old handle drops.
pub fn unregister_child_runner(conn: &Connection, sess: i64, runner: &Arc<dyn ChildRunner>) {
    let key = (db_key(conn), sess);
    if let Ok(mut r) = RUNNERS.lock() {
        if let Some(map) = r.as_mut() {
            if map.get(&key).is_some_and(|cur| Arc::ptr_eq(cur, runner)) {
                map.remove(&key);
            }
        }
    }
}

/// The runner for children of `parent_sess`, if that session is open.
pub fn child_runner_for(conn: &Connection, parent_sess: i64) -> Option<Arc<dyn ChildRunner>> {
    let key = (db_key(conn), parent_sess);
    RUNNERS
        .lock()
        .ok()
        .and_then(|r| r.as_ref().and_then(|m| m.get(&key).cloned()))
}

fn runner_of(conn: &Connection, sub: &Subagent) -> Option<Arc<dyn ChildRunner>> {
    sub.parent_sess.and_then(|p| child_runner_for(conn, p))
}

/// `weight` the orchestrator gave each child, so a resume after a cap pause
/// routes it the same way. Process memory only: the `subagent` table has no
/// column for it, and after a restart a resumed child routes as `normal`.
static WEIGHTS: std::sync::Mutex<Option<HashMap<RunnerKey, String>>> = std::sync::Mutex::new(None);

fn remember_weight(conn: &Connection, id: i64, weight: Option<&str>) {
    if let (Some(w), Ok(mut m)) = (weight, WEIGHTS.lock()) {
        m.get_or_insert_with(HashMap::new)
            .insert((db_key(conn), id), w.to_string());
    }
}

fn weight_of(conn: &Connection, id: i64) -> Option<String> {
    let key = (db_key(conn), id);
    WEIGHTS
        .lock()
        .ok()
        .and_then(|m| m.as_ref().and_then(|m| m.get(&key).cloned()))
}

/// Summary prefix of a child paused at the wall-time cap. Resuming one of
/// these restarts the orchestration's clock; any other resume leaves it.
pub const WALL_TIME_PAUSE: &str = "Orchestration cap reached: wall time";

/// When the wall clock of `parent_sess`'s orchestration was last (re)started,
/// ms since the epoch. Same process-memory caveat as [`WEIGHTS`].
static CLOCK_RESTARTS: std::sync::Mutex<Option<HashMap<RunnerKey, i64>>> =
    std::sync::Mutex::new(None);

/// When `parent_sess`'s orchestration started counting wall time: its first
/// Chorus child, or the last time the user chose to continue past the cap.
pub fn orchestration_started_ms(conn: &Connection, parent_sess: i64) -> Result<Option<i64>> {
    let key = (db_key(conn), parent_sess);
    let restarted = CLOCK_RESTARTS
        .lock()
        .ok()
        .and_then(|m| m.as_ref().and_then(|m| m.get(&key).copied()));
    if restarted.is_some() {
        return Ok(restarted);
    }
    Ok(conn
        .query_row(
            "SELECT MIN(created_ms) FROM subagent WHERE parent_sess = ?1 AND kind = 'chorus'",
            [parent_sess],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()?
        .flatten())
}

/// The ms timestamp past which `parent_sess`'s children must pause.
pub fn wall_deadline_ms(
    conn: &Connection,
    parent_sess: i64,
    caps: &OrchestraCaps,
) -> Result<Option<i64>> {
    Ok(orchestration_started_ms(conn, parent_sess)?
        .map(|start| start.saturating_add((caps.max_wall_time_secs as i64).saturating_mul(1000))))
}

fn restart_clock(conn: &Connection, parent_sess: i64) {
    if let Ok(mut m) = CLOCK_RESTARTS.lock() {
        m.get_or_insert_with(HashMap::new)
            .insert((db_key(conn), parent_sess), now_ms());
    }
}

/// Pause a running child at the wall-time cap: it waits for the user's
/// Continue instead of failing. Only a `running` row moves.
pub fn pause_for_wall_time(
    conn: &Connection,
    subagent_id: i64,
    caps: &OrchestraCaps,
) -> Result<Subagent> {
    conn.execute(
        "UPDATE subagent SET status = 'waiting', summary = ?1, updated_ms = ?2 \
         WHERE id = ?3 AND status = 'running'",
        params![
            format!(
                "{WALL_TIME_PAUSE} limit ({} min) hit. Paused for user approval.",
                caps.max_wall_time_secs / 60
            ),
            now_ms(),
            subagent_id
        ],
    )?;
    get_subagent(conn, subagent_id)
}

/// Execution status of a spawned subagent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Running,
    /// Paused at a cap or waiting for permission.
    Waiting,
    Completed,
    Failed,
    Cancelled,
}

impl SubagentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "running" => Some(Self::Running),
            "waiting" => Some(Self::Waiting),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// Who started a child session.
///
/// `Chorus` children are real sessions the orchestrator spawned and can steer.
/// `Provider` children are a bridge's own subagents (Claude's `Task` tool,
/// Codex `subagent_spawned`), recorded after the parent turn so their events
/// live in their own session instead of the parent's transcript. They are
/// read-only: no bridge lets a client cancel or message one on its own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentKind {
    #[default]
    Chorus,
    Provider,
}

impl SubagentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Chorus => "chorus",
            Self::Provider => "provider",
        }
    }
}

/// A child agent session tracked by Chorus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subagent {
    pub id: i64,
    pub parent_sess: Option<i64>,
    pub child_sess: Option<i64>,
    #[serde(default)]
    pub kind: SubagentKind,
    pub role: String,
    pub task: String,
    pub worktree: Option<String>,
    pub status: SubagentStatus,
    pub summary: Option<String>,
    pub diff_stat: Option<String>,
    pub files_touched: Vec<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
    #[serde(default)]
    pub model: Option<String>,
}

/// A steering message sent to or from a child agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentMessage {
    pub id: i64,
    pub subagent_id: i64,
    pub direction: String,
    pub content: String,
    pub created_ms: i64,
}

/// Status summary for waiting on child agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitReport {
    pub completed: Vec<i64>,
    pub running: Vec<i64>,
    pub cancelled: Vec<i64>,
    pub failed: Vec<i64>,
    pub all_done: bool,
    pub statuses: HashMap<i64, String>,
}

/// Compact brief for a completed subagent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentBrief {
    pub id: i64,
    pub role: String,
    pub task: String,
    pub status: SubagentStatus,
    pub summary: String,
    pub diff_stat: String,
    pub files_touched: Vec<String>,
    pub worktree: Option<String>,
}

/// Aggregated structured results from completed child agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentResultsBrief {
    pub briefs: Vec<SubagentBrief>,
    pub rendered_markdown: String,
}

/// One item in the live task checklist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub text: String,
    pub done: bool,
    #[serde(default)]
    pub in_progress: bool,
}

/// Navigation directive for Workbench UI to highlight a specific file and line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileJump {
    pub id: i64,
    pub sess: i64,
    pub path: String,
    pub line: Option<u32>,
    pub created_ms: i64,
}

/// Interactive human review request for proposed diff hunks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRequest {
    pub id: i64,
    pub sess: i64,
    pub summary: String,
    pub hunks: serde_json::Value,
    pub status: String,
    pub created_ms: i64,
    pub resolved_ms: Option<i64>,
}

fn row_to_subagent(r: &Row) -> rusqlite::Result<Subagent> {
    let status_str: String = r.get("status")?;
    let status = SubagentStatus::from_str(&status_str).unwrap_or(SubagentStatus::Running);
    let files_json: Option<String> = r.get("files_touched")?;
    let files_touched: Vec<String> = files_json
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    Ok(Subagent {
        id: r.get("id")?,
        parent_sess: r.get("parent_sess")?,
        child_sess: r.get("child_sess")?,
        kind: match r.get::<_, String>("kind")?.as_str() {
            "provider" => SubagentKind::Provider,
            _ => SubagentKind::Chorus,
        },
        role: r.get("role")?,
        task: r.get("task")?,
        worktree: r.get("worktree")?,
        status,
        summary: r.get("summary")?,
        diff_stat: r.get("diff_stat")?,
        files_touched,
        created_ms: r.get("created_ms")?,
        updated_ms: r.get("updated_ms")?,
        model: None,
    })
}

/// Spawn a child agent session with options (role, task, weight, worktree, caps).
#[allow(clippy::too_many_arguments)]
pub fn spawn_agent_with_options(
    conn: &Connection,
    parent_sess: Option<i64>,
    role: &str,
    task: &str,
    weight: Option<&str>,
    worktree: bool,
    cwd: &str,
    caps: Option<OrchestraCaps>,
) -> Result<Subagent> {
    if !crate::roles::is_valid_role_name(role) {
        return Err(ChorusError::Invalid(format!(
            "invalid role name {role:?}: must match [a-z0-9-]{{1,64}}"
        )));
    }

    // Only the orchestrator starts Chorus children; a Chorus child may still
    // have provider subagents, but no Chorus children of its own.
    if let Some(parent) = parent_sess {
        let parent_is_chorus_child: bool = conn
            .query_row(
                "SELECT 1 FROM subagent WHERE child_sess = ?1 AND kind = 'chorus'",
                [parent],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if parent_is_chorus_child {
            return Err(ChorusError::Invalid(
                "a Chorus child cannot start Chorus children; ask the orchestrator".into(),
            ));
        }
    }

    let caps = caps
        .or_else(|| {
            parent_sess
                .and_then(|p| child_runner_for(conn, p))
                .map(|r| r.caps())
        })
        .unwrap_or_default();
    let now = now_ms();

    // Check caps if parent session exists
    let mut initial_status = SubagentStatus::Running;
    let mut initial_summary: Option<String> = None;

    if let Some(parent) = parent_sess {
        let existing = list_subagents(conn, Some(parent))?;
        let existing_orchestrated = orchestrated(existing);

        if existing_orchestrated.len() >= caps.max_children {
            initial_status = SubagentStatus::Waiting;
            initial_summary = Some(format!(
                "Orchestration cap reached: max children limit ({}) hit. Paused for user approval.",
                caps.max_children
            ));
        } else {
            let running_count = existing_orchestrated
                .iter()
                .filter(|a| a.status == SubagentStatus::Running)
                .count();
            if running_count >= caps.max_concurrency {
                initial_status = SubagentStatus::Waiting;
                initial_summary = Some(format!(
                    "Orchestration cap reached: concurrency limit ({}) hit. Paused for user approval.",
                    caps.max_concurrency
                ));
            } else if wall_deadline_ms(conn, parent, &caps)?.is_some_and(|d| now >= d) {
                initial_status = SubagentStatus::Waiting;
                initial_summary = Some(format!(
                    "{WALL_TIME_PAUSE} limit ({} min) hit. Paused for user approval.",
                    caps.max_wall_time_secs / 60
                ));
            }
        }
    }

    // A writing child gets its own worktree or does not start. Falling back
    // to an empty non-git directory used to "work": the child ran with
    // nothing to edit, and its diff could never be reviewed or merged.
    let worktree_path = if worktree {
        let wt = crate::worktrees::create_worktree(Path::new(cwd), role).map_err(|e| {
            ChorusError::Invalid(format!("cannot create a worktree for {role:?}: {e}"))
        })?;
        Some(wt.path)
    } else {
        None
    };

    let child_cwd = worktree_path.as_deref().unwrap_or(cwd);
    let child_sess = if let Some(parent) = parent_sess {
        let title = format!("child-agent-{role}");
        conn.execute(
            "INSERT INTO session(title, cwd, started_ms, forked_from, fork_at_seq)
             VALUES (?1, ?2, ?3, ?4, NULL)",
            params![title, child_cwd, now, parent],
        )?;
        Some(conn.last_insert_rowid())
    } else {
        None
    };

    let status_str = initial_status.as_str();
    conn.execute(
        "INSERT INTO subagent(parent_sess, child_sess, role, task, worktree, status, \
         summary, diff_stat, files_touched, created_ms, updated_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, '[]', ?8, ?8)",
        params![
            parent_sess,
            child_sess,
            role,
            task,
            worktree_path,
            status_str,
            initial_summary,
            now
        ],
    )?;

    let id = conn.last_insert_rowid();
    let subagent = get_subagent(conn, id)?;

    remember_weight(conn, id, weight);
    // If running and its parent's runner is registered, launch it.
    if initial_status == SubagentStatus::Running {
        if let Some(runner) = runner_of(conn, &subagent) {
            if let Err(e) = runner.run_child(&subagent, weight) {
                return fail_agent(conn, id, &format!("Failed to start: {e}"));
            }
        }
    }

    Ok(subagent)
}

/// Spawn a child agent session.
pub fn spawn_agent(
    conn: &Connection,
    parent_sess: Option<i64>,
    role: &str,
    task: &str,
    worktree: bool,
    cwd: &str,
) -> Result<Subagent> {
    spawn_agent_with_options(conn, parent_sess, role, task, None, worktree, cwd, None)
}

/// Record a provider subagent that already ran inside a parent turn.
///
/// Creates its child session (linked by `forked_from` with no fork point, so
/// `ledger::history` of the child is its own events only) and the
/// `kind = 'provider'` row, already in its final `status`. The caller appends
/// the child's events to `child_sess`.
pub fn record_provider_subagent(
    conn: &Connection,
    parent_sess: i64,
    name: &str,
    task: &str,
    status: SubagentStatus,
    summary: Option<&str>,
) -> Result<Subagent> {
    let now = now_ms();
    let cwd: String = conn.query_row(
        "SELECT cwd FROM session WHERE id = ?1",
        [parent_sess],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO session(title, cwd, started_ms, forked_from, fork_at_seq)
         VALUES (?1, ?2, ?3, ?4, NULL)",
        params![name, cwd, now, parent_sess],
    )?;
    let child_sess = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO subagent(parent_sess, child_sess, kind, role, task, worktree, status, \
         summary, diff_stat, files_touched, created_ms, updated_ms)
         VALUES (?1, ?2, 'provider', ?3, ?4, NULL, ?5, ?6, NULL, '[]', ?7, ?7)",
        params![
            parent_sess,
            child_sess,
            name,
            task,
            status.as_str(),
            summary,
            now
        ],
    )?;
    get_subagent(conn, conn.last_insert_rowid())
}

fn attach_model(conn: &Connection, sub: &mut Subagent) {
    if let Some(child) = sub.child_sess {
        let model: rusqlite::Result<String> = conn.query_row(
            "SELECT a.name FROM event e JOIN actor a ON e.actor = a.id WHERE e.sess = ?1 AND a.name != 'user' ORDER BY e.seq ASC LIMIT 1",
            [child],
            |r| r.get(0),
        );
        if let Ok(m) = model {
            sub.model = Some(m);
        }
    }
}

/// Fetch a single subagent by ID.
pub fn get_subagent(conn: &Connection, id: i64) -> Result<Subagent> {
    let mut sub = conn
        .query_row(
            "SELECT id, parent_sess, child_sess, kind, role, task, worktree, status, \
             summary, diff_stat, files_touched, created_ms, updated_ms \
             FROM subagent WHERE id = ?1",
            [id],
            row_to_subagent,
        )
        .map_err(|e| ChorusError::Invalid(format!("subagent #{id} not found: {e}")))?;
    attach_model(conn, &mut sub);
    Ok(sub)
}

/// List all subagents, optionally filtered by parent session.
pub fn list_subagents(conn: &Connection, parent_sess: Option<i64>) -> Result<Vec<Subagent>> {
    let mut stmt = if parent_sess.is_some() {
        conn.prepare(
            "SELECT id, parent_sess, child_sess, kind, role, task, worktree, status, \
             summary, diff_stat, files_touched, created_ms, updated_ms \
             FROM subagent WHERE parent_sess = ?1 ORDER BY id ASC",
        )?
    } else {
        conn.prepare(
            "SELECT id, parent_sess, child_sess, kind, role, task, worktree, status, \
             summary, diff_stat, files_touched, created_ms, updated_ms \
             FROM subagent ORDER BY id ASC",
        )?
    };

    let rows = if let Some(parent) = parent_sess {
        stmt.query_map([parent], row_to_subagent)?
    } else {
        stmt.query_map([], row_to_subagent)?
    };

    let mut out = Vec::new();
    for r in rows {
        let mut sub = r?;
        attach_model(conn, &mut sub);
        out.push(sub);
    }
    Ok(out)
}

/// Whether subagent `agent_id` belongs to `root_sess`'s tree: spawned by it,
/// or by a child session that itself descends from it.
pub fn agent_in_session_tree(conn: &Connection, root_sess: i64, agent_id: i64) -> Result<bool> {
    let mut sess = get_subagent(conn, agent_id)?.parent_sess;
    // Bounded walk: a cycle in hand-edited rows must not hang the server.
    for _ in 0..64 {
        let Some(s) = sess else { return Ok(false) };
        if s == root_sess {
            return Ok(true);
        }
        sess = conn
            .query_row(
                "SELECT parent_sess FROM subagent WHERE child_sess = ?1",
                [s],
                |r| r.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten();
    }
    Ok(false)
}

/// Send a steering message or instruction to a child agent session.
///
/// `actor` is who is speaking — the orchestrating agent when this comes from
/// MCP. It is recorded through [`crate::ledger::append`] so the child's
/// ledger keeps its parent chain and never shows agent text as the user's.
pub fn send_message(
    conn: &Connection,
    subagent_id: i64,
    actor: i64,
    message: &str,
) -> Result<SubagentMessage> {
    // Verify subagent exists
    let agent = get_subagent(conn, subagent_id)?;
    let now = now_ms();

    conn.execute(
        "INSERT INTO subagent_message(subagent_id, direction, content, created_ms)
         VALUES (?1, 'parent_to_child', ?2, ?3)",
        params![subagent_id, message, now],
    )?;
    let id = conn.last_insert_rowid();

    conn.execute(
        "UPDATE subagent SET updated_ms = ?1 WHERE id = ?2",
        params![now, subagent_id],
    )?;

    // If subagent has child session, record a Msg event in the child ledger
    if let Some(child_sess) = agent.child_sess {
        let mut ev =
            crate::types::NewEvent::new(child_sess, crate::types::EventKind::Msg, actor, message);
        if let Some(last) = crate::ledger::tail(conn, child_sess, 1)?.last() {
            ev = ev.parent(last.seq);
        }
        crate::ledger::append(conn, &ev)?;
    }

    Ok(SubagentMessage {
        id,
        subagent_id,
        direction: "parent_to_child".into(),
        content: message.into(),
        created_ms: now,
    })
}

/// The children an orchestrator waits on and collects: its own Chorus
/// children. A provider subagent finished inside some earlier turn and its
/// result already reached the parent as that turn's tool result.
fn orchestrated(agents: Vec<Subagent>) -> Vec<Subagent> {
    agents
        .into_iter()
        .filter(|a| a.kind == SubagentKind::Chorus)
        .collect()
}

/// Wait on a specific child agent, or all child agents of `parent_sess`, with timeout.
pub fn wait_agent(
    conn: &Connection,
    parent_sess: Option<i64>,
    subagent_id: Option<i64>,
    timeout_secs: u64,
) -> Result<WaitReport> {
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(timeout_secs);

    loop {
        let agents = if let Some(id) = subagent_id {
            vec![get_subagent(conn, id)?]
        } else {
            orchestrated(list_subagents(conn, parent_sess)?)
        };

        let mut completed = Vec::new();
        let mut running = Vec::new();
        let mut cancelled = Vec::new();
        let mut failed = Vec::new();
        let mut statuses = HashMap::new();

        for a in &agents {
            statuses.insert(a.id, a.status.as_str().to_string());
            match a.status {
                SubagentStatus::Completed => completed.push(a.id),
                // Waiting is paused, not finished: it keeps `all_done` false.
                SubagentStatus::Running | SubagentStatus::Waiting => running.push(a.id),
                SubagentStatus::Cancelled => cancelled.push(a.id),
                SubagentStatus::Failed => failed.push(a.id),
            }
        }

        let all_done = running.is_empty();
        if all_done || timeout_secs == 0 || start.elapsed() >= timeout {
            return Ok(WaitReport {
                completed,
                running,
                cancelled,
                failed,
                all_done,
                statuses,
            });
        }

        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Mark a child agent as completed with summary, diff statistics, and touched files.
pub fn complete_agent(
    conn: &Connection,
    subagent_id: i64,
    summary: &str,
    diff_stat: Option<&str>,
    files_touched: &[String],
) -> Result<Subagent> {
    let now = now_ms();
    let files_json = serde_json::to_string(files_touched).unwrap_or_else(|_| "[]".into());

    conn.execute(
        "UPDATE subagent SET status = 'completed', summary = ?1, diff_stat = ?2, \
         files_touched = ?3, updated_ms = ?4 WHERE id = ?5",
        params![summary, diff_stat, files_json, now, subagent_id],
    )?;

    get_subagent(conn, subagent_id)
}

/// Mark a child agent as failed with an error summary.
pub fn fail_agent(conn: &Connection, subagent_id: i64, error: &str) -> Result<Subagent> {
    let now = now_ms();
    conn.execute(
        "UPDATE subagent SET status = 'failed', summary = ?1, updated_ms = ?2 WHERE id = ?3",
        params![error, now, subagent_id],
    )?;

    get_subagent(conn, subagent_id)
}

/// Cancel a running child agent session.
pub fn cancel_agent(conn: &Connection, subagent_id: i64, reason: Option<&str>) -> Result<Subagent> {
    let now = now_ms();
    let summary_val = reason.unwrap_or("Cancelled by parent agent");

    conn.execute(
        "UPDATE subagent SET status = 'cancelled', summary = COALESCE(summary, ?1), \
         updated_ms = ?2 WHERE id = ?3",
        params![summary_val, now, subagent_id],
    )?;

    let sub = get_subagent(conn, subagent_id)?;
    if let Some(runner) = runner_of(conn, &sub) {
        let _ = runner.cancel_child(subagent_id);
    }
    Ok(sub)
}

/// Resume a waiting child agent session (e.g. after cap pause or user approval).
///
/// Only a `waiting` child moves: resuming a finished one used to run its task
/// again. Continuing past the wall-time cap restarts the orchestration clock,
/// otherwise the very next check would pause it again.
pub fn resume_subagent(conn: &Connection, subagent_id: i64) -> Result<Subagent> {
    let before = get_subagent(conn, subagent_id)?;
    if before.status != SubagentStatus::Waiting {
        return Ok(before);
    }
    if let (Some(parent), true) = (
        before.parent_sess,
        before
            .summary
            .as_deref()
            .is_some_and(|s| s.starts_with(WALL_TIME_PAUSE)),
    ) {
        restart_clock(conn, parent);
    }
    let now = now_ms();
    let moved = conn.execute(
        "UPDATE subagent SET status = 'running', summary = NULL, updated_ms = ?1 \
         WHERE id = ?2 AND status = 'waiting'",
        params![now, subagent_id],
    )?;

    let subagent = get_subagent(conn, subagent_id)?;
    if moved == 1 {
        if let Some(runner) = runner_of(conn, &subagent) {
            if let Err(e) = runner.run_child(&subagent, weight_of(conn, subagent_id).as_deref()) {
                return fail_agent(conn, subagent_id, &format!("Failed to start: {e}"));
            }
        }
    }
    Ok(subagent)
}

/// Cancel all running and waiting children belonging to parent orchestration.
pub fn cancel_orchestration(conn: &Connection, parent_sess: i64) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "UPDATE subagent SET status = 'cancelled', summary = COALESCE(summary, 'Cancelled by parent orchestration'), \
         updated_ms = ?1 WHERE parent_sess = ?2 AND status IN ('running', 'waiting')",
        params![now, parent_sess],
    )?;

    if let Some(runner) = child_runner_for(conn, parent_sess) {
        let _ = runner.cancel_all(parent_sess);
    }

    Ok(())
}

/// Collect structured results and handoff briefs from child sessions.
pub fn collect_results(
    conn: &Connection,
    parent_sess: Option<i64>,
    agent_ids: Option<&[i64]>,
    join_mode: &str,
) -> Result<SubagentResultsBrief> {
    let target_agents: Vec<Subagent> = if let Some(ids) = agent_ids {
        let mut list = Vec::new();
        for id in ids {
            list.push(get_subagent(conn, *id)?);
        }
        list
    } else {
        orchestrated(list_subagents(conn, parent_sess)?)
            .into_iter()
            .filter(|a| a.status != SubagentStatus::Running)
            .collect()
    };

    let briefs: Vec<SubagentBrief> = target_agents
        .into_iter()
        .map(|a| SubagentBrief {
            id: a.id,
            role: a.role,
            task: a.task,
            status: a.status,
            summary: a.summary.unwrap_or_else(|| "No summary reported".into()),
            diff_stat: a.diff_stat.unwrap_or_else(|| "0 files changed".into()),
            files_touched: a.files_touched,
            worktree: a.worktree,
        })
        .collect();

    let mut rendered = String::from("### Subagent Results\n\n");
    if briefs.is_empty() {
        rendered.push_str("No subagent results available to collect.\n");
    } else {
        rendered.push_str("| Agent | Role | Status | Files Touched | Diff Stat | Summary |\n");
        rendered.push_str("| --- | --- | --- | --- | --- | --- |\n");
        for b in &briefs {
            let files = if b.files_touched.is_empty() {
                "none".to_string()
            } else {
                b.files_touched.join(", ")
            };
            let summary_clean = b.summary.replace('|', "/").replace('\n', " ");
            rendered.push_str(&format!(
                "| #{} | {} | {} | {} | {} | {} |\n",
                b.id,
                b.role,
                b.status.as_str(),
                files,
                b.diff_stat,
                summary_clean
            ));
        }

        if join_mode == "full" {
            rendered.push_str("\n#### Detailed Briefs\n");
            for b in &briefs {
                rendered.push_str(&format!(
                    "\n##### Agent #{} ({})\n**Task:** {}\n**Summary:** {}\n",
                    b.id, b.role, b.task, b.summary
                ));
                if !b.files_touched.is_empty() {
                    rendered.push_str(&format!("**Files:** {}\n", b.files_touched.join(", ")));
                }
            }
        }
    }

    Ok(SubagentResultsBrief {
        briefs,
        rendered_markdown: rendered,
    })
}

/// Update the live task checklist for a session.
pub fn update_checklist(
    conn: &Connection,
    sess: i64,
    items: &[ChecklistItem],
) -> Result<Vec<ChecklistItem>> {
    let now = now_ms();
    let json_text = serde_json::to_string(items).unwrap_or_else(|_| "[]".into());

    conn.execute("DELETE FROM task_checklist WHERE sess = ?1", [sess])?;
    conn.execute(
        "INSERT INTO task_checklist(sess, items_json, updated_ms) VALUES (?1, ?2, ?3)",
        params![sess, json_text, now],
    )?;

    Ok(items.to_vec())
}

/// Get the active checklist for a session.
pub fn get_checklist(conn: &Connection, sess: i64) -> Result<Vec<ChecklistItem>> {
    let json_text: Option<String> = conn
        .query_row(
            "SELECT items_json FROM task_checklist WHERE sess = ?1 ORDER BY id DESC LIMIT 1",
            [sess],
            |r| r.get(0),
        )
        .optional()?;

    let items: Vec<ChecklistItem> = json_text
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    Ok(items)
}

/// Trigger Workbench UI navigation to open and highlight a specific file and line.
pub fn jump_to_file(
    conn: &Connection,
    sess: i64,
    path: &str,
    line: Option<u32>,
) -> Result<FileJump> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO ui_navigation(sess, path, line, created_ms) VALUES (?1, ?2, ?3, ?4)",
        params![sess, path, line, now],
    )?;
    let id = conn.last_insert_rowid();
    Ok(FileJump {
        id,
        sess,
        path: path.into(),
        line,
        created_ms: now,
    })
}

/// Get the latest jump to file directive for a session.
pub fn get_latest_jump(conn: &Connection, sess: i64) -> Result<Option<FileJump>> {
    conn.query_row(
        "SELECT id, sess, path, line, created_ms FROM ui_navigation \
         WHERE sess = ?1 ORDER BY id DESC LIMIT 1",
        [sess],
        |r| {
            Ok(FileJump {
                id: r.get(0)?,
                sess: r.get(1)?,
                path: r.get(2)?,
                line: r.get(3)?,
                created_ms: r.get(4)?,
            })
        },
    )
    .optional()
    .map_err(ChorusError::Sqlite)
}

/// Request an interactive human review on diff hunks.
pub fn request_review(
    conn: &Connection,
    sess: i64,
    summary: &str,
    hunks: &serde_json::Value,
) -> Result<ReviewRequest> {
    let now = now_ms();
    let hunks_json = serde_json::to_string(hunks).unwrap_or_else(|_| "[]".into());

    conn.execute(
        "INSERT INTO review_request(sess, summary, hunks_json, status, created_ms, resolved_ms)
         VALUES (?1, ?2, ?3, 'pending', ?4, NULL)",
        params![sess, summary, hunks_json, now],
    )?;

    let id = conn.last_insert_rowid();
    Ok(ReviewRequest {
        id,
        sess,
        summary: summary.into(),
        hunks: hunks.clone(),
        status: "pending".into(),
        created_ms: now,
        resolved_ms: None,
    })
}

/// Resolve an interactive human review request (approve or reject).
pub fn resolve_review(conn: &Connection, review_id: i64, approved: bool) -> Result<()> {
    let now = now_ms();
    let status = if approved { "approved" } else { "rejected" };
    let rows = conn.execute(
        "UPDATE review_request SET status = ?1, resolved_ms = ?2 WHERE id = ?3",
        params![status, now, review_id],
    )?;
    if rows == 0 {
        return Err(ChorusError::Invalid(format!(
            "review #{review_id} not found"
        )));
    }
    Ok(())
}

/// List review requests for a session.
pub fn list_reviews(
    conn: &Connection,
    sess: i64,
    pending_only: bool,
) -> Result<Vec<ReviewRequest>> {
    let mut stmt = if pending_only {
        conn.prepare(
            "SELECT id, sess, summary, hunks_json, status, created_ms, resolved_ms \
             FROM review_request WHERE sess = ?1 AND status = 'pending' ORDER BY id ASC",
        )?
    } else {
        conn.prepare(
            "SELECT id, sess, summary, hunks_json, status, created_ms, resolved_ms \
             FROM review_request WHERE sess = ?1 ORDER BY id ASC",
        )?
    };

    let rows = stmt.query_map([sess], |r| {
        let hunks_json: String = r.get("hunks_json")?;
        let hunks = serde_json::from_str(&hunks_json).unwrap_or(serde_json::Value::Null);
        Ok(ReviewRequest {
            id: r.get("id")?,
            sess: r.get("sess")?,
            summary: r.get("summary")?,
            hunks,
            status: r.get("status")?,
            created_ms: r.get("created_ms")?,
            resolved_ms: r.get("resolved_ms")?,
        })
    })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    #[test]
    fn spawn_and_manage_subagent_lifecycle() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn();

        let agent = spawn_agent(
            conn,
            None,
            "investigator",
            "Trace token counting",
            false,
            "/tmp",
        )
        .unwrap();

        assert_eq!(agent.role, "investigator");
        assert_eq!(agent.status, SubagentStatus::Running);
        assert!(agent.worktree.is_none());

        // Steering message
        let lead = crate::actor::intern(conn, "lead", "agent").unwrap();
        let msg = send_message(conn, agent.id, lead, "Check HeuristicCounter").unwrap();
        assert_eq!(msg.content, "Check HeuristicCounter");

        // Wait check while running
        let wait_rep = wait_agent(conn, None, Some(agent.id), 10).unwrap();
        assert!(!wait_rep.all_done);
        assert_eq!(wait_rep.running, vec![agent.id]);

        // Complete subagent
        let completed = complete_agent(
            conn,
            agent.id,
            "Found token caching logic in tokens.rs",
            Some("+15 -2"),
            &["crates/chorus-core/src/tokens.rs".into()],
        )
        .unwrap();
        assert_eq!(completed.status, SubagentStatus::Completed);
        assert_eq!(completed.files_touched.len(), 1);

        // Collect results
        let collected = collect_results(conn, None, Some(&[agent.id]), "compact").unwrap();
        assert_eq!(collected.briefs.len(), 1);
        assert!(
            collected.rendered_markdown.contains("Trace token counting")
                || collected.rendered_markdown.contains("Found token caching")
        );
    }

    #[test]
    fn writer_without_a_git_repo_fails_to_spawn() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn();
        let dir = std::env::temp_dir().join(format!("chorus-nogit-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let err = spawn_agent(conn, None, "builder", "edit", true, &dir.to_string_lossy());
        assert!(err.is_err(), "no fallback dir for a writer: {err:?}");
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM subagent", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "a failed spawn leaves no row behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resume_only_moves_waiting_children() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn();
        let a = spawn_agent(conn, None, "reviewer", "look", false, "/tmp").unwrap();
        complete_agent(conn, a.id, "done", None, &[]).unwrap();
        let r = resume_subagent(conn, a.id).unwrap();
        assert_eq!(r.status, SubagentStatus::Completed);
    }

    #[test]
    fn checklist_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn();
        let sess = crate::ledger::new_session(conn, "test", "/tmp").unwrap();

        let items = vec![
            ChecklistItem {
                text: "Refactor router".into(),
                done: true,
                in_progress: false,
            },
            ChecklistItem {
                text: "Add MCP tools".into(),
                done: false,
                in_progress: true,
            },
        ];

        let updated = update_checklist(conn, sess, &items).unwrap();
        assert_eq!(updated.len(), 2);

        let loaded = get_checklist(conn, sess).unwrap();
        assert_eq!(loaded, items);
    }

    #[test]
    fn file_jump_and_review_requests() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.conn();
        let sess = crate::ledger::new_session(conn, "test", "/tmp").unwrap();

        let jump = jump_to_file(conn, sess, "src/mcp.rs", Some(42)).unwrap();
        assert_eq!(jump.path, "src/mcp.rs");
        assert_eq!(jump.line, Some(42));

        let latest = get_latest_jump(conn, sess).unwrap().unwrap();
        assert_eq!(latest.line, Some(42));

        let req = request_review(
            conn,
            sess,
            "Verify MCP tool signatures",
            &serde_json::json!([{"file": "src/mcp.rs", "hunk": "@@ -1 +1 @@"}]),
        )
        .unwrap();
        assert_eq!(req.status, "pending");

        let pending = list_reviews(conn, sess, true).unwrap();
        assert_eq!(pending.len(), 1);

        resolve_review(conn, req.id, true).unwrap();
        let pending_after = list_reviews(conn, 1, true).unwrap();
        assert!(pending_after.is_empty());
    }
}
