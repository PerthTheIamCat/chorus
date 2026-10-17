//! The event ledger: an append-only DAG of events per session.
//!
//! Sessions form a fork tree via `session.forked_from`/`fork_at_seq`:
//! forking never copies history. A forked session's own `event` table is
//! empty; reading its history means walking `session.forked_from` back to
//! find the parent's events up to `fork_at_seq`, then this session's own
//! events on top. (`tail`/`search` here operate on a single session's own
//! events — a full history walk across the fork chain is a caller-side
//! concern layered on top of these primitives.)

use crate::types::INLINE_MAX_BYTES;
use crate::{BlobId, ChorusError, Event, EventKind, NewEvent, Result};
use rusqlite::{Connection, OptionalExtension, Row};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn row_to_event(row: &Row) -> rusqlite::Result<Event> {
    let kind_raw: i64 = row.get("kind")?;
    let blob_raw: Option<String> = row.get("blob")?;
    Ok(Event {
        seq: row.get("seq")?,
        t: row.get("t")?,
        sess: row.get("sess")?,
        parent: row.get("parent")?,
        kind: EventKind::from_i64(kind_raw).unwrap_or(EventKind::Msg),
        actor: row.get("actor")?,
        body: row.get("body")?,
        blob: blob_raw.map(BlobId),
        tok_in: row.get("tok_in")?,
        tok_out: row.get("tok_out")?,
        cost_usd: row.get("cost_usd")?,
        model_id: row.get("model_id")?,
    })
}

/// Create a new session, recording `started_ms` as now.
pub fn new_session(c: &Connection, title: &str, cwd: &str) -> Result<i64> {
    c.execute(
        "INSERT INTO session(title, cwd, started_ms, forked_from, fork_at_seq)
         VALUES (?1, ?2, ?3, NULL, NULL)",
        (title, cwd, now_ms()),
    )?;
    Ok(c.last_insert_rowid())
}

/// Append an event. `body` is routed inline when small, or to the blob
/// store (content-addressed, deduplicated) when it's `>= INLINE_MAX_BYTES`.
/// Returns the new `seq`.
pub fn append(c: &Connection, e: &NewEvent) -> Result<i64> {
    let started_ms: i64 = c
        .query_row(
            "SELECT started_ms FROM session WHERE id = ?1",
            [e.sess],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| ChorusError::Invalid(format!("no session {}", e.sess)))?;
    let t = now_ms() - started_ms;

    let (body_col, blob_col): (Option<&str>, Option<String>) = if e.body.len() < INLINE_MAX_BYTES {
        (Some(e.body.as_str()), None)
    } else {
        let id = crate::blob::put(c, e.body.as_bytes())?;
        (None, Some(id.0))
    };

    c.execute(
        "INSERT INTO event(t, sess, parent, kind, actor, body, blob, tok_in, tok_out, cost_usd, model_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            t,
            e.sess,
            e.parent,
            e.kind.as_i64(),
            e.actor,
            body_col,
            blob_col,
            e.tok_in,
            e.tok_out,
            e.cost_usd,
            e.model_id,
        ],
    )?;

    Ok(c.last_insert_rowid())
}

/// Fetch a single event by its `seq`.
pub fn get(c: &Connection, seq: i64) -> Result<Event> {
    c.query_row(
        "SELECT seq, t, sess, parent, kind, actor, body, blob, tok_in, tok_out, cost_usd, model_id
         FROM event WHERE seq = ?1",
        [seq],
        row_to_event,
    )
    .optional()?
    .ok_or(ChorusError::EventMissing(seq))
}

/// The event's body text, whether stored inline or in the blob store.
pub fn body(c: &Connection, e: &Event) -> Result<String> {
    if let Some(b) = &e.body {
        return Ok(b.clone());
    }
    if let Some(id) = &e.blob {
        return crate::blob::get_text(c, id);
    }
    Ok(String::new())
}

/// The last `n` events of a session, in chronological (ascending `seq`) order.
pub fn tail(c: &Connection, sess: i64, n: usize) -> Result<Vec<Event>> {
    let mut stmt = c.prepare(
        "SELECT seq, t, sess, parent, kind, actor, body, blob, tok_in, tok_out, cost_usd, model_id
         FROM event WHERE sess = ?1 ORDER BY seq DESC LIMIT ?2",
    )?;
    let mut rows: Vec<Event> = stmt
        .query_map(rusqlite::params![sess, n as i64], row_to_event)?
        .collect::<rusqlite::Result<_>>()?;
    rows.reverse();
    Ok(rows)
}

/// The full history of `sess`, oldest first: this session's own events,
/// preceded by its parent's events up to the fork point, recursively back
/// to the root. This is the fork-chain walk the `tail`/`search` primitives
/// deliberately leave to callers (see module docs).
///
/// `limit` caps the total returned, keeping the newest.
pub fn history(c: &Connection, sess: i64, limit: usize) -> Result<Vec<Event>> {
    let mut chain: Vec<Event> = Vec::new();
    collect_history(c, sess, None, &mut chain)?;
    chain.sort_by_key(|e| e.seq);
    if chain.len() > limit {
        chain.drain(0..chain.len() - limit);
    }
    Ok(chain)
}

fn collect_history(
    c: &Connection,
    sess: i64,
    up_to_seq: Option<i64>,
    out: &mut Vec<Event>,
) -> Result<()> {
    let parent: Option<(i64, i64)> = c
        .query_row(
            "SELECT forked_from, fork_at_seq FROM session WHERE id = ?1",
            [sess],
            |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?)),
        )
        .optional()?
        .and_then(|(f, a)| Some((f?, a?)));

    if let Some((forked_from, fork_at_seq)) = parent {
        collect_history(c, forked_from, Some(fork_at_seq), out)?;
    }

    let (sql, cap) = match up_to_seq {
        Some(seq) => (
            "SELECT seq, t, sess, parent, kind, actor, body, blob, tok_in, tok_out, cost_usd, model_id
             FROM event WHERE sess = ?1 AND seq <= ?2 ORDER BY seq",
            seq,
        ),
        None => (
            "SELECT seq, t, sess, parent, kind, actor, body, blob, tok_in, tok_out, cost_usd, model_id
             FROM event WHERE sess = ?1 AND ?2 >= 0 ORDER BY seq",
            i64::MAX,
        ),
    };
    let mut stmt = c.prepare(sql)?;
    let rows = stmt
        .query_map(rusqlite::params![sess, cap], row_to_event)?
        .collect::<rusqlite::Result<Vec<Event>>>()?;
    out.extend(rows);
    Ok(())
}

/// Fork a session at `at_seq`: creates a new session row pointing at the
/// parent via `forked_from`/`fork_at_seq`. Copies **no** events — readers
/// walk the parent chain (see module docs) to reconstruct history before
/// the fork point.
pub fn fork(c: &Connection, sess: i64, at_seq: i64, title: &str) -> Result<i64> {
    let cwd: String = c.query_row("SELECT cwd FROM session WHERE id = ?1", [sess], |r| {
        r.get(0)
    })?;
    c.execute(
        "INSERT INTO session(title, cwd, started_ms, forked_from, fork_at_seq)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        (title, cwd, now_ms(), sess, at_seq),
    )?;
    Ok(c.last_insert_rowid())
}

/// Rename a session. The title is what lists show; the ledger is untouched.
pub fn rename_session(c: &Connection, sess: i64, title: &str) -> Result<()> {
    let n = c.execute("UPDATE session SET title = ?1 WHERE id = ?2", (title, sess))?;
    if n == 0 {
        return Err(ChorusError::Invalid(format!("no session {sess}")));
    }
    Ok(())
}

/// Delete a session, its events (and their stored summaries), then any blob
/// nothing references any more.
///
/// Refuses when another session was forked from this one: a fork copies no
/// events and reads its history through the parent, so deleting the parent
/// would silently empty the fork.
pub fn delete_session(c: &Connection, sess: i64) -> Result<()> {
    // Subagent child sessions go with the parent on delete
    let subagent_children: Vec<i64> = {
        let mut stmt = c.prepare(
            "SELECT child_sess FROM subagent WHERE parent_sess = ?1 AND child_sess IS NOT NULL",
        )?;
        let rows = stmt.query_map([sess], |r| r.get(0))?;
        let mut ids = Vec::new();
        for id in rows {
            ids.push(id?);
        }
        ids
    };
    for child in subagent_children {
        delete_session(c, child)?;
    }

    let forks: i64 = c.query_row(
        "SELECT COUNT(*) FROM session WHERE forked_from = ?1",
        [sess],
        |r| r.get(0),
    )?;
    if forks > 0 {
        return Err(ChorusError::Invalid(format!(
            "session {sess} has {forks} fork(s); delete those first"
        )));
    }
    c.execute_batch("BEGIN")?;
    let r = (|| -> Result<()> {
        // Clean up subagents and messages linked to this session
        c.execute(
            "DELETE FROM subagent_message WHERE subagent_id IN (SELECT id FROM subagent WHERE parent_sess = ?1 OR child_sess = ?1)",
            [sess],
        )?;
        c.execute(
            "DELETE FROM subagent WHERE parent_sess = ?1 OR child_sess = ?1",
            [sess],
        )?;
        c.execute("DELETE FROM task_checklist WHERE sess = ?1", [sess])?;
        c.execute("DELETE FROM session_usage WHERE sess = ?1", [sess])?;

        // Attachment links reference events, so clear this session's links
        // and metadata before deleting the event rows they point at.
        c.execute(
            "DELETE FROM attachment_event WHERE attachment_id IN (SELECT id FROM attachment WHERE sess = ?1)",
            [sess],
        )?;
        c.execute("DELETE FROM attachment_access WHERE sess = ?1", [sess])?;
        c.execute("DELETE FROM attachment WHERE sess = ?1", [sess])?;
        c.execute(
            "DELETE FROM summary WHERE seq IN (SELECT seq FROM event WHERE sess = ?1)",
            [sess],
        )?;
        c.execute("DELETE FROM event WHERE sess = ?1", [sess])?;
        let n = c.execute("DELETE FROM session WHERE id = ?1", [sess])?;
        if n == 0 {
            return Err(ChorusError::Invalid(format!("no session {sess}")));
        }
        crate::blob::gc(c, now_ms() + 1)?;
        Ok(())
    })();
    match r {
        Ok(()) => c.execute_batch("COMMIT")?,
        Err(_) => c.execute_batch("ROLLBACK")?,
    }
    r
}

/// Full-text search over event bodies. Uses the `ev_fts` FTS5 index when
/// available, and falls back to a `LIKE` scan otherwise (e.g. this sqlite
/// build lacks FTS5).
pub fn search(c: &Connection, q: &str, limit: usize) -> Result<Vec<Event>> {
    let fts_sql = "SELECT e.seq, e.t, e.sess, e.parent, e.kind, e.actor, e.body, e.blob, e.tok_in, e.tok_out, e.cost_usd, e.model_id
         FROM ev_fts JOIN event e ON e.seq = ev_fts.rowid
         WHERE ev_fts MATCH ?1 ORDER BY e.seq DESC LIMIT ?2";

    let fts_attempt = c.prepare(fts_sql).and_then(|mut stmt| {
        stmt.query_map(rusqlite::params![q, limit as i64], row_to_event)?
            .collect::<rusqlite::Result<Vec<Event>>>()
    });

    match fts_attempt {
        Ok(rows) => Ok(rows),
        Err(_) => {
            // FTS5 table missing (or the query wasn't valid FTS5 syntax) —
            // fall back to a plain substring scan.
            let like = format!("%{q}%");
            let mut stmt = c.prepare(
                "SELECT seq, t, sess, parent, kind, actor, body, blob, tok_in, tok_out, cost_usd, model_id
                 FROM event WHERE body LIKE ?1 ORDER BY seq DESC LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![like, limit as i64], row_to_event)?
                .collect::<rusqlite::Result<Vec<Event>>>()?;
            Ok(rows)
        }
    }
}

/// The total monetary cost of a session in USD from bridge-reported cost,
/// returning None if no bridge has reported cost. No price table estimation is used.
pub fn session_cost_usd(c: &Connection, sess: i64) -> Result<Option<f64>> {
    if let Ok(Some(u)) = crate::usage::get(c, sess) {
        if u.cost_usd.is_some() {
            return Ok(u.cost_usd);
        }
    }
    let mut stmt = c.prepare("SELECT SUM(cost_usd), COUNT(cost_usd) FROM event WHERE sess = ?1")?;
    let (sum, count): (Option<f64>, i64) =
        stmt.query_row(rusqlite::params![sess], |r| Ok((r.get(0)?, r.get(1)?)))?;
    if count > 0 {
        Ok(sum)
    } else {
        Ok(None)
    }
}
