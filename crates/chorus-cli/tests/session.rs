//! The Session Engine, driven against the in-repo mock agent (no network).
//! Proves the core invariant: one prompt turn, every structured thing the
//! agent did, lands in one app-owned ledger with an actor.

use chorus_cli::session::{Policy, Session};

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
