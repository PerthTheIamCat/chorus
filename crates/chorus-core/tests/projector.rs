//! Pure tests for the memory projector.

use chorus_core::projector::{drift, render, AgentFile, Drift, Projection};

fn sample() -> Projection {
    Projection {
        project: "Rust workspace. Build: `cargo build`. Test: `cargo test`.".into(),
        facts: "- ใช้ pnpm ไม่ใช่ npm\n- Postgres 16".into(),
        task_state: "goal: ship it\ndone: -\npending: everything".into(),
    }
}

#[test]
fn filenames_match_each_agent_convention() {
    assert_eq!(AgentFile::Claude.filename(), "CLAUDE.md");
    assert_eq!(AgentFile::Codex.filename(), "AGENTS.md");
    assert_eq!(AgentFile::Gemini.filename(), "GEMINI.md");
}

#[test]
fn rendered_file_carries_marker_and_all_sections() {
    let out = render(AgentFile::Claude, &sample());
    assert!(out.starts_with("<!-- chorus:generated b3:"));
    assert!(out.contains("## Project"));
    assert!(out.contains("## Memory"));
    assert!(out.contains("## Current task"));
    assert!(out.contains("cargo test"));
    assert!(out.contains("Postgres 16"));
}

#[test]
fn empty_sections_are_omitted() {
    let p = Projection {
        facts: "- one fact".into(),
        ..Default::default()
    };
    let out = render(AgentFile::Gemini, &p);
    assert!(out.contains("## Memory"));
    assert!(!out.contains("## Project"));
    assert!(!out.contains("## Current task"));
}

#[test]
fn freshly_rendered_file_reads_back_as_untouched() {
    let out = render(AgentFile::Codex, &sample());
    assert_eq!(drift(Some(&out)), Drift::Untouched);
}

#[test]
fn a_hand_edit_under_the_marker_is_detected() {
    let out = render(AgentFile::Claude, &sample());
    let tampered = out.replace("ship it", "ship it (and also do my laundry)");
    assert_eq!(drift(Some(&tampered)), Drift::Edited);
}

#[test]
fn a_file_without_the_marker_is_foreign() {
    assert_eq!(
        drift(Some("# CLAUDE.md\n\nHand written, do not touch.")),
        Drift::Foreign
    );
    assert_eq!(drift(None), Drift::Missing);
}

#[test]
fn regenerating_identical_projection_is_stable() {
    let a = render(AgentFile::Claude, &sample());
    let b = render(AgentFile::Claude, &sample());
    assert_eq!(a, b, "same projection must hash identically across runs");
}
