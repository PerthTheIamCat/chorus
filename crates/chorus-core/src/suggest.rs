//! A guess at the user's next message, shown greyed out in an empty composer
//! and taken with Tab. It runs on the local model (the router's Ollama
//! model), after a turn, never on the turn path — no provider tokens, and an
//! Ollama that is not there just means no hint.
//!
//! Pure: the caller hands in the last exchange (already masked text from the
//! ledger) and gets one line back. Nothing is stored; the suggestion is not a
//! message until the user sends it.

use crate::compaction::Ollama;

/// What the model sees of each side: enough for "what comes next", short
/// enough for a 1B model to answer in about a second.
const CONTEXT_CHARS: usize = 1_500;
/// Longer than this is not a hint, it is a message the user did not write.
const MAX_CHARS: usize = 120;

/// One line, in the user's language, or `None` (no model, nothing sensible).
pub fn next_message(model: &str, last_user: &str, last_agent: &str) -> Option<String> {
    if model.trim().is_empty() || last_agent.trim().is_empty() {
        return None;
    }
    let prompt = prompt(last_user, last_agent);
    let raw = Ollama::new(model).generate(&prompt)?;
    clean(&raw, last_user)
}

fn prompt(last_user: &str, last_agent: &str) -> String {
    format!(
        "You suggest the next short message a developer will send to their AI coding \
agent. Write it in the same language the developer used. One line, at most 12 words, \
a natural follow-up such as asking to run tests, fix a remaining issue, or continue. \
Output only the message, no quotes, no explanation.\n\n\
Developer said:\n{}\n\nAgent replied:\n{}\n\nNext message:",
        tail(last_user, CONTEXT_CHARS),
        tail(last_agent, CONTEXT_CHARS)
    )
}

/// The end of `s`: an agent's last lines carry the question or the summary.
fn tail(s: &str, max: usize) -> String {
    let n = s.chars().count();
    s.chars().skip(n.saturating_sub(max)).collect()
}

/// First non-empty line, without quotes or a "Next message:" echo; refused
/// when it only repeats what the user already said.
fn clean(raw: &str, last_user: &str) -> Option<String> {
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line
        .strip_prefix("Next message:")
        .unwrap_or(line)
        .trim()
        .trim_matches(|c| matches!(c, '"' | '\'' | '“' | '”' | '`'))
        .trim();
    if line.is_empty() || line.chars().count() > MAX_CHARS {
        return None;
    }
    if line.eq_ignore_ascii_case(last_user.trim()) {
        return None;
    }
    Some(line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_takes_one_tidy_line() {
        assert_eq!(clean("\n  \"รันเทสต์ด้วย\"  \nextra", "แก้บั๊ก").as_deref(), Some("รันเทสต์ด้วย"));
        assert_eq!(clean("Next message: run the tests", "x").as_deref(), Some("run the tests"));
        assert_eq!(clean("   ", "x"), None);
        assert_eq!(clean("same", "same"), None, "echoing the user is no hint");
        assert_eq!(clean(&"ก".repeat(200), "x"), None, "too long to be a hint");
    }

    #[test]
    fn no_model_or_nothing_to_follow_means_no_call() {
        assert_eq!(next_message("", "a", "b"), None);
        assert_eq!(next_message("m", "a", "  "), None);
    }

    #[test]
    fn tail_keeps_the_end() {
        assert_eq!(tail("abcdef", 3), "def");
        assert_eq!(tail("กขค", 5), "กขค");
    }
}
