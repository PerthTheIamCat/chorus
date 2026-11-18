//! Multi-agent v2 end to end with the mock agent: a writer's scope is
//! labelled, and a one-shot opinion really goes to another provider.

use chorus_cli::session::Session;
use chorus_core::agent_control::{self, SubagentStatus, TaskSpec};
use chorus_core::router::Candidate;
use std::path::Path;
use std::process::Command;

mod common;

fn mock_bridge() -> Vec<String> {
    vec![
        common::probe_bin().to_string_lossy().into_owned(),
        "--serve-mock".into(),
    ]
}

fn rogue_bridge(name: &str) -> Vec<String> {
    let mut b = mock_bridge();
    b.extend(["--rogue".into(), name.into()]);
    b
}

fn cand(agent: &str, id: &str, tier: u8) -> Candidate {
    let (base, effort) = chorus_cli::orchestra::split_effort(id);
    Candidate {
        id: format!("{agent}:{id}"),
        agent: agent.into(),
        model: base.into(),
        effort: effort.unwrap_or("default").into(),
        media_types: vec![],
        integrations: vec![],
        ready: true,
        tier: Some(tier),
    }
}

fn init_git_repo(dir: &Path) {
    let run = |args: &[&str]| {
        let _ = Command::new("git").args(args).current_dir(dir).output();
    };
    run(&["init"]);
    run(&["config", "user.email", "chorus@test.local"]);
    run(&["config", "user.name", "Chorus Tester"]);
    std::fs::write(dir.join("README.md"), "# Test Repo\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "initial commit"]);
}

fn remove_worktree(repo: &Path, wt: &str) {
    let _ = Command::new("git")
        .args(["worktree", "remove", "--force", wt])
        .current_dir(repo)
        .output();
}

#[test]
fn a_writer_that_edits_outside_its_files_gets_a_visible_label() {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    let mut session = Session::create(&dir.path().join("store.db"), dir.path(), "lead")
        .unwrap()
        .with_bridge(rogue_bridge("elsewhere.txt"));
    session.set_child_candidates(vec![cand("codex", "gpt-5.6-luna[low]", 0)]);
    // The user allows everything: children are capped by that, not by the
    // read-only lead (this also hands the runner the policy).
    let _ = session.run_orchestrator_turn(
        "plan it",
        &[],
        "",
        "",
        chorus_cli::session::Policy::AllowAll,
    );
    let conn = session.store().conn();

    let spec = TaskSpec {
        goal: "write the parser".into(),
        files_owned: vec!["src/parser".into()],
        ..Default::default()
    };
    let w = agent_control::spawn_agent_with_options(
        conn,
        Some(session.session_id()),
        "builder",
        &spec.encode(),
        Some("light"),
        true,
        dir.path().to_str().unwrap(),
        None,
    )
    .unwrap();
    assert!(
        agent_control::wait_agent(conn, Some(session.session_id()), Some(w.id), 30)
            .unwrap()
            .all_done
    );
    let w = agent_control::get_subagent(conn, w.id).unwrap();
    assert_eq!(w.status, SubagentStatus::Completed, "{w:?}");
    let summary = w.summary.clone().unwrap_or_default();
    assert!(
        summary.contains("แก้ไฟล์นอก files_owned") && summary.contains("elsewhere.txt"),
        "{summary}"
    );
    // The project is not trusted in a test, so the check is reported as not
    // run rather than silently passed.
    assert!(
        summary.contains("ไม่ได้ตรวจ") || summary.contains("ตรวจแล้วผ่าน"),
        "{summary}"
    );
    remove_worktree(dir.path(), &w.worktree.unwrap());
}

/// Who ran the child. A read-only child cannot run without Seatbelt (CLAUDE.md
/// 7.22), so off macOS it fails, naming the agent it tried.
fn agent_of(sub: &agent_control::Subagent) -> String {
    if let Some(m) = &sub.model {
        return m.clone();
    }
    sub.summary
        .as_deref()
        .and_then(|s| s.strip_prefix("Failed: "))
        .and_then(|s| s.split(':').next())
        .unwrap_or_default()
        .to_string()
}

fn ask(session: &Session, cwd: &str, asker: &str) -> agent_control::Subagent {
    let conn = session.store().conn();
    let spec = TaskSpec {
        goal: format!("opinion for {asker}"),
        prefer_provider: format!("other_than:{asker}"),
        ..Default::default()
    };
    let a = agent_control::spawn_agent_with_options(
        conn,
        Some(session.session_id()),
        "reviewer",
        &spec.encode(),
        Some("normal"),
        false,
        cwd,
        None,
    )
    .unwrap();
    agent_control::wait_agent(conn, Some(session.session_id()), Some(a.id), 30).unwrap();
    agent_control::get_subagent(conn, a.id).unwrap()
}

#[test]
fn a_one_shot_opinion_runs_on_another_provider_than_the_asker() {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    let mut session = Session::create(&dir.path().join("store.db"), dir.path(), "lead")
        .unwrap()
        .with_bridge(mock_bridge());
    session.set_child_candidates(vec![
        cand("codex", "gpt-5.6-luna[low]", 0),
        cand("claude", "opus", 2),
    ]);
    let cwd = dir.path().to_str().unwrap().to_string();
    let from_codex = ask(&session, &cwd, "codex");
    assert_eq!(agent_of(&from_codex), "claude", "{from_codex:?}");
    assert!(
        from_codex.worktree.is_none(),
        "an opinion never gets a worktree"
    );
    let from_claude = ask(&session, &cwd, "claude");
    assert_eq!(agent_of(&from_claude), "codex", "{from_claude:?}");

    // With one provider ready there is nobody different to ask.
    session.set_child_candidates(vec![cand("codex", "gpt-5.6-luna[low]", 0)]);
    let alone = ask(&session, &cwd, "codex");
    assert_eq!(alone.status, SubagentStatus::Failed, "{alone:?}");
    assert!(
        alone
            .summary
            .unwrap_or_default()
            .contains("different provider"),
        "the reason is stated"
    );
}
