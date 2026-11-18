//! A new session in a project that already has history: what the agent is
//! handed at turn one, and that facts of every readable scope reach it.

use chorus_cli::session::{Policy, Session};
use chorus_core::memory;

fn mock_bridge() -> Vec<String> {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    vec![p.join("acp-probe").to_string_lossy().into_owned(), "--serve-mock".into()]
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
