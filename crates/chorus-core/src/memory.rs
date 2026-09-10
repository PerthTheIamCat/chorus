//! Durable facts (compaction rung L3, see `types::Rung`) — short, pinned or
//! frequently-used notes that survive far longer than raw events.

use crate::{ChorusError, Result};
use rusqlite::Connection;
use std::time::{SystemTime, UNIX_EPOCH};

/// Facts must stay short: the model never uses id/confidence/timestamp
/// metadata, and carrying it costs roughly 70% of a fact's tokens for
/// nothing. Longer text also defeats the point of a "durable fact" rung.
const MAX_FACT_LEN: usize = 140;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct Fact {
    pub id: i64,
    pub scope: i64,
    pub text: String,
    pub pinned: bool,
    pub hits: i64,
}

/// Add (or, on a duplicate `(scope, text)`, no-op onto) a fact. Rejects text
/// over 140 characters.
pub fn add(c: &Connection, scope: i64, text: &str) -> Result<i64> {
    if text.chars().count() > MAX_FACT_LEN {
        return Err(ChorusError::Invalid(format!(
            "fact text is {} chars, max is {MAX_FACT_LEN}",
            text.chars().count()
        )));
    }

    c.execute(
        "INSERT INTO fact(scope, text, conf, pinned, hits, created_ms)
         VALUES (?1, ?2, 1.0, 0, 0, ?3)
         ON CONFLICT(scope, text) DO UPDATE SET scope = excluded.scope",
        (scope, text, now_ms()),
    )?;

    let id: i64 = c.query_row(
        "SELECT id FROM fact WHERE scope = ?1 AND text = ?2",
        (scope, text),
        |r| r.get(0),
    )?;
    Ok(id)
}

/// The top `k` facts for a scope, ordered pinned-first, then by hits, then
/// most-recently-created. Bumps `hits`/`last_used_ms` for every fact
/// returned.
pub fn top(c: &Connection, scope: i64, k: usize) -> Result<Vec<Fact>> {
    let mut stmt = c.prepare(
        "SELECT id, scope, text, pinned, hits FROM fact
         WHERE scope = ?1
         ORDER BY pinned DESC, hits DESC, created_ms DESC
         LIMIT ?2",
    )?;
    let facts: Vec<Fact> = stmt
        .query_map((scope, k as i64), |r| {
            Ok(Fact {
                id: r.get(0)?,
                scope: r.get(1)?,
                text: r.get(2)?,
                pinned: r.get::<_, i64>(3)? != 0,
                hits: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    let now = now_ms();
    for f in &facts {
        c.execute(
            "UPDATE fact SET hits = hits + 1, last_used_ms = ?1 WHERE id = ?2",
            (now, f.id),
        )?;
    }

    // Reflect the bump in the returned values too.
    let facts = facts
        .into_iter()
        .map(|mut f| {
            f.hits += 1;
            f
        })
        .collect();

    Ok(facts)
}

/// Render facts as the format sent to the model: plain markdown bullets and
/// nothing else. No ids, confidence, or timestamps — that metadata is never
/// used by the model and stripping it is the whole point.
pub fn render(facts: &[Fact]) -> String {
    facts
        .iter()
        .map(|f| format!("- {}", f.text))
        .collect::<Vec<_>>()
        .join("\n")
}
