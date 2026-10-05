//! Pure tests for the handoff brief. No database — events are built by hand.

use chorus_core::handoff::{files_touched, Brief, BriefInput, HandoffMeta, HandoffReason};
use chorus_core::types::{Event, EventKind};

fn ev(seq: i64, kind: EventKind, body: &str) -> (Event, String) {
    (
        Event {
            seq,
            t: seq * 10,
            sess: 1,
            parent: None,
            kind,
            actor: 0,
            body: Some(body.to_string()),
            blob: None,
            tok_in: None,
            tok_out: None,
            cost_usd: None,
        },
        body.to_string(),
    )
}

fn meta() -> HandoffMeta {
    HandoffMeta {
        from: "claude-opus".into(),
        to: "codex".into(),
        reason: HandoffReason::RateLimit,
        at: 1234,
    }
}

#[test]
fn files_are_extracted_from_paths_not_prose() {
    let events = vec![
        ev(
            1,
            EventKind::ToolCall,
            "read src/order/repo.ts and check it",
        ),
        ev(
            2,
            EventKind::Diff,
            "edited migrations/0042_idx.sql:12 to add the index",
        ),
        ev(
            3,
            EventKind::Msg,
            "the order list page is slow, look at findAll please",
        ),
        ev(
            4,
            EventKind::Msg,
            "see https://example.com/docs/perf for background",
        ),
    ];
    let files = files_touched(&events);
    assert!(files.contains(&"src/order/repo.ts".to_string()));
    assert!(files.contains(&"migrations/0042_idx.sql:12".to_string()));
    // Prose words and URLs must not be mistaken for paths.
    assert!(!files.iter().any(|f| f.contains("example.com")));
    assert!(!files.iter().any(|f| f == "findAll"));
}

#[test]
fn files_are_deduplicated_in_first_seen_order() {
    let events = vec![
        ev(1, EventKind::ToolCall, "read a/b.rs"),
        ev(2, EventKind::ToolCall, "read c/d.rs"),
        ev(3, EventKind::ToolCall, "read a/b.rs again"),
    ];
    assert_eq!(files_touched(&events), vec!["a/b.rs", "c/d.rs"]);
}

#[test]
fn decisions_fold_into_done_without_duplicates() {
    let events = vec![
        ev(1, EventKind::Decision, "use SQLite not JSONL"),
        ev(2, EventKind::Msg, "ok moving on"),
        ev(3, EventKind::Decision, "use SQLite not JSONL"),
    ];
    let brief = Brief::build(BriefInput {
        goal: "ship the ledger".into(),
        done: vec!["scaffolded the crate".into()],
        pending: vec!["write append()".into()],
        constraints: vec![],
        events: &events,
        diff: String::new(),
        handoff: meta(),
    });
    assert_eq!(
        brief.done,
        vec!["scaffolded the crate", "use SQLite not JSONL"]
    );
}

#[test]
fn markdown_is_model_facing_and_ends_with_verify() {
    let events = vec![ev(1, EventKind::ToolCall, "read src/main.rs")];
    let brief = Brief::build(BriefInput {
        goal: "  make bench emit json  ".into(),
        done: vec![],
        pending: vec!["add --json flag".into()],
        constraints: vec!["no new deps".into()],
        events: &events,
        diff: "--- a/x\n+++ b/x\n@@\n-1\n+2\n".into(),
        handoff: meta(),
    });
    let md = brief.render_markdown();

    assert!(md.starts_with("# Handoff brief"));
    assert!(
        md.contains("## goal\nmake bench emit json"),
        "goal is trimmed"
    );
    assert!(md.contains("- add --json flag"));
    assert!(md.contains("- no new deps"));
    assert!(md.contains("- src/main.rs"));
    assert!(md.contains("```diff"));
    // Verify step names both agents and the reason.
    assert!(md.contains("You are codex, taking over from claude-opus (rate limit)"));
    // No storage metadata leaks into the model-facing text.
    assert!(!md.contains("\"at\""));
    assert!(!md.to_lowercase().contains("seq"));
}

#[test]
fn empty_fields_render_as_explicit_placeholders() {
    let brief = Brief::build(BriefInput {
        goal: "x".into(),
        done: vec![],
        pending: vec![],
        constraints: vec![],
        events: &[],
        diff: String::new(),
        handoff: meta(),
    });
    let md = brief.render_markdown();
    assert!(md.contains("(nothing recorded — ask before proceeding)"));
    assert!(md.contains("(clean tree)"));
}

#[test]
fn json_round_trips() {
    let brief = Brief::build(BriefInput {
        goal: "g".into(),
        done: vec!["d".into()],
        pending: vec!["p".into()],
        constraints: vec!["c".into()],
        events: &[],
        diff: "d".into(),
        handoff: meta(),
    });
    let json = brief.to_json();
    let back: Brief = serde_json::from_str(&json).unwrap();
    assert_eq!(brief, back);
    assert_eq!(back.handoff.reason, HandoffReason::RateLimit);
}

#[test]
fn failure_classification_routes_only_transient_agent_faults() {
    use chorus_core::handoff::HandoffReason as R;
    assert_eq!(R::from_failure("Error: 429 rate limit exceeded"), Some(R::RateLimit));
    assert_eq!(R::from_failure("insufficient_quota for this org"), Some(R::QuotaExhausted));
    assert_eq!(R::from_failure("context length exceeded the model window"), Some(R::ContextExhausted));
    assert_eq!(R::from_failure("upstream overloaded, 503"), Some(R::Failover));
    // A real bug is not something the next agent should inherit.
    assert_eq!(R::from_failure("panic: index out of bounds"), None);
    assert_eq!(R::from_failure("no such file or directory"), None);
}

#[test]
fn the_real_limit_messages_from_both_bridges_are_classified() {
    use chorus_core::handoff::HandoffReason;

    // Verbatim from real runs recorded in evals/handoff/cases — these are the
    // only two that matter today, and the old classifier matched neither.
    for msg in [
        "stream error: usageLimitExceeded",
        "Error: you are out of extra usage",
        "agent turn: usageLimitExceeded (plan limit reached)",
    ] {
        assert_eq!(
            HandoffReason::from_failure(msg),
            Some(HandoffReason::QuotaExhausted),
            "did not classify: {msg:?}"
        );
    }

    for msg in [
        "Error: 429 rate limit exceeded",
        "HTTP 429 Too Many Requests",
        "rate_limit_error: please retry after 30s",
    ] {
        assert_eq!(
            HandoffReason::from_failure(msg),
            Some(HandoffReason::RateLimit),
            "did not classify: {msg:?}"
        );
    }

    // Still not routed around: a real fault belongs in front of the user.
    for msg in [
        "syntax error in repo.ts",
        "permission denied",
        "agent closed its output before answering request 1",
    ] {
        assert_eq!(HandoffReason::from_failure(msg), None, "wrongly routed: {msg:?}");
    }
}

/// The first limit message this project ever saw in the wild, verbatim.
///
/// Caught on 12 Sep 2026 from `claude-agent-acp` during a skills test. Every
/// key the classifier had at the time came from eval logs, and not one of them
/// matched it — "usage limit" no, "rate limit" no, "429" no. Keep the exact
/// string: a paraphrase would pass while the real message still failed.
#[test]
fn the_real_claude_limit_message_is_classified_as_exhausted() {
    let observed =
        "agent returned error: {\"code\":-32603,\"message\":\"Internal error: You've hit your \
         limit · resets 9:50pm (Asia/Bangkok)\"}";
    assert_eq!(
        HandoffReason::from_failure(observed),
        Some(HandoffReason::QuotaExhausted),
        "the one message the product exists for must not fall through to a generic failover"
    );
}
