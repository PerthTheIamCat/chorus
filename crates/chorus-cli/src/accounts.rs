//! More than one login per provider.
//!
//! An account is nothing but a config directory: Codex reads its login from
//! `CODEX_HOME`, Claude Code from `CLAUDE_CONFIG_DIR`. So `codex@work` is the
//! Codex bridge spawned with `CODEX_HOME=~/.chorus/accounts/codex/work`, and
//! plain `codex` is the CLI's own default home. Chorus never sees or stores a
//! token — each account is logged in through its own CLI, once.
//!
//! The relay treats `claude@work` as just another agent in the chain, so when
//! one account hits its limit the next account of the same provider can take
//! over before Chorus switches provider.

use std::path::PathBuf;

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// `claude@work` → (`claude`, Some(`work`)); `codex` → (`codex`, None).
pub fn split(name: &str) -> (&str, Option<&str>) {
    match name.split_once('@') {
        Some((base, acct)) if !acct.is_empty() => (base, Some(acct)),
        _ => (name, None),
    }
}

/// Root of the extra accounts for one agent.
pub fn root(agent: &str) -> Option<PathBuf> {
    home().map(|h| h.join(".chorus/accounts").join(agent))
}

/// The config dir an agent name runs with. `None` account = the CLI's default.
pub fn dir(agent: &str, account: Option<&str>) -> Option<PathBuf> {
    match account {
        Some(a) => root(agent).map(|r| r.join(a)),
        None if agent.contains("claude") => std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".claude"))),
        None if agent.contains("antigravity") || agent.contains("agy") => {
            std::env::var_os("ANTIGRAVITY_CONFIG_DIR")
                .or_else(|| std::env::var_os("AGY_HOME"))
                .map(PathBuf::from)
                .or_else(|| home().map(|h| h.join(".gemini/antigravity")))
        }
        None => std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".codex"))),
    }
}

fn env_key(agent: &str) -> &'static str {
    if agent.contains("claude") {
        "CLAUDE_CONFIG_DIR"
    } else if agent.contains("antigravity") || agent.contains("agy") {
        "ANTIGRAVITY_CONFIG_DIR"
    } else {
        "CODEX_HOME"
    }
}

/// Extra environment for spawning `name` (empty for a default account).
pub fn env_for(name: &str) -> Vec<(String, String)> {
    let (agent, account) = split(name);
    match (account, dir(agent, account)) {
        (Some(_), Some(d)) => vec![(env_key(agent).to_string(), d.to_string_lossy().into_owned())],
        _ => Vec::new(),
    }
}

/// A name usable as a directory and inside an agent name.
pub fn valid_name(account: &str) -> bool {
    !account.is_empty()
        && account.len() <= 40
        && account
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Every extra account of `agent`, sorted.
pub fn list(agent: &str) -> Vec<String> {
    let Some(r) = root(agent) else {
        return Vec::new();
    };
    let mut out: Vec<String> = std::fs::read_dir(r)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| valid_name(n))
        .collect();
    out.sort();
    out
}

/// Where to look for sessions other tools left on disk: each CLI's default
/// config dir plus every extra Chorus account of Claude and Codex.
pub fn session_roots() -> Vec<chorus_core::external_sessions::Root> {
    use chorus_core::external_sessions::{default_roots, Provider, Root};
    let mut out = default_roots();
    for provider in [Provider::Claude, Provider::Codex] {
        for account in list(provider.agent()) {
            if let Some(dir) = dir(provider.agent(), Some(&account)) {
                out.push(Root { provider, dir, account: Some(account) });
            }
        }
    }
    out
}

/// Create an account's (empty) config dir. Logging in fills it.
pub fn add(agent: &str, account: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(valid_name(account), "ชื่อบัญชีใช้ได้แค่ a-z 0-9 - _ (ไม่เกิน 40 ตัว)");
    let d = dir(agent, Some(account)).ok_or_else(|| anyhow::anyhow!("no HOME"))?;
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

/// Delete an extra account's config dir — its login and local history.
pub fn remove(agent: &str, account: &str) -> anyhow::Result<()> {
    anyhow::ensure!(valid_name(account), "invalid account name");
    let d = dir(agent, Some(account)).ok_or_else(|| anyhow::anyhow!("no HOME"))?;
    if d.exists() {
        std::fs::remove_dir_all(d)?;
    }
    Ok(())
}

/// The command that logs this account in, for the user to run in Terminal.
pub fn login_command(agent: &str, account: Option<&str>) -> String {
    let base = if agent.contains("claude") {
        "claude"
    } else if agent.contains("antigravity") || agent.contains("agy") {
        "agy login"
    } else {
        "codex login"
    };
    match (account, dir(agent, account)) {
        (Some(_), Some(d)) => format!("{}=\"{}\" {base}", env_key(agent), d.display()),
        _ => base.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_env() {
        assert_eq!(split("claude@work"), ("claude", Some("work")));
        assert_eq!(split("codex"), ("codex", None));
        assert_eq!(split("antigravity"), ("antigravity", None));
        assert_eq!(split("codex@"), ("codex@", None));
        assert!(env_for("codex").is_empty());
        assert!(env_for("antigravity").is_empty());
        let e = env_for("codex@work");
        assert_eq!(e[0].0, "CODEX_HOME");
        assert!(e[0].1.ends_with(".chorus/accounts/codex/work"));
        assert_eq!(env_for("claude@x")[0].0, "CLAUDE_CONFIG_DIR");
        assert_eq!(env_for("antigravity@x")[0].0, "ANTIGRAVITY_CONFIG_DIR");
        assert_eq!(login_command("antigravity", None), "agy login");
        assert_eq!(login_command("codex", None), "codex login");
        assert_eq!(login_command("claude", None), "claude");
        assert!(!valid_name("../x"));
        assert!(valid_name("work-2"));
    }
}
