//! What can actually answer a turn on this machine, per provider.
//!
//! This is a pre-flight inventory for the UI — "which agents can I even
//! pick from right now" — not the relay's failure classifier
//! (`chorus_core::handoff::HandoffReason::from_failure`), which reacts to
//! an agent that already started answering and hit a limit mid-turn.
//!
//! Detection here is deliberately cheap and inert:
//! - never runs a login flow or touches the network
//! - never reads, prints or parses a credential file — existence only,
//!   because those files hold live tokens (see CLAUDE.md §11: never read
//!   or print `.env` / credential / token files)
//!
//! On macOS "does the credential file exist" is the wrong question for
//! Claude: the CLI migrates its tokens into the login Keychain and then
//! *deletes* `~/.claude/.credentials.json`, so a freshly logged-in Mac has
//! no file to find. See [`claude_logged_in`].
//!
//! Models come from the agent itself. ACP has no *method* for listing them,
//! which this module once took to mean they could not be known — so it shipped
//! a curated guess. A probe against the real bridge showed the guess was
//! wrong: Chorus offered `gpt-5.1-codex`, and Codex actually runs
//! `gpt-6-astra`, `gpt-5.6-sol`, `gpt-5.6-terra`, `gpt-5.6-luna`, `gpt-5.5`.
//! The list is in the `session/new` *reply* — no method needed, just a session.
//!
//! Opening one costs an `npx` spawn, which is far too slow for a picker that
//! redraws, so [`refresh_models`] probes on demand and caches, and everything
//! else reads the cache. The curated list survives only as the fallback for a
//! provider that has never been probed, and it says so rather than pretending
//! to be authoritative. `CHORUS_CLAUDE_MODELS` / `CHORUS_CODEX_MODELS` still
//! override everything, for a machine where probing is not wanted at all.

use anyhow::Context;
use std::path::PathBuf;
use std::process::Command;

/// One selectable model for an agent, as a picker would show it.
pub struct ModelChoice {
    pub id: String,
    pub label: String,
    pub is_default: bool,
}

/// Where a provider's model list came from, so the UI can say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSource {
    /// Reported by the agent itself and cached.
    Probed,
    /// `CHORUS_*_MODELS`.
    Env,
    /// The built-in guess — never been probed on this machine.
    Fallback,
}

impl ModelSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelSource::Probed => "probed",
            ModelSource::Env => "env",
            ModelSource::Fallback => "fallback",
        }
    }
}

fn cache_path(agent_id: &str) -> Option<PathBuf> {
    home_dir().map(|h| h.join(".cache/chorus").join(format!("models-{agent_id}.json")))
}

/// Models this machine has actually seen `agent_id` report, if any.
///
/// Cached as `id\tname\t0|1` lines rather than JSON: it is written and read
/// only here, and a format with no parser cannot fail to parse.
fn cached_models(agent_id: &str) -> Vec<ModelChoice> {
    let Some(p) = cache_path(agent_id) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(p) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let id = f.next()?.trim();
            if id.is_empty() {
                return None;
            }
            let label = f.next().unwrap_or(id).trim();
            Some(ModelChoice {
                id: id.to_string(),
                label: label.to_string(),
                is_default: f.next().unwrap_or("0").trim() == "1",
            })
        })
        .collect()
}

/// Open a throwaway session with `agent_id` purely to read back its model
/// list, then cache it.
///
/// Slow on purpose and never on the turn path: it spawns the bridge. `cwd`
/// only has to exist — nothing is written there and no prompt is sent.
pub fn refresh_models(agent_id: &str, cwd: &std::path::Path) -> anyhow::Result<Vec<ModelChoice>> {
    use chorus_acp::codex::{CodexAgent, CodexConfig};

    let cfg = CodexConfig {
        command: chorus_acp::codex::bridge_for(agent_id),
        mode: None,
        ..Default::default()
    };
    let agent = CodexAgent::start(cwd, cfg)
        .with_context(|| format!("open a session with {agent_id} to read its models"))?;
    let found: Vec<ModelChoice> = agent
        .models()
        .iter()
        .map(|m| ModelChoice {
            id: m.id.clone(),
            label: m.name.clone(),
            is_default: m.is_current,
        })
        .collect();

    if found.is_empty() {
        anyhow::bail!("{agent_id} did not report any models in session/new");
    }
    if let Some(p) = cache_path(agent_id) {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let body: String = found
            .iter()
            .map(|m| format!("{}\t{}\t{}\n", m.id, m.label, u8::from(m.is_default)))
            .collect();
        let _ = std::fs::write(p, body);
    }
    Ok(found)
}

/// Whether the underlying CLI can actually answer a turn right now.
pub enum AuthState {
    /// CLI present, credentials found — ready to run.
    Ok,
    /// CLI present, no credentials found.
    NeedsLogin,
    /// The CLI itself is not on `PATH`.
    MissingCli,
}

impl AuthState {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthState::Ok => "ok",
            AuthState::NeedsLogin => "needs_login",
            AuthState::MissingCli => "missing_cli",
        }
    }
}

/// Everything the UI needs to render one row of an agent picker: identity,
/// what models it offers, whether it can run right now, and — if not —
/// the exact command that fixes it.
pub struct AgentInfo {
    /// "claude" | "codex".
    pub id: String,
    pub label: String,
    /// The ACP bridge command `chorus_acp::codex::bridge_for` resolves for
    /// this agent — what would actually be spawned.
    pub bridge: Vec<String>,
    pub models: Vec<ModelChoice>,
    /// Where `models` came from — probed, env, or the built-in guess.
    pub model_source: ModelSource,
    pub auth: AuthState,
    /// The exact shell command that fixes the current `auth` state.
    pub hint: String,
    /// One short human sentence about the current state.
    pub detail: String,
}

/// Is `cli` on `PATH`? Shelling out to `which` is the one check that behaves
/// the same whether the CLI is a native binary, an npm shim, or a shell
/// function wrapping one — cheap, synchronous, no network.
fn cli_on_path(cli: &str) -> bool {
    Command::new("which")
        .arg(cli)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Parse `CHORUS_CLAUDE_MODELS` / `CHORUS_CODEX_MODELS`: comma-separated
/// `id=label` pairs (or bare `id`, used as its own label), first entry is
/// the default. `None` when the variable is unset or empty — the caller
/// falls back to the curated list.
fn parse_models_env(var: &str) -> Option<Vec<ModelChoice>> {
    let v = std::env::var(var).ok()?;
    let out: Vec<ModelChoice> = v
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .enumerate()
        .map(|(i, part)| {
            let (id, label) = part.split_once('=').unwrap_or((part, part));
            ModelChoice {
                id: id.trim().to_string(),
                label: label.trim().to_string(),
                is_default: i == 0,
            }
        })
        .collect();
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn models_for(agent_id: &str, env_var: &str, fallback: fn() -> Vec<ModelChoice>) -> (Vec<ModelChoice>, ModelSource) {
    if let Some(v) = parse_models_env(env_var) {
        return (v, ModelSource::Env);
    }
    let cached = cached_models(agent_id);
    if !cached.is_empty() {
        return (cached, ModelSource::Probed);
    }
    (fallback(), ModelSource::Fallback)
}

fn claude_models() -> Vec<ModelChoice> {
    parse_models_env("CHORUS_CLAUDE_MODELS").unwrap_or_else(|| {
        vec![
            ModelChoice {
                id: "claude-opus-4-6".into(),
                label: "Claude Opus".into(),
                is_default: true,
            },
            ModelChoice {
                id: "claude-sonnet-4-6".into(),
                label: "Claude Sonnet".into(),
                is_default: false,
            },
        ]
    })
}

fn codex_models() -> Vec<ModelChoice> {
    parse_models_env("CHORUS_CODEX_MODELS").unwrap_or_else(|| {
        vec![
            ModelChoice {
                id: "gpt-5.1-codex".into(),
                label: "GPT-5.1 Codex".into(),
                is_default: true,
            },
            ModelChoice {
                id: "gpt-5.1-codex-mini".into(),
                label: "GPT-5.1 Codex Mini".into(),
                is_default: false,
            },
        ]
    })
}

fn antigravity_models() -> Vec<ModelChoice> {
    parse_models_env("CHORUS_ANTIGRAVITY_MODELS").unwrap_or_else(|| {
        vec![
            ModelChoice {
                id: "gemini-3.8-flash[high]".into(),
                label: "Gemini 3.8 Flash (High)".into(),
                is_default: true,
            },
            ModelChoice {
                id: "gemini-3.8-flash[medium]".into(),
                label: "Gemini 3.8 Flash (Medium)".into(),
                is_default: false,
            },
            ModelChoice {
                id: "gemini-3.8-flash[low]".into(),
                label: "Gemini 3.8 Flash (Low)".into(),
                is_default: false,
            },
            ModelChoice {
                id: "gemini-3.7-flash[high]".into(),
                label: "Gemini 3.7 Flash (High)".into(),
                is_default: false,
            },
            ModelChoice {
                id: "gemini-3.7-flash[medium]".into(),
                label: "Gemini 3.7 Flash (Medium)".into(),
                is_default: false,
            },
            ModelChoice {
                id: "gemini-3.7-flash[low]".into(),
                label: "Gemini 3.7 Flash (Low)".into(),
                is_default: false,
            },
            ModelChoice {
                id: "gemini-3.1-pro[high]".into(),
                label: "Gemini 3.1 Pro (High)".into(),
                is_default: false,
            },
            ModelChoice {
                id: "gemini-3.1-pro[low]".into(),
                label: "Gemini 3.1 Pro (Low)".into(),
                is_default: false,
            },
            ModelChoice {
                id: "claude-opus-5-5[high]".into(),
                label: "Claude Opus 5.5 (High)".into(),
                is_default: false,
            },
            ModelChoice {
                id: "claude-opus-5-5[medium]".into(),
                label: "Claude Opus 5.5 (Medium)".into(),
                is_default: false,
            },
            ModelChoice {
                id: "claude-opus-5-5[low]".into(),
                label: "Claude Opus 5.5 (Low)".into(),
                is_default: false,
            },
            ModelChoice {
                id: "claude-sonnet-5-5[high]".into(),
                label: "Claude Sonnet 5.5 (High)".into(),
                is_default: false,
            },
            ModelChoice {
                id: "claude-sonnet-5-5[medium]".into(),
                label: "Claude Sonnet 5.5 (Medium)".into(),
                is_default: false,
            },
            ModelChoice {
                id: "claude-sonnet-5-5[low]".into(),
                label: "Claude Sonnet 5.5 (Low)".into(),
                is_default: false,
            },
            ModelChoice {
                id: "gpt-oss-120b[medium]".into(),
                label: "GPT-OSS 120B (Medium)".into(),
                is_default: false,
            },
        ]
    })
}

pub(crate) fn antigravity_logged_in() -> bool {
    if let Some(h) = home_dir() {
        if h.join(".gemini/antigravity").exists() {
            return true;
        }
    }
    cli_on_path("agy")
}

/// Shared shape for one provider's check: CLI on `PATH`, then whether the
/// caller could find credentials at all (never their contents).
fn check(
    id: &str,
    label: &str,
    cli: &str,
    login_hint: &str,
    install_hint: &str,
    logged_in: bool,
    models: (Vec<ModelChoice>, ModelSource),
) -> AgentInfo {
    let (models, model_source) = models;
    let bridge = chorus_acp::codex::bridge_for(id);
    if !cli_on_path(cli) {
        return AgentInfo {
            id: id.to_string(),
            label: label.to_string(),
            bridge,
            models,
            model_source,
            auth: AuthState::MissingCli,
            hint: install_hint.to_string(),
            detail: format!("`{cli}` is not on PATH."),
        };
    }
    if !logged_in {
        return AgentInfo {
            id: id.to_string(),
            label: label.to_string(),
            bridge,
            models,
            model_source,
            auth: AuthState::NeedsLogin,
            hint: login_hint.to_string(),
            detail: format!("`{cli}` is installed, but not logged in yet."),
        };
    }
    AgentInfo {
        id: id.to_string(),
        label: label.to_string(),
        bridge,
        models,
        model_source,
        auth: AuthState::Ok,
        hint: login_hint.to_string(),
        detail: format!("`{cli}` is installed and logged in."),
    }
}

/// Does a generic-password item with this service name exist in the login
/// Keychain?
///
/// Deliberately **no `-w`**: that flag prints the secret and, on a fresh
/// login, pops a Keychain access prompt. Without it `security` only looks
/// the item up, and both pipes are sent to `/dev/null` so not one byte of
/// credential material enters this process — we read the exit status and
/// nothing else (44 = not found).
#[cfg(target_os = "macos")]
fn keychain_has(service: &str) -> bool {
    use std::process::Stdio;
    Command::new("security")
        .args(["find-generic-password", "-s", service])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(target_os = "macos"))]
fn keychain_has(_service: &str) -> bool {
    false
}

/// Claude keeps its tokens in one of two places depending on the platform,
/// and on macOS it actively removes the other one: after `claude` logs in
/// there, `~/.claude/.credentials.json` is deleted and the tokens live in
/// the login Keychain under the service `Claude Code-credentials`. Checking
/// only the file therefore reports `needs_login` forever on exactly the
/// machine this app is being built for. Check the file first anyway — Linux,
/// and Macs where a token was placed by hand, still use it.
fn claude_logged_in(cred_paths: &[PathBuf]) -> bool {
    match claude_auth_status(None) {
        Some(v) => v,
        None => cred_paths.iter().any(|p| p.exists()) || keychain_has("Claude Code-credentials"),
    }
}

/// Ask the CLI itself: `claude auth status --json` → `loggedIn`. Fast (no
/// network) and the only check that knows about a *revoked* login — the
/// keychain item survives revocation, so the old check said "ready" while
/// every turn spent ~3 minutes retrying a 401 before failing. Also the only
/// check that works for an extra account (`CLAUDE_CONFIG_DIR`), whose
/// credentials macOS keeps under a keychain name we do not know.
///
/// Reads one boolean; the rest of the output (it includes the account email)
/// is dropped unprinted. `None` when the CLI is missing or too old to have
/// the subcommand, so callers fall back to the file/keychain check.
pub fn claude_auth_status(config_dir: Option<&std::path::Path>) -> Option<bool> {
    use std::io::Read;
    use std::process::Stdio;
    let mut cmd = Command::new("claude");
    cmd.args(["auth", "status", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for var in ["CLAUDECODE", "CLAUDE_CODE_SSE_PORT", "CLAUDE_CODE_ENTRYPOINT"] {
        cmd.env_remove(var);
    }
    if let Some(d) = config_dir {
        cmd.env("CLAUDE_CONFIG_DIR", d);
    }
    let mut child = cmd.spawn().ok()?;
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    let _ = child.wait();
    let v: serde_json::Value = serde_json::from_str(out.trim()).ok()?;
    v.get("loggedIn").and_then(serde_json::Value::as_bool)
}

/// Detect one agent's current runnability. Cheap and non-blocking: a
/// `which` shell-out and a file-existence check, nothing more.
pub fn auth_status(agent_id: &str) -> AgentInfo {
    let aid = agent_id.to_lowercase();
    if aid.contains("claude") {
        let dir = std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|h| h.join(".claude")));
        let cred = dir
            .map(|d| vec![d.join(".credentials.json")])
            .unwrap_or_default();
        check(
            "claude",
            "Claude",
            "claude",
            "claude login",
            "npm install -g @anthropic-ai/claude-code",
            claude_logged_in(&cred) && !crate::limits::auth_failed("claude"),
            models_for("claude", "CHORUS_CLAUDE_MODELS", claude_models),
        )
    } else if aid.contains("antigravity") || aid.contains("agy") {
        check(
            "antigravity",
            "Antigravity",
            "agy",
            "agy login",
            "brew install antigravity หรือติดตั้ง agy CLI",
            antigravity_logged_in() && !crate::limits::auth_failed("antigravity"),
            models_for("antigravity", "CHORUS_ANTIGRAVITY_MODELS", antigravity_models),
        )
    } else {
        let dir = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|h| h.join(".codex")));
        let cred = dir.map(|d| vec![d.join("auth.json")]).unwrap_or_default();
        check(
            "codex",
            "Codex",
            "codex",
            "codex login",
            "npm install -g @openai/codex",
            cred.iter().any(|p| p.exists()) && !crate::limits::auth_failed("codex"),
            models_for("codex", "CHORUS_CODEX_MODELS", codex_models),
        )
    }
}

/// Every agent Chorus knows how to bridge to, with its current runnability.
/// Always returns all entries — this is a status report, not a filter; an
/// agent that can't run yet still needs to show up so the user knows what
/// to fix.
pub fn list_agents() -> Vec<AgentInfo> {
    vec![
        auth_status("claude"),
        auth_status("codex"),
        auth_status("antigravity"),
    ]
}
