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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelQuotaGroup {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly: Option<Window>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<Window>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RateLimits {
    pub primary: Option<Window>,
    pub secondary: Option<Window>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<ModelQuotaGroup>>,
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

/// Lines kept in the unclassified-failure log, newest last.
const UNCLASSIFIED_KEEP: usize = 50;
/// Characters kept per message: enough to write a classifier key from, short
/// enough that a pasted payload in an error does not end up on disk whole.
const UNCLASSIFIED_MAX_CHARS: usize = 400;

/// A failed turn the failover classifier did not recognise.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UnclassifiedFailure {
    pub at_ms: i64,
    pub agent: String,
    pub message: String,
}

fn unclassified_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".chorus/unclassified-failures.jsonl"))
}

/// Remember a failure `HandoffReason::from_failure` returned `None` for.
///
/// Failover is the headline feature and it has broken silently twice because
/// a provider phrased its limit in words no key matched (CLAUDE.md 7.6b,
/// 7.17). Every unmatched message lands here so `chorus doctor` can show it
/// — the next new limit wording is then one look away instead of a mystery.
pub fn record_unclassified_failure(agent: &str, msg: &str) {
    if let Some(p) = unclassified_path() {
        append_unclassified(&p, agent, msg, now_ms());
    }
}

fn append_unclassified(path: &Path, agent: &str, msg: &str, at_ms: i64) {
    let message: String = msg.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(UNCLASSIFIED_MAX_CHARS).collect();
    let entry = UnclassifiedFailure { at_ms, agent: agent.to_string(), message };
    let mut lines: Vec<String> = std::fs::read_to_string(path)
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default();
    // The same failure repeating every turn is one finding, not fifty.
    let duplicate = lines
        .last()
        .and_then(|l| serde_json::from_str::<UnclassifiedFailure>(l).ok())
        .is_some_and(|last| last.agent == entry.agent && last.message == entry.message);
    if duplicate {
        return;
    }
    let Ok(line) = serde_json::to_string(&entry) else { return };
    lines.push(line);
    let start = lines.len().saturating_sub(UNCLASSIFIED_KEEP);
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(path, lines[start..].join("\n") + "\n");
}

/// The newest `n` unclassified failures, newest first.
pub fn unclassified_failures(n: usize) -> Vec<UnclassifiedFailure> {
    unclassified_path().map(|p| read_unclassified(&p, n)).unwrap_or_default()
}

fn read_unclassified(path: &Path, n: usize) -> Vec<UnclassifiedFailure> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str(l).ok())
        .take(n)
        .collect()
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
    files.sort_by_key(|f| std::cmp::Reverse(f.1));
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
            groups: None,
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
                groups: None,
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

fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

fn parse_iso_to_unix(s: &str) -> Option<i64> {
    let parts: Vec<&str> = s.split(['T', 'Z', '-', ':']).filter(|x| !x.is_empty()).collect();
    if parts.len() >= 6 {
        let year: i64 = parts[0].parse().ok()?;
        let month: i64 = parts[1].parse().ok()?;
        let day: i64 = parts[2].parse().ok()?;
        let hour: i64 = parts[3].parse().ok()?;
        let min: i64 = parts[4].parse().ok()?;
        let sec: i64 = parts[5].parse().ok()?;
        let mut d = day - 1;
        for y in 1970..year {
            d += if is_leap_year(y) { 366 } else { 365 };
        }
        let days_in_months = [31, if is_leap_year(year) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        if (month as usize) >= 1 && (month as usize) <= 12 {
            d += days_in_months[..(month - 1) as usize].iter().sum::<i64>();
            Some(d * 86400 + hour * 3600 + min * 60 + sec)
        } else {
            None
        }
    } else {
        None
    }
}

/// Parse Antigravity quota snapshots from statusline bridges (AIUsageBar, Chorus cache, or raw status JSON).
pub fn parse_antigravity_usage(text: &str) -> Option<RateLimits> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;

    // 1. Antigravity RetrieveUserQuotaSummary groups (Gemini Models, Claude and GPT models)
    let groups_val = v.get("response").and_then(|r| r.get("groups")).or_else(|| v.get("groups"));
    if let Some(groups_arr) = groups_val.and_then(|g| g.as_array()) {
        let mut model_groups = Vec::new();
        let mut primary_win = None;
        let mut secondary_win = None;

        for g in groups_arr {
            let name = g.get("displayName").and_then(|x| x.as_str()).unwrap_or("Model Group").to_string();
            let desc = g.get("description").and_then(|x| x.as_str()).map(|x| x.to_string());
            let mut weekly_win = None;
            let mut five_hour_win = None;

            if let Some(buckets) = g.get("buckets").and_then(|b| b.as_array()) {
                for b in buckets {
                    let window_kind = b.get("window").and_then(|w| w.as_str()).unwrap_or("");
                    let rem_frac = b.get("remainingFraction").and_then(|x| x.as_f64()).unwrap_or(1.0);
                    let used_pct = (1.0 - rem_frac).clamp(0.0, 1.0) * 100.0;
                    let desc_str = b.get("description").and_then(|x| x.as_str()).map(|x| x.to_string());
                    let reset_iso = b.get("resetTime").and_then(|x| x.as_str());
                    let resets_at = reset_iso.and_then(parse_iso_to_unix);

                    let win = Window {
                        used_percent: used_pct,
                        window_minutes: if window_kind == "weekly" { Some(10080) } else { Some(300) },
                        resets_at,
                        resets_label: desc_str,
                    };

                    if window_kind == "weekly" {
                        weekly_win = Some(win);
                    } else if window_kind == "5h" || window_kind == "five_hour" {
                        five_hour_win = Some(win);
                    }
                }
            }

            if name.contains("Gemini") {
                if primary_win.is_none() { primary_win = five_hour_win.clone(); }
                if secondary_win.is_none() { secondary_win = weekly_win.clone(); }
            }

            model_groups.push(ModelQuotaGroup {
                name,
                description: desc,
                weekly: weekly_win,
                five_hour: five_hour_win,
            });
        }

        if !model_groups.is_empty() {
            return Some(RateLimits {
                primary: primary_win.or_else(|| model_groups.first().and_then(|g| g.five_hour.clone())),
                secondary: secondary_win.or_else(|| model_groups.first().and_then(|g| g.weekly.clone())),
                groups: Some(model_groups),
            });
        }
    }

    // 2. Legacy / StatusLine single-window format
    let mut five_hour_used = v.get("five_hour_used").and_then(|x| x.as_f64());
    let mut five_hour_reset = v.get("five_hour_reset").and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)));
    let mut weekly_used = v.get("weekly_used").and_then(|x| x.as_f64());
    let mut weekly_reset = v.get("weekly_reset").and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)));

    if let Some(rl) = v.get("rate_limits").and_then(|x| x.as_object()) {
        if let Some(fh) = rl.get("five_hour").and_then(|x| x.as_object()) {
            if five_hour_used.is_none() {
                five_hour_used = fh.get("used_percentage")
                    .or_else(|| fh.get("used_percent"))
                    .and_then(|x| x.as_f64())
                    .or_else(|| fh.get("remaining_fraction").and_then(|x| x.as_f64()).map(|f| (1.0 - f) * 100.0));
            }
            if five_hour_reset.is_none() {
                five_hour_reset = fh.get("resets_at").and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)));
            }
        }
        if let Some(sd) = rl.get("seven_day").or_else(|| rl.get("weekly")).and_then(|x| x.as_object()) {
            if weekly_used.is_none() {
                weekly_used = sd.get("used_percentage")
                    .or_else(|| sd.get("used_percent"))
                    .and_then(|x| x.as_f64())
                    .or_else(|| sd.get("remaining_fraction").and_then(|x| x.as_f64()).map(|f| (1.0 - f) * 100.0));
            }
            if weekly_reset.is_none() {
                weekly_reset = sd.get("resets_at").and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)));
            }
        }
        if let Some(p) = rl.get("primary").and_then(|x| x.as_object()) {
            if five_hour_used.is_none() {
                five_hour_used = p.get("used_percent").and_then(|x| x.as_f64());
            }
            if five_hour_reset.is_none() {
                five_hour_reset = p.get("resets_at").and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)));
            }
        }
        if let Some(s) = rl.get("secondary").and_then(|x| x.as_object()) {
            if weekly_used.is_none() {
                weekly_used = s.get("used_percent").and_then(|x| x.as_f64());
            }
            if weekly_reset.is_none() {
                weekly_reset = s.get("resets_at").and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)));
            }
        }
    }

    if five_hour_used.is_none() {
        if let Some(rf) = v.get("remaining_fraction").and_then(|x| x.as_f64()) {
            five_hour_used = Some((1.0 - rf) * 100.0);
            if five_hour_reset.is_none() {
                if let Some(sec) = v.get("reset_in_seconds").and_then(|x| x.as_f64()) {
                    let now_sec = now_ms() / 1000;
                    five_hour_reset = Some(now_sec + sec as i64);
                }
            }
        }
    }

    let primary = five_hour_used.map(|used_percent| Window {
        used_percent,
        window_minutes: Some(300),
        resets_at: five_hour_reset,
        resets_label: None,
    });
    let secondary = weekly_used.map(|used_percent| Window {
        used_percent,
        window_minutes: Some(10080),
        resets_at: weekly_reset,
        resets_label: None,
    });

    if primary.is_some() || secondary.is_some() {
        let groups = vec![
            ModelQuotaGroup {
                name: "Gemini Models".into(),
                description: Some("Models within this group: Gemini Flash, Gemini Pro".into()),
                weekly: secondary.clone(),
                five_hour: primary.clone(),
            }
        ];
        Some(RateLimits { primary, secondary, groups: Some(groups) })
    } else {
        None
    }
}

/// Antigravity 5-hour and weekly usage snapshot. Reads only statusline bridge
/// files (AIUsageBar, Chorus cache, account directory); otherwise unknown.
pub fn antigravity_rate_limits(dir: &Path) -> Option<RateLimits> {
    let local_custom = dir.join("antigravity-statusline.json");
    if local_custom.exists() {
        if let Ok(text) = std::fs::read_to_string(&local_custom) {
            if let Some(limits) = parse_antigravity_usage(&text) {
                return Some(limits);
            }
        }
    }

    // No live query. Antigravity exposes quota only through its language
    // server, and reaching that meant scraping another product's CSRF token
    // out of `ps` (every user's processes) and replaying it with `curl -k`.
    // Chorus never reads another tool's credentials, so without a statusline
    // bridge file the quota is unknown (`None`).

    let mut candidates = Vec::new();
    if let Some(h) = std::env::var_os("HOME").map(PathBuf::from) {
        candidates.push(h.join("Library/Application Support/AIUsageBar/antigravity-statusline.json"));
        candidates.push(h.join(".chorus/cache/antigravity-statusline.json"));
        candidates.push(h.join(".chorus/cache/antigravity-usage.json"));
        candidates.push(h.join(".chorus/cache/antigravity-statusline-raw.json"));
    }
    candidates.push(dir.join("antigravity-statusline.json"));
    candidates.push(dir.join("cache/antigravity-statusline.json"));

    for path in candidates {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Some(limits) = parse_antigravity_usage(&text) {
                return Some(limits);
            }
        }
    }
    None
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
    } else if agent.contains("antigravity") || agent.contains("agy") {
        crate::agents::antigravity_logged_in()
    } else {
        dir.join("auth.json").exists()
    }
}

/// Every account of every agent, with whatever usage is knowable for it.
pub fn usage_report() -> Vec<AccountUsage> {
    let hits = hits();
    let mut out = Vec::new();
    for agent in ["claude", "codex", "antigravity"] {
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
                } else if agent == "claude" && ready {
                    claude_rate_limits(&dir, a.is_some())
                } else if agent.contains("antigravity") || agent.contains("agy") {
                    antigravity_rate_limits(&dir)
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

    #[test]
    fn parses_the_real_antigravity_statusline_output() {
        let t = r#"{"five_hour_reset":1791564080,"five_hour_used":0,"observed_at":1791546086.3337841,"weekly_reset":1791949101,"weekly_used":46.693975000000002}"#;
        let r = parse_antigravity_usage(t).unwrap();
        let p = r.primary.unwrap();
        assert_eq!(p.used_percent, 0.0);
        assert_eq!(p.window_minutes, Some(300));
        assert_eq!(p.resets_at, Some(1791564080));
        let s = r.secondary.unwrap();
        assert_eq!(s.used_percent, 46.693975);
        assert_eq!(s.window_minutes, Some(10080));
        assert_eq!(s.resets_at, Some(1791949101));
    }

    #[test]
    fn usage_report_includes_antigravity_rate_limits() {
        let temp = std::env::temp_dir().join(format!("anti-test-{}", std::process::id()));
        std::fs::create_dir_all(&temp).unwrap();
        std::fs::write(
            temp.join("antigravity-statusline.json"),
            r#"{"five_hour_reset":1791564080,"five_hour_used":0,"weekly_reset":1791949101,"weekly_used":46.5}"#,
        ).unwrap();
        let rl = antigravity_rate_limits(&temp).unwrap();
        assert!(rl.secondary.is_some());
        assert_eq!(rl.secondary.unwrap().used_percent, 46.5);
        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn parses_antigravity_quota_summary_groups() {
        let t = r#"{"response":{"groups":[{"displayName":"Gemini Models","description":"Models within this group: Gemini Flash, Gemini Pro","buckets":[{"bucketId":"gemini-weekly","displayName":"Weekly Limit Remaining","description":"You have used some of your weekly limit, it will fully refresh in 4 days, 15 hours.","window":"weekly","remainingFraction":0.51,"resetTime":"2026-10-14T03:38:21Z"},{"bucketId":"gemini-5h","displayName":"Five Hour Limit Remaining","description":"You have used some of your 5-hour limit, it will fully refresh in 2 hours, 5 minutes.","window":"5h","remainingFraction":0.38,"resetTime":"2026-10-09T14:09:04Z"}]},{"displayName":"Claude and GPT models","description":"Models within this group: Claude Opus, Claude Sonnet, GPT-OSS","buckets":[{"bucketId":"3p-weekly","displayName":"Weekly Limit Remaining","description":"You have used some of your weekly limit, it will fully refresh in 4 days, 19 hours.","window":"weekly","remainingFraction":0.48,"resetTime":"2026-10-14T07:54:57Z"},{"bucketId":"3p-5h","displayName":"Five Hour Limit Remaining","window":"5h","remainingFraction":1.0,"resetTime":"2026-10-09T17:46:13Z"}]}]}}"#;
        let rl = parse_antigravity_usage(t).unwrap();
        let groups = rl.groups.unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "Gemini Models");
        assert_eq!((groups[0].weekly.as_ref().unwrap().used_percent * 10.0).round(), 490.0);
        assert_eq!((groups[0].five_hour.as_ref().unwrap().used_percent * 10.0).round(), 620.0);
        assert_eq!(groups[1].name, "Claude and GPT models");
        assert_eq!((groups[1].weekly.as_ref().unwrap().used_percent * 10.0).round(), 520.0);
        assert_eq!(groups[1].five_hour.as_ref().unwrap().used_percent, 0.0);
    }
}

#[cfg(test)]
mod auth_tests {
    use super::*;

    #[test]
    fn revoked_login_is_remembered_then_cleared_by_a_good_turn() {
        let old_home = std::env::var_os("HOME");
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
        if let Some(h) = old_home {
            std::env::set_var("HOME", h);
        }
    }
}

#[cfg(test)]
mod unclassified_tests {
    use super::*;

    #[test]
    fn unmatched_failures_are_kept_clipped_deduped_and_capped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("u.jsonl");
        let long = format!("Internal error: something new {}", "x".repeat(1000));
        append_unclassified(&p, "claude", &long, 1);
        append_unclassified(&p, "claude", &long, 2); // same again: one finding
        append_unclassified(&p, "codex", "stream closed\n  unexpectedly", 3);

        let got = read_unclassified(&p, 10);
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].agent.as_str(), got[0].message.as_str()), ("codex", "stream closed unexpectedly"));
        assert_eq!(got[1].message.chars().count(), UNCLASSIFIED_MAX_CHARS);

        for i in 0..(UNCLASSIFIED_KEEP + 10) {
            append_unclassified(&p, "codex", &format!("error {i}"), i as i64);
        }
        let all = read_unclassified(&p, usize::MAX);
        assert_eq!(all.len(), UNCLASSIFIED_KEEP);
        assert_eq!(all[0].message, format!("error {}", UNCLASSIFIED_KEEP + 9));
    }
}
