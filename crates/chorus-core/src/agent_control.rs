//! Agent control surface: subagent orchestration, task checklist,
//! interactive reviews, and file jump primitives.
//!
//! Exposes the shared engine primitives used by both the Streamable-HTTP MCP
//! server and the CLI (`chorus agent ...`).

use crate::{ChorusError, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Execution status of a spawned subagent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl SubagentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// A child agent session tracked by Chorus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subagent {
    pub id: i64,
    pub parent_sess: Option<i64>,
    pub child_sess: Option<i64>,
    pub role: String,
    pub task: String,
    pub worktree: Option<String>,
    pub status: SubagentStatus,
    pub summary: Option<String>,
    pub diff_stat: Option<String>,
    pub files_touched: Vec<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
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
        role: r.get("role")?,
        task: r.get("task")?,
        worktree: r.get("worktree")?,
        status,
        summary: r.get("summary")?,
        diff_stat: r.get("diff_stat")?,
        files_touched,
        created_ms: r.get("created_ms")?,
        updated_ms: r.get("updated_ms")?,
    })
}

/// Spawn a child agent session.
///
/// If `parent_sess` is provided, a child session is created in the session table.
/// If `worktree` is true, an isolated worktree directory is assigned.
pub fn spawn_agent(
    conn: &Connection,
    parent_sess: Option<i64>,
    role: &str,
    task: &str,
    worktree: bool,
    cwd: &str,
) -> Result<Subagent> {
    let now = now_ms();
    let child_sess = if let Some(parent) = parent_sess {
        let title = format!("child-agent-{role}");
        conn.execute(
            "INSERT INTO session(title, cwd, started_ms, forked_from, fork_at_seq)
             VALUES (?1, ?2, ?3, ?4, NULL)",
            params![title, cwd, now, parent],
        )?;
        Some(conn.last_insert_rowid())
    } else {
        None
    };

    let worktree_path = if worktree {
        Some(format!(".chorus/worktrees/agent-{}", now))
    } else {
        None
    };

    conn.execute(
        "INSERT INTO subagent(parent_sess, child_sess, role, task, worktree, status, \
         summary, diff_stat, files_touched, created_ms, updated_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, 'running', NULL, NULL, '[]', ?6, ?6)",
        params![parent_sess, child_sess, role, task, worktree_path, now],
    )?;

    let id = conn.last_insert_rowid();
    get_subagent(conn, id)
}

/// Fetch a single subagent by ID.
pub fn get_subagent(conn: &Connection, id: i64) -> Result<Subagent> {
    conn.query_row(
        "SELECT id, parent_sess, child_sess, role, task, worktree, status, \
         summary, diff_stat, files_touched, created_ms, updated_ms \
         FROM subagent WHERE id = ?1",
        [id],
        row_to_subagent,
    )
    .map_err(|e| ChorusError::Invalid(format!("subagent #{id} not found: {e}")))
}

/// List all subagents, optionally filtered by parent session.
pub fn list_subagents(conn: &Connection, parent_sess: Option<i64>) -> Result<Vec<Subagent>> {
    let mut stmt = if parent_sess.is_some() {
        conn.prepare(
            "SELECT id, parent_sess, child_sess, role, task, worktree, status, \
             summary, diff_stat, files_touched, created_ms, updated_ms \
             FROM subagent WHERE parent_sess = ?1 ORDER BY id ASC",
        )?
    } else {
        conn.prepare(
            "SELECT id, parent_sess, child_sess, role, task, worktree, status, \
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
        out.push(r?);
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
            .query_row("SELECT parent_sess FROM subagent WHERE child_sess = ?1", [s], |r| {
                r.get::<_, Option<i64>>(0)
            })
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
        let mut ev = crate::types::NewEvent::new(child_sess, crate::types::EventKind::Msg, actor, message);
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

/// Wait on a specific child agent, or all child agents of `parent_sess`.
pub fn wait_agent(
    conn: &Connection,
    parent_sess: Option<i64>,
    subagent_id: Option<i64>,
    _timeout_secs: u64,
) -> Result<WaitReport> {
    let agents = if let Some(id) = subagent_id {
        vec![get_subagent(conn, id)?]
    } else {
        list_subagents(conn, parent_sess)?
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
            SubagentStatus::Running => running.push(a.id),
            SubagentStatus::Cancelled => cancelled.push(a.id),
            SubagentStatus::Failed => failed.push(a.id),
        }
    }

    let all_done = running.is_empty();

    Ok(WaitReport {
        completed,
        running,
        cancelled,
        failed,
        all_done,
        statuses,
    })
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

/// Cancel a running child agent session.
pub fn cancel_agent(
    conn: &Connection,
    subagent_id: i64,
    reason: Option<&str>,
) -> Result<Subagent> {
    let now = now_ms();
    let summary_val = reason.unwrap_or("Cancelled by parent agent");

    conn.execute(
        "UPDATE subagent SET status = 'cancelled', summary = COALESCE(summary, ?1), \
         updated_ms = ?2 WHERE id = ?3",
        params![summary_val, now, subagent_id],
    )?;

    get_subagent(conn, subagent_id)
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
        list_subagents(conn, parent_sess)?
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
        return Err(ChorusError::Invalid(format!("review #{review_id} not found")));
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
            true,
            "/tmp",
        )
        .unwrap();

        assert_eq!(agent.role, "investigator");
        assert_eq!(agent.status, SubagentStatus::Running);
        assert!(agent.worktree.is_some());

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
        assert!(collected.rendered_markdown.contains("Trace token counting") || collected.rendered_markdown.contains("Found token caching"));
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
