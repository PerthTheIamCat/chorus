//! Removing sessions nobody ever used.
//!
//! "New session" used to insert a row at once, so every press of ✎ that was
//! never followed by a message left an empty session in the sidebar. New
//! sessions now start as an in-memory draft (`chorus_cli::drafts`); this
//! clears the rows the old behaviour left.
//!
//! "Empty" is strict: a session is deletable only if nothing anywhere points
//! at it and it is not itself a fork — no event, no attachment, no child
//! (fork or subagent), no checklist, review or checkpoint. Metadata that exists only because the
//! session was opened (`session_usage`, `ui_navigation`) goes with it.

use crate::Result;
use rusqlite::{params, Connection, OptionalExtension};

/// `(table, column)` pairs whose rows make a session not empty. Tables
/// other modules create lazily may be missing; those are skipped.
const REFERENCES: &[(&str, &str)] = &[
    ("event", "sess"),
    ("attachment", "sess"),
    ("attachment_access", "sess"),
    ("subagent", "parent_sess"),
    ("subagent", "child_sess"),
    ("session", "forked_from"),
    ("task_checklist", "sess"),
    ("review_request", "sess"),
    ("inline_review_comment", "session_id"),
    ("checkpoint_turn", "session_id"),
    ("interaction", "session_id"),
];

/// Rows that only say the session was opened; deleted with it.
const METADATA: &[(&str, &str)] = &[("session_usage", "sess"), ("ui_navigation", "sess")];

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2",
            params![table, column],
            |r| r.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

fn guard_sql(conn: &Connection) -> Result<String> {
    // A fork owns no events (it reads its history through the parent), yet
    // it is a branch the user made: never empty.
    let mut clauses = vec!["s.forked_from IS NULL".to_string()];
    for (table, column) in REFERENCES {
        if has_column(conn, table, column)? {
            clauses.push(format!(
                "NOT EXISTS (SELECT 1 FROM {table} x WHERE x.{column} = s.id)"
            ));
        }
    }
    Ok(clauses.join(" AND "))
}

/// Sessions with nothing in them, oldest first. `keep` (the session open in
/// the window) is never listed, empty or not.
pub fn empty_sessions(conn: &Connection, keep: &[i64]) -> Result<Vec<i64>> {
    let sql = format!(
        "SELECT s.id FROM session s WHERE {} ORDER BY s.id",
        guard_sql(conn)?
    );
    let mut stmt = conn.prepare(&sql)?;
    let ids = stmt
        .query_map([], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(ids.into_iter().filter(|id| !keep.contains(id)).collect())
}

/// Delete the sessions [`empty_sessions`] lists, re-checked inside one
/// transaction so a session that gained an event after the count was shown
/// is left alone. Returns the ids actually deleted.
pub fn delete_empty_sessions(conn: &Connection, keep: &[i64]) -> Result<Vec<i64>> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let run = || -> Result<Vec<i64>> {
        let ids = empty_sessions(conn, keep)?;
        for id in &ids {
            for (table, column) in METADATA {
                if has_column(conn, table, column)? {
                    conn.execute(&format!("DELETE FROM {table} WHERE {column} = ?1"), [id])?;
                }
            }
            conn.execute("DELETE FROM session WHERE id = ?1", [id])?;
        }
        Ok(ids)
    };
    match run() {
        Ok(ids) => {
            conn.execute_batch("COMMIT")?;
            Ok(ids)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}
