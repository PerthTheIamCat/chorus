//! The context inspector's data: what the agent saw this turn, and what a
//! different agent would get if the user switched to it now.
//!
//! Read-only end to end. Building this writes nothing (not the ledger, not
//! memory `hits`), calls no model and touches no network (CLAUDE.md 7.2b), so
//! opening the inspector can never change what the next turn sees. Swift only
//! draws the JSON this becomes (2.3).

use crate::session::Session;
use anyhow::Result;
use chorus_core::context_report::ContextReport;
use chorus_core::tokens::{CachedCounter, HeuristicCounter, TokenCounter};
use serde::Serialize;

/// What the incoming agent would receive on a hand-off right now.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TargetPreview {
    pub agent: String,
    /// Window the preview was assembled for.
    pub budget: i64,
    /// False = nothing tells us this agent's window, so the current budget
    /// was reused. The shell says so rather than implying it was measured.
    pub budget_known: bool,
    /// The assembled context at that budget.
    pub context_tokens: i64,
    pub demoted: usize,
    pub dropped: usize,
    /// The Verify brief a hand-off would send first (markdown, as sent).
    pub brief_markdown: String,
    pub brief_tokens: i64,
    /// Project instruction file this agent reads on its own, on top of the
    /// context above (`CLAUDE.md` / `AGENTS.md`), when we know it.
    pub instruction_file: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Inspection {
    pub budget: i64,
    pub text: String,
    pub tokens: i64,
    pub report: ContextReport,
    pub target: Option<TargetPreview>,
}

/// `CHORUS_CONTEXT_WINDOW_CODEX=258000`: the window to assume for an agent
/// family. ACP gives no way to ask a bridge before a session exists, so an
/// unset variable means "unknown", not a guess.
fn window_override(family: &str) -> Option<i64> {
    let key = format!("CHORUS_CONTEXT_WINDOW_{}", family.to_uppercase());
    std::env::var(key).ok()?.trim().parse::<i64>().ok().filter(|n| *n > 0)
}

/// `claude-opus` → `claude`, `codex` → `codex`: the name the per-family
/// settings use.
fn family(agent: &str) -> &str {
    agent.split(['-', '@']).next().unwrap_or(agent)
}

fn instruction_file(family: &str) -> Option<&'static str> {
    match family {
        "claude" => Some("CLAUDE.md"),
        "codex" => Some("AGENTS.md"),
        _ => None,
    }
}

/// Inspect `session` at `budget`. `target` (an agent name; `""` = none) adds
/// the "if sent to <agent> now" preview. `instruction` is the message about to
/// be sent (memory is ranked against it, as in a real turn); `None` = the
/// previous message in the ledger.
pub fn inspect(
    session: &Session,
    budget: i64,
    target: &str,
    instruction: Option<&str>,
) -> Result<Inspection> {
    let (text, report) = session.assemble_report(budget, instruction)?;
    let counter = CachedCounter::new(HeuristicCounter);
    let tokens = counter.count(&text);
    let target = if target.trim().is_empty() {
        None
    } else {
        Some(preview_for(session, &counter, budget, target.trim(), instruction)?)
    };
    Ok(Inspection {
        budget,
        text,
        tokens,
        report,
        target,
    })
}

fn preview_for(
    session: &Session,
    counter: &impl TokenCounter,
    budget: i64,
    agent: &str,
    instruction: Option<&str>,
) -> Result<TargetPreview> {
    let fam = family(agent);
    let window = window_override(fam);
    let budget_for = window.unwrap_or(budget);
    let (ctx, r) = session.assemble_report(budget_for, instruction)?;
    let (_from, brief) = session.handoff_preview(agent)?;
    Ok(TargetPreview {
        agent: agent.to_string(),
        budget: budget_for,
        budget_known: window.is_some(),
        context_tokens: counter.count(&ctx),
        demoted: r.conversation.demoted_by_budget,
        dropped: r.conversation.dropped,
        brief_tokens: counter.count(&brief),
        brief_markdown: brief,
        instruction_file: instruction_file(fam).map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_strips_model_and_account_suffixes() {
        assert_eq!(family("claude-opus"), "claude");
        assert_eq!(family("claude@work"), "claude");
        assert_eq!(family("codex"), "codex");
        assert_eq!(instruction_file("claude"), Some("CLAUDE.md"));
        assert_eq!(instruction_file("codex"), Some("AGENTS.md"));
        assert_eq!(instruction_file("antigravity"), None);
    }

    #[test]
    fn an_unset_window_is_unknown_not_guessed() {
        assert_eq!(window_override("nosuchagentfamily"), None);
    }
}
