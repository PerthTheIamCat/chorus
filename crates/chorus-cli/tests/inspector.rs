//! The context inspector: what it reports must be what a turn really gets,
//! and looking must not change anything.

use chorus_cli::inspector;
use chorus_cli::session::{Policy, Session};
use chorus_core::memory;

mod common;

fn mock_bridge() -> Vec<String> {
    vec![common::probe_bin().to_string_lossy().into_owned(), "--serve-mock".into()]
}

fn session(dir: &tempfile::TempDir) -> Session {
    let db = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();
    Session::create(&db, &cwd, "inspect").unwrap().with_bridge(mock_bridge())
}

#[test]
fn the_report_describes_the_context_a_turn_would_get() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    memory::add(s.store().conn(), 1, "ใช้ pnpm ไม่ใช่ npm").unwrap();
    s.run_turn("codex", "openai", "แก้ปัญหา login ของหน้า order", Policy::AllowAll).unwrap();

    // The message about to be sent decides which facts are relevant (R2).
    let ask = "ใช้ pnpm ไม่ใช่ npm";
    let i = inspector::inspect(&s, 120_000, "", Some(ask)).unwrap();
    // Same text as the real path, and the tiers add up to the same tokens.
    assert_eq!(i.text, s.assemble_context_for(120_000, Some(ask)).unwrap());
    // Without the new message the inspector agrees with the plain path too.
    let plain = inspector::inspect(&s, 120_000, "", None).unwrap();
    assert_eq!(plain.text, s.assemble_context(120_000).unwrap());
    assert_eq!(i.report.total_tokens, i.report.tiers.iter().map(|t| t.tokens).sum::<i64>());
    let task = i.report.tiers.iter().find(|t| t.tier == "TaskState").unwrap();
    assert!(task.tokens > 0, "a session that has talked has task state");
    // The fact is listed with a score because the user said something to score by.
    let m = i.report.memory.iter().find(|m| m.text.contains("pnpm")).expect("fact listed");
    assert!(m.included);
    assert!(m.score.is_some());
    assert!(!m.text.starts_with("- "), "the inspector shows the fact, not the bullet");
    assert!(i.target.is_none());
}

#[test]
fn inspecting_does_not_change_what_ranks_memory() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    memory::add(s.store().conn(), 1, "ใช้ pnpm ไม่ใช่ npm").unwrap();
    s.run_turn("codex", "openai", "แก้ปัญหา login", Policy::AllowAll).unwrap();
    let hits = |s: &Session| -> i64 {
        s.store()
            .conn()
            .query_row("SELECT SUM(hits) FROM fact", [], |r| r.get::<_, Option<i64>>(0))
            .unwrap()
            .unwrap_or(0)
    };
    let before = hits(&s);
    // Ranked against the new message, as a real turn is (R2 drops facts that
    // do not match it, so the query has to match the fact).
    let ask = "ใช้ pnpm ไม่ใช่ npm";
    for _ in 0..3 {
        inspector::inspect(&s, 120_000, "codex", Some(ask)).unwrap();
    }
    assert_eq!(hits(&s), before, "looking must not count as the agent using the fact");
    // A real assembly still counts, as it always did.
    s.assemble_context_for(120_000, Some(ask)).unwrap();
    assert!(hits(&s) > before);
}

#[test]
fn the_target_preview_carries_the_brief_and_writes_no_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(&dir);
    s.run_turn("claude", "anthropic", "ย้าย auth ไป OAuth", Policy::AllowAll).unwrap();
    let events_before = s.events_since(0).unwrap().len();

    let i = inspector::inspect(&s, 120_000, "codex", None).unwrap();
    let t = i.target.expect("target preview");
    assert_eq!(t.agent, "codex");
    assert!(t.brief_markdown.contains("OAuth"), "{}", t.brief_markdown);
    assert!(t.brief_tokens > 0 && t.context_tokens > 0);
    assert_eq!(t.instruction_file.as_deref(), Some("AGENTS.md"));
    assert!(!t.budget_known, "no window is configured, and the preview says so");
    assert_eq!(t.budget, 120_000);
    assert_eq!(s.events_since(0).unwrap().len(), events_before, "a preview is not a hand-off");
}
