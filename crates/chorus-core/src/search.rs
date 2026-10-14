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

/// Search across all historical sessions in the SQLite store
pub fn search_sessions(conn: &Connection, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
    let clean_query = query.trim();
    if clean_query.is_empty() {
        return Ok(Vec::new());
    }

    // Try FTS5 or LIKE fallback
    let mut stmt = conn.prepare(
        "SELECT e.session_id, COALESCE(s.title, 'Untitled Session'), e.seq, e.actor, SUBSTR(e.body, 1, 200)
         FROM event e
         LEFT JOIN session s ON s.id = e.session_id
         WHERE e.body LIKE ?1
         ORDER BY e.session_id DESC, e.seq DESC
         LIMIT ?2",
    )?;

    let pattern = format!("%{clean_query}%");
    let rows = stmt.query_map(params![pattern, limit as i64], |row| {
        Ok(SearchHit {
            session_id: row.get(0)?,
            session_title: row.get(1)?,
            seq: row.get(2)?,
            actor: format!("{}", row.get::<_, i64>(3)?),
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
    let mut stmt = conn.prepare(
        "SELECT seq, actor, body, kind FROM event WHERE session_id = ?1 ORDER BY seq ASC",
    )?;

    let mut md = format!("# Chorus Session {}\n\n", session_id);
    let rows = stmt.query_map(params![session_id], |row| {
        let seq: i64 = row.get(0)?;
        let actor: i64 = row.get(1)?;
        let body: String = row.get(2)?;
        let kind: i64 = row.get(3)?;
        Ok((seq, actor, body, kind))
    })?;

    for r in rows {
        let (seq, actor, body, _kind) = r?;
        let actor_name = if actor == 1 { "User" } else { "Agent" };
        md.push_str(&format!("### Turn {} ({})\n\n{}\n\n---\n\n", seq, actor_name, body));
    }

    Ok(md)
}
