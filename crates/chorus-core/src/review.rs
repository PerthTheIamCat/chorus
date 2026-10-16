//! Inline review comments and steering feedback loop.
//! Stores gutter hover review comments and generates steering prompts for agents.

use crate::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewComment {
    pub id: i64,
    pub session_id: i64,
    pub file_path: String,
    pub line_number: usize,
    pub comment_text: String,
    pub code_snippet: Option<String>,
    pub created_ms: i64,
    pub status: String, // "pending", "sent"
}

pub fn init_review_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS inline_review_comment (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id INTEGER NOT NULL,
            file_path TEXT NOT NULL,
            line_number INTEGER NOT NULL,
            comment_text TEXT NOT NULL,
            code_snippet TEXT,
            created_ms INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending'
        );
        CREATE INDEX IF NOT EXISTS idx_inline_review_sess ON inline_review_comment(session_id, status);",
    )?;
    Ok(())
}

pub fn add_review_comment(
    conn: &Connection,
    session_id: i64,
    file_path: &str,
    line_number: usize,
    comment_text: &str,
    code_snippet: Option<&str>,
) -> Result<i64> {
    init_review_table(conn)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    conn.execute(
        "INSERT INTO inline_review_comment (session_id, file_path, line_number, comment_text, code_snippet, created_ms, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending')",
        params![session_id, file_path, line_number as i64, comment_text, code_snippet, now],
    )?;

    Ok(conn.last_insert_rowid())
}

pub fn list_pending_comments(conn: &Connection, session_id: i64) -> Result<Vec<ReviewComment>> {
    init_review_table(conn)?;
    let mut stmt = conn.prepare(
        "SELECT id, session_id, file_path, line_number, comment_text, code_snippet, created_ms, status
         FROM inline_review_comment
         WHERE session_id = ?1 AND status = 'pending'
         ORDER BY file_path ASC, line_number ASC",
    )?;

    let rows = stmt.query_map(params![session_id], |row| {
        Ok(ReviewComment {
            id: row.get(0)?,
            session_id: row.get(1)?,
            file_path: row.get(2)?,
            line_number: row.get::<_, i64>(3)? as usize,
            comment_text: row.get(4)?,
            code_snippet: row.get(5)?,
            created_ms: row.get(6)?,
            status: row.get(7)?,
        })
    })?;

    let mut comments = Vec::new();
    for r in rows {
        comments.push(r?);
    }
    Ok(comments)
}

pub fn clear_pending_comments(conn: &Connection, session_id: i64) -> Result<()> {
    init_review_table(conn)?;
    conn.execute(
        "UPDATE inline_review_comment SET status = 'sent' WHERE session_id = ?1 AND status = 'pending'",
        params![session_id],
    )?;
    Ok(())
}

/// Synthesize a steering prompt from pending review comments
pub fn generate_steering_prompt(comments: &[ReviewComment]) -> String {
    if comments.is_empty() {
        return String::new();
    }

    let mut out = String::from("Please address the following feedback from code review:\n\n");
    for c in comments {
        out.push_str(&format!("- `{}:{}`: {}\n", c.file_path, c.line_number, c.comment_text));
        if let Some(snippet) = &c.code_snippet {
            if !snippet.trim().is_empty() {
                out.push_str(&format!("  > `{}`\n", snippet.trim()));
            }
        }
    }
    out.push_str("\nPlease update the code to resolve these issues and run tests to verify.");
    out
}
