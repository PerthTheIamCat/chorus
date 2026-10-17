//! Integration tests for Orchestra mode core (Issue 12).
//!
//! Done when:
//! - Integration test with the mock agent: spawn two children, one writer and one reader,
//!   collect their briefs, hit the cap and pause, cancel, and check no orphan processes remain.
//! - The `complete_agent` and `wait_agent` timeouts work for real.

use chorus_cli::session::Session;
use chorus_core::agent_control::{self, complete_agent, wait_agent, OrchestraCaps, SubagentStatus};
use chorus_core::Store;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn probe_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("acp-probe")
}

fn mock_bridge() -> Vec<String> {
    vec![
        probe_bin().to_string_lossy().into_owned(),
        "--serve-mock".into(),
    ]
}

fn init_git_repo(dir: &Path) {
    let _ = Command::new("git").arg("init").current_dir(dir).output();
    let _ = Command::new("git")
        .args(["config", "user.email", "chorus@test.local"])
        .current_dir(dir)
        .output();
    let _ = Command::new("git")
        .args(["config", "user.name", "Chorus Tester"])
        .current_dir(dir)
        .output();
    fs::write(dir.join("README.md"), "# Test Repo\n").unwrap();
    let _ = Command::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .output();
    let _ = Command::new("git")
        .args(["commit", "-m", "initial commit"])
        .current_dir(dir)
        .output();
}

#[test]
fn complete_agent_and_wait_agent_timeouts_work_for_real() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");
    let store = Store::open(&db).unwrap();
    let sess = chorus_core::ledger::new_session(store.conn(), "root", dir.path().to_str().unwrap())
        .unwrap();

    // 1. Spawn a subagent
    let agent = agent_control::spawn_agent(
        store.conn(),
        Some(sess),
        "investigator",
        "Investigate codebase",
        false,
        dir.path().to_str().unwrap(),
    )
    .unwrap();

    // 2. Wait with 1s timeout when agent is still running: must timeout and return all_done = false
    let start = std::time::Instant::now();
    let rep = wait_agent(store.conn(), Some(sess), Some(agent.id), 1).unwrap();
    assert!(
        !rep.all_done,
        "agent is running, wait must report all_done = false"
    );
    assert!(
        start.elapsed() >= std::time::Duration::from_millis(900),
        "wait_agent must respect timeout"
    );

    // 3. Complete the agent in a background thread after a short delay
    let db_clone = db.clone();
    let agent_id = agent.id;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        let s = Store::open(&db_clone).unwrap();
        complete_agent(
            s.conn(),
            agent_id,
            "Done investigating: all clean.",
            Some("+5 -0"),
            &["src/lib.rs".to_string()],
        )
        .unwrap();
    });

    // 4. Wait with 5s timeout: must return early as soon as complete_agent completes
    let start2 = std::time::Instant::now();
    let rep2 = wait_agent(store.conn(), Some(sess), Some(agent.id), 5).unwrap();
    assert!(
        rep2.all_done,
        "agent completed, wait must report all_done = true"
    );
    assert_eq!(rep2.completed, vec![agent.id]);
    assert!(
        start2.elapsed() < std::time::Duration::from_secs(3),
        "wait_agent must return promptly on completion without waiting full timeout"
    );
}

#[test]
fn orchestra_core_integration_mock_agent() {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    let db = dir.path().join("store.db");

    let mut session = Session::create(&db, dir.path(), "orchestrator session")
        .unwrap()
        .with_bridge(mock_bridge());

    let sess_id = session.session_id();
    let cwd_str = dir.path().to_str().unwrap();

    // 1. Spawn two children: one reader (worktree: false) and one writer (worktree: true)
    let reader = agent_control::spawn_agent_with_options(
        session.store().conn(),
        Some(sess_id),
        "investigator",
        "Inspect codebase structure",
        Some("light"),
        false,
        cwd_str,
        None,
    )
    .unwrap();
    assert_eq!(reader.role, "investigator");
    assert!(reader.worktree.is_none());

    let writer = agent_control::spawn_agent_with_options(
        session.store().conn(),
        Some(sess_id),
        "builder",
        "Build a new module",
        Some("normal"),
        true,
        cwd_str,
        None,
    )
    .unwrap();
    assert_eq!(writer.role, "builder");
    assert!(
        writer.worktree.is_some(),
        "writer must receive an isolated worktree"
    );

    // 2. Wait for both children to complete with real timeout
    let wait_rep = wait_agent(session.store().conn(), Some(sess_id), None, 15).unwrap();
    assert!(
        wait_rep.all_done,
        "both children must complete; wait_report: {:?}",
        wait_rep
    );
    assert!(wait_rep.completed.contains(&reader.id));
    assert!(wait_rep.completed.contains(&writer.id));

    // 3. Collect their briefs
    let briefs =
        agent_control::collect_results(session.store().conn(), Some(sess_id), None, "full")
            .unwrap();
    assert_eq!(briefs.briefs.len(), 2);
    let r_brief = briefs.briefs.iter().find(|b| b.id == reader.id).unwrap();
    let w_brief = briefs.briefs.iter().find(|b| b.id == writer.id).unwrap();
    assert_eq!(r_brief.status, SubagentStatus::Completed);
    assert_eq!(w_brief.status, SubagentStatus::Completed);
    assert!(briefs.rendered_markdown.contains("investigator"));
    assert!(briefs.rendered_markdown.contains("builder"));

    // 4. Test Cap enforcement: concurrency limit hit pauses and asks
    let custom_caps = OrchestraCaps {
        max_concurrency: 1,
        max_children: 10,
        max_wall_time_secs: 1800,
    };

    // Spawn a child that stays running
    let running_child = agent_control::spawn_agent_with_options(
        session.store().conn(),
        Some(sess_id),
        "investigator",
        "Long task",
        None,
        false,
        cwd_str,
        Some(custom_caps),
    )
    .unwrap();
    assert_eq!(running_child.status, SubagentStatus::Running);

    // Spawning another child while running_child is running must hit concurrency cap and PAUSE (status: Waiting)
    let capped_child = agent_control::spawn_agent_with_options(
        session.store().conn(),
        Some(sess_id),
        "builder",
        "Excess task",
        None,
        true,
        cwd_str,
        Some(custom_caps),
    )
    .unwrap();
    assert_eq!(
        capped_child.status,
        SubagentStatus::Waiting,
        "hitting cap must pause into Waiting status"
    );
    let summary = capped_child.summary.unwrap_or_default();
    assert!(
        summary.contains("concurrency limit (1) hit"),
        "summary must state cap hit reason: {summary}"
    );

    // 5. Test Cancel and Reap: cancelling orchestrator reaps children process groups
    session.cancel_orchestration().unwrap();
    let cancelled_running =
        agent_control::get_subagent(session.store().conn(), running_child.id).unwrap();
    let cancelled_capped =
        agent_control::get_subagent(session.store().conn(), capped_child.id).unwrap();
    assert_eq!(cancelled_running.status, SubagentStatus::Cancelled);
    assert_eq!(cancelled_capped.status, SubagentStatus::Cancelled);

    // Check no orphan processes remain
    assert!(!chorus_acp::turn_running());
}
