//! Memory that learns, end to end through a real `Session`: usage counters are
//! written by the turn (not by assembling), compaction drafts lessons into the
//! approve queue, and a rejected hunk leaves a trace.

use chorus_cli::session::{Policy, Session};
use chorus_core::memory::{self, ProposalKind};

mod common;

fn mock_bridge() -> Vec<String> {
    vec![common::probe_bin().to_string_lossy().into_owned(), "--serve-mock".into()]
}

fn counters(s: &Session, id: i64) -> (i64, i64) {
    s.store().conn().query_row("SELECT injected, referenced FROM fact WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
}

fn workspace() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();
    (dir, db, cwd)
}

#[test]
fn a_turn_counts_the_facts_it_carried_and_the_ones_the_agent_used() {
    let (_d, db, cwd) = workspace();
    let mut s = Session::create(&db, &cwd, "counters").unwrap().with_bridge(mock_bridge());
    let used = memory::add(s.store().conn(), 1, "README.md ต้องเป็นภาษาไทย").unwrap();
    let idle = memory::add(s.store().conn(), 1, "deploy ด้วย fly deploy เท่านั้น").unwrap();

    // The first turn has no history to assemble, so nothing is injected.
    s.run_turn("codex", "openai", "แก้ README", Policy::AllowAll).unwrap();
    assert_eq!(counters(&s, used), (0, 0));

    // Assembling by itself changes no counter (rule 7.2b)…
    s.assemble_context(200_000).unwrap();
    s.assemble_context(200_000).unwrap();
    assert_eq!(counters(&s, used), (0, 0));

    // …the turn that follows does, for the facts that are about what the user
    // just said (R2: an unrelated fact is no longer topped up). The mock touches
    // README.md and says nothing about deploying.
    s.run_turn("codex", "openai", "แก้ README.md ต่อ", Policy::AllowAll).unwrap();
    assert_eq!(counters(&s, used), (1, 1));
    assert_eq!(counters(&s, idle), (0, 0), "an unrelated fact is not carried");
    s.run_turn("codex", "openai", "deploy ด้วย fly deploy เลย", Policy::AllowAll).unwrap();
    assert_eq!(counters(&s, idle), (1, 0));
}

#[test]
fn compact_drafts_lessons_once_and_never_into_the_facts() {
    let (_d, db, cwd) = workspace();
    let mut s = Session::create(&db, &cwd, "lessons").unwrap().with_bridge(mock_bridge());
    s.run_turn("codex", "openai", "ทำ cache ให้หน่อย", Policy::AllowAll).unwrap();
    s.run_turn("codex", "openai", "ไม่ใช่ อย่าใช้ redis", Policy::AllowAll).unwrap();

    let r = s.compact(200).unwrap();
    assert_eq!(r.learned.lessons, 1, "{r:?}");
    let q = memory::pending(s.store().conn()).unwrap();
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].kind, ProposalKind::Lesson);
    assert!(q[0].text.contains("อย่าใช้ redis"), "{}", q[0].text);
    assert!(q[0].evidence.as_deref().unwrap().starts_with(&format!("session {}", s.session_id())));
    assert!(memory::top(s.store().conn(), 1, 10).unwrap().is_empty(), "a lesson is not a fact until approved");
    let (_, pending) = s.memory_state().unwrap();
    assert_eq!(pending.len(), 1);

    // A second pass over the same ledger drafts nothing, even after a reject.
    memory::reject(s.store().conn(), q[0].id).unwrap();
    assert_eq!(s.compact(200).unwrap().learned.lessons, 0);
    assert!(memory::pending(s.store().conn()).unwrap().is_empty());
}

#[test]
fn compact_suggests_retiring_a_fact_that_points_at_a_deleted_file() {
    let (_d, db, cwd) = workspace();
    let mut s = Session::create(&db, &cwd, "retire").unwrap().with_bridge(mock_bridge());
    memory::add(s.store().conn(), 1, "entry point คือ src/old_main.rs").unwrap();
    s.run_turn("codex", "openai", "hi", Policy::AllowAll).unwrap();
    assert_eq!(s.compact(200).unwrap().learned.retirements, 1);
    let q = memory::pending(s.store().conn()).unwrap();
    assert_eq!((q[0].kind, q[0].evidence.as_deref().map(|e| e.contains("src/old_main.rs"))), (ProposalKind::Retire, Some(true)));
}

#[test]
fn rejecting_hunks_leaves_a_decision_the_lesson_pass_turns_into_advice() {
    let (_d, db, cwd) = workspace();
    std::fs::write(cwd.join("a.txt"), "one\n").unwrap();
    let mut s = Session::create(&db, &cwd, "reject").unwrap().with_bridge(mock_bridge());
    s.run_turn("codex", "openai", "hi", Policy::AllowAll).unwrap();
    std::fs::write(cwd.join("a.txt"), "two\n").unwrap();
    let patch = s.working_diff();
    assert!(patch.contains("a.txt"), "needs git for the shadow repo: {patch:?}");

    s.reject_hunks(&patch).unwrap();
    assert_eq!(std::fs::read_to_string(cwd.join("a.txt")).unwrap(), "one\n");
    let trail: Vec<_> = s.transcript().unwrap().into_iter().map(|(_, b)| b).collect();
    assert!(trail.iter().any(|b| b.starts_with(chorus_core::lessons::REJECTED_PREFIX) && b.contains("a.txt")), "{trail:?}");

    assert!(s.compact(200).unwrap().learned.lessons >= 1);
    let q = memory::pending(s.store().conn()).unwrap();
    assert!(q.iter().any(|p| p.text.contains("a.txt") && p.kind == ProposalKind::Lesson), "{q:?}");
}

#[test]
fn claude_auto_memory_is_switched_off_for_claude_unless_the_user_allows_it() {
    use chorus_cli::session::provider_memory_env;
    let off = vec![("CLAUDE_CODE_DISABLE_AUTO_MEMORY".to_string(), "1".to_string())];
    assert_eq!(provider_memory_env("claude", false), off);
    assert_eq!(provider_memory_env("claude-opus@work", false), off);
    // Allowed: add nothing — the user's own setting stays as it is.
    assert!(provider_memory_env("claude", true).is_empty());
    // Other agents have no such memory to switch off.
    assert!(provider_memory_env("codex", false).is_empty());
    assert!(provider_memory_env("antigravity", false).is_empty());
}
