//! Pure tests for the memory projector.

use chorus_core::projector::{drift, render, AgentFile, Drift, Projection};

fn sample() -> Projection {
    Projection {
        project: "Rust workspace. Build: `cargo build`. Test: `cargo test`.".into(),
        facts: "- ใช้ pnpm ไม่ใช่ npm\n- Postgres 16".into(),
        task_state: "goal: ship it\ndone: -\npending: everything".into(),
        skills: "name | when to use | path\ndeploy | ship it | .chorus/skills/deploy/SKILL.md".into(),
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

/// A skill file keeps its own frontmatter on line 1 — Claude's loader requires
/// it — so its marker trails the content. If `drift_raw` did not exist, every
/// projected skill would read as `Foreign` on the next turn and silently stop
/// being refreshed.
#[test]
fn a_raw_projection_round_trips_and_a_hand_edit_is_noticed() {
    use chorus_core::projector::{drift, drift_raw, render_raw, Drift};

    let skill = "---\nname: deploy\ndescription: ship it\n---\n\nRun the thing.\n";
    let out = render_raw(skill);

    assert!(out.starts_with("---"), "frontmatter must stay the first line: {out}");
    assert_eq!(drift_raw(Some(&out)), Drift::Untouched);
    assert_eq!(
        drift(Some(&out)),
        Drift::Foreign,
        "the leading-marker check cannot read this shape — that is why drift_raw exists"
    );

    let edited = out.replace("Run the thing.", "Run the other thing.");
    assert_eq!(drift_raw(Some(&edited)), Drift::Edited);

    assert_eq!(drift_raw(Some(skill)), Drift::Foreign, "a user's own skill is never ours");
    assert_eq!(drift_raw(None), Drift::Missing);
}

/// Claude loads real skill files, so repeating the index in `CLAUDE.md` would
/// pay for the same skills twice in one prompt. Codex has no loader, so the
/// index is the only way it learns they exist.
#[test]
fn the_skill_index_goes_only_to_agents_without_a_skill_loader() {
    let p = sample();
    let claude = render(AgentFile::Claude, &p);
    let codex = render(AgentFile::Codex, &p);

    assert!(!claude.contains("## Skills"), "Claude reads .claude/skills instead:\n{claude}");
    assert!(codex.contains("## Skills"), "{codex}");
    assert!(codex.contains(".chorus/skills/deploy/SKILL.md"), "{codex}");
}
