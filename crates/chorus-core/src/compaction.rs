//! The compaction ladder (Storage Spec §5): turn raw events into shorter
//! rungs so old conversation demotes (L0 → L1 → L2) instead of being
//! truncated off the tail.
//!
//! A [`Summarizer`] does the shortening. [`Extractive`] needs no model —
//! it keeps the load-bearing lines (a path, a number, a decision) and drops
//! the rest, which is enough to keep the budget honest offline. A
//! model-backed summarizer (Ollama on an idle machine) implements the same
//! trait and slots in without touching the assembler.

/// Shortens `text` to roughly `target_chars` characters, keeping whatever
/// carries the most meaning. Never returns more than the input.
pub trait Summarizer: Send + Sync {
    fn summarize(&self, text: &str, target_chars: usize) -> String;
}

/// Model-free compaction. Ranks lines by how much they look like they
/// matter — a file path, a number, an imperative, a decision word — and
/// keeps the top ones in original order until the budget is hit.
pub struct Extractive;

const SIGNAL_WORDS: &[&str] = &[
    "error", "fail", "fix", "bug", "add", "remove", "change", "because", "so that", "decided",
    "must", "don't", "do not", "instead", "todo", "next", "blocked", "passes", "fails",
];

fn line_score(line: &str) -> i32 {
    let l = line.to_lowercase();
    let mut s = 0;
    if line.contains('/') && line.contains('.') {
        s += 3; // path-ish
    }
    if line.chars().any(|c| c.is_ascii_digit()) {
        s += 1;
    }
    for w in SIGNAL_WORDS {
        if l.contains(w) {
            s += 2;
        }
    }
    // Prefer short, declarative lines over walls of text.
    if line.len() < 120 {
        s += 1;
    }
    s
}

impl Summarizer for Extractive {
    fn summarize(&self, text: &str, target_chars: usize) -> String {
        let text = text.trim();
        if text.chars().count() <= target_chars {
            return text.to_string();
        }

        let lines: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if lines.len() <= 1 {
            // One long line — take the head, on a word boundary.
            let mut out: String = text.chars().take(target_chars.saturating_sub(1)).collect();
            if let Some(cut) = out.rfind(' ') {
                out.truncate(cut);
            }
            out.push('…');
            return out;
        }

        // Rank lines, keep the best until the budget fills, then re-emit in
        // original order so the summary still reads top-to-bottom.
        let mut ranked: Vec<(usize, i32)> = lines
            .iter()
            .enumerate()
            .map(|(i, l)| (i, line_score(l)))
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1));

        let mut keep = vec![false; lines.len()];
        let mut used = 0usize;
        for (i, _) in ranked {
            let cost = lines[i].chars().count() + 1;
            if used + cost > target_chars && used > 0 {
                continue;
            }
            keep[i] = true;
            used += cost;
            if used >= target_chars {
                break;
            }
        }

        let mut kept: Vec<&str> = lines
            .iter()
            .zip(keep.iter())
            .filter(|(_, k)| **k)
            .map(|(l, _)| *l)
            .collect();
        if kept.is_empty() {
            kept.push(lines[0]);
        }
        let dropped = lines.len() - kept.len();
        let mut out = kept.join(" · ");
        if dropped > 0 {
            out.push_str(&format!(" (+{dropped} more)"));
        }
        out
    }
}

/// The three shortened forms of one turn, generated ahead of time so the
/// assembler can pick a rung without doing any work (see `context::TurnItem`).
#[derive(Debug, Clone)]
pub struct Rungs {
    /// L1 — a per-turn summary, ~250 chars.
    pub l1: String,
    /// L2 — an episode-grade gist, ~90 chars.
    pub l2: String,
}

/// Pre-compute L1/L2 for one turn's text with `s`.
pub fn rungs_for(s: &dyn Summarizer, text: &str) -> Rungs {
    Rungs {
        l1: s.summarize(text, 250),
        l2: s.summarize(text, 90),
    }
}
