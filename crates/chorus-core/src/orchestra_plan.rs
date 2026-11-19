//! Multi-agent v2: the lead's written plan, and what children say back.
//!
//! Three small things live here, all on top of tables and event kinds that
//! already exist (no schema change):
//!
//! - **The plan** (F4). Before a second child runs next to a writer, the lead
//!   writes subtasks, dependencies, file ownership, agreed signatures and a
//!   `done_when` per subtask. It goes into the ledger (one `ToolCall` event
//!   carrying the JSON) and into the lead's checklist (what the Task pane
//!   shows). `check_spawn` then holds every later spawn to it.
//! - **Child → lead** (F5/1). `subagent_message` rows with direction
//!   `child_to_parent`, mirrored into the lead's ledger so the thread shows
//!   them and the lead's next context contains them.
//! - **The board** (F5/3). Short "I changed interface X" notes (direction
//!   `board`) that every sibling and the lead can read.
//!
//! Everything is capped per turn, deduplicated, and masked like any other
//! entry path (CLAUDE.md 7.28). Decisions and merges stay with the lead.

use crate::agent_control::{self, ChecklistItem, Subagent, SubagentKind, SubagentStatus, TaskSpec};
use crate::types::{EventKind, NewEvent};
use crate::{actor, ledger};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// `body` of the ledger event that carries the plan starts with this.
const PLAN_MARK: &str = "{\"event\":\"orchestra_plan\"";

/// Direction of a child's question to its lead.
pub const CHILD_TO_PARENT: &str = "child_to_parent";
/// Direction of a board note.
pub const BOARD: &str = "board";

/// Questions one child may ask its lead in its lifetime.
pub const MAX_QUESTIONS_PER_CHILD: usize = 3;
/// Board notes one child may post.
pub const MAX_BOARD_POSTS_PER_CHILD: usize = 5;
/// One-shot cross-provider opinions per turn of the asker.
pub const MAX_OPINIONS_PER_TURN: usize = 3;
/// Longest question / note, in characters. Short on purpose: a board note is
/// "I changed X", not a design document.
pub const MAX_NOTE_CHARS: usize = 400;
/// Largest plan the ledger keeps inline (events over 2 KB go to the blob store
/// and could no longer be found by their marker).
const MAX_PLAN_BYTES: usize = 1800;
const MAX_PLAN_TASKS: usize = 8;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// One subtask of the lead's plan.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanTask {
    pub id: String,
    pub goal: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files_owned: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub done_when: String,
}

/// The lead's plan for a split.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrchestraPlan {
    pub tasks: Vec<PlanTask>,
    /// Interfaces fixed up front (`fn parse(&str) -> Ast`), copied into every
    /// child's brief.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signatures: Vec<String>,
}

impl OrchestraPlan {
    pub fn task(&self, id: &str) -> Option<&PlanTask> {
        self.tasks.iter().find(|t| t.id == id)
    }

    /// True when `a` depends on `b`, directly or through other subtasks.
    fn depends_transitively(&self, a: &str, b: &str) -> bool {
        let mut seen = HashSet::new();
        let mut stack = vec![a.to_string()];
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur.clone()) {
                continue;
            }
            if let Some(t) = self.task(&cur) {
                for d in &t.depends_on {
                    if d == b {
                        return true;
                    }
                    stack.push(d.clone());
                }
            }
        }
        false
    }

    /// Why this plan cannot be run, or `None` when it can.
    pub fn problem(&self) -> Option<String> {
        if self.tasks.is_empty() {
            return Some("a plan needs at least one subtask".into());
        }
        if self.tasks.len() > MAX_PLAN_TASKS {
            return Some(format!("at most {MAX_PLAN_TASKS} subtasks per plan"));
        }
        let mut ids = HashSet::new();
        for t in &self.tasks {
            if t.id.trim().is_empty() || t.goal.trim().is_empty() {
                return Some("every subtask needs an `id` and a `goal`".into());
            }
            if !ids.insert(t.id.as_str()) {
                return Some(format!("subtask id {:?} appears twice", t.id));
            }
        }
        for t in &self.tasks {
            for d in &t.depends_on {
                if self.task(d).is_none() {
                    return Some(format!("{} depends on unknown subtask {d:?}", t.id));
                }
            }
            if self.depends_transitively(&t.id, &t.id) {
                return Some(format!("{} depends on itself (cycle)", t.id));
            }
            if !t.files_owned.is_empty() && t.done_when.trim().is_empty() {
                return Some(format!(
                    "{} owns files, so it needs a `done_when` (a command that proves it works)",
                    t.id
                ));
            }
        }
        // Two subtasks that can run at the same time must not own the same
        // files; one that waits for the other may build on them.
        for (i, a) in self.tasks.iter().enumerate() {
            for b in &self.tasks[i + 1..] {
                if self.depends_transitively(&a.id, &b.id)
                    || self.depends_transitively(&b.id, &a.id)
                {
                    continue;
                }
                let clash = a.files_owned.iter().find_map(|x| {
                    b.files_owned
                        .iter()
                        .find(|y| agent_control::paths_overlap(x, y))
                        .map(|y| (x, y))
                });
                if let Some((x, y)) = clash {
                    return Some(format!(
                        "{} ({x}) and {} ({y}) can run at the same time but own overlapping files; \
                         split the files or make one depend on the other",
                        a.id, b.id
                    ));
                }
            }
        }
        None
    }

    /// The checklist lines the Task pane shows for this plan.
    fn checklist(&self) -> Vec<ChecklistItem> {
        self.tasks
            .iter()
            .map(|t| {
                let mut text = format!("{} · {}", t.id, t.goal.trim());
                if !t.depends_on.is_empty() {
                    text.push_str(&format!(" · รอ {}", t.depends_on.join(", ")));
                }
                if !t.files_owned.is_empty() {
                    text.push_str(&format!(" · ไฟล์: {}", t.files_owned.join(", ")));
                }
                if !t.done_when.trim().is_empty() {
                    text.push_str(&format!(" · เสร็จเมื่อ: {}", t.done_when.trim()));
                }
                ChecklistItem {
                    text,
                    done: false,
                    in_progress: false,
                }
            })
            .collect()
    }
}

fn bad(msg: impl Into<String>) -> String {
    msg.into()
}

/// Record `plan` for `sess`: validates it, writes it to the ledger and
/// replaces the lead's checklist with its subtasks.
pub fn record_plan(
    conn: &Connection,
    sess: i64,
    actor_id: i64,
    plan: &OrchestraPlan,
) -> std::result::Result<(), String> {
    if let Some(why) = plan.problem() {
        return Err(bad(why));
    }
    let body = serde_json::json!({ "event": "orchestra_plan", "plan": plan }).to_string();
    if body.len() > MAX_PLAN_BYTES {
        return Err(bad(format!(
            "plan is too long ({} bytes, max {MAX_PLAN_BYTES}): shorten goals and done_when",
            body.len()
        )));
    }
    let mut ev = NewEvent::new(sess, EventKind::ToolCall, actor_id, body);
    if let Some(last) = ledger::tail(conn, sess, 1)
        .map_err(|e| e.to_string())?
        .last()
    {
        ev = ev.parent(last.seq);
    }
    ledger::append(conn, &ev).map_err(|e| e.to_string())?;
    agent_control::update_checklist(conn, sess, &plan.checklist()).map_err(|e| e.to_string())?;
    Ok(())
}

/// The newest plan the lead of `sess` wrote, if any.
pub fn latest_plan(conn: &Connection, sess: i64) -> Option<OrchestraPlan> {
    let body: String = conn
        .query_row(
            "SELECT body FROM event WHERE sess = ?1 AND kind = ?2 AND body LIKE ?3 \
             ORDER BY seq DESC LIMIT 1",
            params![sess, EventKind::ToolCall.as_i64(), format!("{PLAN_MARK}%")],
            |r| r.get(0),
        )
        .optional()
        .ok()??;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    serde_json::from_value(v.get("plan")?.clone()).ok()
}

fn is_writer(sub: &Subagent) -> bool {
    sub.worktree.is_some()
}

fn active_children(conn: &Connection, sess: i64) -> Vec<Subagent> {
    agent_control::list_subagents(conn, Some(sess))
        .unwrap_or_default()
        .into_iter()
        .filter(|a| {
            a.kind == SubagentKind::Chorus
                && matches!(a.status, SubagentStatus::Running | SubagentStatus::Waiting)
                // A one-shot opinion is not part of the split.
                && TaskSpec::split(&a.task).1.is_none_or(|s| s.prefer_provider.is_empty())
        })
        .collect()
}

fn plan_task_of(sub: &Subagent) -> String {
    TaskSpec::split(&sub.task)
        .1
        .map(|s| s.plan_task)
        .unwrap_or_default()
}

/// Subtasks nobody has finished whose dependencies are all finished: what
/// could run right now, independent of one another.
fn ready_unfinished<'a>(plan: &'a OrchestraPlan, done: &HashSet<String>) -> Vec<&'a PlanTask> {
    plan.tasks
        .iter()
        .filter(|t| !done.contains(&t.id) && t.depends_on.iter().all(|d| done.contains(d)))
        .collect()
}

/// Hold a spawn to the lead's plan, and fill `spec` from it.
///
/// - A writer may not run next to another writer without a plan.
/// - With a plan, a writer names the subtask it carries out (`plan_task`);
///   that subtask must be unfinished, not already running, with its
///   dependencies finished.
/// - Running writers never outnumber the independent subtasks.
/// - The child's `files_owned` must lie inside the subtask's; left empty it is
///   copied from the plan, as are `done_when` and the agreed signatures.
///
/// Readers (no worktree) run freely next to each other, as before.
pub fn check_spawn(
    conn: &Connection,
    sess: i64,
    spec: &mut TaskSpec,
    writer: bool,
) -> std::result::Result<(), String> {
    let active = active_children(conn, sess);
    let plan = latest_plan(conn, sess);
    let Some(plan) = plan else {
        if writer && active.iter().any(is_writer) {
            return Err(bad(
                "write the plan first: a second agent that edits files would run next to \
                 another. Call `chorus_plan_orchestra` with the subtasks, their dependencies, the \
                 files each one owns, the agreed signatures and a `done_when` for each, then \
                 spawn with `plan_task`.",
            ));
        }
        return Ok(());
    };
    if !writer {
        // A reader may still name its subtask; unknown names are not an error.
        if let Some(t) = plan.task(&spec.plan_task) {
            spec.signatures = plan.signatures.clone();
            if spec.done_when.trim().is_empty() {
                spec.done_when = t.done_when.clone();
            }
        }
        return Ok(());
    }

    let ids: Vec<String> = plan.tasks.iter().map(|t| t.id.clone()).collect();
    if spec.plan_task.trim().is_empty() {
        return Err(bad(format!(
            "name the plan subtask this agent carries out with `plan_task` (one of: {})",
            ids.join(", ")
        )));
    }
    let Some(task) = plan.task(&spec.plan_task) else {
        return Err(bad(format!(
            "no subtask {:?} in the plan (subtasks: {})",
            spec.plan_task,
            ids.join(", ")
        )));
    };

    let all = agent_control::list_subagents(conn, Some(sess)).unwrap_or_default();
    let done: HashSet<String> = all
        .iter()
        .filter(|a| a.kind == SubagentKind::Chorus && a.status == SubagentStatus::Completed)
        .map(plan_task_of)
        .filter(|t| !t.is_empty())
        .collect();
    if done.contains(&task.id) {
        return Err(bad(format!("subtask {} is already done", task.id)));
    }
    if let Some(running) = active.iter().find(|a| plan_task_of(a) == task.id) {
        return Err(bad(format!(
            "subtask {} is already being done by agent #{}",
            task.id, running.id
        )));
    }
    if let Some(wait) = task.depends_on.iter().find(|d| !done.contains(*d)) {
        return Err(bad(format!(
            "subtask {} depends on {wait}, which is not finished yet",
            task.id
        )));
    }
    let writers_after = active.iter().filter(|a| is_writer(a)).count() + 1;
    let independent = ready_unfinished(&plan, &done).len();
    if writers_after > independent {
        return Err(bad(format!(
            "{writers_after} writers would run but the plan has only {independent} independent \
             subtask(s) ready; wait for one to finish or add subtasks to the plan"
        )));
    }

    if spec.files_owned.is_empty() {
        spec.files_owned = task.files_owned.clone();
    } else if !task.files_owned.is_empty() {
        if let Some(out) = spec
            .files_owned
            .iter()
            .find(|f| !within_any(f, &task.files_owned))
        {
            return Err(bad(format!(
                "files_owned {out:?} is outside what the plan gives {} ({})",
                task.id,
                task.files_owned.join(", ")
            )));
        }
    }
    if spec.done_when.trim().is_empty() {
        spec.done_when = task.done_when.clone();
    }
    spec.signatures = plan.signatures.clone();
    Ok(())
}

/// True when `path` is `owned` itself or lies under it.
fn within(path: &str, owned: &str) -> bool {
    let (p, o) = (
        agent_control::norm_path(path),
        agent_control::norm_path(owned),
    );
    if o.is_empty() {
        return true; // `**` owns everything
    }
    p == o || p.starts_with(&format!("{o}/"))
}

fn within_any(path: &str, owned: &[String]) -> bool {
    owned.iter().any(|o| within(path, o))
}

/// Files a child touched that its `files_owned` does not cover. Empty when it
/// owned nothing in particular.
pub fn outside_owned(files_touched: &[String], files_owned: &[String]) -> Vec<String> {
    if files_owned.is_empty() {
        return Vec::new();
    }
    files_touched
        .iter()
        .filter(|f| !within_any(f, files_owned))
        .cloned()
        .collect()
}

/// The visible label a child's summary gets for editing outside its files.
pub fn scope_label(outside: &[String]) -> String {
    const SHOWN: usize = 6;
    let mut names: Vec<&str> = outside.iter().take(SHOWN).map(String::as_str).collect();
    if outside.len() > SHOWN {
        names.push("…");
    }
    format!(
        "⚠ แก้ไฟล์นอก files_owned ({} ไฟล์): {}",
        outside.len(),
        names.join(", ")
    )
}

// ---- child <-> lead -------------------------------------------------------

/// The Chorus child whose own session is `sess`, if it is one.
pub fn child_of(conn: &Connection, sess: i64) -> Option<Subagent> {
    let id: i64 = conn
        .query_row(
            "SELECT id FROM subagent WHERE child_sess = ?1 AND kind = 'chorus'",
            [sess],
            |r| r.get(0),
        )
        .optional()
        .ok()??;
    agent_control::get_subagent(conn, id).ok()
}

fn clip(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() <= MAX_NOTE_CHARS {
        t.to_string()
    } else {
        format!("{}…", t.chars().take(MAX_NOTE_CHARS).collect::<String>())
    }
}

fn count_rows(conn: &Connection, subagent_id: i64, direction: &str) -> usize {
    conn.query_row(
        "SELECT COUNT(*) FROM subagent_message WHERE subagent_id = ?1 AND direction = ?2",
        params![subagent_id, direction],
        |r| r.get::<_, i64>(0),
    )
    .unwrap_or(0) as usize
}

fn insert_row(conn: &Connection, subagent_id: i64, direction: &str, content: &str) -> i64 {
    let now = now_ms();
    let _ = conn.execute(
        "INSERT INTO subagent_message(subagent_id, direction, content, created_ms) \
         VALUES (?1, ?2, ?3, ?4)",
        params![subagent_id, direction, content, now],
    );
    conn.last_insert_rowid()
}

/// Put a line in the lead's ledger as the child, so the thread shows it and the
/// lead's next context contains it.
fn mirror_to_lead(conn: &Connection, lead_sess: i64, actor_id: i64, text: &str) {
    let mut ev = NewEvent::new(lead_sess, EventKind::Msg, actor_id, text.to_string());
    if let Ok(tail) = ledger::tail(conn, lead_sess, 1) {
        if let Some(last) = tail.last() {
            ev = ev.parent(last.seq);
        }
    }
    let _ = ledger::append(conn, &ev);
}

fn mask(text: &str) -> String {
    crate::secrets::user_vault().mask(text).text
}

/// What happened to a question.
#[derive(Debug, PartialEq, Eq)]
pub enum Asked {
    /// Stored; the answer will be fetched with [`answer_for`].
    New(i64),
    /// The same question is already waiting.
    Duplicate(i64),
}

/// A child asks its lead a question. Capped, deduplicated, masked, and mirrored
/// into the lead's ledger. The caller then waits for an answer.
pub fn ask_lead(
    conn: &Connection,
    child: &Subagent,
    actor_id: i64,
    question: &str,
) -> std::result::Result<Asked, String> {
    let lead = child
        .parent_sess
        .ok_or_else(|| bad("this agent has no lead to ask"))?;
    let q = clip(&mask(question));
    if q.is_empty() {
        return Err(bad("question is empty"));
    }
    if let Some(dup) = open_questions(conn, lead)
        .into_iter()
        .find(|o| o.subagent_id == child.id && o.text.eq_ignore_ascii_case(&q))
    {
        return Ok(Asked::Duplicate(dup.id));
    }
    if count_rows(conn, child.id, CHILD_TO_PARENT) >= MAX_QUESTIONS_PER_CHILD {
        return Err(bad(format!(
            "you have already asked {MAX_QUESTIONS_PER_CHILD} questions; decide with what you \
             know and state your assumption in your final summary"
        )));
    }
    let id = insert_row(conn, child.id, CHILD_TO_PARENT, &q);
    mirror_to_lead(
        conn,
        lead,
        actor_id,
        &format!(
            "❓ agent #{} ({}) ถาม lead: {q}\n(ตอบด้วย chorus_send_message ไปที่ agent #{}, หรือถามผู้ใช้ด้วย chorus_ask_user)",
            child.id, child.role, child.id
        ),
    );
    Ok(Asked::New(id))
}

/// The lead's answer to question `question_id`: the first message it sent that
/// child after the question.
pub fn answer_for(conn: &Connection, child: &Subagent, question_id: i64) -> Option<String> {
    conn.query_row(
        "SELECT content FROM subagent_message WHERE subagent_id = ?1 \
         AND direction = 'parent_to_child' AND id > ?2 ORDER BY id LIMIT 1",
        params![child.id, question_id],
        |r| r.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

/// A question still waiting for an answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenQuestion {
    pub id: i64,
    pub subagent_id: i64,
    pub role: String,
    pub text: String,
}

/// Questions the children of `lead` asked that the lead has not answered.
pub fn open_questions(conn: &Connection, lead: i64) -> Vec<OpenQuestion> {
    let mut out = Vec::new();
    for sub in agent_control::list_subagents(conn, Some(lead)).unwrap_or_default() {
        if sub.kind != SubagentKind::Chorus {
            continue;
        }
        let Ok(mut stmt) = conn.prepare(
            "SELECT id, content FROM subagent_message WHERE subagent_id = ?1 AND direction = ?2 \
             ORDER BY id",
        ) else {
            continue;
        };
        let rows: Vec<(i64, String)> = stmt
            .query_map(params![sub.id, CHILD_TO_PARENT], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map(|it| it.flatten().collect())
            .unwrap_or_default();
        for (id, text) in rows {
            if answer_for(conn, &sub, id).is_none() {
                out.push(OpenQuestion {
                    id,
                    subagent_id: sub.id,
                    role: sub.role.clone(),
                    text,
                });
            }
        }
    }
    out
}

// ---- board ------------------------------------------------------------------

/// A sibling posts "I changed interface X". Returns how many notes it has
/// posted. Mirrored into the lead's ledger (which is also what the lead and
/// every later child reads).
pub fn post_board(
    conn: &Connection,
    child: &Subagent,
    actor_id: i64,
    note: &str,
) -> std::result::Result<usize, String> {
    let lead = child
        .parent_sess
        .ok_or_else(|| bad("this agent has no lead to share a board with"))?;
    let n = clip(&mask(note));
    if n.is_empty() {
        return Err(bad("note is empty"));
    }
    let existing = board(conn, lead);
    if existing
        .iter()
        .any(|b| b.subagent_id == child.id && b.text.eq_ignore_ascii_case(&n))
    {
        return Ok(count_rows(conn, child.id, BOARD));
    }
    if count_rows(conn, child.id, BOARD) >= MAX_BOARD_POSTS_PER_CHILD {
        return Err(bad(format!(
            "you have posted {MAX_BOARD_POSTS_PER_CHILD} notes already; keep the board for \
             interface changes only"
        )));
    }
    insert_row(conn, child.id, BOARD, &n);
    mirror_to_lead(
        conn,
        lead,
        actor_id,
        &format!("📌 board · agent #{} ({}): {n}", child.id, child.role),
    );
    Ok(count_rows(conn, child.id, BOARD))
}

/// One board note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BoardNote {
    pub subagent_id: i64,
    pub role: String,
    pub text: String,
}

/// Every note on `lead`'s board, oldest first.
pub fn board(conn: &Connection, lead: i64) -> Vec<BoardNote> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT m.subagent_id, s.role, m.content FROM subagent_message m \
         JOIN subagent s ON s.id = m.subagent_id \
         WHERE s.parent_sess = ?1 AND m.direction = ?2 ORDER BY m.id",
    ) else {
        return Vec::new();
    };
    stmt.query_map(params![lead, BOARD], |r| {
        Ok(BoardNote {
            subagent_id: r.get(0)?,
            role: r.get(1)?,
            text: r.get(2)?,
        })
    })
    .map(|it| it.flatten().collect())
    .unwrap_or_default()
}

/// The board as text for an agent, without its own notes when `except` is set.
pub fn render_board(notes: &[BoardNote], except: Option<i64>) -> String {
    let mut s = String::new();
    for n in notes.iter().filter(|n| Some(n.subagent_id) != except) {
        s.push_str(&format!("- #{} ({}): {}\n", n.subagent_id, n.role, n.text));
    }
    s
}

// ---- opinions ---------------------------------------------------------------

/// Start of the asker's current turn, in ms since the epoch: the lead's last
/// message from the human, or a child's creation time.
pub fn turn_start_ms(conn: &Connection, sess: i64) -> i64 {
    if let Some(child) = child_of(conn, sess) {
        return child.created_ms;
    }
    let user = actor::intern(conn, "user", "human").unwrap_or(-1);
    conn.query_row(
        "SELECT s.started_ms + e.t FROM event e JOIN session s ON s.id = e.sess \
         WHERE e.sess = ?1 AND e.kind = ?2 AND e.actor = ?3 ORDER BY e.seq DESC LIMIT 1",
        params![sess, EventKind::Msg.as_i64(), user],
        |r| r.get(0),
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or(0)
}

/// One-shot opinion children `lead` already started since `since_ms`.
pub fn opinions_since(conn: &Connection, lead: i64, since_ms: i64) -> usize {
    agent_control::list_subagents(conn, Some(lead))
        .unwrap_or_default()
        .iter()
        .filter(|a| a.created_ms >= since_ms)
        .filter(|a| {
            TaskSpec::split(&a.task)
                .1
                .is_some_and(|s| !s.prefer_provider.is_empty())
        })
        .count()
}

// ---- what the lead reads next turn --------------------------------------------

/// A few lines for the lead's next prompt: where each child stands, which
/// questions wait for it, what the board says. Empty when there is nothing to
/// say (no children). Not written to the ledger: the questions and notes
/// already are, and this only points at them.
pub fn lead_digest(conn: &Connection, lead: i64) -> String {
    let kids: Vec<Subagent> = agent_control::list_subagents(conn, Some(lead))
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a.kind == SubagentKind::Chorus)
        .collect();
    if kids.is_empty() {
        return String::new();
    }
    let mut s = String::from("(Chorus: your agents right now —\n");
    for a in kids.iter().rev().take(8).rev() {
        let outside = a
            .summary
            .as_deref()
            .is_some_and(|x| x.contains("นอก files_owned"));
        s.push_str(&format!(
            "  #{} {} · {}{}\n",
            a.id,
            a.role,
            a.status.as_str(),
            if outside {
                " · ⚠ แก้นอก files_owned"
            } else {
                ""
            }
        ));
    }
    let open = open_questions(conn, lead);
    if !open.is_empty() {
        s.push_str("  Waiting for your answer (reply with chorus_send_message):\n");
        for q in open.iter().take(5) {
            s.push_str(&format!("   #{}: {}\n", q.subagent_id, q.text));
        }
    }
    let notes = board(conn, lead);
    if !notes.is_empty() {
        s.push_str("  Board:\n");
        for n in notes.iter().rev().take(5).rev() {
            s.push_str(&format!("   #{}: {}\n", n.subagent_id, n.text));
        }
    }
    s.push_str(
        "  Finished agents' results are waiting: `chorus_collect_results` before you answer.)\n",
    );
    s
}

// ---- MCP tools ----------------------------------------------------------------

use serde_json::{json, Value};

fn text_result(text: String) -> Value {
    json!({ "content": [{ "type": "text", "text": text }] })
}

fn actor_id(conn: &Connection, name: &str) -> std::result::Result<i64, String> {
    let found: Option<i64> = conn
        .query_row("SELECT id FROM actor WHERE name = ?1", [name], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?;
    match found {
        Some(id) => Ok(id),
        None => actor::intern(conn, name, "agent").map_err(|e| e.to_string()),
    }
}

fn env_secs(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

pub fn plan_orchestra_schema() -> Value {
    json!({
        "name": "chorus_plan_orchestra",
        "description": "Write down how you will split the work BEFORE spawning more than one agent when any of them edits files. Lists the subtasks, what each depends on, which files each one owns, the interfaces (signatures) everyone agrees on, and the command that proves each one is done. Chorus shows it in the Task pane and holds every `chorus_spawn_agent` to it: writers must name their subtask with `plan_task`, cannot start before its dependencies finish, and may only touch the files the plan gives them.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "description": "The subtasks (at most 8).",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "string", "description": "Short id, e.g. s1"},
                            "goal": {"type": "string"},
                            "depends_on": {"type": "array", "items": {"type": "string"}, "description": "Ids that must finish first"},
                            "files_owned": {"type": "array", "items": {"type": "string"}, "description": "Files or directories only this subtask edits"},
                            "done_when": {"type": "string", "description": "A command whose success proves it is done (required when it owns files)"}
                        },
                        "required": ["id", "goal"]
                    }
                },
                "signatures": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Interfaces fixed up front, e.g. `fn parse(&str) -> Result<Ast>`. Every child is told not to change them."
                }
            },
            "required": ["tasks"]
        }
    })
}

pub fn ask_lead_schema() -> Value {
    json!({
        "name": "chorus_ask_lead",
        "description": "Ask the lead agent that started you a question when you are blocked or an assumption would be costly to get wrong (an interface you may not change, an unclear requirement). Waits up to about a minute for the answer. If none comes, carry on with your best assumption and state it in your final summary. At most 3 questions; never ask what you can find in the files.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "question": {"type": "string", "description": "One specific question, under 400 characters"}
            },
            "required": ["question"]
        }
    })
}

pub fn board_post_schema() -> Value {
    json!({
        "name": "chorus_board_post",
        "description": "Tell the agents working next to you about a change that affects them, in one line: \"I changed interface X: now takes (a, b)\". Everyone sees the board on their next turn. Interface changes only, at most 5 notes; use `chorus_ask_lead` for questions.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "note": {"type": "string", "description": "One line, under 400 characters"}
            },
            "required": ["note"]
        }
    })
}

pub fn board_read_schema() -> Value {
    json!({
        "name": "chorus_board_read",
        "description": "Read the shared board: the notes the agents of this split have posted about changed interfaces.",
        "inputSchema": {"type": "object", "properties": {}}
    })
}

pub fn ask_provider_schema() -> Value {
    json!({
        "name": "chorus_ask_provider",
        "description": "Ask ONE question of an agent from a different provider (for example Claude asks Codex \"any holes in this plan?\"). It runs as a read-only agent that can look at the project but not change it, and its answer comes back once. Costs a real turn on another account: use it for a second opinion on a plan or a risky change, not for lookups. At most 3 per turn.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "question": {"type": "string", "description": "A self-contained question; say what you want checked"},
                "provider": {
                    "type": "string",
                    "description": "Who to ask: 'claude', 'codex', 'antigravity', or 'other' (default: any provider but yours)",
                    "enum": ["claude", "codex", "antigravity", "other"]
                }
            },
            "required": ["question"]
        }
    })
}

/// Handle the orchestra tools; `None` when `name` is not one of them.
pub fn call(
    name: &str,
    args: &Value,
    conn: &Connection,
    session: Option<i64>,
    actor_name: &str,
) -> Option<std::result::Result<Value, String>> {
    let run = |f: &dyn Fn(i64) -> std::result::Result<Value, String>| {
        Some(
            session
                .ok_or_else(|| "this tool needs a session".to_string())
                .and_then(f),
        )
    };
    match name {
        "chorus_plan_orchestra" => run(&|sess| plan_call(conn, sess, actor_name, args)),
        "chorus_ask_lead" => run(&|sess| ask_lead_call(conn, sess, actor_name, args)),
        "chorus_board_post" => run(&|sess| board_post_call(conn, sess, actor_name, args)),
        "chorus_board_read" => run(&|sess| {
            let lead = child_of(conn, sess)
                .and_then(|c| c.parent_sess)
                .unwrap_or(sess);
            let notes = board(conn, lead);
            Ok(text_result(if notes.is_empty() {
                "The board is empty.".into()
            } else {
                render_board(&notes, None)
            }))
        }),
        "chorus_ask_provider" => run(&|sess| ask_provider_call(conn, sess, actor_name, args)),
        _ => None,
    }
}

fn plan_call(
    conn: &Connection,
    sess: i64,
    actor_name: &str,
    args: &Value,
) -> std::result::Result<Value, String> {
    if child_of(conn, sess).is_some() {
        return Err(bad("only the lead writes the plan; use chorus_ask_lead"));
    }
    let tasks = args
        .get("tasks")
        .cloned()
        .ok_or_else(|| bad("tasks is required"))?;
    let tasks: Vec<PlanTask> =
        serde_json::from_value(tasks).map_err(|e| bad(format!("tasks: {e}")))?;
    let signatures: Vec<String> = args
        .get("signatures")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let plan = OrchestraPlan { tasks, signatures };
    let who = actor_id(conn, actor_name)?;
    record_plan(conn, sess, who, &plan)?;
    let ids: Vec<&str> = plan.tasks.iter().map(|t| t.id.as_str()).collect();
    Ok(text_result(format!(
        "Plan recorded ({} subtasks: {}). Spawn each writer with `plan_task` set to its id; \
         one that depends on another can start once that one has finished.",
        plan.tasks.len(),
        ids.join(", ")
    )))
}

fn text_arg<'a>(args: &'a Value, key: &str) -> std::result::Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| bad(format!("{key} is required")))
}

fn ask_lead_call(
    conn: &Connection,
    sess: i64,
    actor_name: &str,
    args: &Value,
) -> std::result::Result<Value, String> {
    let child = child_of(conn, sess).ok_or_else(|| {
        bad("only a child agent has a lead to ask; ask the user with chorus_ask_user")
    })?;
    let question = text_arg(args, "question")?;
    let who = actor_id(conn, actor_name)?;
    let qid = match ask_lead(conn, &child, who, question)? {
        Asked::New(id) | Asked::Duplicate(id) => id,
    };
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs(env_secs("CHORUS_CHILD_ASK_WAIT_S", 60));
    loop {
        if let Some(answer) = answer_for(conn, &child, qid) {
            return Ok(text_result(format!("Lead answered: {answer}")));
        }
        // A child that was cancelled has nobody to answer.
        let status = agent_control::get_subagent(conn, child.id).map(|c| c.status);
        if status.is_ok_and(|s| s != SubagentStatus::Running)
            || std::time::Instant::now() >= deadline
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Ok(text_result(
        "No answer yet. The lead will see your question. Carry on with your best assumption and \
         state it in your final summary."
            .into(),
    ))
}

fn board_post_call(
    conn: &Connection,
    sess: i64,
    actor_name: &str,
    args: &Value,
) -> std::result::Result<Value, String> {
    let child = child_of(conn, sess).ok_or_else(|| bad("only a child agent posts to the board"))?;
    let note = text_arg(args, "note")?;
    let who = actor_id(conn, actor_name)?;
    let n = post_board(conn, &child, who, note)?;
    let lead = child.parent_sess.unwrap_or(sess);
    let others = render_board(&board(conn, lead), Some(child.id));
    Ok(text_result(format!(
        "Posted ({n}/{MAX_BOARD_POSTS_PER_CHILD}).{}",
        if others.is_empty() {
            String::new()
        } else {
            format!("\nNotes from the others:\n{others}")
        }
    )))
}

fn provider_id(name: &str) -> Option<&'static str> {
    match name.trim().to_ascii_lowercase().as_str() {
        "claude" | "anthropic" => Some("anthropic"),
        "codex" | "openai" | "gpt" => Some("openai"),
        "antigravity" | "gemini" | "google" | "agy" => Some("google"),
        _ => None,
    }
}

fn ask_provider_call(
    conn: &Connection,
    sess: i64,
    actor_name: &str,
    args: &Value,
) -> std::result::Result<Value, String> {
    let question = clip(&mask(text_arg(args, "question")?));
    // The opinion is a child of the lead, whoever asks.
    let asker_child = child_of(conn, sess);
    if let Some(c) = &asker_child {
        if TaskSpec::split(&c.task)
            .1
            .is_some_and(|s| !s.prefer_provider.is_empty())
        {
            return Err(bad("an agent asked for an opinion cannot ask another one"));
        }
    }
    let lead = asker_child
        .as_ref()
        .and_then(|c| c.parent_sess)
        .unwrap_or(sess);
    let prefer = match args.get("provider").and_then(Value::as_str).map(str::trim) {
        Some(p) if !p.is_empty() && !p.eq_ignore_ascii_case("other") => {
            provider_id(p).map(str::to_string).ok_or_else(|| {
                bad(format!(
                    "unknown provider {p:?}: claude, codex, antigravity or other"
                ))
            })?
        }
        _ => format!("other_than:{actor_name}"),
    };
    let goal = format!(
        "A colleague on another AI provider asks for your opinion. Answer this ONE question in \
         under 200 words, from your own read-only look at the project. Do not edit anything.\n\n\
         Question: {question}"
    );
    // The same question twice in a turn is a loop, not a second opinion.
    let since = turn_start_ms(conn, sess);
    let kids = agent_control::list_subagents(conn, Some(lead)).map_err(|e| e.to_string())?;
    if let Some(prev) = kids.iter().find(|a| {
        a.created_ms >= since
            && TaskSpec::split(&a.task).0 == goal
            && TaskSpec::split(&a.task)
                .1
                .is_some_and(|s| !s.prefer_provider.is_empty())
    }) {
        return Ok(text_result(match (&prev.status, &prev.summary) {
            (SubagentStatus::Completed, Some(a)) => {
                format!("You already asked this; the answer was:\n{a}")
            }
            _ => "You already asked this; the answer is not in yet.".into(),
        }));
    }
    if opinions_since(conn, lead, since) >= MAX_OPINIONS_PER_TURN {
        return Err(bad(format!(
            "at most {MAX_OPINIONS_PER_TURN} outside opinions per turn; decide with what you have"
        )));
    }

    let spec = TaskSpec {
        goal,
        output: "A short answer, under 200 words".into(),
        prefer_provider: prefer,
        ..Default::default()
    };
    let cwd: String = conn
        .query_row("SELECT cwd FROM session WHERE id = ?1", [lead], |r| {
            r.get(0)
        })
        .map_err(|e| format!("session {lead}: {e}"))?;
    let who = actor_id(conn, actor_name)?;
    mirror_to_lead(
        conn,
        lead,
        who,
        &format!("🔎 {actor_name} ขอความเห็นจาก provider อื่น: {question}"),
    );
    let agent = agent_control::spawn_agent_with_options(
        conn,
        Some(lead),
        "reviewer",
        &spec.encode(),
        Some("normal"),
        false,
        &cwd,
        None,
    )
    .map_err(|e| e.to_string())?;
    if agent.status == SubagentStatus::Waiting {
        let _ =
            agent_control::cancel_agent(conn, agent.id, Some("opinion not started: cap reached"));
        return Err(bad(
            "the orchestra is at its cap (children or concurrency); finish or cancel an agent first",
        ));
    }
    let wait = env_secs("CHORUS_OPINION_WAIT_S", 120);
    let rep = agent_control::wait_agent(conn, Some(lead), Some(agent.id), wait)
        .map_err(|e| e.to_string())?;
    let done = agent_control::get_subagent(conn, agent.id).map_err(|e| e.to_string())?;
    let _ = rep;
    match done.status {
        SubagentStatus::Completed => {
            let answer = done.summary.unwrap_or_default();
            mirror_to_lead(conn, lead, who, &format!("🔎 ความเห็นที่ได้กลับมา: {}", clip(&answer)));
            Ok(text_result(format!("Opinion from another provider:\n{answer}")))
        }
        SubagentStatus::Failed | SubagentStatus::Cancelled => Err(bad(format!(
            "no opinion: {}",
            done.summary.unwrap_or_else(|| "the other agent did not answer".into())
        ))),
        _ => Ok(text_result(format!(
            "The other agent has not answered yet (agent #{}); its answer will be in chorus_collect_results.",
            agent.id
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, deps: &[&str], files: &[&str], done: &str) -> PlanTask {
        PlanTask {
            id: id.into(),
            goal: format!("do {id}"),
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            files_owned: files.iter().map(|s| s.to_string()).collect(),
            done_when: done.into(),
        }
    }

    #[test]
    fn a_plan_with_a_cycle_unknown_dependency_or_clashing_files_is_refused() {
        let cyc = OrchestraPlan {
            tasks: vec![task("a", &["b"], &[], ""), task("b", &["a"], &[], "")],
            ..Default::default()
        };
        assert!(cyc.problem().unwrap().contains("cycle"));

        let unknown = OrchestraPlan {
            tasks: vec![task("a", &["zz"], &[], "")],
            ..Default::default()
        };
        assert!(unknown.problem().unwrap().contains("unknown"));

        let clash = OrchestraPlan {
            tasks: vec![
                task("a", &[], &["src/x"], "cargo test"),
                task("b", &[], &["src/x/y.rs"], "cargo test"),
            ],
            ..Default::default()
        };
        assert!(clash.problem().unwrap().contains("overlapping"));

        // The same files are fine when one waits for the other.
        let ordered = OrchestraPlan {
            tasks: vec![
                task("a", &[], &["src/x"], "cargo test"),
                task("b", &["a"], &["src/x/y.rs"], "cargo test"),
            ],
            ..Default::default()
        };
        assert_eq!(ordered.problem(), None);

        let no_done = OrchestraPlan {
            tasks: vec![task("a", &[], &["src/x"], "")],
            ..Default::default()
        };
        assert!(no_done.problem().unwrap().contains("done_when"));
    }

    #[test]
    fn outside_owned_names_only_what_the_child_did_not_own() {
        let owned = vec!["src/parser".to_string(), "docs/p.md".to_string()];
        let touched = vec![
            "src/parser/mod.rs".to_string(),
            "docs/p.md".to_string(),
            "src/lexer.rs".to_string(),
            "src/parser_old.rs".to_string(),
        ];
        assert_eq!(
            outside_owned(&touched, &owned),
            vec!["src/lexer.rs".to_string(), "src/parser_old.rs".to_string()],
            "a sibling directory with the same prefix is not inside"
        );
        assert!(
            outside_owned(&touched, &[]).is_empty(),
            "owning nothing in particular is not a breach"
        );
        assert!(scope_label(&["a.rs".into()]).contains("นอก files_owned"));
    }
}
