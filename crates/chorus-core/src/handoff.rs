//! The handoff brief: the structured note one agent leaves for the next.
//!
//! This is the *Brief* step of the five-step pipeline
//! (Freeze → Repack → Brief → Verify → Resume). It takes what the app
//! already knows — the task state, the recent ledger, the checkpoint diff —
//! and produces one compact object with two faces:
//!
//! - [`Brief::to_json`] — the full record, for the audit log and the UI.
//!   Carries `handoff{from,to,reason,at}` metadata.
//! - [`Brief::render_markdown`] — the model-facing form. No ids, no
//!   timestamps, no confidence: only what the receiving agent needs to
//!   pick the work up (see the hot/cold rule in the crate docs).
//!
//! Nothing here reads storage. The caller assembles [`BriefInput`] from the
//! ledger and hands it in; that keeps this module a pure, testable function
//! of its inputs.

use crate::types::{Event, EventKind};
use serde::{Deserialize, Serialize};

/// Why a turn changed hands. Kept as an enum so the relay/failover logic can
/// match on it rather than parsing a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffReason {
    /// The outgoing agent hit a request-rate limit.
    RateLimit,
    /// The outgoing agent's usage quota is spent.
    QuotaExhausted,
    /// The conversation no longer fits the outgoing agent's context window.
    ContextExhausted,
    /// A person asked for the switch.
    UserRequested,
    /// The outgoing agent errored and a relay peer is taking over.
    Failover,
    /// A deliberate switch with no forced cause.
    Manual,
}

impl HandoffReason {
    /// Classify an agent failure message: `Some` when it's the kind of
    /// transient, agent-specific failure a relay chain should route around
    /// (rate limit, quota, context window), `None` when it's a real error
    /// the next agent would hit too.
    /// Classify a failed turn into a reason to hand off — or `None`, meaning
    /// the failure is the user's problem to see, not something to route around.
    ///
    /// The strings here are not guesses. They are what the bridges actually
    /// emit, collected from real runs (see `evals/handoff/cases`):
    ///
    /// - Codex: `usageLimitExceeded`
    /// - claude-agent-acp: `out of extra usage`
    ///
    /// Neither contains "rate limit", "429" or "quota", so a classifier built
    /// only from the obvious words silently fails to fire on the one case the
    /// whole product exists for. Whenever a new limit message turns up in the
    /// wild, add it here *and* to `tests/handoff.rs`.
    pub fn from_failure(msg: &str) -> Option<Self> {
        let m = msg.to_lowercase();

        // Usage/quota exhaustion — the plan is spent. Checked first because
        // several providers phrase it with the word "limit" too, and this is
        // the more specific reading.
        const EXHAUSTED: &[&str] = &[
            "usagelimitexceeded",
            "usage limit",
            // Observed live from claude-agent-acp on 12 Sep 2026:
            //   "Internal error: You've hit your limit · resets 9:50pm (Asia/Bangkok)"
            // It matched none of the keys above — not "usage limit", not
            // "rate limit", not "429". A classifier written from eval logs
            // only knows the wordings those logs happened to contain, which
            // is why every real limit message that turns up gets added here
            // verbatim rather than paraphrased.
            "hit your limit",
            "out of extra usage",
            "quota",
            "insufficient_quota",
            "billing",
            "credit balance",
            "payment required",
            "402",
        ];
        if EXHAUSTED.iter().any(|k| m.contains(k)) {
            return Some(Self::QuotaExhausted);
        }

        // Throttling — the plan is fine, we are going too fast.
        const THROTTLED: &[&str] = &[
            "rate limit",
            "rate_limit",
            "ratelimit",
            "429",
            "too many requests",
            "slow down",
            "retry after",
        ];
        if THROTTLED.iter().any(|k| m.contains(k)) {
            return Some(Self::RateLimit);
        }

        // The login is gone (revoked, expired, never done). This agent cannot
        // answer until a person logs in again, so the turn moves on. Checked
        // before the context rule: these messages say "token" too.
        //
        // Observed live from claude-agent-acp on 5 Oct 2026, after ~190 s of
        // the CLI retrying on its own:
        //   "Failed to authenticate. API Error: 401 … OAuth access token has been revoked."
        //   "Authentication required"   (same machine, sandbox off)
        const AUTH: &[&str] = &[
            "failed to authenticate",
            "authentication required",
            "authentication_error",
            "token has been revoked",
            "not logged in",
            "please run /login",
            "invalid api key",
            "401",
        ];
        if AUTH.iter().any(|k| m.contains(k)) {
            return Some(Self::Failover);
        }

        // The window is full.
        if (m.contains("context") || m.contains("token"))
            && (m.contains("exhaust")
                || m.contains("length")
                || m.contains("too long")
                || m.contains("too large")
                || m.contains("window")
                || m.contains("maximum"))
        {
            return Some(Self::ContextExhausted);
        }

        // The provider is having a bad day.
        const UPSTREAM: &[&str] = &[
            "overloaded",
            "503",
            "502",
            "unavailable",
            "internal server error",
            "upstream",
            "gateway",
        ];
        if UPSTREAM.iter().any(|k| m.contains(k)) {
            return Some(Self::Failover);
        }

        // The agent itself is gone or unreachable: its process died or never
        // started, its pipe closed, the network dropped, or it went silent
        // past the stall watchdog. None of these are the user's problem to
        // see while another agent in the chain can still answer.
        const INFRA: &[&str] = &[
            // process crash / never started
            "agent closed its output",
            "broken pipe",
            "unexpected eof",
            "exited with",
            "process exited",
            // `Agent::spawn` wraps a failed launch as `spawn ["cmd", …]`
            "spawn [",
            "command not found",
            // hang (see `Agent::wait_for` idle watchdog)
            "agent stalled",
            "timed out",
            "timeout",
            // network
            "connection reset",
            "connection refused",
            "connection closed",
            "econnreset",
            "econnrefused",
            "etimedout",
            "enotfound",
            "socket hang up",
            "fetch failed",
            "network error",
            "network is unreachable",
            "dns error",
            "temporary failure in name resolution",
            "tls handshake",
            "error sending request",
            "stream disconnected",
        ];
        if INFRA.iter().any(|k| m.contains(k)) {
            return Some(Self::Failover);
        }

        None
    }

    /// A short phrase for the resume divider ("switched to codex — rate limit").
    pub fn as_phrase(self) -> &'static str {
        match self {
            Self::RateLimit => "rate limit",
            Self::QuotaExhausted => "quota exhausted",
            Self::ContextExhausted => "context window full",
            Self::UserRequested => "user asked",
            Self::Failover => "previous agent failed",
            Self::Manual => "manual switch",
        }
    }
}

/// Provenance for one handoff: who, to whom, why, when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffMeta {
    pub from: String,
    pub to: String,
    pub reason: HandoffReason,
    /// ms offset from the session start, matching `Event::t`.
    pub at: i64,
}

/// Everything the caller pulls from the ledger and task state to build a
/// brief. `goal`/`pending` can't be inferred from events — they come from
/// the task-state tier — so they're required inputs, not derived here.
pub struct BriefInput<'a> {
    /// The single sentence from the task state's `goal:` line.
    pub goal: String,
    /// Task-state `done:` items, already split into lines by the caller.
    pub done: Vec<String>,
    /// Task-state `pending:` items, first = do-this-next.
    pub pending: Vec<String>,
    /// Constraints, typically rendered project-scope memory facts.
    pub constraints: Vec<String>,
    /// The recent ledger slice, oldest-first, paired with resolved bodies.
    pub events: &'a [(Event, String)],
    /// Unified diff since the last checkpoint. Empty string = clean tree.
    pub diff: String,
    pub handoff: HandoffMeta,
}

/// The structured brief. Field order matches the storage spec's §9 example.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Brief {
    pub goal: String,
    pub done: Vec<String>,
    pub pending: Vec<String>,
    pub constraints: Vec<String>,
    pub files: Vec<String>,
    pub diff: String,
    pub handoff: HandoffMeta,
}

/// The hot form of a stored handoff event: one line for the conversation
/// tier, or `None` when `stored` is not a brief.
///
/// The JSON stays on disk because `task_state` reads pending and constraints
/// back from it; the model gets those through the task state, the touched
/// files through the workspace, and never the diff — a snapshot that is stale
/// after the next edit. Sending the stored brief instead cost 8,250 tokens
/// per later turn for a 300-line diff (bench `handoff in later turns`).
pub fn context_line(stored: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(stored).ok()?;
    let h = v.get("handoff")?;
    let from = h["from"].as_str().unwrap_or("?");
    // An import marker (`external_sessions::import`): the conversation above
    // came from another tool, which is worth one line to the next agent.
    if let Some(source) = v["source"].as_str() {
        let tool = source
            .split(':')
            .next()
            .and_then(crate::external_sessions::Provider::parse)
            .map(|p| p.label())
            .unwrap_or(from);
        return Some(format!("(conversation above imported from {tool})"));
    }
    let to = h["to"].as_str().unwrap_or("?");
    let why = serde_json::from_value::<HandoffReason>(h["reason"].clone())
        .map(HandoffReason::as_phrase)
        .unwrap_or("handoff");
    Some(format!("handoff: {from} → {to} ({why})"))
}

impl Brief {
    /// Build a brief from ledger-derived inputs. The one piece of real work
    /// is [`files_touched`]: everything else is a move.
    pub fn build(input: BriefInput<'_>) -> Self {
        let mut done = input.done;
        // Decisions recorded mid-session are "done" facts the next agent
        // must not relitigate. Fold them in after the task-state items.
        for (e, body) in input.events {
            if e.kind == EventKind::Decision {
                let line = body.trim();
                if !line.is_empty() && !done.iter().any(|d| d == line) {
                    done.push(line.to_string());
                }
            }
        }

        Brief {
            goal: input.goal.trim().to_string(),
            done,
            pending: input.pending,
            constraints: input.constraints,
            files: files_touched(input.events),
            diff: input.diff,
            handoff: input.handoff,
        }
    }

    /// The model-facing brief: one markdown section per field, no metadata
    /// the receiving agent won't act on. Ends with the echo-back ask that is
    /// the Verify step.
    pub fn render_markdown(&self) -> String {
        let mut s = String::new();
        s.push_str("# Handoff brief\n\n");

        s.push_str("## goal\n");
        s.push_str(self.goal.trim());
        s.push_str("\n\n");

        s.push_str("## done\n");
        push_bullets(&mut s, &self.done, "(nothing recorded)");

        s.push_str("\n## pending\n");
        push_bullets(
            &mut s,
            &self.pending,
            "(nothing recorded — ask before proceeding)",
        );

        s.push_str("\n## constraints\n");
        push_bullets(&mut s, &self.constraints, "(none stated)");

        s.push_str("\n## files\n");
        push_bullets(&mut s, &self.files, "(none touched yet)");

        s.push_str("\n## diff since checkpoint\n");
        if self.diff.trim().is_empty() {
            s.push_str("(clean tree)\n");
        } else {
            s.push_str("```diff\n");
            s.push_str(self.diff.trim_end());
            s.push_str("\n```\n");
        }

        s.push_str(&format!(
            "\n## verify\nYou are {to}, taking over from {from} ({why}). Before you touch \
             anything, reply with three lines: (1) the goal in your own words, (2) your \
             first step, (3) anything unclear.\n",
            to = self.handoff.to,
            from = self.handoff.from,
            why = self.handoff.reason.as_phrase(),
        ));
        s
    }

    /// The full record, pretty JSON, for the audit log / UI.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }
}

fn push_bullets(s: &mut String, items: &[String], empty: &str) {
    if items.is_empty() {
        s.push_str(empty);
        s.push('\n');
        return;
    }
    for it in items {
        s.push_str("- ");
        s.push_str(it.trim());
        s.push('\n');
    }
}

/// Distinct file paths mentioned across the event bodies, in first-seen
/// order. A token counts as a path if it contains `/` and either carries a
/// `.ext` or a trailing `:line` locator — enough to catch `src/order/repo.ts`
/// and `migrations/0042_idx.sql:12` without pulling in prose.
pub fn files_touched(events: &[(Event, String)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let split = |c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';'
            )
    };
    for (_, body) in events {
        for raw in body.split(split) {
            // Keep a trailing `:line` locator; strip other trailing punctuation.
            let tok = raw.trim_end_matches(['.', ')', ']']);
            if looks_like_path(tok) && !out.iter().any(|p| p == tok) {
                out.push(tok.to_string());
            }
        }
    }
    out
}

fn looks_like_path(tok: &str) -> bool {
    if tok.len() < 3 || tok.len() > 200 || !tok.contains('/') {
        return false;
    }
    // Reject URLs and absolute-looking noise that isn't a repo path.
    if tok.contains("://") || tok.starts_with("//") {
        return false;
    }
    let (path_part, loc_part) = match tok.rsplit_once(':') {
        Some((p, l)) if l.chars().all(|c| c.is_ascii_digit()) && !l.is_empty() => (p, true),
        _ => (tok, false),
    };
    let has_ext = path_part
        .rsplit_once('.')
        .map(|(_, ext)| {
            !ext.is_empty() && ext.len() <= 12 && ext.chars().all(|c| c.is_ascii_alphanumeric())
        })
        .unwrap_or(false);
    has_ext || loc_part
}
