//! Integration tests for Orchestra mode core (Issue 12).
//!
//! Done when:
//! - Integration test with the mock agent: spawn two children, one writer and one reader,
//!   collect their briefs, hit the cap and pause, cancel, and check no orphan processes remain.
//! - The `complete_agent` and `wait_agent` timeouts work for real.

use chorus_cli::session::{Policy, Session};
use chorus_core::agent_control::{self, complete_agent, wait_agent, OrchestraCaps, SubagentStatus};
use chorus_core::router::Candidate;
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

/// A mock that holds every prompt open for `ms` milliseconds.
fn slow_bridge(ms: u64) -> Vec<String> {
    let mut b = mock_bridge();
    b.extend(["--sleep-ms".into(), ms.to_string()]);
    b
}

/// A mock that writes `name` into its cwd mid-turn without asking.
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

/// Two ready agents a light and a heavy task land on differently: codex has
/// only a small model, claude only a large one.
fn two_agents() -> Vec<Candidate> {
    vec![
        cand("codex", "gpt-5.6-luna[low]", 0),
        cand("claude", "opus", 2),
    ]
}

fn wait_until(secs: u64, mut f: impl FnMut() -> bool) -> bool {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    f()
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
    session.set_child_candidates(two_agents());

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
        Some("heavy"),
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
    let wt = writer.worktree.clone().unwrap();
    let _ = Command::new("git")
        .args(["worktree", "remove", "--force", &wt])
        .current_dir(dir.path())
        .output();

    // The router, not a hardcoded codex, picked each child's agent: a light
    // investigation fits the small model, a normal build the large one.
    let conn = session.store().conn();
    let r = agent_control::get_subagent(conn, reader.id).unwrap();
    let w = agent_control::get_subagent(conn, writer.id).unwrap();
    assert_eq!(r.model.as_deref(), Some("codex"), "{r:?}");
    assert_eq!(w.model.as_deref(), Some("claude"), "{w:?}");

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
    assert!(wait_until(10, || session
        .child_pids(running_child.id)
        .is_empty()));
    assert!(wait_until(10, || !chorus_acp::turn_running()));
}

#[test]
fn heavy_weight_and_role_pin_route_children() {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    let roles = dir.path().join(".chorus/agents");
    fs::create_dir_all(&roles).unwrap();
    fs::write(
        roles.join("pinned-doer.md"),
        "---\nname: pinned-doer\ndescription: \"always codex\"\nmodel: gpt-5.6-luna[low]\n---\nDo it.\n",
    )
    .unwrap();
    let db = dir.path().join("store.db");
    let mut session = Session::create(&db, dir.path(), "orch")
        .unwrap()
        .with_bridge(mock_bridge());
    session.set_child_candidates(two_agents());
    let cwd = dir.path().to_str().unwrap();
    let conn = session.store().conn();
    let sid = session.session_id();

    let light = agent_control::spawn_agent_with_options(
        conn,
        Some(sid),
        "builder",
        "small fix",
        Some("light"),
        false,
        cwd,
        None,
    )
    .unwrap();
    let pinned = agent_control::spawn_agent_with_options(
        conn,
        Some(sid),
        "pinned-doer",
        "big job",
        Some("heavy"),
        false,
        cwd,
        None,
    )
    .unwrap();
    let rep = wait_agent(conn, Some(sid), None, 20).unwrap();
    assert!(
        rep.all_done,
        "{rep:?} {:?}",
        agent_control::list_subagents(conn, Some(sid))
    );
    let light = agent_control::get_subagent(conn, light.id).unwrap();
    let pinned = agent_control::get_subagent(conn, pinned.id).unwrap();
    assert_eq!(light.model.as_deref(), Some("codex"), "{light:?}");
    assert_eq!(
        pinned.model.as_deref(),
        Some("codex"),
        "role model: pins the agent that offers it, whatever the weight: {pinned:?}"
    );

    // A pin nobody offers fails the spawn instead of running elsewhere.
    fs::write(
        roles.join("ghost.md"),
        "---\nname: ghost\nmodel: gemini-9\n---\nx\n",
    )
    .unwrap();
    let ghost = agent_control::spawn_agent_with_options(
        conn,
        Some(sid),
        "ghost",
        "x",
        None,
        false,
        cwd,
        None,
    )
    .unwrap();
    assert_eq!(ghost.status, SubagentStatus::Failed, "{ghost:?}");
    assert!(ghost.summary.unwrap_or_default().contains("gemini-9"));
}

#[test]
fn cancelling_one_child_leaves_its_sibling_running() {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    let db = dir.path().join("store.db");
    let mut session = Session::create(&db, dir.path(), "orch")
        .unwrap()
        .with_bridge(slow_bridge(2500));
    session.set_child_candidates(two_agents());
    let cwd = dir.path().to_str().unwrap();
    let conn = session.store().conn();
    let sid = session.session_id();
    let a = agent_control::spawn_agent(conn, Some(sid), "investigator", "a", false, cwd).unwrap();
    let b = agent_control::spawn_agent(conn, Some(sid), "investigator", "b", false, cwd).unwrap();
    assert!(wait_until(10, || !session.child_pids(a.id).is_empty()
        && !session.child_pids(b.id).is_empty()));

    agent_control::cancel_agent(conn, a.id, None).unwrap();
    assert!(
        wait_until(5, || session.child_pids(a.id).is_empty()),
        "a not reaped"
    );
    assert!(
        !session.child_pids(b.id).is_empty(),
        "stopping a must not stop b"
    );

    let rep = wait_agent(conn, Some(sid), Some(b.id), 20).unwrap();
    assert_eq!(rep.completed, vec![b.id], "{rep:?}");
    let a = agent_control::get_subagent(conn, a.id).unwrap();
    assert_eq!(a.status, SubagentStatus::Cancelled);
}

#[test]
fn running_child_pauses_at_the_wall_clock_and_resumes() {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    let db = dir.path().join("store.db");
    let mut session = Session::create(&db, dir.path(), "orch")
        .unwrap()
        .with_bridge(slow_bridge(4000));
    session.set_child_candidates(two_agents());
    session.set_orchestra_caps(OrchestraCaps {
        max_concurrency: 3,
        max_children: 10,
        max_wall_time_secs: 1,
    });
    let cwd = dir.path().to_str().unwrap();
    let conn = session.store().conn();
    let sid = session.session_id();
    let a =
        agent_control::spawn_agent(conn, Some(sid), "investigator", "slow", false, cwd).unwrap();
    assert_eq!(a.status, SubagentStatus::Running);

    assert!(
        wait_until(5, || {
            agent_control::get_subagent(conn, a.id).unwrap().status == SubagentStatus::Waiting
        }),
        "a running child past the wall clock must pause"
    );
    let paused = agent_control::get_subagent(conn, a.id).unwrap();
    assert!(paused
        .summary
        .unwrap_or_default()
        .starts_with(agent_control::WALL_TIME_PAUSE));
    assert!(
        wait_until(5, || session.child_pids(a.id).is_empty()),
        "paused child still running"
    );

    // A spawn while the clock is spent waits too.
    let late =
        agent_control::spawn_agent(conn, Some(sid), "investigator", "late", false, cwd).unwrap();
    assert_eq!(late.status, SubagentStatus::Waiting);

    // Continue restarts the clock: the child runs again instead of pausing at once.
    let resumed = agent_control::resume_subagent(conn, a.id).unwrap();
    assert_eq!(resumed.status, SubagentStatus::Running);
    assert!(wait_until(5, || !session.child_pids(a.id).is_empty()));
    session.cancel_orchestration().unwrap();
    assert!(wait_until(10, || session.child_pids(a.id).is_empty()));
}

#[test]
fn orchestrator_never_writes_but_its_writer_children_may() {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    let db = dir.path().join("store.db");
    let mut session = Session::create(&db, dir.path(), "orch")
        .unwrap()
        .with_bridge(rogue_bridge("lead-wrote-this"));
    session.set_child_candidates(two_agents());

    // The user allows everything; the orchestrator still runs read-only.
    let turn = session.run_orchestrator_turn("plan it", &[], "", "", Policy::AllowAll);
    assert!(
        !dir.path().join("lead-wrote-this").exists(),
        "the orchestrator wrote into the workspace: {:?}",
        turn.as_ref()
            .map(|r| r.relay.answered_by.clone())
            .map_err(|e| e.to_string())
    );
    if let Ok(r) = &turn {
        assert_eq!(r.lead[0].agent, "claude");
        assert_eq!(r.lead[0].model, "opus");
    }

    // Its children are capped by the user's permission, not by its own: a
    // writer in a worktree can write, and its diff counts the new file.
    let conn = session.store().conn();
    let w = agent_control::spawn_agent_with_options(
        conn,
        Some(session.session_id()),
        "builder",
        "write",
        Some("light"),
        true,
        dir.path().to_str().unwrap(),
        None,
    )
    .unwrap();
    assert!(
        wait_agent(conn, Some(session.session_id()), Some(w.id), 20)
            .unwrap()
            .all_done
    );
    let w = agent_control::get_subagent(conn, w.id).unwrap();
    assert_eq!(w.status, SubagentStatus::Completed, "{w:?}");
    assert!(
        w.files_touched.contains(&"lead-wrote-this".to_string()),
        "{w:?}"
    );
    assert!(w
        .diff_stat
        .unwrap_or_default()
        .starts_with("1 file changed"));
    let wt = PathBuf::from(w.worktree.unwrap());
    let _ = Command::new("git")
        .args(["worktree", "remove", "--force", wt.to_str().unwrap()])
        .current_dir(dir.path())
        .output();
}
