//! `chorus_ask_user`: an agent asks the person a question mid-turn.
//!
//! ACP has no way for an agent to put a question to the user — only
//! `session/request_permission`, which is yes/no about a tool call. So the
//! question goes through Chorus's own MCP server, which every agent already
//! has: one tool, the same for Claude, Codex and Antigravity. The shell
//! registers who answers ([`set_asker`]); the call blocks until they do.
//!
//! The question and the answer reach the ledger as the tool call and its
//! result, like any other tool — the next agent sees what was decided.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value};

use crate::agent_control::db_key;

/// What the person is shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Question {
    /// Who is asking (may carry `@account`); empty when unknown.
    pub agent: String,
    pub question: String,
    /// Buttons, in order. Empty = free text only.
    pub options: Vec<String>,
    /// Whether a typed answer is accepted besides the options.
    pub allow_other: bool,
}

/// Blocks until the person answers; `None` = dismissed without an answer.
/// Called off the UI thread.
pub type Asker = Arc<dyn Fn(&Question) -> Option<String> + Send + Sync>;

static ASKERS: Mutex<Option<HashMap<(String, i64), Asker>>> = Mutex::new(None);

const MAX_QUESTION: usize = 2_000;
const MAX_OPTIONS: usize = 8;
const MAX_OPTION: usize = 200;
/// Longer answers are cut, not refused: the person already typed them.
pub const MAX_ANSWER: usize = 4_000;

/// Who answers questions for `sess`; `None` removes them.
pub fn set_asker(conn: &Connection, sess: i64, asker: Option<Asker>) {
    let mut guard = ASKERS.lock().unwrap_or_else(|p| p.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    let key = (db_key(conn), sess);
    match asker {
        Some(a) => {
            map.insert(key, a);
        }
        None => {
            map.remove(&key);
        }
    }
}

/// Listed only when someone can answer: an agent that never sees the tool
/// never waits on a question nobody will read.
pub fn available(conn: &Connection, sess: i64) -> bool {
    asker(conn, sess).is_some()
}

fn asker(conn: &Connection, sess: i64) -> Option<Asker> {
    let guard = ASKERS.lock().unwrap_or_else(|p| p.into_inner());
    guard.as_ref()?.get(&(db_key(conn), sess)).cloned()
}

pub fn tool_schema() -> Value {
    json!({
        "name": "chorus_ask_user",
        "description": "Ask the user a question and wait for the answer. Use it when you need a \
decision only the user can make (which approach, which file, missing requirements) instead of \
guessing. Give `options` for a quick choice; the user can also type their own answer unless \
`allow_other` is false. Ask one question at a time and keep it short.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "question": { "type": "string", "description": "The question, in the user's language" },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "maxItems": MAX_OPTIONS,
                    "description": "Choices shown as buttons, most likely first"
                },
                "allow_other": {
                    "type": "boolean",
                    "default": true,
                    "description": "Accept a typed answer besides the options"
                }
            },
            "required": ["question"],
            "additionalProperties": false
        }
    })
}

/// Validate the arguments into what the person will see.
pub fn parse(args: &Value) -> Result<Question, String> {
    let question = args
        .get("question")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .ok_or("question is required")?;
    if question.chars().count() > MAX_QUESTION {
        return Err(format!("question must be at most {MAX_QUESTION} characters"));
    }
    let mut options = Vec::new();
    if let Some(v) = args.get("options").filter(|v| !v.is_null()) {
        let arr = v.as_array().ok_or("options must be an array of strings")?;
        if arr.len() > MAX_OPTIONS {
            return Err(format!("at most {MAX_OPTIONS} options"));
        }
        for o in arr {
            let o = o
                .as_str()
                .map(str::trim)
                .filter(|o| !o.is_empty())
                .ok_or("options must be non-empty strings")?;
            if o.chars().count() > MAX_OPTION {
                return Err(format!("each option must be at most {MAX_OPTION} characters"));
            }
            if !options.iter().any(|x: &String| x == o) {
                options.push(o.to_string());
            }
        }
    }
    let allow_other = args
        .get("allow_other")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if options.is_empty() && !allow_other {
        return Err("allow_other=false needs at least one option".into());
    }
    Ok(Question {
        agent: String::new(),
        question: question.to_string(),
        options,
        allow_other,
    })
}

/// Handle one `tools/call`. Text coming back from the person is masked like
/// any message: it is about to enter the ledger and a provider's prompt.
pub fn call(conn: &Connection, sess: i64, agent: &str, args: &Value) -> Result<Value, String> {
    let mut q = parse(args)?;
    q.agent = agent.to_string();
    let ask = asker(conn, sess).ok_or("no one is available to answer right now; decide yourself and say what you assumed")?;
    let text = match ask(&q) {
        Some(a) => {
            let a: String = a.trim().chars().take(MAX_ANSWER).collect();
            if a.is_empty() {
                "The user closed the question without answering. Do not ask again; \
                 decide yourself and say what you assumed."
                    .to_string()
            } else {
                let masked = crate::secrets::user_vault().mask(&a).text;
                format!("User answered: {masked}")
            }
        }
        None => "The user closed the question without answering. Do not ask again; \
                 decide yourself and say what you assumed."
            .to_string(),
    };
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn parse_keeps_options_in_order_without_duplicates() {
        let q = parse(&json!({"question":" which? ","options":["a","b","a"," "]}));
        assert!(q.is_err(), "blank option is refused");
        let q = parse(&json!({"question":" which? ","options":["a","b","a"]})).unwrap();
        assert_eq!(q.question, "which?");
        assert_eq!(q.options, vec!["a", "b"]);
        assert!(q.allow_other);
    }

    #[test]
    fn parse_refuses_a_question_nobody_can_answer() {
        assert!(parse(&json!({})).is_err());
        assert!(parse(&json!({"question":"x","allow_other":false})).is_err());
        let many: Vec<String> = (0..9).map(|i| i.to_string()).collect();
        assert!(parse(&json!({"question":"x","options":many})).is_err());
    }

    #[test]
    fn call_without_an_asker_tells_the_agent_to_decide() {
        let conn = mem();
        assert!(!available(&conn, 1));
        let e = call(&conn, 1, "claude", &json!({"question":"x"})).unwrap_err();
        assert!(e.contains("decide yourself"));
    }

    #[test]
    fn call_returns_the_answer_and_dismissal_is_not_an_error() {
        std::env::set_var("CHORUS_SECRETS_BACKEND", "memory");
        let conn = mem();
        let seen = Arc::new(Mutex::new(None));
        let s = seen.clone();
        set_asker(
            &conn,
            7,
            Some(Arc::new(move |q: &Question| {
                *s.lock().unwrap() = Some(q.clone());
                Some("  ใช้ตัวที่สอง ".into())
            })),
        );
        assert!(available(&conn, 7));
        assert!(!available(&conn, 8), "scoped to the session");
        let r = call(&conn, 7, "claude", &json!({"question":"เลือกอันไหน","options":["หนึ่ง","สอง"]})).unwrap();
        assert_eq!(r["content"][0]["text"], "User answered: ใช้ตัวที่สอง");
        let q = seen.lock().unwrap().clone().unwrap();
        assert_eq!(q.options, vec!["หนึ่ง", "สอง"]);
        assert_eq!(q.agent, "claude");

        set_asker(&conn, 7, Some(Arc::new(|_: &Question| None)));
        let r = call(&conn, 7, "claude", &json!({"question":"x"})).unwrap();
        assert!(r["content"][0]["text"].as_str().unwrap().contains("without answering"));

        set_asker(&conn, 7, None);
        assert!(!available(&conn, 7));
    }
}
