//! "What did the agent see this turn?" — the read-only report behind the
//! context inspector.
//!
//! The assembler fills the parts it decided itself (tokens per tier, which
//! turns were demoted or dropped and why, which memory/code items did not
//! fit). The session adds what only it knows (facts with their relevance
//! score). Nothing here is stored, and building it makes no model call and
//! no network call (CLAUDE.md 7.2b): it is the assembler's own bookkeeping,
//! handed to the shell as JSON so Swift only draws it (2.3).

use serde::Serialize;

/// One tier's share of the window.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TierReport {
    /// `System`, `TaskState`, `Memory`, `Conversation`, `Code`, `Reserve`.
    pub tier: String,
    pub tokens: i64,
    /// The tier's cap (`System` has none that binds; it reports the window).
    pub cap: i64,
    /// `tokens` as a percentage of `cap`, 0–100+ (task state may exceed it:
    /// that tier is never trimmed).
    pub fill_pct: f64,
    /// Why this tier is the size it is, in Thai, empty when nothing happened.
    pub note: String,
}

/// A turn that did not reach the model at full size.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TurnNote {
    /// 0 = newest turn.
    pub age: usize,
    /// Rung the turn would have had by age alone (0/1/2).
    pub from_rung: u8,
    /// Rung it ended at; `dropped` turns report their last rung.
    pub to_rung: u8,
    pub dropped: bool,
    pub tokens_before: i64,
    /// 0 when dropped.
    pub tokens_after: i64,
    /// First characters of the turn, so a person can tell which one it was.
    pub preview: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ConversationReport {
    pub turns_total: usize,
    /// How many of the turns that reached the model sit at each rung.
    pub at_l0: usize,
    pub at_l1: usize,
    pub at_l2: usize,
    /// Pushed one rung down (or more) by the budget, not by age.
    pub demoted_by_budget: usize,
    pub dropped: usize,
    /// The turns above, newest first, capped (`NOTES_MAX`).
    pub notes: Vec<TurnNote>,
    /// Notes that did not fit in `notes`.
    pub notes_omitted: usize,
}

/// A memory fact (or other line) with what decided its place.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MemoryNote {
    pub text: String,
    /// Similarity to what the user last said; `None` = no query to score by.
    pub score: Option<f64>,
    pub pinned: bool,
    /// False = ranked but cut because the memory tier was full.
    pub included: bool,
    pub tokens: i64,
    /// Whether the agent's answers mentioned it. `None` until Chorus counts
    /// references (memory counters); the shell hides the column then.
    pub referenced: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CodeNote {
    /// `diff`, `repo-map`, …
    pub label: String,
    pub tokens: i64,
    /// `kept`, `repeat` (same content as an earlier item, shown as a
    /// reference) or `omitted` (over budget, replaced by a manifest line).
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ContextReport {
    pub budget: i64,
    /// Sum of every tier's tokens (the `Reserve` tier is headroom, 0 used).
    pub total_tokens: i64,
    pub tiers: Vec<TierReport>,
    pub conversation: ConversationReport,
    pub memory: Vec<MemoryNote>,
    pub code: Vec<CodeNote>,
}

/// Longest turn list sent to the shell.
pub const NOTES_MAX: usize = 40;

/// First `n` characters of `s` on one line, for `TurnNote::preview`.
pub fn preview(s: &str, n: usize) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= n {
        flat
    } else {
        let cut: String = flat.chars().take(n).collect();
        format!("{cut}…")
    }
}

impl TierReport {
    pub fn new(tier: &str, tokens: i64, cap: i64, note: String) -> Self {
        let fill_pct = if cap > 0 {
            tokens as f64 * 100.0 / cap as f64
        } else {
            0.0
        };
        Self {
            tier: tier.to_string(),
            tokens,
            cap,
            fill_pct,
            note,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_is_one_line_and_bounded() {
        assert_eq!(preview("a\n  b\tc", 10), "a b c");
        assert_eq!(preview("abcdefghij", 4), "abcd…");
        // Thai has no spaces: bound by characters, not bytes.
        assert_eq!(preview("สวัสดีครับ", 3), "สวั…");
    }

    #[test]
    fn fill_pct_handles_a_zero_cap() {
        assert_eq!(TierReport::new("Reserve", 0, 0, String::new()).fill_pct, 0.0);
        assert_eq!(TierReport::new("Memory", 50, 200, String::new()).fill_pct, 25.0);
    }
}
