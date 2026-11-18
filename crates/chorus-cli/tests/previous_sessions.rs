//! A new session in a project that already has history: what the agent is
//! handed at turn one, and that facts of every readable scope reach it.

use chorus_cli::session::{Policy, Session};
use chorus_core::memory;

mod common;

fn mock_bridge() -> Vec<String> {
    vec![common::probe_bin().to_string_lossy().into_owned(), "--serve-mock".into()]
}

#[test]
fn a_new_session_starts_knowing_the_earlier_one() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut first = Session::create(&db, &cwd, "first").unwrap().with_bridge(mock_bridge());
    first.run_turn("codex", "openai", "ย้าย auth ไป OAuth ให้เสร็จ", Policy::AllowAll).unwrap();
    let first_id = first.session_id();
    drop(first);

    let mut second = Session::create(&db, &cwd, "second").unwrap().with_bridge(mock_bridge());
    second.run_turn("codex", "openai", "ต่อจากเมื่อวาน", Policy::AllowAll).unwrap();
    let ctx = second.assemble_context(200_000).unwrap();
    assert!(ctx.contains("previous sessions"), "{ctx}");
    assert!(ctx.contains(&format!("#{first_id}")), "{ctx}");
    assert!(ctx.contains("ย้าย auth ไป OAuth"), "{ctx}");

    // The lines are task state: they survive a tight window, and stay short.
    let tight = second.assemble_context(800).unwrap();
    assert!(tight.contains("ย้าย auth ไป OAuth"), "{tight}");

    // They are context only: not part of the task state that goes into
    // handoff briefs or the projected agent files.
    assert!(!second.task_state().unwrap().render().contains("previous sessions"));

    // The first session itself has nothing earlier to show.
    let again = Session::open(&db, first_id).unwrap();
    assert!(!again.assemble_context(200_000).unwrap().contains("previous sessions"));
}

#[test]
fn approved_facts_of_every_scope_are_assembled_and_listed() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();
    let s = Session::create(&db, &cwd, "scopes").unwrap();
    let c = s.store().conn();
    memory::add(c, 0, "global-fact ตอบไทย").unwrap();
    memory::add(c, 1, "project-fact ใช้ pnpm").unwrap();
    memory::add(c, 2, "legacy-session-fact อย่าแตะ migrations").unwrap();

    let ctx = s.assemble_context(200_000).unwrap();
    for needle in ["global-fact", "project-fact", "legacy-session-fact"] {
        assert!(ctx.contains(needle), "{needle} missing from context:\n{ctx}");
    }
    let (facts, _) = s.memory_state().unwrap();
    assert_eq!(facts.len(), 3, "{facts:?}");
}

#[test]
fn the_first_turn_of_a_new_session_already_carries_the_earlier_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut first = Session::create(&db, &cwd, "first").unwrap().with_bridge(mock_bridge());
    first.run_turn("codex", "openai", "ย้าย auth ไป OAuth ให้เสร็จ", Policy::AllowAll).unwrap();
    let first_id = first.session_id();
    drop(first);

    // The mock writes what reached it to mock-saw.txt when asked to.
    let mut second = Session::create(&db, &cwd, "second").unwrap().with_bridge(mock_bridge());
    second.run_turn("codex", "openai", "MOCK_ECHO_SECRETS ต่อจากเมื่อวาน", Policy::AllowAll).unwrap();
    let saw = std::fs::read_to_string(cwd.join("mock-saw.txt")).unwrap();
    assert!(saw.contains("previous sessions:"), "{saw}");
    assert!(saw.contains(&format!("#{first_id}")) && saw.contains("ย้าย auth ไป OAuth"), "{saw}");
    assert!(saw.contains("ต่อจากเมื่อวาน"), "{saw}");

    // Read-only: the ledger holds the person's words, not the block.
    let stored: i64 = second
        .store()
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM event WHERE sess = ?1 AND body LIKE '%previous sessions%'",
            [second.session_id()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, 0, "the previous-sessions block leaked into the ledger");

    // The very first session of a folder is sent untouched.
    let cwd2 = dir.path().join("w2");
    std::fs::create_dir_all(&cwd2).unwrap();
    let mut alone = Session::create(&dir.path().join("t.db"), &cwd2, "alone").unwrap().with_bridge(mock_bridge());
    alone.run_turn("codex", "openai", "MOCK_ECHO_SECRETS hello", Policy::AllowAll).unwrap();
    let saw = std::fs::read_to_string(cwd2.join("mock-saw.txt")).unwrap();
    assert!(!saw.contains("previous sessions"), "{saw}");
}
