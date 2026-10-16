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
//! If it is missing, [`wrap`] returns an error. Restrictive callers must refuse
//! to run rather than silently downgrade to an unconfined agent.

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
    let profile_text = checked_policy(cwd, profile)?;
    let mut out = vec![
        "/usr/bin/sandbox-exec".to_string(),
        "-p".to_string(),
        profile_text,
    ];
    out.extend(cmd.iter().cloned());
    Ok(out)
}

fn regex_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if "\\.^$|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The Seatbelt profile, as a single `-p` argument.
///
/// `-p` rather than a temp file on purpose: a profile file is one more thing
/// to clean up, and one more thing an attacker could swap between write and
/// exec.
pub fn policy(cwd: &Path, profile: Profile) -> String {
    // Keep the public string API fail-closed; spawning uses the checked builder
    // so it can report an invalid metadata path before starting the agent.
    checked_policy(cwd, profile).unwrap_or_else(|_| "(version 1)\n(deny default)\n".into())
}

fn checked_policy(cwd: &Path, profile: Profile) -> Result<String, String> {
    let ws = cwd
        .canonicalize()
        .map_err(|e| format!("resolve sandbox workspace: {e}"))?;
    checked_path(&ws)?;
    let mut guards = git_guards(&ws, profile)?;
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
            ".codex",
            ".claude",
            ".config",
            ".cache",
            ".npm",
            ".local/share",
            ".local/state",
            "Library/Caches",
            "Library/Logs",
            ".bun/install/cache",
            // Extra accounts: each one is a CODEX_HOME / CLAUDE_CONFIG_DIR here.
            ".chorus/accounts",
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
    // Claude Code keeps its main config as a *file* beside its home dir
    // (`~/.claude.json`, plus `.backup`/`.lock` siblings). It is not under
    // `~/.claude`, so the subpath above misses it, and a denied write there
    // made a failed login take minutes instead of seconds.
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let base = canonical(&home).join(".claude.json");
        if let Some(b) = base.to_str().filter(|b| !b.contains('"')) {
            s.push_str(&format!("(allow file-write* (literal \"{b}\"))\n"));
            s.push_str(&format!(
                "(allow file-write* (regex #\"^{}\\.[^/]*$\"))\n",
                regex_escape(b)
            ));
        }
    }
    // `git push` over SSH has to record a new host key, and ssh multiplexing
    // keeps its sockets beside it. Denied, `ssh` fails with "Failed to add the
    // host to the list of known hosts" and the agent cannot use the user's Git
    // account. Only those two — `config`, `authorized_keys` and the keys stay
    // read-only, because `ProxyCommand`/`LocalCommand` in `config` runs later
    // outside any sandbox.
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let ssh = canonical(&home).join(".ssh");
        if let Some(kh) = ssh.join("known_hosts").to_str().filter(|b| !b.contains('"')) {
            s.push_str(&format!("(allow file-write* (literal \"{kh}\"))\n"));
            s.push_str(&format!(
                "(allow file-write* (regex #\"^{}\\.[^/]*$\"))\n",
                regex_escape(kh)
            ));
        }
        if let Some(c) = ssh.join("control").to_str().filter(|b| !b.contains('"')) {
            s.push_str(&format!("(allow file-write* (subpath \"{c}\"))\n"));
        }
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
    if profile == Profile::ReadOnly {
        // The workspace can itself be under an allowed temp/cache directory.
        // An explicit deny keeps read-only meaningful in that case too.
        let ws = canonical(cwd);
        if let Some(p) = ws.to_str().filter(|p| !p.contains('"')) {
            s.push_str(&format!("(deny file-write* (subpath \"{p}\"))\n"));
        }
    }
    // Protect routing entries themselves as well as their resolved targets.
    // A linked worktree's shared hooks/config may live in an allowed temp tree.
    let shadow = ws.join(".chorus");
    guards.push((protected_target(&shadow)?, false));
    guards.push((shadow, false));
    // A protected file's absolute path changes if an ancestor is renamed.
    // Literal directory denies seal that namespace without blocking writes
    // to ordinary children (index, refs and workspace files).
    let ancestors: Vec<_> = guards
        .iter()
        .flat_map(|(path, _)| path.parent().into_iter().flat_map(Path::ancestors))
        .map(|path| (path.to_path_buf(), true))
        .collect();
    guards.extend(ancestors);
    guards.sort();
    guards.dedup();
    for (path, literal) in guards {
        let p = checked_path(&path)?;
        let filter = if literal { "literal" } else { "subpath" };
        s.push_str(&format!("(deny file-write* ({filter} \"{p}\"))\n"));
    }
    Ok(s)
}

// Never silently omit a protection rule for a path we cannot represent safely.
fn checked_path(path: &Path) -> Result<&str, String> {
    path.to_str()
        .filter(|p| !p.contains(['"', '\\', '\n', '\r']))
        .ok_or_else(|| "sandbox path cannot be safely represented".into())
}

fn protected_target(path: &Path) -> Result<PathBuf, String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => path
            .canonicalize()
            .map_err(|e| format!("resolve protected metadata symlink: {e}")),
        Ok(_) => Ok(canonical(path)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(e) => Err(format!("inspect protected metadata: {e}")),
    }
}

fn metadata_pointer(path: &Path, prefix: &str) -> Result<PathBuf, String> {
    use std::io::Read;
    if !std::fs::metadata(path)
        .map_err(|e| format!("inspect Git metadata pointer: {e}"))?
        .is_file()
    {
        return Err("Git metadata pointer is not a regular file".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(4097).read_to_end(&mut bytes))
        .map_err(|e| format!("read Git metadata pointer: {e}"))?;
    if bytes.len() > 4096 {
        return Err("Git metadata pointer exceeds 4096 bytes".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "invalid Git metadata pointer encoding")?;
    let value = text
        .strip_prefix(prefix)
        .ok_or("invalid Git metadata pointer")?
        .trim_end_matches(['\r', '\n']);
    if value.is_empty() || value.contains(['\r', '\n']) {
        return Err("invalid Git metadata pointer path".into());
    }
    let target = path
        .parent()
        .ok_or("Git metadata pointer has no parent")?
        .join(value);
    target
        .canonicalize()
        .map_err(|e| format!("resolve Git metadata pointer: {e}"))
}

fn git_guards(ws: &Path, profile: Profile) -> Result<Vec<(PathBuf, bool)>, String> {
    let mut guards = Vec::new();
    // Also seal a future .git entry in a non-repository workspace.
    let mut entry = ws.join(".git");
    for ancestor in ws.ancestors() {
        let candidate = ancestor.join(".git");
        match std::fs::symlink_metadata(&candidate) {
            Ok(_) => {
                entry = candidate;
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("inspect Git metadata: {e}")),
        }
    }
    guards.push((entry.clone(), true));
    // The conventional paths remain protected even before metadata exists.
    for rel in ["hooks", "config", "config.worktree", "commondir"] {
        guards.push((entry.join(rel), false));
    }
    if let Some(root) = entry.parent() {
        let shadow = root.join(".chorus");
        guards.push((protected_target(&shadow)?, false));
        guards.push((shadow, false));
    }
    match std::fs::symlink_metadata(&entry) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(guards),
        Err(e) => return Err(format!("inspect Git metadata: {e}")),
        Ok(_) => {}
    }
    let git_dir = if entry.is_dir() {
        entry
            .canonicalize()
            .map_err(|e| format!("resolve Git directory: {e}"))?
    } else {
        metadata_pointer(&entry, "gitdir: ")?
    };
    if !git_dir.is_dir() {
        return Err("Git metadata target is not a directory".into());
    }
    let common_pointer = git_dir.join("commondir");
    let common_dir = match std::fs::symlink_metadata(&common_pointer) {
        Ok(_) => metadata_pointer(&common_pointer, "")?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => git_dir.clone(),
        Err(e) => return Err(format!("inspect Git common directory: {e}")),
    };
    if !common_dir.is_dir() {
        return Err("Git common metadata target is not a directory".into());
    }
    for dir in [git_dir, common_dir] {
        // Deny replacement of the directory without denying normal index/ref
        // writes in Workspace mode. ReadOnly also protects external metadata.
        guards.push((dir.clone(), profile == Profile::Workspace));
        for rel in ["hooks", "config", "config.worktree", "commondir"] {
            let path = dir.join(rel);
            guards.push((protected_target(&path)?, false));
            guards.push((path, false));
        }
    }
    Ok(guards)
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
    fn ssh_allows_known_hosts_but_not_the_whole_dir() {
        let ws = std::env::temp_dir();
        let p = policy(&ws, Profile::Workspace);
        let ssh = std::env::var_os("HOME")
            .map(|h| canonical(&PathBuf::from(h)).join(".ssh"))
            .unwrap();
        let ssh = ssh.to_str().unwrap();
        assert!(p.contains(&format!("(allow file-write* (literal \"{ssh}/known_hosts\"))")));
        assert!(!p.contains(&format!("(allow file-write* (subpath \"{ssh}\"))")));
        assert!(!p.contains("authorized_keys"));
    }

    #[test]
    fn the_workspace_profile_allows_the_workspace_and_read_only_does_not() {
        let dir = std::env::temp_dir();
        let ws = policy(&dir, Profile::Workspace);
        let ro = policy(&dir, Profile::ReadOnly);
        let resolved = canonical(&dir);
        let resolved = resolved.to_str().unwrap();

        assert!(ws.contains(&format!("(subpath \"{resolved}\")")), "{ws}");
        assert!(
            ws.starts_with("(version 1)\n(allow default)\n(deny file-write*)"),
            "{ws}"
        );
        // Temp is allowed in both; ReadOnly needs an explicit workspace deny.
        assert!(
            ro.contains(&format!("(deny file-write* (subpath \"{resolved}\"))")),
            "{ro}"
        );
    }

    #[test]
    fn paths_are_resolved_through_the_private_symlink() {
        let p = policy(Path::new("/tmp"), Profile::Workspace);
        assert!(
            p.contains("/private/tmp"),
            "Seatbelt matches resolved paths: {p}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn wrapping_puts_sandbox_exec_in_front_and_keeps_the_command() {
        let cmd = vec!["npx".to_string(), "-y".to_string(), "thing".to_string()];
        let out = wrap(&cmd, Path::new("/tmp"), Profile::Workspace).unwrap();
        assert_eq!(out[0], "/usr/bin/sandbox-exec");
        assert_eq!(out[1], "-p");
        assert_eq!(
            &out[3..],
            &cmd[..],
            "the original command must survive intact"
        );
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
            assert!(
                !sh(&ws, p, &format!("echo x > {}/no.txt", out.display())),
                "outside write leaked"
            );
            assert!(!sh(&ws, p, "echo x > .chorus/tamper"), ".chorus writable");
            assert!(
                !sh(&ws, p, "echo x > .git/hooks/pre-commit"),
                "git hook writable"
            );
            assert!(!sh(&ws, p, "echo x > .git/config"), "git config writable");
            assert!(
                sh(&ws, p, "echo x > .git/index-ish"),
                "rest of .git must stay writable"
            );
            assert!(
                !sh(&ws, Profile::ReadOnly, "echo x > ro.txt"),
                "read-only profile wrote"
            );

            assert!(ws.join("ok.txt").exists());
            assert!(!out.join("no.txt").exists());
            let _ = std::fs::remove_dir_all(&ws);
            let _ = std::fs::remove_dir_all(&out);
        }

        #[test]
        fn linked_worktree_metadata_is_protected() {
            let ws =
                std::env::temp_dir().join(format!("chorus-linked-sandbox-{}", std::process::id()));
            let admin = ws.join("admin");
            let common = ws.join("common");
            let tree = ws.join("tree");
            std::fs::create_dir_all(&admin).unwrap();
            std::fs::create_dir_all(common.join("hooks")).unwrap();
            std::fs::create_dir_all(tree.join("nested")).unwrap();
            std::fs::create_dir_all(tree.join(".chorus")).unwrap();
            std::fs::write(tree.join(".git"), "gitdir: ../admin\n").unwrap();
            std::fs::write(admin.join("commondir"), "../common\n").unwrap();
            std::fs::write(common.join("config"), "fixture").unwrap();
            let p = Profile::Workspace;
            assert!(sh(&tree, p, "echo allowed > ordinary.txt"));
            assert!(sh(&tree, p, "echo allowed > ../admin/index"));
            let relocated = ws.with_extension("relocated");
            let moved = sh(
                &tree,
                p,
                &format!("mv '{}' '{}'", ws.display(), relocated.display()),
            );
            if moved {
                std::fs::rename(&relocated, &ws).unwrap();
            }
            assert!(
                !moved,
                "metadata ancestor can be relocated outside protection"
            );
            for script in [
                "echo changed > ../common/config",
                "echo changed > ../common/hooks/pre-commit",
                "echo changed > ../admin/config.worktree",
                "echo changed > ../admin/commondir",
                "echo changed > .git",
                "echo replacement > staging; mv staging ../common/config",
                "echo replacement > staging; mv staging ../admin/commondir",
                "echo replacement > staging; mv staging .git",
                "mv ../common ../relocated-common",
                "mv ../admin ../relocated-admin",
            ] {
                assert!(!sh(&tree, p, script), "metadata write leaked: {script}");
            }
            assert!(!sh(
                &tree.join("nested"),
                p,
                "echo changed > ../../common/config"
            ));
            assert!(!sh(
                &tree.join("nested"),
                p,
                "echo changed > ../.chorus/tamper"
            ));
            assert!(!sh(
                &tree,
                Profile::ReadOnly,
                "echo changed > ../admin/index"
            ));
            assert_eq!(
                std::fs::read_to_string(common.join("config")).unwrap(),
                "fixture"
            );
            std::fs::remove_dir_all(ws).unwrap();
        }

        #[test]
        fn symlinked_metadata_and_shadow_targets_are_protected() {
            use std::os::unix::fs::symlink;
            let ws =
                std::env::temp_dir().join(format!("chorus-symlink-sandbox-{}", std::process::id()));
            let tree = ws.join("tree");
            let admin = ws.join("admin");
            let hooks = ws.join("hooks");
            let shadow = ws.join("shadow");
            for dir in [&tree, &admin, &hooks, &shadow] {
                std::fs::create_dir_all(dir).unwrap();
            }
            std::fs::write(ws.join("configuration"), "fixture").unwrap();
            symlink(&admin, tree.join(".git")).unwrap();
            symlink(&hooks, admin.join("hooks")).unwrap();
            symlink(ws.join("configuration"), admin.join("config")).unwrap();
            symlink(&shadow, tree.join(".chorus")).unwrap();
            for script in [
                "echo changed > ../configuration",
                "echo changed > ../hooks/pre-commit",
                "echo changed > ../shadow/tamper",
                "rm .git",
                "rm .chorus",
                "rm ../admin/hooks",
                "rm ../admin/config",
            ] {
                assert!(
                    !sh(&tree, Profile::Workspace, script),
                    "symlink guard leaked: {script}"
                );
            }
            assert!(sh(&tree, Profile::Workspace, "echo allowed > .git/index"));
            std::fs::remove_dir_all(ws).unwrap();
        }

        #[test]
        fn invalid_metadata_and_unsafe_workspace_paths_fail_closed() {
            let ws =
                std::env::temp_dir().join(format!("chorus-invalid-sandbox-{}", std::process::id()));
            std::fs::create_dir_all(&ws).unwrap();
            std::fs::write(ws.join(".git"), "gitdir: missing\n").unwrap();
            let cmd = vec!["/bin/sh".to_string()];
            assert!(wrap(&cmd, &ws, Profile::Workspace).is_err());
            assert_eq!(
                policy(&ws, Profile::Workspace),
                "(version 1)\n(deny default)\n"
            );
            std::fs::write(ws.join(".git"), "x".repeat(4097)).unwrap();
            assert!(wrap(&cmd, &ws, Profile::Workspace).is_err());
            let quoted = ws.join("quoted\"path");
            std::fs::create_dir_all(&quoted).unwrap();
            assert!(wrap(&cmd, &quoted, Profile::ReadOnly).is_err());
            std::fs::remove_dir_all(ws).unwrap();
        }

        #[test]
        fn dangling_protected_symlinks_fail_closed() {
            use std::os::unix::fs::symlink;
            let ws = std::env::temp_dir()
                .join(format!("chorus-dangling-sandbox-{}", std::process::id()));
            std::fs::create_dir_all(ws.join(".git")).unwrap();
            symlink(ws.join("missing"), ws.join(".git/hooks")).unwrap();
            assert!(checked_policy(&ws, Profile::Workspace).is_err());
            std::fs::remove_dir_all(ws).unwrap();
        }

        #[test]
        fn read_only_workspace_inside_temp_is_still_protected() {
            let ws =
                std::env::temp_dir().join(format!("chorus-readonly-temp-{}", std::process::id()));
            std::fs::create_dir_all(&ws).unwrap();
            assert!(!sh(&ws, Profile::ReadOnly, "echo x > forbidden.txt"));
            assert!(!ws.join("forbidden.txt").exists());
            assert!(sh(&ws, Profile::Workspace, "echo x > allowed.txt"));
            let _ = std::fs::remove_dir_all(ws);
        }
    }
}
