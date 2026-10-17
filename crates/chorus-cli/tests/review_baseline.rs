use chorus_cli::session::Session;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn init_git_repo(path: &std::path::Path) {
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(path)
        .status()
        .expect("git init");
    Command::new("git")
        .args(["config", "user.email", "tester@chorus.local"])
        .current_dir(path)
        .status()
        .expect("git config email");
    Command::new("git")
        .args(["config", "user.name", "Tester"])
        .current_dir(path)
        .status()
        .expect("git config name");
    fs::write(path.join("README.md"), "# Init\n").unwrap();
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(path)
        .status()
        .expect("git add");
    Command::new("git")
        .args(["commit", "-m", "initial commit", "-q"])
        .current_dir(path)
        .status()
        .expect("git commit");
}

#[test]
fn test_agent_commit_during_turn_keeps_baseline() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    init_git_repo(root);

    let db_path = root.join("ledger.db");
    let s = Session::create(&db_path, root, "test agent commit").unwrap();
    let initial_base = s.review_base();

    // Simulate an agent making a commit during a turn:
    fs::write(root.join("agent_work.rs"), "pub fn work() {}\n").unwrap();
    Command::new("git")
        .args(["add", "agent_work.rs"])
        .current_dir(root)
        .status()
        .expect("git add");
    Command::new("git")
        .args(["commit", "-m", "agent commit", "-q"])
        .current_dir(root)
        .status()
        .expect("git commit");

    // Before any subsequent user action, check_user_commit is NOT called.
    // Baseline stays at initial_base:
    assert_eq!(
        s.review_base(),
        initial_base,
        "agent commit during turn must NOT move baseline"
    );

    // The diff from baseline still contains agent_work.rs
    let diff = s.working_diff();
    assert!(
        diff.contains("agent_work.rs"),
        "agent commit must stay in review diff: {diff}"
    );
}

#[test]
fn test_mark_reviewed_moves_baseline() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    init_git_repo(root);

    let db_path = root.join("ledger.db");
    let mut s = Session::create(&db_path, root, "test mark reviewed").unwrap();
    let base0 = s.review_base();
    assert!(!base0.is_empty());

    // Make an edit
    fs::write(root.join("hello.txt"), "hello world\n").unwrap();
    let diff = s.working_diff();
    assert!(
        diff.contains("hello.txt"),
        "diff should contain hello.txt: {diff}"
    );

    // Mark reviewed
    let new_base = s.mark_reviewed().unwrap();
    assert_ne!(new_base, base0);
    assert_eq!(s.review_base(), new_base);

    // Diff after mark reviewed should now be empty
    let diff_after = s.working_diff();
    assert!(
        diff_after.trim().is_empty(),
        "diff after mark reviewed should be empty: {diff_after}"
    );
}

#[test]
fn test_baseline_survives_reopen() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    init_git_repo(root);

    let db_path = root.join("ledger.db");
    let sess_id;
    let base_reviewed;
    {
        let mut s = Session::create(&db_path, root, "test reopen").unwrap();
        sess_id = s.session_id();
        fs::write(root.join("file1.txt"), "version 1\n").unwrap();
        base_reviewed = s.mark_reviewed().unwrap();
    }

    // Reopen session
    let s2 = Session::open(&db_path, sess_id).unwrap();
    assert_eq!(
        s2.review_base(),
        base_reviewed,
        "review baseline must survive reopen"
    );
}

#[test]
fn test_user_commit_between_turns_moves_baseline() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    init_git_repo(root);

    let db_path = root.join("ledger.db");
    let mut s = Session::create(&db_path, root, "test user commit").unwrap();
    let initial_base = s.review_base();

    // User edits and commits in git terminal between turns
    fs::write(root.join("feature.rs"), "pub fn feature() {}\n").unwrap();
    Command::new("git")
        .args(["add", "feature.rs"])
        .current_dir(root)
        .status()
        .expect("git add");
    Command::new("git")
        .args(["commit", "-m", "user adds feature", "-q"])
        .current_dir(root)
        .status()
        .expect("git commit");

    // Check user commit
    let moved = s.check_user_commit().unwrap();
    assert!(moved, "check_user_commit must detect user commit");
    assert_ne!(
        s.review_base(),
        initial_base,
        "baseline must move forward on user commit"
    );

    let diff = s.working_diff();
    assert!(
        diff.trim().is_empty(),
        "diff should be empty after user committed feature: {diff}"
    );
}

#[test]
fn test_diff_stats_and_scoped_options() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    init_git_repo(root);

    let db_path = root.join("ledger.db");
    let s = Session::create(&db_path, root, "test stats").unwrap();

    fs::write(root.join("Cargo.lock"), "lock content\n").unwrap();
    fs::write(root.join("code.rs"), "line 1\nline 2\n").unwrap();
    fs::write(root.join("CLAUDE.md"), "projected\n").unwrap();

    // Stats without projected files
    let stats = s.diff_stats("session", false).unwrap();
    assert_eq!(
        stats.files.len(),
        2,
        "should have 2 files excluding CLAUDE.md: {:?}",
        stats.files
    );
    let lock_file = stats
        .files
        .iter()
        .find(|f| f.path == "Cargo.lock")
        .expect("Cargo.lock");
    assert!(lock_file.is_lockfile);

    let code_file = stats
        .files
        .iter()
        .find(|f| f.path == "code.rs")
        .expect("code.rs");
    assert!(!code_file.is_lockfile);
    assert_eq!(code_file.additions, 2);

    // Stats with projected files
    let stats_all = s.diff_stats("session", true).unwrap();
    assert_eq!(
        stats_all.files.len(),
        3,
        "should include CLAUDE.md when include_projected=true"
    );
}
