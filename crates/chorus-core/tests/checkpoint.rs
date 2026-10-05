//! Checkpoint tests. These shell out to `git`; skipped cleanly if it's absent.

use chorus_core::checkpoint::Checkpoint;
use std::process::Command;

fn have_git() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn workspace() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "chorus-ckpt-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn snapshot_then_edit_then_diff() {
    if !have_git() {
        return;
    }
    let ws = workspace();
    std::fs::write(ws.join("a.txt"), "one\n").unwrap();

    let ckpt = Checkpoint::open(&ws).unwrap();
    let base = ckpt.snapshot("initial").unwrap();
    assert_eq!(base.len(), 40, "sha should be a full hash");

    std::fs::write(ws.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(ws.join("b.txt"), "new file\n").unwrap();

    let diff = ckpt.diff_since(&base).unwrap();
    assert!(diff.contains("+two"), "edit not in diff:\n{diff}");
    assert!(diff.contains("b.txt"), "new file not in diff:\n{diff}");

    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn the_shadow_repo_ignores_dot_chorus_and_target() {
    if !have_git() {
        return;
    }
    let ws = workspace();
    std::fs::write(ws.join("keep.txt"), "keep\n").unwrap();
    std::fs::create_dir_all(ws.join("target")).unwrap();
    std::fs::write(ws.join("target/junk.o"), "binary\n").unwrap();

    let ckpt = Checkpoint::open(&ws).unwrap();
    let base = ckpt.snapshot("first").unwrap();
    let diff = ckpt.diff_since("").unwrap();

    assert!(diff.contains("keep.txt"));
    assert!(
        !diff.contains("target/junk.o"),
        "target/ leaked into the shadow repo:\n{diff}"
    );
    // .chorus/ (where the shadow repo itself lives) must never be tracked.
    assert!(
        !diff.contains(".chorus"),
        ".chorus/ leaked into the shadow repo:\n{diff}"
    );
    let _ = base;

    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn restore_rolls_the_tree_back() {
    if !have_git() {
        return;
    }
    let ws = workspace();
    std::fs::write(ws.join("f.txt"), "v1\n").unwrap();
    let ckpt = Checkpoint::open(&ws).unwrap();
    let v1 = ckpt.snapshot("v1").unwrap();

    std::fs::write(ws.join("f.txt"), "v2\n").unwrap();
    std::fs::write(ws.join("scratch.txt"), "unwanted\n").unwrap();
    ckpt.snapshot("v2").unwrap();

    ckpt.restore(&v1).unwrap();
    assert_eq!(std::fs::read_to_string(ws.join("f.txt")).unwrap(), "v1\n");
    assert!(
        !ws.join("scratch.txt").exists(),
        "clean should have removed the untracked file"
    );

    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn apply_reverse_undoes_the_hunks_in_a_patch() {
    if !have_git() {
        return;
    }
    let ws = workspace();
    std::fs::write(ws.join("a.txt"), "keep1\nkeep2\nkeep3\n").unwrap();
    std::fs::write(ws.join("b.txt"), "one\ntwo\n").unwrap();
    let ckpt = Checkpoint::open(&ws).unwrap();
    let base = ckpt.snapshot("v1").unwrap();

    // Change both files.
    std::fs::write(ws.join("a.txt"), "KEEP1\nkeep2\nkeep3\n").unwrap();
    std::fs::write(ws.join("b.txt"), "ONE\ntwo\n").unwrap();
    let full = ckpt.diff_since(&base).unwrap();

    // A patch with only b.txt's change.
    let b_start = full.find("diff --git a/b.txt").unwrap();
    let b_patch = &full[b_start..];

    ckpt.apply_reverse(b_patch).unwrap();
    assert_eq!(
        std::fs::read_to_string(ws.join("b.txt")).unwrap(),
        "one\ntwo\n",
        "b not reverted"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("a.txt")).unwrap(),
        "KEEP1\nkeep2\nkeep3\n",
        "a wrongly reverted"
    );

    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn user_diff_leaves_out_files_chorus_generates() {
    let dir = std::env::temp_dir().join(format!("chorus-ckpt-gen-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ck = chorus_core::checkpoint::Checkpoint::open(&dir).unwrap();
    let base = ck.snapshot("base").unwrap();
    std::fs::write(dir.join("AGENTS.md"), "generated").unwrap();
    std::fs::write(dir.join("CLAUDE.md"), "generated").unwrap();
    std::fs::write(dir.join("real.txt"), "work").unwrap();
    let d = ck.diff_since_user(&base).unwrap();
    assert!(d.contains("real.txt"));
    assert!(!d.contains("AGENTS.md") && !d.contains("CLAUDE.md"), "{d}");
    // the revert guard still sees everything
    assert!(ck.diff_since(&base).unwrap().contains("AGENTS.md"));
}
