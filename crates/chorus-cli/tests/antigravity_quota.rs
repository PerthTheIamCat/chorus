//! P3-1: the Antigravity quota poll used to scrape `ps -A` for another
//! product's `--csrf_token` and replay it with `curl -k`. It must run no
//! process at all and report the quota as unknown.
//!
//! Own test binary: it replaces PATH and HOME for the whole process.

use std::os::unix::fs::PermissionsExt;

#[test]
fn antigravity_quota_never_runs_ps_or_curl() {
    let base = std::env::temp_dir().join(format!("chorus-agq-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let bin = base.join("bin");
    let home = base.join("home");
    let acct = base.join("acct");
    for d in [&bin, &home, &acct] {
        std::fs::create_dir_all(d).unwrap();
    }
    let marker = base.join("ran");
    for tool in ["ps", "lsof", "curl"] {
        let p = bin.join(tool);
        std::fs::write(&p, format!("#!/bin/sh\necho {tool} >> {}\n", marker.display())).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::set_var("PATH", &bin);
    std::env::set_var("HOME", &home);

    let rl = chorus_cli::limits::antigravity_rate_limits(&acct);

    assert!(!marker.exists(), "ran: {}", std::fs::read_to_string(&marker).unwrap_or_default());
    assert!(rl.is_none(), "quota must be unknown without a statusline file");
    let _ = std::fs::remove_dir_all(&base);
}
