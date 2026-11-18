//! R6b: a push or PR the agent starts is journaled before it runs and closed
//! when it reports back; whatever is left open reaches the next agent's
//! brief as "may be half-done".
//!
//! Frames come from `tests/fixtures/side_effect_*.jsonl`, replayed by the mock.

use chorus_cli::session::{Policy, Session};
use chorus_core::handoff::{half_done_notes, HandoffReason};
use chorus_core::journal;
use chorus_core::{ledger, EventKind, Store};
use std::path::{Path, PathBuf};

mod common;

fn replay_bridge(name: &str) -> Vec<String> {
    let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    vec![
        common::probe_bin().to_string_lossy().into_owned(),
        "--serve-mock".into(),
        "--replay".into(),
        f.to_string_lossy().into_owned(),
    ]
}

fn run(fixture: &str) -> (tempfile::TempDir, PathBuf, Session) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");
    let mut s = Session::create(&db, dir.path(), "journal")
        .unwrap()
        .with_bridge(replay_bridge(fixture));
    s.run_turn("claude", "anthropic", "ship it", Policy::AllowAll).unwrap();
    (dir, db, s)
}

fn markers(db: &Path, sess: i64) -> Vec<String> {
    let store = Store::open(db).unwrap();
    ledger::history(store.conn(), sess, 1_000)
        .unwrap()
        .iter()
        .filter(|e| e.kind == EventKind::Decision)
        .map(|e| ledger::body(store.conn(), e).unwrap())
        .filter(|b| journal::is_marker(b))
        .collect()
}

#[test]
fn a_push_that_reported_back_is_journaled_and_closed() {
    let (_dir, db, s) = run("side_effect_done.jsonl");
    let rows = markers(&db, s.session_id());
    assert_eq!(rows.len(), 2, "intent + done, got {rows:?}");
    assert!(rows[0].starts_with(journal::INTENT) && rows[0].contains("git push origin main"));
    assert!(rows[1].starts_with(journal::INTENT_DONE) && rows[1].contains("\"ok\":true"));
    assert!(half_done_notes(&s.transcript().unwrap()).is_empty());

    // Bookkeeping is not a decision: it must not read as "done" work.
    let ts = s.task_state().unwrap();
    assert!(ts.done.iter().all(|d| !journal::is_marker(d)), "markers leaked into done: {:?}", ts.done);
}

#[test]
fn a_push_the_turn_never_finished_reaches_the_next_agent_as_half_done() {
    let (_dir, db, mut s) = run("side_effect_open.jsonl");
    let rows = markers(&db, s.session_id());
    assert_eq!(rows.len(), 1, "an intent and no done mark, got {rows:?}");

    let notes = half_done_notes(&s.transcript().unwrap());
    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains("git push origin main") && notes[0].contains("may be half-done"));

    let r = s
        .handoff(
            "codex",
            "openai",
            HandoffReason::QuotaExhausted,
            "ship it",
            vec![],
            vec![],
            vec![],
            String::new(),
            Policy::AllowAll,
        )
        .unwrap();
    assert!(
        r.brief.pending.iter().any(|p| p.contains("may be half-done") && p.contains("git push origin main")),
        "brief pending: {:?}",
        r.brief.pending
    );
    assert!(r.brief.render_markdown().contains("may be half-done"));
    // And the bookkeeping did not become a "done" decision in the brief.
    assert!(r.brief.done.iter().all(|d| !journal::is_marker(d)));
}
