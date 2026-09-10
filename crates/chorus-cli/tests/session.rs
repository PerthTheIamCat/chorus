//! The Session Engine, driven against the in-repo mock agent (no network).
//! Proves the core invariant: one prompt turn, every structured thing the
//! agent did, lands in one app-owned ledger with an actor.

use chorus_cli::session::{Policy, Session};
use chorus_core::handoff::HandoffReason;

fn mock_bridge() -> Vec<String> {
    // target/debug/deps/<test> → target/debug/acp-probe --serve-mock
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    vec![p.join("acp-probe").to_string_lossy().into_owned(), "--serve-mock".into()]
}

#[test]
fn one_turn_persists_user_and_every_agent_event_to_the_ledger() {
    let tmp = std::env::temp_dir().join(format!("chorus-sess-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let db = tmp.join("store.db");
    let _ = std::fs::remove_file(&db);

    let mut s = Session::create(&db, &tmp, "mock task")
        .unwrap()
        .with_bridge(mock_bridge());

    let report = s
        .run_turn("codex", "openai", "look at the readme", Policy::AllowAll)
        .expect("turn runs");

    assert_eq!(report.stop_reason, "end_turn");
    assert!(!report.agent_seqs.is_empty(), "no agent events persisted");

    // Everything is in one session, user first, agent events parented to it.
    let transcript = s.transcript().unwrap();
    let kinds: Vec<_> = transcript.iter().map(|(e, _)| e.kind).collect();
    assert_eq!(kinds.first(), Some(&chorus_core::EventKind::Msg), "user msg leads");
    assert!(
        kinds.contains(&chorus_core::EventKind::ToolCall),
        "tool call not in ledger: {kinds:?}"
    );
    assert!(
        kinds.contains(&chorus_core::EventKind::Diff),
        "diff not in ledger: {kinds:?}"
    );

    // The diff event carries a path files_touched can recover.
    let diff_body = transcript
        .iter()
        .find(|(e, _)| e.kind == chorus_core::EventKind::Diff)
        .map(|(_, b)| b.clone())
        .unwrap();
    let files = chorus_core::handoff::files_touched(&[(
        transcript[0].0.clone(),
        diff_body.clone(),
    )]);
    assert!(files.iter().any(|f| f.ends_with("README.md")), "diff path lost: {diff_body}");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn a_second_turn_reconstructs_history_from_the_ledger() {
    let tmp = std::env::temp_dir().join(format!("chorus-multi-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let db = tmp.join("store.db");
    let _ = std::fs::remove_file(&db);

    let mut s = Session::create(&db, &tmp, "multi turn")
        .unwrap()
        .with_bridge(mock_bridge());

    s.run_turn("codex", "openai", "first: look at the readme", Policy::AllowAll).unwrap();

    // Context assembled from the ledger carries the first turn back.
    let ctx = s.assemble_context(120_000).unwrap();
    assert!(ctx.contains("first: look at the readme"), "history missing from context:\n{ctx}");
    assert!(ctx.contains("user:") || ctx.contains("user :") || ctx.contains("codex:"));

    let r2 = s
        .run_turn("codex", "openai", "second: now summarise", Policy::AllowAll)
        .unwrap();
    assert_eq!(r2.stop_reason, "end_turn");

    let user_msgs: Vec<_> = s
        .transcript()
        .unwrap()
        .into_iter()
        .filter(|(e, _)| e.kind == chorus_core::EventKind::Msg && e.body.is_some())
        .filter(|(_, b)| b.starts_with("first:") || b.starts_with("second:"))
        .map(|(_, b)| b)
        .collect();
    assert_eq!(user_msgs, vec!["first: look at the readme", "second: now summarise"]);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn handoff_records_the_switch_and_runs_the_incoming_agent() {
    let tmp = std::env::temp_dir().join(format!("chorus-ho-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let db = tmp.join("store.db");
    let _ = std::fs::remove_file(&db);

    let mut s = Session::create(&db, &tmp, "n+1 query")
        .unwrap()
        .with_bridge(mock_bridge());

    // A first turn by "claude", so the handoff has a `from`.
    s.run_turn("claude-opus", "anthropic", "the order page is slow", Policy::AllowAll)
        .unwrap();

    let r = s
        .handoff(
            "codex",
            "openai",
            HandoffReason::RateLimit,
            "get the order list under 200ms",
            vec!["found the N+1 in findAll".into()],
            vec!["add eager-load".into(), "run the order tests".into()],
            vec!["do not touch the schema".into()],
            "--- a/repo.ts\n+++ b/repo.ts\n@@\n-x\n+y\n".into(),
            Policy::AllowAll,
        )
        .expect("handoff runs");

    assert_eq!(r.from, "claude-opus");
    assert_eq!(r.to, "codex");
    assert_eq!(r.brief.goal, "get the order list under 200ms");
    assert_eq!(r.brief.pending, vec!["add eager-load", "run the order tests"]);
    assert!(!r.echo_back.is_empty(), "incoming agent didn't respond");

    // The ledger holds: claude's turn, a Handoff event, then codex's events.
    let t = s.transcript().unwrap();
    let handoff_at = t
        .iter()
        .position(|(e, _)| e.kind == chorus_core::EventKind::Handoff)
        .expect("handoff event in ledger");
    // The handoff body is the full brief JSON.
    assert!(t[handoff_at].1.contains("\"goal\": \"get the order list under 200ms\""));
    // The handoff event and everything after share one actor (codex), and
    // it differs from claude's actor id on the first turn.
    let claude_actor = t[0].0.actor;
    let codex_actor = t[handoff_at].0.actor;
    assert_ne!(claude_actor, codex_actor, "handoff should switch actor");
    assert!(
        t[handoff_at + 1..].iter().all(|(e, _)| e.actor == codex_actor),
        "post-handoff events must belong to the incoming agent"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn the_ledger_survives_when_the_agent_says_no() {
    let tmp = std::env::temp_dir().join(format!("chorus-sess-deny-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let db = tmp.join("store.db");
    let _ = std::fs::remove_file(&db);

    let mut s = Session::create(&db, &tmp, "deny task")
        .unwrap()
        .with_bridge(mock_bridge());

    // DenyAll: the mock still finishes the turn, marking its tool call failed.
    let report = s
        .run_turn("codex", "openai", "try to edit something", Policy::DenyAll)
        .expect("turn still completes");

    assert_eq!(report.stop_reason, "end_turn");
    let transcript = s.transcript().unwrap();
    assert!(
        transcript
            .iter()
            .any(|(e, b)| e.kind == chorus_core::EventKind::ToolResult && b.contains("failed")),
        "denied tool call should be recorded as failed"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
