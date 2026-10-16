//! The environment a spawned agent inherits, repaired for GUI launches.
//!
//! An app opened from Finder, the Dock or `open` is started by launchd, not by
//! the user's shell: `PATH` is `/usr/bin:/bin:/usr/sbin:/sbin` and nothing from
//! `.zshrc` (Homebrew, nvm, pnpm) is in it. The agents we spawn then cannot
//! find `gh`, `node` or `npx`, and "git doesn't work with my account" is the
//! visible symptom — Apple's `git` is found, everything around it is not.
//!
//! Fix: ask the user's login shell for its `PATH` once, and append whatever the
//! process is missing. Appended, never prepended — a `PATH` the caller (or a
//! terminal launch) already chose keeps its order. Only `PATH` is imported;
//! copying a whole shell environment would drag tokens into every agent.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Interactive on purpose: nvm and friends are usually set up in `.zshrc`,
/// which a non-interactive login shell never reads. Slow rc files make this
/// take seconds, hence the cap and the cache.
const PROBE_TIMEOUT: Duration = Duration::from_secs(6);

/// Where tools live on a stock Mac when no shell has been consulted.
fn well_known_dirs() -> Vec<PathBuf> {
    let mut v = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/opt/homebrew/sbin"),
        PathBuf::from("/usr/local/bin"),
    ];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        v.push(home.join(".local/bin"));
        v.push(home.join(".cargo/bin"));
        v.push(home.join(".npm-global/bin"));
        v.push(home.join(".bun/bin"));
    }
    v
}

fn login_shell_path() -> Option<OsString> {
    static CACHE: OnceLock<Option<OsString>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let shell = std::env::var_os("SHELL")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "/bin/zsh".into());
            let mut child = Command::new(shell)
                // Markers keep rc-file banners out of the value we read.
                .args(["-ilc", "printf '<<chorus>>%s<<chorus>>' \"$PATH\""])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .ok()?;
            let deadline = Instant::now() + PROBE_TIMEOUT;
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(25))
                    }
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return None;
                    }
                }
            }
            let mut out = String::new();
            std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
            let start = out.find("<<chorus>>")? + "<<chorus>>".len();
            let len = out[start..].find("<<chorus>>")?;
            Some(OsString::from(&out[start..start + len]))
        })
        .clone()
}

/// `base` followed by every directory of `extra` it does not already contain.
pub fn merge_paths(base: &OsString, extra: &[PathBuf]) -> OsString {
    let mut dirs: Vec<PathBuf> = std::env::split_paths(base).collect();
    for d in extra {
        if !d.as_os_str().is_empty() && !dirs.contains(d) {
            dirs.push(d.clone());
        }
    }
    std::env::join_paths(dirs).unwrap_or_else(|_| base.clone())
}

/// `PATH` for a spawned agent: `explicit` (what the caller set) or ours,
/// extended with the login shell's directories and the well-known ones.
pub fn agent_path(explicit: Option<&OsString>) -> OsString {
    let base = explicit
        .cloned()
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    let mut extra: Vec<PathBuf> = login_shell_path()
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    extra.extend(well_known_dirs().into_iter().filter(|d| d.is_dir()));
    merge_paths(&base, &extra)
}

/// Make this process's own `PATH` the repaired one, once.
///
/// Spawned agents get it through [`agent_path`], but the app also shells out
/// itself — `which claude` for detection, `claude -p /usage`, `cargo`, linters.
/// From a Finder launch every one of those saw launchd's bare `PATH`, and the
/// app reported "no agents found" on a machine full of them. Call before the
/// first such lookup; later calls are free.
pub fn repair_process_path() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let fixed = agent_path(None);
        std::env::set_var("PATH", fixed);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_appends_missing_and_keeps_order() {
        let base = OsString::from("/usr/bin:/bin");
        let out = merge_paths(
            &base,
            &[PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/bin")],
        );
        assert_eq!(out, OsString::from("/usr/bin:/bin:/opt/homebrew/bin"));
    }

    #[test]
    fn merge_never_reorders_a_chosen_path() {
        let base = OsString::from("/custom:/usr/bin");
        let out = merge_paths(&base, &[PathBuf::from("/usr/bin"), PathBuf::from("/x")]);
        assert_eq!(out, OsString::from("/custom:/usr/bin:/x"));
    }
}
