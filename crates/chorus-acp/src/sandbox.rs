//! Keep an agent's writes inside the workspace, using the OS rather than a
//! promise.
//!
//! Measured on 12 Sep 2026: asked for ACP's `read-only` mode, Codex reported
//! `read-only` as current and then wrote files both inside the workspace and
//! outside it, without sending one `session/request_permission`. The ledger
//! guard catches the first case by reverting afterwards; nothing in Chorus
//! could touch the second. This closes it, before the write happens.
//!
//! ## What this does and does not restrict
//!
//! Only `file-write*`. Reads stay open, network stays open, spawning stays
//! open — an agent that cannot read the machine or reach its own API is not an
//! agent. Narrowing those is a different project with a much worse failure
//! mode (an agent that mysteriously half-works), and the hole actually
//! measured was writes.
//!
//! Writes are then allowed back for the places the agent has to have: its own
//! config and session state (`~/.codex`, `~/.claude`), package and build
//! caches, and temp. Leave one of those out and the agent breaks in a way that
//! looks like a Chorus bug — the first version of this profile did exactly
//! that.
//!
//! ## Honesty about the mechanism
//!
//! `sandbox-exec` is macOS's Seatbelt, marked deprecated by Apple for years
//! and still what everything uses, Chrome and Codex's own sandbox included.
//! If it is missing, [`wrap`] returns the command unchanged rather than
//! refusing to run: the caller says so out loud, and the revert guard is still
//! there. A sandbox that stops people from working gets turned off.

use std::path::{Path, PathBuf};

/// How much the agent is allowed to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Nothing in the workspace. Caches, config and temp only.
    ReadOnly,
    /// The workspace, plus the same caches, config and temp.
    Workspace,
}

/// Wrap `cmd` so its writes are confined, or return it unchanged when that
/// cannot be done here.
///
/// `Ok(cmd)` means confined; `Err(reason)` means it is not, and the reason is
/// meant to be shown to the user — silently running unconfined is how a safety
/// feature becomes a lie.
pub fn wrap(cmd: &[String], cwd: &Path, profile: Profile) -> Result<Vec<String>, String> {
    if std::env::var("CHORUS_SANDBOX").is_ok_and(|v| matches!(v.as_str(), "off" | "0" | "false")) {
        return Err("ปิดด้วย CHORUS_SANDBOX=off".into());
    }
    if !cfg!(target_os = "macos") {
        return Err("sandbox ใช้ได้เฉพาะ macOS".into());
    }
    if !Path::new("/usr/bin/sandbox-exec").is_file() {
        return Err("ไม่มี /usr/bin/sandbox-exec".into());
    }
    if cmd.is_empty() {
        return Err("คำสั่งว่าง".into());
    }
    let profile_text = policy(cwd, profile);
    let mut out = vec![
        "/usr/bin/sandbox-exec".to_string(),
        "-p".to_string(),
        profile_text,
    ];
    out.extend(cmd.iter().cloned());
    Ok(out)
}

/// The Seatbelt profile, as a single `-p` argument.
///
/// `-p` rather than a temp file on purpose: a profile file is one more thing
/// to clean up, and one more thing an attacker could swap between write and
/// exec.
pub fn policy(cwd: &Path, profile: Profile) -> String {
    let mut allow: Vec<PathBuf> = Vec::new();

    // Temp. Node, npx and every agent write here constantly.
    allow.push(PathBuf::from("/private/tmp"));
    allow.push(PathBuf::from("/private/var/folders"));
    allow.push(PathBuf::from("/private/var/tmp"));
    if let Some(t) = std::env::var_os("TMPDIR") {
        allow.push(PathBuf::from(t));
    }

    // The agents' own homes: auth, session state, rollout logs. Denying these
    // does not make anything safer — the agent is already authenticated as the
    // user — and it does break login.
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for rel in [
            ".codex", ".claude", ".config", ".cache", ".npm", ".local/share",
            ".local/state", "Library/Caches", "Library/Logs", ".bun/install/cache",
        ] {
            allow.push(home.join(rel));
        }
    }

    if profile == Profile::Workspace {
        allow.push(canonical(cwd));
    }

    let mut s = String::from(
        "(version 1)\n\
         (allow default)\n\
         (deny file-write*)\n",
    );
    // /dev is written by every process that opens a tty or /dev/null.
    s.push_str("(allow file-write* (subpath \"/dev\"))\n");
    for p in allow {
        let p = canonical(&p);
        let Some(p) = p.to_str() else { continue };
        if p.contains('"') {
            continue; // no quoting rule in the profile grammar is worth guessing at
        }
        s.push_str(&format!("(allow file-write* (subpath \"{p}\"))\n"));
    }
    // Carve-outs *inside* the workspace. Seatbelt picks the last matching
    // rule, so these must come after the allows above.
    //
    // `.chorus` holds the shadow repo that `revert_if_touched` trusts: an
    // agent that can rewrite it can make its own writes look like nothing
    // happened. `.git/hooks` and `.git/config` are the other kind of hole —
    // a file written there is executed later by the user's own `git`,
    // outside any sandbox, which turns "writes stay in the workspace" into
    // "writes run anywhere".
    if profile == Profile::Workspace {
        let ws = canonical(cwd);
        for rel in [".chorus", ".git/hooks", ".git/config"] {
            let Some(p) = ws.join(rel).to_str().map(str::to_owned) else { continue };
            if p.contains('"') {
                continue;
            }
            s.push_str(&format!("(deny file-write* (subpath \"{p}\"))\n"));
        }
    }
    s
}

/// macOS hands out `/tmp` and `/var` as symlinks into `/private`, and Seatbelt
/// matches the resolved path — a profile written with the symlink silently
/// allows nothing.
fn canonical(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_workspace_profile_allows_the_workspace_and_read_only_does_not() {
        let dir = std::env::temp_dir();
        let ws = policy(&dir, Profile::Workspace);
        let ro = policy(&dir, Profile::ReadOnly);
        let resolved = canonical(&dir);
        let resolved = resolved.to_str().unwrap();

        assert!(ws.contains(&format!("(subpath \"{resolved}\")")), "{ws}");
        assert!(ws.starts_with("(version 1)\n(allow default)\n(deny file-write*)"), "{ws}");
        // Both profiles share the temp allowance, so the read-only one is only
        // meaningfully different when cwd is outside temp — assert the shape,
        // not a substring that temp happens to satisfy.
        assert!(ro.lines().count() < ws.lines().count(), "read-only allowed as much as workspace");
    }

    #[test]
    fn paths_are_resolved_through_the_private_symlink() {
        let p = policy(Path::new("/tmp"), Profile::Workspace);
        assert!(p.contains("/private/tmp"), "Seatbelt matches resolved paths: {p}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn wrapping_puts_sandbox_exec_in_front_and_keeps_the_command() {
        let cmd = vec!["npx".to_string(), "-y".to_string(), "thing".to_string()];
        let out = wrap(&cmd, Path::new("/tmp"), Profile::Workspace).unwrap();
        assert_eq!(out[0], "/usr/bin/sandbox-exec");
        assert_eq!(out[1], "-p");
        assert_eq!(&out[3..], &cmd[..], "the original command must survive intact");
    }

    /// Runs real `sandbox-exec`, not a string match on the profile: the only
    /// way to know the profile says what we think it says.
    #[cfg(target_os = "macos")]
    mod enforced {
        use super::*;
        use std::process::Command;

        /// `~/` on purpose: `/private/tmp` is on the allow list, so a scratch
        /// dir there would pass whether or not the sandbox works.
        fn scratch(name: &str) -> PathBuf {
            let home = PathBuf::from(std::env::var_os("HOME").unwrap());
            let d = home.join(format!(".chorus-sbx-test-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&d).unwrap();
            d
        }

        fn sh(ws: &Path, profile: Profile, script: &str) -> bool {
            let cmd: Vec<String> = ["/bin/sh", "-c", script].map(String::from).to_vec();
            let argv = wrap(&cmd, ws, profile).expect("sandbox-exec available");
            Command::new(&argv[0])
                .args(&argv[1..])
                .current_dir(ws)
                .output()
                .unwrap()
                .status
                .success()
        }

        #[test]
        fn writes_inside_pass_outside_and_protected_paths_fail() {
            let ws = scratch("ws");
            let out = scratch("out");
            std::fs::create_dir_all(ws.join(".chorus")).unwrap();
            std::fs::create_dir_all(ws.join(".git/hooks")).unwrap();

            let p = Profile::Workspace;
            assert!(sh(&ws, p, "echo x > ok.txt"), "workspace write must work");
            assert!(!sh(&ws, p, &format!("echo x > {}/no.txt", out.display())), "outside write leaked");
            assert!(!sh(&ws, p, "echo x > .chorus/tamper"), ".chorus writable");
            assert!(!sh(&ws, p, "echo x > .git/hooks/pre-commit"), "git hook writable");
            assert!(!sh(&ws, p, "echo x > .git/config"), "git config writable");
            assert!(sh(&ws, p, "echo x > .git/index-ish"), "rest of .git must stay writable");
            assert!(!sh(&ws, Profile::ReadOnly, "echo x > ro.txt"), "read-only profile wrote");

            assert!(ws.join("ok.txt").exists());
            assert!(!out.join("no.txt").exists());
            let _ = std::fs::remove_dir_all(&ws);
            let _ = std::fs::remove_dir_all(&out);
        }
    }
}
