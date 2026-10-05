//! One MCP config for every agent.
//!
//! The third of the three duplications this app exists to remove. Memory was
//! `CLAUDE.md` / `AGENTS.md` / `GEMINI.md`; skills were `.claude/skills/` and
//! nothing at all for Codex; tools are `.mcp.json`, `~/.codex/config.toml`,
//! and a different file per CLI — the same servers declared three times, and
//! three places to forget when one changes.
//!
//! ACP removes it for us if we let it: `session/new` takes an `mcpServers`
//! array, and whatever is in it is what that agent can reach for this session,
//! regardless of the agent's own config. So Chorus keeps one file,
//! `.chorus/mcp.json`, and hands the same list to every agent it starts.
//!
//! Shape verified against agentclientprotocol.com/protocol/session-setup
//! (12 Sep 2026): stdio entries are `{name, command, args, env:[{name,value}]}`
//! and HTTP entries are `{type:"http", name, url, headers:[{name,value}]}` —
//! note `env` and `headers` are **arrays of pairs**, not objects, which is
//! where the familiar Claude-Desktop-shaped config has to be converted.
//!
//! ## Secrets
//!
//! These files hold API keys. This module reads them, converts them, and hands
//! them to the agent — and that is all. Nothing here prints, logs or returns a
//! value from `env` or `headers`; [`Redacted`] exists so the CLI can show a
//! user what is configured without a token ever reaching a terminal, a log, or
//! a bug report.

use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// The one file, relative to the workspace root.
pub const CONFIG: &str = ".chorus/mcp.json";

/// Claude Code's project-scope MCP file — the one worth importing, because
/// it is the format people already have and the one its docs tell them to
/// commit.
pub const CLAUDE_PROJECT_CONFIG: &str = ".mcp.json";

/// A server, ready to go into `session/new`, plus a name to show a human.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub name: String,
    /// The ACP entry, already in protocol shape.
    pub entry: Value,
}

/// What a server looks like when it is safe to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redacted {
    pub name: String,
    /// "stdio" | "http" | "sse"
    pub transport: String,
    /// The command line or the URL — never a credential.
    pub target: String,
    /// The *names* of the env vars or headers it carries. Never the values:
    /// an env var called `GITHUB_TOKEN` tells a user everything they need to
    /// debug a config, and its value tells an attacker everything they need.
    pub secret_keys: Vec<String>,
}

/// Result of adding user-scope catalog MCP servers to project-authored config.
/// A collision is returned explicitly; the user's entry always wins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composition {
    pub servers: Vec<Server>,
    pub collisions: Vec<String>,
}

/// Read `<cwd>/.chorus/mcp.json`.
///
/// Accepts both shapes people actually have: the Claude-Desktop object map
/// (`{"mcpServers": {"github": {...}}}`) and a bare ACP array. A missing file
/// is not an error — most workspaces have no MCP servers, and that must not
/// stop a turn.
pub fn load(cwd: &Path) -> Vec<Server> {
    read(&cwd.join(CONFIG))
}

/// Compose enabled catalog servers for this session without writing either
/// `.chorus/mcp.json` or the catalog state. Project-authored entries take
/// precedence, and collisions are surfaced to the caller.
pub fn compose(cwd: &Path, catalog: impl IntoIterator<Item = Server>) -> Composition {
    let mut servers = load(cwd);
    let mut names = configured_names(&cwd.join(CONFIG));
    names.extend(servers.iter().map(|s| s.name.clone()));
    let mut collisions = Vec::new();
    for server in catalog {
        if names.contains(&server.name) {
            collisions.push(server.name);
        } else {
            names.insert(server.name.clone());
            servers.push(server);
        }
    }
    servers.sort_by(|a,b| a.name.cmp(&b.name));
    collisions.sort();
    Composition { servers, collisions }
}

fn configured_names(path: &Path) -> std::collections::BTreeSet<String> {
    let Ok(text) = std::fs::read_to_string(path) else { return Default::default(); };
    let Ok(value) = serde_json::from_str::<Value>(&text) else { return Default::default(); };
    if let Some(map) = value.get("mcpServers").and_then(Value::as_object) {
        return map.keys().cloned().collect();
    }
    value.as_array().into_iter().flatten()
        .filter_map(|item| item.get("name").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

/// Convenience adapter for catalog entries already expressed as ACP JSON.
pub fn compose_catalog(cwd: &Path, catalog_entries: &[Value]) -> Composition {
    let entries = from_value(&Value::Array(catalog_entries.to_vec()));
    compose(cwd, entries)
}

fn read(path: &Path) -> Vec<Server> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    from_value(&v)
}

/// Convert either accepted shape into ACP entries, sorted by name so the
/// list handed to an agent is stable between turns.
pub fn from_value(v: &Value) -> Vec<Server> {
    let mut out = Vec::new();

    if let Some(map) = v.get("mcpServers").and_then(Value::as_object) {
        for (name, cfg) in map {
            if let Some(s) = convert(name, cfg) {
                out.push(s);
            }
        }
    } else if let Some(arr) = v.as_array() {
        for cfg in arr {
            let name = cfg.get("name").and_then(Value::as_str).unwrap_or_default();
            if !name.is_empty() {
                out.push(Server { name: name.to_string(), entry: cfg.clone() });
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// One entry of a Claude-Desktop-shaped `mcpServers` map → one ACP entry.
///
/// `disabled: true` is honoured: someone who turned a server off in their
/// config did not mean "turn it off for Claude but leave it on for Codex".
fn convert(name: &str, cfg: &Value) -> Option<Server> {
    if cfg.get("disabled").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let entry = if let Some(url) = cfg.get("url").and_then(Value::as_str) {
        let kind = match cfg.get("type").and_then(Value::as_str) {
            Some("sse") => "sse",
            _ => "http",
        };
        json!({
            "type": kind,
            "name": name,
            "url": url,
            "headers": pairs(cfg.get("headers")),
        })
    } else {
        let command = cfg.get("command").and_then(Value::as_str)?;
        json!({
            "name": name,
            "command": command,
            "args": cfg.get("args").cloned().unwrap_or_else(|| json!([])),
            "env": pairs(cfg.get("env")),
        })
    };
    Some(Server { name: name.to_string(), entry })
}

/// `{"K": "v"}` → `[{"name":"K","value":"v"}]`, which is the shape ACP wants
/// and the shape no existing config file is written in.
fn pairs(v: Option<&Value>) -> Value {
    match v {
        Some(Value::Object(m)) => Value::Array(
            m.iter()
                .map(|(k, val)| {
                    json!({ "name": k, "value": val.as_str().unwrap_or_default() })
                })
                .collect(),
        ),
        Some(a @ Value::Array(_)) => a.clone(),
        _ => json!([]),
    }
}

/// Describe servers without exposing a single secret value.
pub fn redact(servers: &[Server]) -> Vec<Redacted> {
    servers
        .iter()
        .map(|s| {
            let e = &s.entry;
            let transport = e.get("type").and_then(Value::as_str).unwrap_or("stdio");
            let target = if let Some(u) = e.get("url").and_then(Value::as_str) {
                u.to_string()
            } else {
                let args: Vec<String> = e
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                let cmd = e.get("command").and_then(Value::as_str).unwrap_or("");
                format!("{cmd} {}", args.join(" ")).trim().to_string()
            };
            let key = if e.get("url").is_some() { "headers" } else { "env" };
            let secret_keys = e
                .get(key)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|p| p.get("name").and_then(Value::as_str).map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            Redacted {
                name: s.name.clone(),
                transport: transport.to_string(),
                target,
                secret_keys,
            }
        })
        .collect()
}

/// Merge servers found in `.mcp.json` into `.chorus/mcp.json`, returning the
/// names that were added.
///
/// Existing entries win. Import is meant to be safe to run twice, and a second
/// run must not quietly revert an edit someone made in the Chorus file after
/// the first — that is the failure that teaches people not to trust an import.
pub fn import_claude_project(cwd: &Path) -> std::io::Result<Vec<String>> {
    let src_text = match std::fs::read_to_string(cwd.join(CLAUDE_PROJECT_CONFIG)) {
        Ok(t) => t,
        Err(_) => return Ok(Vec::new()),
    };
    let Ok(source) = serde_json::from_str::<Value>(&src_text) else {
        return Ok(Vec::new());
    };
    let Some(source_map) = source.get("mcpServers").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };

    let dest = cwd.join(CONFIG);
    let mut existing: Map<String, Value> = std::fs::read_to_string(&dest)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("mcpServers").and_then(Value::as_object).cloned())
        .unwrap_or_default();

    // Copy the user's own wording, entry for entry, including ones they have
    // disabled — `load` is what decides whether a server is handed to an
    // agent, and importing is not the place to drop something someone may
    // simply have paused. A round trip through ACP shape here would rewrite
    // every entry into something they never typed, in a file they own.
    let mut added = Vec::new();
    for (name, cfg) in source_map {
        if existing.contains_key(name) {
            continue;
        }
        existing.insert(name.clone(), cfg.clone());
        added.push(name.clone());
    }
    if added.is_empty() {
        return Ok(added);
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let out = json!({ "mcpServers": Value::Object(existing) });
    std::fs::write(&dest, format!("{}\n", serde_json::to_string_pretty(&out)?))?;
    added.sort();
    Ok(added)
}

/// Where the config would live, for messages.
pub fn config_path(cwd: &Path) -> PathBuf {
    cwd.join(CONFIG)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE_SHAPE: &str = r#"{
      "mcpServers": {
        "github":  { "command": "npx", "args": ["-y","gh-mcp"], "env": { "GITHUB_TOKEN": "ghp_secret" } },
        "api":     { "url": "https://api.example.com/mcp", "headers": { "Authorization": "Bearer tok" } },
        "turned-off": { "command": "nope", "disabled": true }
      }
    }"#;

    #[test]
    fn a_claude_shaped_config_becomes_acp_entries() {
        let v: Value = serde_json::from_str(CLAUDE_SHAPE).unwrap();
        let s = from_value(&v);
        assert_eq!(s.len(), 2, "a disabled server is not handed to anyone");
        assert_eq!(s[0].name, "api");
        assert_eq!(s[0].entry["type"], "http");
        assert_eq!(s[0].entry["headers"][0]["name"], "Authorization");

        assert_eq!(s[1].name, "github");
        assert_eq!(s[1].entry["command"], "npx");
        assert_eq!(
            s[1].entry["env"],
            json!([{ "name": "GITHUB_TOKEN", "value": "ghp_secret" }]),
            "env must be an array of pairs — no config file on disk is written this way"
        );
    }

    #[test]
    fn redaction_keeps_the_key_names_and_drops_every_value() {
        let v: Value = serde_json::from_str(CLAUDE_SHAPE).unwrap();
        let r = redact(&from_value(&v));
        let printed = format!("{r:?}");

        assert!(printed.contains("GITHUB_TOKEN"), "the name is what makes a config debuggable");
        assert!(printed.contains("Authorization"));
        assert!(!printed.contains("ghp_secret"), "a token reached a printable string: {printed}");
        assert!(!printed.contains("Bearer tok"), "a token reached a printable string: {printed}");
        assert_eq!(r[1].target, "npx -y gh-mcp");
    }

    #[test]
    fn import_is_safe_to_run_twice_and_never_overwrites_an_edit() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join(CLAUDE_PROJECT_CONFIG), CLAUDE_SHAPE).unwrap();

        let first = import_claude_project(t.path()).unwrap();
        assert_eq!(first, vec!["api", "github", "turned-off"], "import copies verbatim, disabled included");

        // The user edits one entry afterwards.
        let dest = t.path().join(CONFIG);
        let mut cfg: Value = serde_json::from_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
        cfg["mcpServers"]["github"]["args"] = json!(["-y", "gh-mcp", "--verbose"]);
        std::fs::write(&dest, cfg.to_string()).unwrap();

        let second = import_claude_project(t.path()).unwrap();
        assert!(second.is_empty(), "a second import must add nothing");
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
        assert_eq!(
            after["mcpServers"]["github"]["args"],
            json!(["-y", "gh-mcp", "--verbose"]),
            "the edit survived"
        );
    }

    #[test]
    fn no_config_is_not_an_error() {
        let t = tempfile::tempdir().unwrap();
        assert!(load(t.path()).is_empty());
        assert!(import_claude_project(t.path()).unwrap().is_empty());
    }
}
