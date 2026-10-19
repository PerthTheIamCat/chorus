//! SQLite FTS5 cross-session search and session export (Markdown/HTML).

use crate::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub session_id: i64,
    pub session_title: String,
    pub seq: i64,
    pub actor: String,
    pub snippet: String,
}

/// Search across all historical sessions in the SQLite store. A plain
/// substring match: the query is user text, and FTS5 `MATCH` syntax would turn
/// a stray `"` or `-` into an error instead of a search.
pub fn search_sessions(conn: &Connection, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
    let clean_query = query.trim();
    if clean_query.is_empty() {
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare(
        "SELECT e.sess, COALESCE(s.title, 'Untitled Session'), e.seq, COALESCE(a.name, ''), SUBSTR(e.body, 1, 200)
         FROM event e
         LEFT JOIN session s ON s.id = e.sess
         LEFT JOIN actor a ON a.id = e.actor
         WHERE e.body LIKE ?1
         ORDER BY e.sess DESC, e.seq DESC
         LIMIT ?2",
    )?;

    let pattern = format!("%{clean_query}%");
    let rows = stmt.query_map(params![pattern, limit as i64], |row| {
        Ok(SearchHit {
            session_id: row.get(0)?,
            session_title: row.get(1)?,
            seq: row.get(2)?,
            actor: row.get(3)?,
            snippet: row.get(4)?,
        })
    })?;

    let mut hits = Vec::new();
    for r in rows {
        hits.push(r?);
    }
    Ok(hits)
}

/// Export entire conversation session to formatted Markdown
pub fn export_session_markdown(conn: &Connection, session_id: i64) -> Result<String> {
    let mut md = format!("# Chorus Session {}\n\n", session_id);
    for e in crate::ledger::history(conn, session_id, usize::MAX)? {
        // Large bodies live in the blob store; `ledger::body` resolves both.
        let body = crate::ledger::body(conn, &e)?;
        let actor_name = crate::actor::name(conn, e.actor).unwrap_or_else(|_| "agent".into());
        md.push_str(&format!("### Turn {} ({})\n\n{}\n\n---\n\n", e.seq, actor_name, body));
    }
    Ok(md)
}

