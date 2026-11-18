//! The numbers behind the metrics strip under the chat: tok/s, cache hit,
//! cost, context fill and % of the account's limit.
//!
//! Every field is an `Option`: a number the provider does not give us is
//! `None` and the shell hides it — never a guess dressed up as a reading.
//! What *is* derived (tok/s) says so with an `_estimated` flag.
//!
//! None of this is written to the ledger or the database (CLAUDE.md 2.1):
//! cost and context come from `session_usage` as before, tok/s lives in
//! memory only and is gone when the app quits.

use crate::limits::AccountUsage;
use chorus_core::tokens::{HeuristicCounter, TokenCounter};
use chorus_core::usage::SessionUsage;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;

/// Below this the sample is too small for a rate to mean anything.
const MIN_STREAM_MS: u64 = 500;
const MIN_TOKENS: i64 = 8;

static SPEED: Mutex<Option<HashMap<i64, f64>>> = Mutex::new(None);

/// Tokens per second over the time the agent was actually streaming text.
/// `None` for a turn too short to measure.
pub fn tok_per_s(tokens: i64, stream_ms: u64) -> Option<f64> {
    if stream_ms < MIN_STREAM_MS || tokens < MIN_TOKENS {
        return None;
    }
    Some(tokens as f64 * 1000.0 / stream_ms as f64)
}

/// Remember the speed of `sess`'s latest turn. `text` is what the agent
/// streamed (messages + thoughts); tokens are counted with the same
/// heuristic as everywhere else, so the figure is an estimate.
pub fn record_speed(sess: i64, text: &str, stream_ms: u64) {
    let Some(v) = tok_per_s(HeuristicCounter.count(text), stream_ms) else {
        return;
    };
    if let Ok(mut m) = SPEED.lock() {
        m.get_or_insert_with(HashMap::new).insert(sess, v);
    }
}

fn last_speed(sess: i64) -> Option<f64> {
    SPEED.lock().ok()?.as_ref()?.get(&sess).copied()
}

/// The higher-pressure of the 5-hour and weekly windows of `account`
/// (`codex`, `claude@work`), as `(percent, window label)`.
pub fn limit_of(report: &[AccountUsage], account: &str) -> Option<(f64, &'static str)> {
    let rl = report
        .iter()
        .find(|a| a.name == account)?
        .rate_limits
        .as_ref()?;
    let five = rl.primary.as_ref().map(|w| (w.used_percent, "5h"));
    let week = rl.secondary.as_ref().map(|w| (w.used_percent, "สัปดาห์"));
    match (five, week) {
        (Some(a), Some(b)) => Some(if b.0 > a.0 { b } else { a }),
        (a, b) => a.or(b),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Metrics {
    /// Output speed. Always an estimate (see `tok_per_s_estimated`).
    pub tok_per_s: Option<f64>,
    pub tok_per_s_estimated: bool,
    /// Prompt-cache hit rate, 0–100. ACP's `usage_update` carries no cache
    /// counters, so this stays `None` until a bridge reports one.
    pub cache_hit_pct: Option<f64>,
    /// Session total in USD, exactly as the bridge reported it (Codex
    /// reports none, so it is `None` there rather than a price-list guess).
    pub cost_usd: Option<f64>,
    pub context_pct: Option<f64>,
    pub limit_pct: Option<f64>,
    pub limit_window: Option<String>,
}

/// Pure assembly of the strip from what is already known.
pub fn build(
    usage: Option<&SessionUsage>,
    speed: Option<f64>,
    limit: Option<(f64, &str)>,
) -> Metrics {
    let context_pct = usage.and_then(|u| match (u.context_used, u.context_window) {
        (Some(used), Some(win)) if win > 0 => Some((used as f64 * 100.0 / win as f64).min(100.0)),
        _ => None,
    });
    Metrics {
        tok_per_s: speed,
        tok_per_s_estimated: speed.is_some(),
        cache_hit_pct: None,
        cost_usd: usage.and_then(|u| u.cost_usd),
        context_pct,
        limit_pct: limit.map(|l| l.0),
        limit_window: limit.map(|l| l.1.to_string()),
    }
}

/// Strip for one session. `account` names the account whose limit to show
/// (`""` = skip the limit lookup, which can spawn the agent CLI on a cold
/// cache — call this off the main thread).
pub fn session_metrics(
    store: &chorus_core::store::Store,
    sess: i64,
    account: &str,
) -> anyhow::Result<Metrics> {
    let usage = chorus_core::usage::last_known(store.conn(), sess)?;
    let limit = if account.is_empty() {
        None
    } else {
        limit_of(&crate::limits::usage_report(), account)
    };
    Ok(build(usage.as_ref(), last_speed(sess), limit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::{RateLimits, Window};

    fn acct(name: &str, five: Option<f64>, week: Option<f64>) -> AccountUsage {
        let w = |p| Window { used_percent: p, window_minutes: None, resets_at: None, resets_label: None };
        AccountUsage {
            name: name.into(),
            agent: name.into(),
            account: String::new(),
            dir: String::new(),
            ready: true,
            login: String::new(),
            rate_limits: Some(RateLimits { primary: five.map(w), secondary: week.map(w), groups: None }),
            hit: None,
            quota_note: None,
            label: None,
            icon: None,
            color: None,
        }
    }

    #[test]
    fn too_small_a_sample_gives_no_rate() {
        assert_eq!(tok_per_s(100, 100), None);
        assert_eq!(tok_per_s(3, 5_000), None);
        assert_eq!(tok_per_s(200, 4_000), Some(50.0));
    }

    #[test]
    fn limit_shows_the_window_under_more_pressure() {
        let r = [acct("codex", Some(18.0), Some(46.0)), acct("claude", Some(70.0), Some(20.0))];
        assert_eq!(limit_of(&r, "codex"), Some((46.0, "สัปดาห์")));
        assert_eq!(limit_of(&r, "claude"), Some((70.0, "5h")));
        assert_eq!(limit_of(&r, "nobody"), None);
        assert_eq!(limit_of(&[acct("x", None, None)], "x"), None);
    }

    #[test]
    fn unknown_numbers_stay_none() {
        let m = build(None, None, None);
        assert_eq!(m, Metrics::default());
        let u = SessionUsage { context_used: Some(50_000), context_window: Some(200_000), cost_usd: None, ..Default::default() };
        let m = build(Some(&u), Some(42.0), Some((12.0, "5h")));
        assert_eq!(m.context_pct, Some(25.0));
        assert_eq!(m.cost_usd, None);
        assert!(m.tok_per_s_estimated);
        assert_eq!(m.cache_hit_pct, None);
        // no window reported -> no percentage invented
        let u = SessionUsage { context_used: Some(5), context_window: None, ..Default::default() };
        assert_eq!(build(Some(&u), None, None).context_pct, None);
    }

    #[test]
    fn speed_is_remembered_per_session() {
        record_speed(990_001, &"word ".repeat(200), 2_000);
        assert!(last_speed(990_001).unwrap() > 0.0);
        assert_eq!(last_speed(990_002), None);
        // a tiny turn does not overwrite a good reading with noise
        record_speed(990_001, "hi", 100);
        assert!(last_speed(990_001).is_some());
    }
}
