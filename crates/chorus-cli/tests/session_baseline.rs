use chorus_cli::session::Session;
use std::fs;
use tempfile::tempdir;

/// Files that already existed (untracked) when a session opened are not that
/// session's work, even if an earlier session left an older snapshot behind.
#[test]
fn a_new_session_does_not_inherit_files_from_before_it_opened() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let db = root.join("ledger.db");

    fs::write(root.join("seed.txt"), "seed\n").unwrap();

    // The earlier session snapshots at the start of a turn (the turn guard),
    // and its agent writes a file during that turn — after the snapshot.
    let first = Session::create(&db, root, "first").unwrap();
    first.snapshot("turn guard").unwrap();
    fs::write(root.join("pre_existing.txt"), "from before\n").unwrap();
    drop(first);

    let second = Session::create(&db, root, "second").unwrap();
    fs::write(root.join("mine.txt"), "this session\n").unwrap();

    let diff = second.working_diff();
    assert!(diff.contains("mine.txt"), "session work must show: {diff}");
    assert!(!diff.contains("pre_existing.txt"), "older file leaked into the session: {diff}");
}
