//! `chorus_request_secret`: an agent needs a credential it was not given and
//! asks the person for it.
//!
//! The value travels shell → Rust → Keychain ([`crate::secrets::Vault`])
//! and nowhere else. It is never in the tool arguments, never in the tool
//! result, never in the ledger: the agent is told a *name*, and the value
//! reaches its shell as an environment variable from its next turn on
//! (a running bridge's environment is fixed when it starts — measured in
//! `Session::run_turn`, which builds `env` before `CodexAgent::start`).
//!
//! Not MCP elicitation: the spec forbids requesting secrets through form
//! mode (modelcontextprotocol.io/specification/2025-11-25/client/elicitation
//! — "servers MUST NOT use form mode to request sensitive information").
//! This is Chorus's own sheet, drawn by the shell, same as `chorus_ask_user`.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value};

use crate::agent_control::db_key;
use crate::secrets::{self, SCOPE_ALL};

/// What the person is shown. No value, obviously.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Request {
    pub agent: String,
    /// Variable name the agent asked for, e.g. `STRIPE_API_KEY`.
    pub name: String,
    pub reason: String,
}

/// How long the person wants the value kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remember {
    /// This run of Chorus only; never written to the Keychain.
    Once,
    /// Keychain, for turns in this project.
    Project,
    /// Keychain, for every project.
    All,
}

/// The person's answer. `Debug` hides the value so a stray `{:?}` or panic
/// message cannot leak it.
pub struct Answer {
    pub value: String,
    pub remember: Remember,
}

impl fmt::Debug for Answer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Answer")
            .field("value", &"<hidden>")
            .field("remember", &self.remember)
            .finish()
    }
}

/// Blocks until the person answers; `None` = cancelled. Called off the UI thread.
pub type Asker = Arc<dyn Fn(&Request) -> Option<Answer> + Send + Sync>;

#[derive(Clone)]
struct Registered {
    asker: Asker,
    /// [`secrets::project_scope`] of the session's folder.
    project: String,
}

static ASKERS: Mutex<Option<HashMap<(String, i64), Registered>>> = Mutex::new(None);

const MAX_REASON: usize = 400;
const MAX_NAME: usize = 64;
/// A value longer than this is a mistake (a pasted file), not a credential.
pub const MAX_VALUE: usize = 8_192;

/// Who answers secret requests for `sess`, and the project scope their
/// "keep for this project" choice applies to; `None` removes them.
pub fn set_asker(conn: &Connection, sess: i64, project: &str, asker: Option<Asker>) {
    let mut guard = ASKERS.lock().unwrap_or_else(|p| p.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    let key = (db_key(conn), sess);
    match asker {
        Some(asker) => {
            map.insert(
                key,
                Registered {
                    asker,
                    project: project.to_string(),
                },
            );
        }
        None => {
            map.remove(&key);
        }
    }
}

/// Listed only when someone can answer.
pub fn available(conn: &Connection, sess: i64) -> bool {
    registered(conn, sess).is_some()
}

fn registered(conn: &Connection, sess: i64) -> Option<Registered> {
    let guard = ASKERS.lock().unwrap_or_else(|p| p.into_inner());
    guard.as_ref()?.get(&(db_key(conn), sess)).cloned()
}

pub fn tool_schema() -> Value {
    json!({
        "name": "chorus_request_secret",
        "description": "Ask the user for a credential you need (an API key, a token, a password) \
and have Chorus keep it safely. The user types it into a private field in the app; the value \
never passes through this conversation and you never see it. You get back only a variable \
name. It is set in your shell environment from your NEXT turn on (this running shell does not \
have it): finish what does not need it, tell the user plainly what is waiting, and stop. \
Use it instead of asking the user to paste a key into chat. Never print, echo, log or write \
the value anywhere: anything you echo goes to your provider.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "pattern": "^[A-Z][A-Z0-9_]{1,63}$",
                    "description": "Environment variable name to use, e.g. STRIPE_API_KEY"
                },
                "reason": {
                    "type": "string",
                    "maxLength": MAX_REASON,
                    "description": "What you need it for, shown to the user"
                }
            },
            "required": ["name", "reason"],
            "additionalProperties": false
        }
    })
}

/// Validate the arguments into what the person will see.
pub fn parse(args: &Value) -> Result<Request, String> {
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .ok_or("name is required")?;
    let valid = name.len() <= MAX_NAME
        && name.len() >= 2
        && name.starts_with(|c: char| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if !valid {
        return Err("name must look like STRIPE_API_KEY (A-Z, 0-9, _; 2-64 characters)".into());
    }
    if secrets::reserved_env(name) {
        return Err(format!(
            "{name} is used by Chorus or your own CLI for sign-in and cannot be set this way; \
             pick a different name"
        ));
    }
    let reason = args
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .ok_or("reason is required: say what you need it for")?;
    if reason.chars().count() > MAX_REASON {
        return Err(format!("reason must be at most {MAX_REASON} characters"));
    }
    Ok(Request {
        agent: String::new(),
        name: name.to_string(),
        reason: reason.to_string(),
    })
}

/// Handle one `tools/call`. The text returned names the variable, never the value.
pub fn call(conn: &Connection, sess: i64, agent: &str, args: &Value) -> Result<Value, String> {
    let mut req = parse(args)?;
    req.agent = agent.to_string();
    let reg = registered(conn, sess).ok_or(
        "no one is available to enter it right now; say what you need and stop",
    )?;
    let text = match (reg.asker)(&req) {
        None => "The user declined to provide it. Do not ask again this turn; continue without it \
                 or say what is blocked."
            .to_string(),
        Some(ans) => {
            if ans.value.is_empty() || ans.value.chars().count() > MAX_VALUE {
                "The user closed the sheet without a usable value. Do not ask again this turn."
                    .to_string()
            } else {
                let scope = match ans.remember {
                    Remember::Once => None,
                    Remember::Project => Some(reg.project.as_str()),
                    Remember::All => Some(SCOPE_ALL),
                };
                let (placeholder, stored) =
                    secrets::user_vault().put_requested(&req.name, &ans.value, scope);
                let kept = match ans.remember {
                    Remember::Once => "for this run of Chorus only",
                    Remember::Project => "for this project",
                    Remember::All => "for all projects",
                };
                let warn = if stored {
                    ""
                } else {
                    " The Keychain refused it, so it only lasts until Chorus quits."
                };
                format!(
                    "The user provided {name} ({kept}).{warn} It is stored; you do not have the \
                     value. From your next turn it is set in your shell as ${name} (also \
                     ${placeholder}). This running shell does not have it yet: finish what does \
                     not need it, tell the user what is waiting, and stop. Never print or echo it.",
                    name = req.name
                )
            }
        }
    };
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    fn answer(value: &str, remember: Remember) -> Asker {
        let v = value.to_string();
        Arc::new(move |_| {
            Some(Answer {
                value: v.clone(),
                remember,
            })
        })
    }

    #[test]
    fn names_are_checked_and_reserved_ones_refused() {
        let ok = parse(&json!({"name":"STRIPE_API_KEY","reason":" billing sync "})).unwrap();
        assert_eq!(ok.reason, "billing sync");
        for bad in ["stripe", "1KEY", "A", "A-B", "KEY WITH SPACE", "ANTHROPIC_API_KEY", "PATH", "CHORUS_X"] {
            assert!(parse(&json!({"name":bad,"reason":"r"})).is_err(), "{bad}");
        }
        assert!(parse(&json!({"name":"STRIPE_API_KEY"})).is_err(), "reason is required");
        assert!(parse(&json!({"name":"STRIPE_API_KEY","reason":"x".repeat(401)})).is_err());
    }

    #[test]
    fn listed_only_when_someone_can_answer() {
        let conn = mem();
        assert!(!available(&conn, 31));
        let e = call(&conn, 31, "claude", &json!({"name":"MY_TOKEN","reason":"r"})).unwrap_err();
        assert!(e.contains("stop"));
        set_asker(&conn, 31, "project:/x", Some(answer("v", Remember::Once)));
        assert!(available(&conn, 31));
        assert!(!available(&conn, 32));
        set_asker(&conn, 31, "project:/x", None);
        assert!(!available(&conn, 31));
    }

    #[test]
    fn the_value_never_comes_back_to_the_agent() {
        std::env::set_var("CHORUS_SECRETS_BACKEND", "memory");
        let conn = mem();
        let value = "correct-horse-battery-staple-7731";
        set_asker(&conn, 41, "project:/x", Some(answer(value, Remember::All)));
        let r = call(&conn, 41, "codex", &json!({"name":"REQ_SECRET_A","reason":"deploy"})).unwrap();
        let text = r["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("REQ_SECRET_A"));
        assert!(!text.contains(value), "{text}");
        assert!(!format!("{r}").contains(value));
        // …but the next turn's environment has it, under both names.
        let env = secrets::user_vault().requested("project:/other");
        assert!(env.names.contains(&"REQ_SECRET_A".to_string()));
        assert!(env.env.contains(&("REQ_SECRET_A".into(), value.into())));
        assert!(env
            .env
            .contains(&("CHORUS_SECRET_REQ_SECRET_A".into(), value.into())));
        // And a later message that repeats it is masked before the ledger.
        let masked = secrets::user_vault().mask(&format!("export X={value}"));
        assert!(!masked.text.contains(value), "{}", masked.text);
        let _ = secrets::user_vault().forget("CHORUS_SECRET_REQ_SECRET_A");
    }

    #[test]
    fn scope_decides_which_turns_get_it() {
        std::env::set_var("CHORUS_SECRETS_BACKEND", "memory");
        let conn = mem();
        set_asker(&conn, 51, "project:/mine", Some(answer("proj-only-value-1", Remember::Project)));
        call(&conn, 51, "claude", &json!({"name":"REQ_SECRET_B","reason":"r"})).unwrap();
        let v = secrets::user_vault();
        // The vault is process-wide and tests run in parallel: look only at our own name.
        assert!(v.requested("project:/mine").names.contains(&"REQ_SECRET_B".to_string()));
        assert!(!v.requested("project:/elsewhere").names.contains(&"REQ_SECRET_B".to_string()));

        // "once" lives in memory: no index entry, so nothing for Settings to list.
        set_asker(&conn, 51, "project:/mine", Some(answer("once-value-12345", Remember::Once)));
        call(&conn, 51, "claude", &json!({"name":"REQ_SECRET_C","reason":"r"})).unwrap();
        assert!(v.list().iter().all(|e| e.name != "CHORUS_SECRET_REQ_SECRET_C"));
        assert!(v.requested("project:/anywhere").names.contains(&"REQ_SECRET_C".to_string()));
        // An old kept value does not outlive a newer "once" for the same name.
        set_asker(&conn, 51, "project:/mine", Some(answer("newer-once-value-9", Remember::Once)));
        call(&conn, 51, "claude", &json!({"name":"REQ_SECRET_B","reason":"r"})).unwrap();
        let env = v.requested("project:/mine");
        assert!(env.env.contains(&("REQ_SECRET_B".into(), "newer-once-value-9".into())));
        assert!(!env.env.iter().any(|(_, val)| val == "proj-only-value-1"));
        for n in ["B", "C"] {
            let _ = v.forget(&format!("CHORUS_SECRET_REQ_SECRET_{n}"));
        }
    }

    #[test]
    fn declining_is_not_an_error_and_stores_nothing() {
        let conn = mem();
        set_asker(&conn, 61, "project:/x", Some(Arc::new(|_| None)));
        let r = call(&conn, 61, "claude", &json!({"name":"REQ_SECRET_D","reason":"r"})).unwrap();
        assert!(r["content"][0]["text"].as_str().unwrap().contains("declined"));
        assert!(!secrets::user_vault()
            .requested("project:/x")
            .names
            .contains(&"REQ_SECRET_D".to_string()));
    }

    #[test]
    fn answer_debug_hides_the_value() {
        let a = Answer {
            value: "hunter2-hunter2".into(),
            remember: Remember::Once,
        };
        assert!(!format!("{a:?}").contains("hunter2"));
    }
}
