//! P2-5: `bridge_for` must never pick a bridge binary out of the process cwd.
//!
//! Its own test binary: it changes the process cwd and PATH, which would race
//! with any other test in the same process.

use chorus_acp::codex::bridge_for;

#[test]
fn antigravity_bridge_is_not_taken_from_a_cwd_relative_target_dir() {
    let base = std::env::temp_dir().join(format!("chorus-bridge-cwd-{}", std::process::id()));
    let repo = base.join("a").join("b").join("repo");
    for d in [repo.join("target/debug"), repo.join("target/release"), base.join("a/target/debug")] {
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("antigravity-acp"), "#!/bin/sh\n").unwrap();
    }
    std::fs::write(repo.join("antigravity-acp"), "#!/bin/sh\n").unwrap();
    std::env::remove_var("CHORUS_ANTIGRAVITY_BRIDGE");
    // A relative PATH entry must not resolve into the cwd either.
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut path = std::ffi::OsString::from(".:target/debug:");
    path.push(&old_path);
    std::env::set_var("PATH", &path);
    let old_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(&repo).unwrap();

    let cmd = bridge_for("antigravity");

    std::env::set_current_dir(old_cwd).unwrap();
    std::env::set_var("PATH", old_path);
    let base_real = base.canonicalize().unwrap();
    let _ = std::fs::remove_dir_all(&base);
    let first = std::path::Path::new(&cmd[0]);
    assert!(
        !first.is_relative() || cmd[0] == "antigravity-acp",
        "relative bridge path: {cmd:?}"
    );
    assert!(
        !first.starts_with(&base_real) && !first.starts_with(&base),
        "bridge taken from the cwd repo: {cmd:?}"
    );
}
