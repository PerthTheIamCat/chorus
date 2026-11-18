//! A child forked from the lead sees the lead's goal through the normal
//! context rungs, and stays inside its budget (F3).

use chorus_cli::session::{Policy, Session};
use chorus_core::agent_control;
use std::path::PathBuf;

fn mock_bridge() -> Vec<String> {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    vec![
        p.join("acp-probe").to_string_lossy().into_owned(),
        "--serve-mock".into(),
    ]
}

#[test]
fn child_context_carries_the_leads_goal_within_budget() {
    let dir = tempfile::tempdir().unwrap();
    let db: PathBuf = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut lead = Session::create(&db, &cwd, "lead")
        .unwrap()
        .with_bridge(mock_bridge());
    lead.run_turn(
        "codex",
        "openai",
        "ทำ order list ให้เร็วกว่า 200ms",
        Policy::AllowAll,
    )
    .unwrap();
    for i in 0..12 {
        lead.run_turn(
            "codex",
            "openai",
            &format!("continue step {i}: keep editing the order repository"),
            Policy::AllowAll,
        )
        .unwrap();
    }
    let lead_id = lead.session_id();

    let sub = agent_control::spawn_agent(
        lead.store().conn(),
        Some(lead_id),
        "investigator",
        "check the index",
        false,
        cwd.to_str().unwrap(),
    )
    .unwrap();
    let child = Session::open(&db, sub.child_sess.unwrap()).unwrap();

    let roomy = child.assemble_context(120_000).unwrap();
    assert!(
        roomy.contains("order list"),
        "the lead's goal never reached the child:\n{roomy}"
    );

    // Squeezed, the child still gets the goal and does not blow the budget.
    let budget = 1_500;
    let tight = child.assemble_context(budget).unwrap();
    assert!(tight.contains("order list"), "goal trimmed:\n{tight}");
    assert!(
        tight.len() < (budget as usize) * 5,
        "child context {} bytes is far over a {budget}-token budget",
        tight.len()
    );
}

#[test]
fn child_view_and_goal_are_its_own_while_context_keeps_the_lead() {
    let dir = tempfile::tempdir().unwrap();
    let db: PathBuf = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut lead = Session::create(&db, &cwd, "lead")
        .unwrap()
        .with_bridge(mock_bridge());
    lead.run_turn("codex", "openai", "ทำ order list ให้เร็วกว่า 200ms", Policy::AllowAll)
        .unwrap();
    let lead_events = lead.events_since(0).unwrap().len();
    assert!(lead_events > 0);

    let sub = agent_control::spawn_agent(
        lead.store().conn(),
        Some(lead.session_id()),
        "investigator",
        "check the index on orders",
        false,
        cwd.to_str().unwrap(),
    )
    .unwrap();
    let child_id = sub.child_sess.unwrap();
    let mut child = Session::open(&db, child_id).unwrap().with_bridge(mock_bridge());
    child
        .run_turn("codex", "openai", "child prompt", Policy::AllowAll)
        .unwrap();

    // Display: only the child's own events, not the lead's replay.
    let shown = child.events_since(0).unwrap();
    assert!(!shown.is_empty());
    assert!(
        shown.iter().all(|e| !e.3.contains("order list")),
        "lead history leaked into the child's view: {shown:?}"
    );
    assert!(child.transcript().unwrap().iter().all(|(e, _)| e.sess == child_id));
    // The lead's own view is unchanged.
    assert_eq!(lead.events_since(0).unwrap().len(), lead_events);

    // Goal: the child's own, with the lead's as a context line.
    let ts = child.task_state().unwrap();
    assert!(ts.goal.starts_with("check the index on orders"), "{ts:?}");
    assert!(ts.goal.contains("lead goal: ทำ order list"), "{ts:?}");
    // Context still carries the lead's history.
    assert!(child.assemble_context(120_000).unwrap().contains("order list"));
}

#[test]
fn a_fork_lead_without_own_events_still_passes_its_history_down() {
    let dir = tempfile::tempdir().unwrap();
    let db: PathBuf = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();
    let mut base = Session::create(&db, &cwd, "base")
        .unwrap()
        .with_bridge(mock_bridge());
    base.run_turn("codex", "openai", "inherited goal text", Policy::AllowAll)
        .unwrap();
    let head = base.events_since(0).unwrap().last().unwrap().0;
    let forked = chorus_core::ledger::fork(base.store().conn(), base.session_id(), head, "fork")
        .unwrap();

    let sub = agent_control::spawn_agent(
        base.store().conn(),
        Some(forked),
        "investigator",
        "look",
        false,
        cwd.to_str().unwrap(),
    )
    .unwrap();
    let child = Session::open(&db, sub.child_sess.unwrap()).unwrap();
    assert!(child
        .assemble_context(120_000)
        .unwrap()
        .contains("inherited goal text"));
}
