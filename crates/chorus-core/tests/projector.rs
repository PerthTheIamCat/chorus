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

#[test]
fn loads_rules_from_dot_chorus_rules_in_deterministic_order() {
    use chorus_core::projector::load_project_rules;
    let dir = tempfile::tempdir().unwrap();
    let rules_dir = dir.path().join(".chorus").join("rules");
    std::fs::create_dir_all(&rules_dir).unwrap();

    std::fs::write(
        rules_dir.join("02-testing.md"),
        "Always run cargo test before finishing.",
    )
    .unwrap();
    std::fs::write(
        rules_dir.join("01-architecture.md"),
        "Use headless core architecture.",
    )
    .unwrap();
    // Non-markdown file should be ignored
    std::fs::write(rules_dir.join("notes.txt"), "ignored").unwrap();

    let loaded = load_project_rules(dir.path());
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].source, ".chorus/rules/01-architecture.md");
    assert_eq!(loaded[0].content, "Use headless core architecture.");
    assert_eq!(loaded[1].source, ".chorus/rules/02-testing.md");
    assert_eq!(
        loaded[1].content,
        "Always run cargo test before finishing."
    );
}

#[test]
fn loads_foreign_agents_and_claude_md_and_deduplicates_identical() {
    use chorus_core::projector::load_project_rules;
    let dir = tempfile::tempdir().unwrap();

    std::fs::write(
        dir.path().join("AGENTS.md"),
        "# Shared Instructions\nPrefer small functions.",
    )
    .unwrap();
    // Identical CLAUDE.md is deduplicated to save context tokens
    std::fs::write(
        dir.path().join("CLAUDE.md"),
        "# Shared Instructions\nPrefer small functions.",
    )
    .unwrap();

    let loaded = load_project_rules(dir.path());
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].source, "AGENTS.md");

    // But different CLAUDE.md is preserved
    std::fs::write(
        dir.path().join("CLAUDE.md"),
        "# Claude Specific\nBe concise.",
    )
    .unwrap();
    let loaded2 = load_project_rules(dir.path());
    assert_eq!(loaded2.len(), 2);
    assert_eq!(loaded2[0].source, "AGENTS.md");
    assert_eq!(loaded2[1].source, "CLAUDE.md");
}

#[test]
fn ignores_untouched_generated_agent_files_when_loading_rules() {
    use chorus_core::projector::{load_project_rules, render, AgentFile};
    let dir = tempfile::tempdir().unwrap();

    let p = sample();
    let rendered = render(AgentFile::Codex, &p);
    std::fs::write(dir.path().join("AGENTS.md"), rendered).unwrap();

    let loaded = load_project_rules(dir.path());
    assert!(
        loaded.is_empty(),
        "Chorus generated files should not be re-ingested as project rules"
    );
}

#[test]
fn assembles_rules_into_system_prompt_across_bridges() {
    use chorus_core::projector::{
        assemble_rules, project_rules_into_system_prompt,
        project_system_prompt_for_bridge, AgentFile, ProjectRule,
    };

    let rules = vec![
        ProjectRule::new(".chorus/rules/style.md", "Use snake_case for vars."),
        ProjectRule::new("AGENTS.md", "Never break tests."),
    ];

    let assembled = assemble_rules(&rules);
    assert!(assembled.contains("# Project Rules & Instructions"));
    assert!(assembled.contains("## Rule: .chorus/rules/style.md"));
    assert!(assembled.contains("Use snake_case for vars."));
    assert!(assembled.contains("## Rule: AGENTS.md"));
    assert!(assembled.contains("Never break tests."));

    let base = "You are an autonomous engineering assistant.";
    let prompt = project_rules_into_system_prompt(base, &rules);
    assert!(prompt.starts_with(base));
    assert!(prompt.contains("# Project Rules & Instructions"));

    // Across bridges
    let claude_prompt = project_system_prompt_for_bridge(AgentFile::Claude, base, &rules);
    assert!(claude_prompt.contains("Target Agent Bridge: CLAUDE.md"));
    assert!(claude_prompt.contains("Use snake_case for vars."));

    let codex_prompt = project_system_prompt_for_bridge(AgentFile::Codex, base, &rules);
    assert!(codex_prompt.contains("Target Agent Bridge: AGENTS.md"));
    assert!(codex_prompt.contains("Use snake_case for vars."));
}

