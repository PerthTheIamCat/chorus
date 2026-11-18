//! Reading other sessions of the same project: the "previous session" lines a
//! new session starts with, and the search behind `chorus_search_history`.
//!
//! Both are read-only over what the ledger already holds (events, the
//! `summary` table). Nothing here calls a model or the network, so it is safe
//! on the turn path (CLAUDE.md 7.2b).

use crate::{ledger, Result};
use crate::types::EventKind;
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;

/// One earlier session of the project, boiled down to a line a model can use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviousSession {
    pub id: i64,
    /// When anything last happened in it (ms since epoch).
    pub last_ms: i64,
    /// The agent that spoke last, if any agent spoke.
    pub agent: String,
    pub goal: String,
    /// What was left to do (last handoff brief) or, failing that, how the last
    /// turn ended (its stored L2 line). Empty when the ledger says neither.
    pub pending: String,
    /// `pending` came from the last turn's summary, not from a handoff brief.
    pub from_summary: bool,
}

/// Lines longer than this are cut (characters, not bytes: Thai is 3 bytes each).
const GOAL_MAX: usize = 90;
const PENDING_MAX: usize = 90;

fn clip(text: &str, max: usize) -> String {
    let first = text.trim().lines().next().unwrap_or("").trim();
    if first.chars().count() <= max {
        return first.to_string();
    }
    let mut out: String = first.chars().take(max).collect();
    out.push('\u{2026}');
    out
}

/// Sessions that are part of `sess`'s own history (it and the sessions it was
/// forked from): they are already in its context, so they are not "previous".
fn own_chain(c: &Connection, sess: i64) -> Vec<i64> {
    let mut chain = vec![sess];
    let mut cur = sess;
    while let Ok(Some(Some(parent))) = c
        .query_row("SELECT forked_from FROM session WHERE id = ?1", [cur], |r| r.get::<_, Option<i64>>(0))
        .optional()
    {
        if chain.contains(&parent) {
            break;
        }
        chain.push(parent);
        cur = parent;
    }
    chain
}

/// The most recent `max` earlier sessions (lower id) that share `sess`'s folder and have
/// something in them, newest first. Forks (compare branches, sub-agents) are
/// skipped: they are branches of a session, not earlier work.
pub fn previous_sessions(c: &Connection, sess: i64, max: usize) -> Result<Vec<PreviousSession>> {
    if max == 0 {
        return Ok(Vec::new());
    }
    let Some(cwd): Option<String> = c
        .query_row("SELECT cwd FROM session WHERE id = ?1", [sess], |r| r.get(0))
        .optional()?
    else {
        return Ok(Vec::new());
    };
    let chain = own_chain(c, sess);

    let mut stmt = c.prepare(
        "SELECT id, started_ms FROM session
         WHERE cwd = ?1 AND forked_from IS NULL AND id < ?2
         ORDER BY id DESC LIMIT 30",
    )?;
    let candidates = stmt
        .query_map((&cwd, sess), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::new();
    for (id, started_ms) in candidates {
        if out.len() >= max {
            break;
        }
        if chain.contains(&id) {
            continue;
        }
        if let Some(p) = summarize(c, id, started_ms)? {
            out.push(p);
        }
    }
    Ok(out)
}

fn summarize(c: &Connection, id: i64, started_ms: i64) -> Result<Option<PreviousSession>> {
    // The first thing the person said is the goal. A session with no user
    // message (a crashed start, an import marker only) has nothing to carry.
    let goal_ev: Option<i64> = c
        .query_row(
            "SELECT e.seq FROM event e JOIN actor a ON a.id = e.actor
             WHERE e.sess = ?1 AND e.kind = ?2 AND a.name = 'user'
             ORDER BY e.seq ASC LIMIT 1",
            (id, EventKind::Msg.as_i64()),
            |r| r.get(0),
        )
        .optional()?;
    let Some(goal_seq) = goal_ev else { return Ok(None) };
    let goal = clip(&ledger::body(c, &ledger::get(c, goal_seq)?)?, GOAL_MAX);
    if goal.is_empty() {
        return Ok(None);
    }

    let last_t: i64 = c
        .query_row("SELECT COALESCE(MAX(t), 0) FROM event WHERE sess = ?1", [id], |r| r.get(0))?;

    let agent: String = c
        .query_row(
            "SELECT a.name FROM event e JOIN actor a ON a.id = e.actor
             WHERE e.sess = ?1 AND e.kind = ?2 AND a.name != 'user'
             ORDER BY e.seq DESC LIMIT 1",
            (id, EventKind::Msg.as_i64()),
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_default();

    // What was left: the last handoff brief is the only place a person says it.
    let mut pending = String::new();
    let brief_seq: Option<i64> = c
        .query_row(
            "SELECT seq FROM event WHERE sess = ?1 AND kind = ?2 ORDER BY seq DESC LIMIT 1",
            (id, EventKind::Handoff.as_i64()),
            |r| r.get(0),
        )
        .optional()?;
    if let Some(seq) = brief_seq {
        if let Ok(v) = serde_json::from_str::<Value>(&ledger::body(c, &ledger::get(c, seq)?)?) {
            let items: Vec<&str> = v["pending"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            pending = clip(&items.join("; "), PENDING_MAX);
        }
    }
    let mut from_summary = false;
    if pending.is_empty() {
        // Otherwise how the last summarised turn ended. Stored by `chorus
        // compact`; never generated here.
        let l2: Option<String> = c
            .query_row(
                "SELECT s.l2 FROM summary s JOIN event e ON e.seq = s.seq
                 WHERE e.sess = ?1 ORDER BY s.seq DESC LIMIT 1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(l2) = l2 {
            pending = clip(&l2, PENDING_MAX);
            from_summary = !pending.is_empty();
        }
    }

    Ok(Some(PreviousSession { id, last_ms: started_ms + last_t, agent, goal, pending, from_summary }))
}

/// "วันนี้" / "เมื่อวาน" / "N วันก่อน" for the gap between `then_ms` and `now_ms`.
pub fn ago_th(then_ms: i64, now_ms: i64) -> String {
    const DAY: i64 = 86_400_000;
    let days = (now_ms - then_ms).max(0) / DAY;
    match days {
        0 => "วันนี้".into(),
        1 => "เมื่อวาน".into(),
        n => format!("{n} วันก่อน"),
    }
}

/// The task-state lines for `previous_sessions`, ready to append to the
/// rendered task state. Empty when there is nothing earlier. Carries the
/// session id so an agent can point at it with `chorus_search_history`.
pub fn render_previous(prev: &[PreviousSession], now_ms: i64) -> String {
    if prev.is_empty() {
        return String::new();
    }
    let mut s = String::from("previous sessions:\n");
    for p in prev {
        let mut line = format!("  - #{} {}", p.id, ago_th(p.last_ms, now_ms));
        if !p.agent.is_empty() {
            line.push_str(" · ");
            line.push_str(&p.agent);
        }
        line.push_str(" · ");
        line.push_str(&p.goal);
        if !p.pending.is_empty() {
            line.push_str(if p.from_summary { " · ล่าสุด: " } else { " · ค้าง: " });
            line.push_str(&p.pending);
        }
        s.push_str(&line);
        s.push('\n');
    }
    s
}

/// One search hit, already short.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub sess: i64,
    pub seq: i64,
    pub agent: String,
    pub snippet: String,
}

const SNIPPET_MAX: usize = 160;

/// FTS5 query for free text: each word quoted (so punctuation can't be read as
/// syntax) and prefix-matched, all required.
fn fts_query(q: &str) -> Option<String> {
    let words: Vec<String> = q
        .split_whitespace()
        .map(|w| w.replace('"', ""))
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect();
    if words.is_empty() {
        None
    } else {
        Some(words.join(" "))
    }
}

/// Messages and decisions from the sessions of `sess`'s project (same folder),
/// best match first. Tool output is left out: it is bulky and rarely what a
/// person means by "what did we decide". Bodies are masked at write time, so
/// nothing here needs scrubbing again.
pub fn search(c: &Connection, sess: i64, query: &str, limit: usize) -> Result<Vec<Hit>> {
    let limit = limit.clamp(1, 20);
    let Some(cwd): Option<String> = c
        .query_row("SELECT cwd FROM session WHERE id = ?1", [sess], |r| r.get(0))
        .optional()?
    else {
        return Ok(Vec::new());
    };
    let (msg, dec) = (EventKind::Msg.as_i64(), EventKind::Decision.as_i64());

    let mut hits: Vec<Hit> = Vec::new();
    if let Some(m) = fts_query(query) {
        let attempt = c
            .prepare(
                "SELECT e.sess, e.seq, a.name, e.body
                 FROM ev_fts JOIN event e ON e.seq = ev_fts.rowid
                 JOIN session s ON s.id = e.sess JOIN actor a ON a.id = e.actor
                 WHERE ev_fts MATCH ?1 AND s.cwd = ?2 AND e.kind IN (?3, ?4) AND e.body IS NOT NULL
                 ORDER BY rank LIMIT ?5",
            )
            .and_then(|mut stmt| {
                stmt.query_map((&m, &cwd, msg, dec, limit as i64), row_hit)?
                    .collect::<rusqlite::Result<Vec<_>>>()
            });
        if let Ok(rows) = attempt {
            hits = rows;
        }
    }
    if hits.is_empty() && !query.trim().is_empty() {
        // No FTS5 in this build, or a match the tokenizer can't make (Thai has
        // no spaces, so a word from the middle of a run is not a prefix).
        let like = format!("%{}%", query.trim().replace(['%', '_'], ""));
        let mut stmt = c.prepare(
            "SELECT e.sess, e.seq, a.name, e.body
             FROM event e JOIN session s ON s.id = e.sess JOIN actor a ON a.id = e.actor
             WHERE s.cwd = ?1 AND e.kind IN (?2, ?3) AND e.body LIKE ?4
             ORDER BY e.seq DESC LIMIT ?5",
        )?;
        hits = stmt
            .query_map((&cwd, msg, dec, &like, limit as i64), row_hit)?
            .collect::<rusqlite::Result<_>>()?;
    }
    Ok(hits)
}

fn row_hit(r: &rusqlite::Row) -> rusqlite::Result<Hit> {
    let body: String = r.get(3)?;
    Ok(Hit { sess: r.get(0)?, seq: r.get(1)?, agent: r.get(2)?, snippet: clip(&body, SNIPPET_MAX) })
}

/// Token-cheap text for the tool result: one line per hit.
pub fn render_hits(hits: &[Hit]) -> String {
    if hits.is_empty() {
        return "no matches".into();
    }
    hits.iter()
        .map(|h| format!("#{} seq {} {}: {}", h.sess, h.seq, h.agent, h.snippet))
        .collect::<Vec<_>>()
        .join("\n")
}
