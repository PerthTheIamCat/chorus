//! How close each agent/account is to its plan limit.
//!
//! Two sources, both honest about what they are:
//!
//! - **Codex reports it.** Every turn the CLI writes `rate_limits` into its
//!   session log (`$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl`): a 5-hour
//!   window and a weekly one, each with `used_percent` and `resets_at`. We read
//!   only those two objects from the newest log, nothing else in the file.
//! - **Claude is asked.** No file or ACP message carries its usage (the ACP
//!   bridge drops the SDK's `rate_limit_event`), but the CLI's own headless
//!   `claude -p "/usage"` prints the 5-hour and weekly percentages. We run
//!   that, cache the answer for a couple of minutes, and parse the text. No
//!   token is read. When that fails we still know the last time it refused a
//!   turn for hitting a limit, and the reset time it printed ("resets
//!   9:50pm"). The relay records that here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Window {
    pub used_percent: f64,
    pub window_minutes: Option<i64>,
    /// Unix seconds.
    pub resets_at: Option<i64>,
    /// The reset time as the agent printed it, when it gave no timestamp
    /// ("Oct 5 at 9:30pm (Asia/Bangkok)").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_label: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RateLimits {
    pub primary: Option<Window>,
    pub secondary: Option<Window>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Hit {
    pub at_ms: i64,
    /// The reset text the agent printed, e.g. "9:50pm (Asia/Bangkok)".
    pub resets: Option<String>,
}

fn store_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".chorus/limits.json"))
}

pub fn hits() -> BTreeMap<String, Hit> {
    store_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(map: &BTreeMap<String, Hit>) {
    if let Some(p) = store_path() {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(s) = serde_json::to_string_pretty(map) {
            let _ = std::fs::write(p, s);
        }
    }
}

/// "…hit your limit · resets 9:50pm (Asia/Bangkok)" → "9:50pm (Asia/Bangkok)".
pub fn parse_reset(msg: &str) -> Option<String> {
    let i = msg.to_lowercase().find("resets")?;
    let rest = msg[i + "resets".len()..]
        .trim_start_matches([' ', ':'])
        .trim();
    let end = rest.find(['\n', '"']).unwrap_or(rest.len());
    let r = rest[..end].trim().trim_end_matches(['.', ',']);
    (!r.is_empty()).then(|| r.chars().take(60).collect())
}

/// The relay saw `agent` refuse a turn for a limit.
pub fn record_hit(agent: &str, message: &str) {
    let mut m = hits();
    let at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    m.insert(
        agent.to_string(),
        Hit {
            at_ms,
            resets: parse_reset(message),
        },
    );
    save(&m);
}

/// `agent` answered a turn, so whatever limit it had is over.
pub fn clear(agent: &str) {
    let mut m = hits();
    if m.remove(agent).is_some() {
        save(&m);
    }
    clear_auth_failure(agent);
}

/// How long a rejected login is believed without asking the agent again.
/// Short enough that a fresh `claude /login` is picked up on its own, long
/// enough that a revoked token is not retried (and waited on) every turn.
const AUTH_FAILURE_TTL_MS: i64 = 30 * 60 * 1000;

fn auth_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".chorus/auth-failed.json"))
}

/// "claude-opus" and "claude" are the same login; so are the codex names.
fn login_key(agent: &str) -> String {
    let a = agent.to_lowercase();
    if a.contains("claude") {
        "claude".into()
    } else if a.contains("codex") || a.contains("gpt") {
        "codex".into()
    } else {
        a
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn auth_failures() -> BTreeMap<String, i64> {
    auth_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_auth_failures(m: &BTreeMap<String, i64>) {
    if let Some(p) = auth_path() {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(s) = serde_json::to_string(m) {
            let _ = std::fs::write(p, s);
        }
    }
}

/// The agent answered with a rejected login (401, revoked token). The CLI's
/// own `auth status` can still say "logged in" in that state, so remember it.
pub fn record_auth_failure(agent: &str) {
    let mut m = auth_failures();
    m.insert(login_key(agent), now_ms());
    save_auth_failures(&m);
}

pub fn clear_auth_failure(agent: &str) {
    let mut m = auth_failures();
    if m.remove(&login_key(agent)).is_some() {
        save_auth_failures(&m);
    }
}

/// Does this failure text say the login itself was rejected?
pub fn is_auth_failure(msg: &str) -> bool {
    let m = msg.to_lowercase();
    ["failed to authenticate", "authentication_error", "token has been revoked", "authentication required", "not logged in", "please run /login"]
        .iter()
        .any(|k| m.contains(k))
}

/// True while a recent turn on this agent's login was rejected.
pub fn auth_failed(agent: &str) -> bool {
    auth_failures()
        .get(&login_key(agent))
        .is_some_and(|at| now_ms() - at < AUTH_FAILURE_TTL_MS)
}

/// The newest `rate_limits` Codex wrote under `codex_home`.
pub fn codex_rate_limits(codex_home: &Path) -> Option<RateLimits> {
    let mut files = newest_rollouts(&codex_home.join("sessions"), 8);
    files.sort_by(|a, b| b.1.cmp(&a.1));
    for (f, _) in files {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let Some(line) = text.lines().rev().find(|l| l.contains("\"rate_limits\"")) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(rl) = find_key(&v, "rate_limits") else {
            continue;
        };
        let win = |k: &str| {
            rl.get(k)
                .and_then(|w| serde_json::from_value::<Window>(w.clone()).ok())
        };
        let r = RateLimits {
            primary: win("primary"),
            secondary: win("secondary"),
        };
        if r.primary.is_some() || r.secondary.is_some() {
            return Some(r);
        }
    }
    None
}

fn find_key<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    match v {
        serde_json::Value::Object(m) => m
            .get(key)
            .filter(|x| x.is_object())
            .or_else(|| m.values().find_map(|x| find_key(x, key))),
        serde_json::Value::Array(a) => a.iter().find_map(|x| find_key(x, key)),
        _ => None,
    }
}

/// Up to `n` most recently modified `*.jsonl` under `sessions/YYYY/MM/DD`,
/// walking newest date folders first so an old history stays cheap.
fn newest_rollouts(root: &Path, n: usize) -> Vec<(PathBuf, std::time::SystemTime)> {
    fn sorted_dirs(p: &Path) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(p)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        v.sort();
        v.reverse();
        v
    }
    let mut out = Vec::new();
    for y in sorted_dirs(root) {
        for m in sorted_dirs(&y) {
            for d in sorted_dirs(&m) {
                for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x == "jsonl") {
                        if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                            out.push((p, t));
                        }
                    }
                }
                if out.len() >= n {
                    return out;
                }
            }
        }
    }
    out
}

/// One row of the Settings → Accounts & usage table.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AccountUsage {
    /// What goes in a relay chain: `codex`, `claude@work`.
    pub name: String,
    pub agent: String,
    /// Empty for the CLI's own default login.
    pub account: String,
    pub dir: String,
    pub ready: bool,
    pub login: String,
    pub rate_limits: Option<RateLimits>,
    pub hit: Option<Hit>,
}

/// `Current session: 18% used · resets Oct 5 at 9:30pm (Asia/Bangkok)` and
/// `Current week (all models): 20% used · resets Oct 7 at 6pm (Asia/Bangkok)`
/// from `claude -p "/usage"`. Session = the 5-hour window, "week (all
/// models)" = the weekly one; per-model weekly lines are ignored.
pub fn parse_claude_usage(text: &str) -> Option<RateLimits> {
    let mut out = RateLimits::default();
    for line in text.lines() {
        let l = line.trim();
        let Some(rest) = l.strip_prefix("Current ") else {
            continue;
        };
        let Some((label, tail)) = rest.split_once(':') else {
            continue;
        };
        let label = label.trim().to_lowercase();
        let Some((pct, after)) = tail.split_once("% used") else {
            continue;
        };
        let Ok(used_percent) = pct.trim().parse::<f64>() else {
            continue;
        };
        let resets_label = after
            .split_once("resets")
            .map(|(_, r)| r.trim().to_string())
            .filter(|r| !r.is_empty());
        let (slot, minutes) = if label == "session" {
            (&mut out.primary, 300)
        } else if label == "week (all models)" || label == "week" {
            (&mut out.secondary, 10080)
        } else {
            continue;
        };
        *slot = Some(Window {
            used_percent,
            window_minutes: Some(minutes),
            resets_at: None,
            resets_label,
        });
    }
    (out.primary.is_some() || out.secondary.is_some()).then_some(out)
}

/// How long a `/usage` answer is reused. Each call spawns the CLI (a couple
/// of seconds with the user's hooks), and the settings pane asks often.
const USAGE_TTL_MS: i64 = 2 * 60 * 1000;

fn usage_cache_path(dir: &Path) -> Option<PathBuf> {
    let tag: String = dir
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(format!(".chorus/cache/claude-usage-{tag}.json")))
}

/// Claude's 5-hour / weekly usage for the login in `dir` (`custom` = a
/// Chorus-made account that needs `CLAUDE_CONFIG_DIR`; otherwise the CLI's
/// own default). Cached; `None` if the CLI is missing, logged out or slow.
pub fn claude_rate_limits(dir: &Path, custom: bool) -> Option<RateLimits> {
    #[derive(serde::Deserialize, serde::Serialize)]
    struct Cached {
        at_ms: i64,
        limits: RateLimits2,
    }
    #[derive(serde::Deserialize, serde::Serialize)]
    struct RateLimits2 {
        primary: Option<Window>,
        secondary: Option<Window>,
    }
    let cache = usage_cache_path(dir);
    if let Some(c) = cache
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Cached>(&s).ok())
    {
        if now_ms() - c.at_ms < USAGE_TTL_MS {
            return Some(RateLimits {
                primary: c.limits.primary,
                secondary: c.limits.secondary,
            });
        }
    }
    let mut cmd = std::process::Command::new("claude");
    cmd.args(["-p", "/usage"])
        .current_dir(std::env::temp_dir())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    if custom {
        cmd.env("CLAUDE_CONFIG_DIR", dir);
    }
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut s);
        s
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    let r = parse_claude_usage(&reader.join().ok()?)?;
    if let Some(p) = cache {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let c = Cached {
            at_ms: now_ms(),
            limits: RateLimits2 {
                primary: r.primary.clone(),
                secondary: r.secondary.clone(),
            },
        };
        if let Ok(s) = serde_json::to_string(&c) {
            let _ = std::fs::write(p, s);
        }
    }
    Some(r)
}

fn account_ready(agent: &str, account: Option<&str>, dir: &Path) -> bool {
    if account.is_none() {
        return matches!(
            crate::agents::auth_status(agent).auth,
            crate::agents::AuthState::Ok
        );
    }
    if agent.contains("claude") {
        crate::agents::claude_auth_status(Some(dir)).unwrap_or_else(|| dir.join(".credentials.json").exists())
    } else {
        dir.join("auth.json").exists()
    }
}

/// Every account of every agent, with whatever usage is knowable for it.
pub fn usage_report() -> Vec<AccountUsage> {
    let hits = hits();
    let mut out = Vec::new();
    for agent in ["claude", "codex"] {
        let accounts: Vec<Option<String>> = std::iter::once(None)
            .chain(crate::accounts::list(agent).into_iter().map(Some))
            .collect();
        for acct in accounts {
            let a = acct.as_deref();
            let Some(dir) = crate::accounts::dir(agent, a) else {
                continue;
            };
            let name = match a {
                Some(x) => format!("{agent}@{x}"),
                None => agent.to_string(),
            };
            let ready = account_ready(agent, a, &dir);
            out.push(AccountUsage {
                ready,
                login: crate::accounts::login_command(agent, a),
                rate_limits: if agent == "codex" {
                    codex_rate_limits(&dir)
                } else if ready {
                    claude_rate_limits(&dir, a.is_some())
                } else {
                    None
                },
                hit: hits.get(&name).cloned(),
                agent: agent.to_string(),
                account: a.unwrap_or("").to_string(),
                dir: dir.to_string_lossy().into_owned(),
                name,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_text_from_the_real_claude_message() {
        let m = "Internal error: You've hit your limit · resets 9:50pm (Asia/Bangkok)";
        assert_eq!(parse_reset(m).as_deref(), Some("9:50pm (Asia/Bangkok)"));
        assert_eq!(parse_reset("rate limit exceeded"), None);
    }

    #[test]
    fn parses_the_real_claude_usage_output() {
        let t = "You are currently using your subscription\n\nCurrent session: 18% used · resets Oct 5 at 9:30pm (Asia/Bangkok)\nCurrent week (all models): 20% used · resets Oct 7 at 6pm (Asia/Bangkok)\nCurrent week (Sonnet only): 3% used · resets Oct 7 at 6pm\n\nWhat's contributing";
        let r = parse_claude_usage(t).unwrap();
        let p = r.primary.unwrap();
        assert_eq!(p.used_percent, 18.0);
        assert_eq!(p.window_minutes, Some(300));
        assert_eq!(p.resets_label.as_deref(), Some("Oct 5 at 9:30pm (Asia/Bangkok)"));
        assert_eq!(r.secondary.unwrap().used_percent, 20.0);
        assert!(parse_claude_usage("Not logged in").is_none());
    }

    #[test]
    fn reads_rate_limits_from_a_rollout() {
        let root = std::env::temp_dir().join(format!("chorus-rl-{}", std::process::id()));
        let day = root.join("sessions/2026/10/05");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(
            day.join("rollout-x.jsonl"),
            "{\"type\":\"session_meta\"}\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"rate_limits\":{\"primary\":{\"used_percent\":6.0,\"window_minutes\":300,\"resets_at\":1791185007},\"secondary\":{\"used_percent\":12.0,\"window_minutes\":10080,\"resets_at\":1791619366}}}}\n",
        )
        .unwrap();
        let r = codex_rate_limits(&root).unwrap();
        assert_eq!(r.primary.unwrap().used_percent, 6.0);
        assert_eq!(r.secondary.unwrap().window_minutes, Some(10080));
        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod auth_tests {
    use super::*;

    #[test]
    fn revoked_login_is_remembered_then_cleared_by_a_good_turn() {
        let home = std::env::temp_dir().join(format!("chorus-auth-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        std::env::set_var("HOME", &home);
        assert!(!auth_failed("claude-opus"));
        assert!(is_auth_failure("Failed to authenticate. API Error: 401 OAuth access token has been revoked."));
        record_auth_failure("claude-opus");
        assert!(auth_failed("claude"));
        assert!(!auth_failed("codex"));
        clear("claude");
        assert!(!auth_failed("claude-opus"));
    }
}
