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
