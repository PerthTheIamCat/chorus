//! Per-session metadata kept beside the ledger, never in it: the last known
//! usage (`session_usage`) and the review baseline (`session.review_base`).
//!
//! Both are metadata about a session, not something anyone said. Writing
//! them as events would distort `history()` and the dedup numbers, the same
//! reason L1/L2 live in `summary`.

use crate::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The last usage a session reported. One row per session.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionUsage {
    pub context_used: Option<i64>,
    pub context_window: Option<i64>,
    pub tokens_in: i64,
    pub tokens_out: i64,
    /// Session total in USD, `None` until a bridge reports a cost.
    pub cost_usd: Option<f64>,
    pub updated_ms: i64,
}

/// Record live context pressure (a `usage_update` mid-turn). Either side may
/// be missing; a missing side keeps its stored value.
pub fn record_context(
    conn: &Connection,
    sess: i64,
    context_used: Option<i64>,
    context_window: Option<i64>,
) -> Result<()> {
    record_turn(conn, sess, context_used, context_window, 0, 0, None)
}

/// Record the end of a turn: the last context pressure, token deltas and the
/// turn's cost, which is added to the session total.
pub fn record_turn(
    conn: &Connection,
    sess: i64,
    context_used: Option<i64>,
    context_window: Option<i64>,
    tokens_in: i64,
    tokens_out: i64,
    cost_usd: Option<f64>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO session_usage(sess, context_used, context_window, tokens_in, tokens_out, \
         cost_usd, updated_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(sess) DO UPDATE SET
           context_used   = COALESCE(excluded.context_used, context_used),
           context_window = COALESCE(excluded.context_window, context_window),
           tokens_in      = tokens_in + excluded.tokens_in,
           tokens_out     = tokens_out + excluded.tokens_out,
           cost_usd       = CASE WHEN excluded.cost_usd IS NULL THEN cost_usd
                                 ELSE COALESCE(cost_usd, 0) + excluded.cost_usd END,
           updated_ms     = excluded.updated_ms",
        params![
            sess,
            context_used,
            context_window,
            tokens_in,
            tokens_out,
            cost_usd,
            now_ms()
        ],
    )?;
    Ok(())
}

/// The stored usage for `sess`, if any turn has reported one.
pub fn get(conn: &Connection, sess: i64) -> Result<Option<SessionUsage>> {
    Ok(conn
        .query_row(
            "SELECT context_used, context_window, tokens_in, tokens_out, cost_usd, updated_ms \
             FROM session_usage WHERE sess = ?1",
            [sess],
            |r| {
                Ok(SessionUsage {
                    context_used: r.get(0)?,
                    context_window: r.get(1)?,
                    tokens_in: r.get(2)?,
                    tokens_out: r.get(3)?,
                    cost_usd: r.get(4)?,
                    updated_ms: r.get(5)?,
                })
            },
        )
        .optional()?)
}

/// The review baseline: a shadow-repo sha, or `None` when never set.
pub fn review_base(conn: &Connection, sess: i64) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT review_base FROM session WHERE id = ?1",
            [sess],
            |r| r.get(0),
        )
        .optional()?
        .flatten())
}

pub fn set_review_base(conn: &Connection, sess: i64, sha: Option<&str>) -> Result<()> {
    conn.execute(
        "UPDATE session SET review_base = ?2 WHERE id = ?1",
        params![sess, sha],
    )?;
    Ok(())
}
