//! Which processes Chorus itself started.
//!
//! The system monitor labels rows with this ("agent", "spawned by agent X")
//! and only lets `killpg` touch a process group whose leader is listed here.
//! A label read back from the process (argv, name) is something the agent can
//! influence; this list is written only by our own spawn path.
//!
//! The spawn time is recorded so a reused pid is not mistaken for an agent
//! that has since exited: the monitor compares it with the kernel's start
//! time for that pid.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spawned {
    pub pid: u32,
    /// Short name for the UI, e.g. `codex-acp`. Never the full command line:
    /// a sandboxed bridge's argv carries the whole Seatbelt profile.
    pub label: String,
    /// Wall clock (ms since the epoch) just after `spawn` returned.
    pub spawned_ms: u64,
    /// The process leads its own group (spawned with `process_group(0)`),
    /// so `killpg(pid)` reaches only what it started.
    pub group_leader: bool,
}

static REGISTRY: Mutex<Vec<Spawned>> = Mutex::new(Vec::new());

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Record a process Chorus just spawned.
pub fn register(pid: u32, label: impl Into<String>, group_leader: bool) {
    if let Ok(mut r) = REGISTRY.lock() {
        r.retain(|s| s.pid != pid);
        r.push(Spawned {
            pid,
            label: label.into(),
            spawned_ms: now_ms(),
            group_leader,
        });
    }
}

/// Forget `pid` once it has been reaped.
pub fn unregister(pid: u32) {
    if let Ok(mut r) = REGISTRY.lock() {
        r.retain(|s| s.pid != pid);
    }
}

/// Everything currently registered.
pub fn list() -> Vec<Spawned> {
    REGISTRY.lock().map(|r| r.clone()).unwrap_or_default()
}

/// A short, display-only name for an agent command line: the first argument
/// (from the end) whose file name mentions `acp`, else the program's name.
/// `sandbox-exec -p <profile> npx -y @agentclientprotocol/codex-acp` →
/// `codex-acp`.
pub fn label_for(cmd: &[String]) -> String {
    fn base(s: &str) -> &str {
        let s = s.rsplit('/').next().unwrap_or(s);
        s.split('@').next().filter(|p| !p.is_empty()).unwrap_or(s)
    }
    cmd.iter()
        .rev()
        .filter(|a| !a.starts_with('-') && a.len() < 200 && !a.contains(char::is_whitespace))
        .map(|a| base(a))
        .find(|b| b.contains("acp"))
        .or_else(|| cmd.first().map(|p| base(p)))
        .unwrap_or("agent")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn label_picks_the_bridge_not_the_wrapper() {
        assert_eq!(
            label_for(&v(&["npx", "-y", "@agentclientprotocol/codex-acp"])),
            "codex-acp"
        );
        assert_eq!(
            label_for(&v(&[
                "sandbox-exec",
                "-p",
                "(version 1)\n(allow default)",
                "npx",
                "-y",
                "@zed-industries/claude-agent-acp@1.2"
            ])),
            "claude-agent-acp"
        );
        assert_eq!(
            label_for(&v(&["/usr/local/bin/gemini", "--experimental-acp"])),
            "gemini"
        );
    }

    #[test]
    fn register_replaces_a_reused_pid() {
        register(999_001, "a", true);
        register(999_001, "b", true);
        let mine: Vec<_> = list().into_iter().filter(|s| s.pid == 999_001).collect();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].label, "b");
        unregister(999_001);
        assert!(list().iter().all(|s| s.pid != 999_001));
    }
}
