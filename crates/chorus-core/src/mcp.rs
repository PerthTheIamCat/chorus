//! A tiny MCP server Chorus hands to every agent, so the agent can reach
//! back into the app for the few things it must not do directly.
//!
//! Right now it exposes exactly one tool: `memory_propose`. An agent may
//! *propose* a durable fact; a human approves it later (design §4, trap
//! §7.2). The agent never writes `fact` itself.
//!
//! Transport is MCP Streamable HTTP on `127.0.0.1:<port>` — no HTTP-crate
//! dependency, ~200 lines of `std::net`. What Codex's client actually needs
//! (learned the hard way):
//!
//! - HTTP/1.1 persistent connections: `initialize`, `notifications/initialized`,
//!   `tools/list`, `tools/call` all arrive on one socket.
//! - The `GET` request opens an SSE stream for server→client messages. We send
//!   none, but the client *cancels its tool calls* if we close it — so the GET
//!   is held open with keep-alive comments.
//! - `Mcp-Session-Id` on responses; `202` for notifications.
//!
//! Verified end to end: Codex calls `memory_propose`, the proposal lands in
//! the queue. In `read-only` mode the call is still gated through our
//! permission engine first (`--yes` to allow) — that's the design working.
//!
//! ## Authentication
//!
//! Loopback TCP is reachable by every local process, other macOS users
//! included. So every request must carry `Authorization: Bearer <secret>`,
//! a random per-agent secret Chorus mints and hands to the agent inside ACP
//! `mcpServers[].headers` ([`McpServer::mcp_server_entry`]). The credential —
//! never a tool argument — decides which session the caller acts in and which
//! actor its ledger writes are attributed to.

use crate::types::{EventKind, NewEvent};
use crate::{
    actor, agent_control, attachments, computer, ledger, memory, notify, roles, Result, Store,
};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// Who is calling: derived from the bearer credential, never from arguments.
#[derive(Debug, Clone)]
struct Caller {
    /// The session this server was started for. `None` = project-memory server.
    session: Option<i64>,
    /// The agent the credential was minted for; ledger writes use this actor.
    actor: String,
}

#[derive(Debug, Clone)]
struct Credential {
    secret: String,
    caller: Caller,
}

type Credentials = Arc<Mutex<Vec<Credential>>>;

/// A running MCP server. Drop it (or call `stop`) to shut down.
pub struct McpServer {
    url: String,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    scope: Option<i64>,
    creds: Credentials,
}

/// 32 bytes from the OS CSPRNG, hex-encoded.
fn random_secret() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Compare without an early exit, so response time does not reveal how many
/// leading bytes of a guessed secret were right.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn authenticate(creds: &Credentials, presented: Option<&str>) -> Option<Caller> {
    let presented = presented?;
    let list = creds.lock().ok()?;
    let mut found = None;
    // Check every credential: no short-circuit on the first match.
    for c in list.iter() {
        if ct_eq(c.secret.as_bytes(), presented.as_bytes()) {
            found = Some(c.caller.clone());
        }
    }
    found
}

impl McpServer {
    /// Bind to an ephemeral localhost port and start serving. `db_path` is
    /// the same store the session uses; the server opens its own connection.
    pub fn start(db_path: &str) -> Result<Self> {
        Self::start_scoped(db_path, None)
    }

    /// Start an MCP server with an app-supplied attachment session scope.
    /// Scope is never accepted from tool arguments.
    pub fn start_for_session(db_path: &str, session_id: i64) -> Result<Self> {
        Self::start_scoped(db_path, Some(session_id))
    }

    fn start_scoped(db_path: &str, session_scope: Option<i64>) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let url = format!("http://127.0.0.1:{port}/mcp");

        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let db = db_path.to_string();
        let creds: Credentials = Arc::new(Mutex::new(Vec::new()));
        let creds_thread = creds.clone();

        let handle = std::thread::spawn(move || {
            while !stop_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        // One SQLite connection per connection thread — the
                        // GET/SSE stream is long-lived, so a shared handle
                        // would serialise everything behind it.
                        let db = db.clone();
                        let stop = stop_thread.clone();
                        let creds = creds_thread.clone();
                        std::thread::spawn(move || {
                            if let Ok(store) = Store::open(std::path::Path::new(&db)) {
                                let _ = handle_conn(stream, store.conn(), &stop, &creds);
                            }
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            url,
            stop,
            handle: Some(handle),
            scope: session_scope,
            creds,
        })
    }

    /// The bearer secret for `agent` on this server, minted on first use.
    /// The agent's ledger writes through this server are attributed to it.
    pub fn credential(&self, agent: &str) -> Result<String> {
        // Never mint a credential that writes as the human.
        let agent = if agent.trim().is_empty() || agent == "user" {
            "agent"
        } else {
            agent
        };
        let mut list = self
            .creds
            .lock()
            .map_err(|_| crate::ChorusError::Invalid("mcp credentials poisoned".into()))?;
        if let Some(c) = list.iter().find(|c| c.caller.actor == agent) {
            return Ok(c.secret.clone());
        }
        let secret = random_secret()?;
        list.push(Credential {
            secret: secret.clone(),
            caller: Caller {
                session: self.scope,
                actor: agent.to_string(),
            },
        });
        Ok(secret)
    }

    /// The entry to put in `session/new`'s `mcpServers` for `agent` (ACP HTTP
    /// shape: `type` + `name` + `url` + `headers`, headers as `{name, value}`).
    pub fn mcp_server_entry(&self, agent: &str) -> Result<Value> {
        let secret = self.credential(agent)?;
        Ok(json!({
            "type": "http",
            "name": "chorus",
            "url": self.url,
            "headers": [{ "name": "Authorization", "value": format!("Bearer {secret}") }]
        }))
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn debug_log(dir: &str, what: &str) {
    if std::env::var("CHORUS_MCP_LOG").is_err() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(std::env::var("CHORUS_MCP_LOG").unwrap_or_default())
    {
        let _ = writeln!(f, "[{dir}] {what}");
    }
}

/// Nothing this server accepts is large: a `memory_propose` call is a fact of
/// at most 140 characters. Allocating whatever `Content-Length` claims is how
/// a one-line header turns into an out-of-memory abort.
const MAX_BODY: usize = 64 * 1024;

/// A connection that opens and then says nothing must not pin a thread and an
/// SQLite handle forever. Generous on purpose: a client legitimately holds a
/// persistent connection idle across a long model turn, so this bounds the
/// leak without ever being reached in normal use.
const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1800);

fn handle_conn(
    mut stream: TcpStream,
    conn: &rusqlite::Connection,
    stop: &AtomicBool,
    creds: &Credentials,
) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(IDLE_TIMEOUT))?;
    let mut reader = BufReader::new(stream.try_clone()?);

    // HTTP/1.1 persistent connection: the MCP client sends several requests
    // (initialize, notifications/initialized, tools/list, tools/call…) down
    // one socket. Keep reading until it closes.
    loop {
        let mut request_line = String::new();
        if reader.read_line(&mut request_line)? == 0 {
            return Ok(()); // client hung up
        }
        let mut content_length = 0usize;
        let mut has_origin = false;
        let mut bearer: Option<String> = None;
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h)? == 0 {
                return Ok(());
            }
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            let lower = h.to_lowercase();
            if let Some(v) = lower.strip_prefix("content-length:") {
                content_length = v.trim().parse().unwrap_or(0);
            }
            // A local MCP client never sends `Origin`; a browser always does.
            // Refusing it is the DNS-rebinding guard the MCP spec asks for on
            // loopback servers, and it costs no interop with the bridges.
            if lower.starts_with("origin:") {
                has_origin = true;
            }
            if lower.starts_with("authorization:") {
                let v = h["authorization:".len()..].trim();
                if v.len() > 7 && v[..7].eq_ignore_ascii_case("bearer ") {
                    bearer = Some(v[7..].trim().to_string());
                }
            }
        }

        if has_origin {
            write_http(
                &mut stream,
                403,
                &json!({"error": "this endpoint does not serve browser origins"}),
            )?;
            return Ok(());
        }

        let Some(caller) = authenticate(creds, bearer.as_deref()) else {
            write_raw(
                &mut stream,
                "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Bearer\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                b"",
            )?;
            return Ok(());
        };

        if request_line.starts_with("GET") {
            // (the Origin check above already rejected browser-initiated GETs)
            // The SSE stream. We never send server-initiated messages, but
            // the client keeps this request in flight and cancels its tool
            // calls if we close it — so hold it open with keep-alives.
            debug_log("in", "GET (sse open, holding)");
            stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\n",
            )?;
            stream.flush()?;
            let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(5)));
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_secs(10));
                if stream
                    .write_all(b": keep-alive\n\n")
                    .and_then(|_| stream.flush())
                    .is_err()
                {
                    break;
                }
            }
            return Ok(());
        }
        if request_line.starts_with("DELETE") {
            return write_raw(
                &mut stream,
                "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
                b"",
            );
        }
        if !request_line.starts_with("POST") {
            write_http(&mut stream, 405, &json!({"error": "POST only"}))?;
            return Ok(());
        }

        if content_length > MAX_BODY {
            write_http(
                &mut stream,
                413,
                &json!({"error": format!("body too large (max {MAX_BODY} bytes)")}),
            )?;
            return Ok(());
        }
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body)?;
        debug_log("in", &String::from_utf8_lossy(&body));

        let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let resp = dispatch(&req, conn, &caller);
        if resp.is_null() {
            // A notification — no body, keep the connection open.
            write_raw(
                &mut stream,
                "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n",
                b"",
            )?;
        } else {
            debug_log("out", &resp.to_string());
            write_http(&mut stream, 200, &resp)?;
        }
    }
}

fn write_raw(stream: &mut TcpStream, head: &str, body: &[u8]) -> std::io::Result<()> {
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn dispatch(req: &Value, conn: &rusqlite::Connection, caller: &Caller) -> Value {
    let session_scope = caller.session;
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");

    let result: std::result::Result<Value, String> = match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2025-06-18",
            "serverInfo": { "name": "chorus", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": { "tools": {} },
            // Without this an agent has the tool but no reason to reach for
            // it: asked to "remember X" it answers "ok" and the fact never
            // reaches the Memory pane.
            "instructions": MEMORY_INSTRUCTIONS
        })),
        "notifications/initialized" | "notifications/cancelled" => return Value::Null,
        "tools/list" => {
            let mut tools = vec![propose_tool_schema()];
            // Session-bound tools only exist on a session's own server; the
            // project-memory server has no session to act in.
            if session_scope.is_some() {
                tools.extend([attachment_list_schema(), attachment_read_schema()]);
                tools.extend([
                    spawn_agent_schema(),
                    send_message_schema(),
                    wait_agent_schema(),
                    collect_results_schema(),
                    cancel_agent_schema(),
                    update_checklist_schema(),
                    jump_to_file_schema(),
                    request_review_schema(),
                ]);
                // Listed only for a session whose user turned it on: an agent
                // that never sees the tool never tries it.
                if session_scope.is_some_and(|s| computer::enabled(conn, s)) {
                    tools.push(computer::tool_schema());
                }
                if session_scope.is_some_and(|s| crate::ask_user::available(conn, s)) {
                    tools.push(crate::ask_user::tool_schema());
                }
            }
            tools.extend([synthesize_role_schema(), notify_user_schema()]);
            Ok(json!({ "tools": tools }))
        }
        "tools/call" => call_tool(req.get("params").unwrap_or(&Value::Null), conn, caller),
        other => Err(format!("unknown method {other}")),
    };

    match result {
        Ok(v) => json!({ "jsonrpc": "2.0", "id": id, "result": v }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": e } }),
    }
}

const MEMORY_INSTRUCTIONS: &str = "When the user asks you to remember, note or keep something \
for later, or states a durable fact about the project (a convention, a decision, a preference), \
call `memory_propose` with that fact in one short sentence. Do this instead of your own memory \
notes: Chorus shares this memory with every agent in the session. A person approves it afterwards.";

fn propose_tool_schema() -> Value {
    json!({
        "name": "memory_propose",
        "description": "Call this whenever the user asks you to remember something or states a \
                        durable project fact. Proposes it for the human to review, shared with every \
                        agent in this session. Keep it under 140 characters. It is NOT remembered \
                        until a person approves it.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "the fact, ≤140 chars" },
                "scope": {
                    "type": "integer",
                    "description": "0 global · 1 project (default) · 2 session",
                    "default": 1
                }
            },
            "required": ["text"]
        }
    })
}

/// The caller's actor id. Looked up by name first so an existing actor's
/// provider is not overwritten; created only for a name never seen before.
fn caller_actor(conn: &rusqlite::Connection, caller: &Caller) -> std::result::Result<i64, String> {
    use rusqlite::OptionalExtension;
    let found: Option<i64> = conn
        .query_row(
            "SELECT id FROM actor WHERE name = ?1",
            [&caller.actor],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    match found {
        Some(id) => Ok(id),
        None => actor::intern(conn, &caller.actor, "agent").map_err(|e| e.to_string()),
    }
}

/// The session the caller acts in, or an error on the unscoped server.
fn require_session(caller: &Caller) -> std::result::Result<i64, String> {
    caller
        .session
        .ok_or_else(|| "this tool needs a session; it is unavailable on this server".to_string())
}

/// Refuse any `agent_id` outside the caller's session tree.
fn require_owned(
    conn: &rusqlite::Connection,
    caller: &Caller,
    agent_id: i64,
) -> std::result::Result<(), String> {
    let sess = require_session(caller)?;
    match agent_control::agent_in_session_tree(conn, sess, agent_id) {
        Ok(true) => Ok(()),
        // Same message whether it exists elsewhere or not at all.
        _ => Err(format!("no agent #{agent_id} in this session")),
    }
}

fn call_tool(
    params: &Value,
    conn: &rusqlite::Connection,
    caller: &Caller,
) -> std::result::Result<Value, String> {
    let session_scope = caller.session;
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    if name == "attachment_list" {
        let sess = session_scope.ok_or("attachment access is unavailable")?;
        let entries = attachments::list(conn, sess).map_err(|e| e.to_string())?;
        let value: Vec<Value> = entries.iter().map(|a| json!({"id":a.id,"name":a.name,"mediaType":a.media_type,"size":a.size,"status":a.status,"messageEvent":a.event_seq})).collect();
        return Ok(
            json!({"content":[{"type":"text","text":serde_json::to_string_pretty(&value).unwrap_or_default()}]}),
        );
    }
    if name == "attachment_read" {
        let sess = session_scope.ok_or("attachment access is unavailable")?;
        let id = args
            .get("attachment_id")
            .and_then(Value::as_i64)
            .ok_or("attachment_id is required")?;
        let limit = args
            .get("max_chars")
            .and_then(Value::as_u64)
            .unwrap_or(12000)
            .min(12000) as usize;
        if limit == 0 {
            return Err("max_chars must be at least 1".into());
        }
        let offset = match args.get("offset_chars").and_then(Value::as_i64) {
            Some(value) if value >= 0 => usize::try_from(value).unwrap_or(usize::MAX),
            Some(_) => return Err("offset_chars must be nonnegative".into()),
            None => 0,
        };
        let (meta, bytes, mime) =
            attachments::get_for_agent(conn, sess, id).map_err(|e| e.to_string())?;
        let block = if mime.starts_with("image/") {
            if offset != 0 {
                return Err("offset_chars is only valid for text attachments".into());
            }
            json!({"type":"image","data":attachments::encode_base64(&bytes),"mimeType":mime})
        } else {
            let text = String::from_utf8(bytes)
                .map_err(|_| "attachment is binary and cannot be returned as text")?;
            let total = text.chars().count();
            let start = offset.min(total);
            let excerpt: String = text.chars().skip(start).take(limit).collect();
            let end = start + excerpt.chars().count();
            let next_offset = (end < total).then_some(end);
            let next_offset =
                next_offset.map_or_else(|| "none".to_string(), |value| value.to_string());
            // Same masking as a message: a pasted .env must not reach the model.
            let excerpt = crate::secrets::user_vault().mask(&excerpt).text;
            json!({"type":"text","text":format!("Attachment {} ({}); characters {}–{} of {}; next_offset_chars={}\n{}",meta.name,mime,start,end,total,next_offset,excerpt)})
        };
        return Ok(json!({"content":[block]}));
    }
    if name == "chorus_ask_user" {
        let sess = session_scope.ok_or("asking the user is only available in a session")?;
        return crate::ask_user::call(conn, sess, &caller.actor, &args);
    }
    if name == "computer" {
        let sess = session_scope.ok_or("computer use is only available in a session")?;
        return computer::call(conn, sess, &args, computer::system());
    }
    if name == "memory_propose" {
        let text = args
            .get("text")
            .and_then(Value::as_str)
            .ok_or("text is required")?;
        let scope = args.get("scope").and_then(Value::as_i64).unwrap_or(1);
        // Memory is projected into CLAUDE.md/AGENTS.md on disk: never a value.
        let masked = crate::secrets::user_vault().mask(text).text;
        let text = masked.as_str();

        return match memory::propose(conn, scope, text, None) {
            Ok(id) => Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!("Queued proposal #{id} for review: {text:?}")
                }]
            })),
            Err(e) => Err(e.to_string()),
        };
    }

    if name == "chorus_spawn_agent" {
        let role = args
            .get("role")
            .and_then(Value::as_str)
            .ok_or("role is required")?;
        let task = args
            .get("task")
            .and_then(Value::as_str)
            .ok_or("task is required")?;
        let weight = args.get("weight").and_then(Value::as_str);
        let default_worktree = role == "builder";
        let worktree = args
            .get("worktree")
            .and_then(Value::as_bool)
            .unwrap_or(default_worktree);

        let sess = require_session(caller)?;
        let cwd: String = conn
            .query_row("SELECT cwd FROM session WHERE id = ?1", [sess], |r| {
                r.get(0)
            })
            .map_err(|e| format!("session {sess}: {e}"))?;

        let agent = agent_control::spawn_agent_with_options(
            conn,
            Some(sess),
            role,
            task,
            weight,
            worktree,
            &cwd,
            None,
        )
        .map_err(|e| e.to_string())?;

        let payload = serde_json::json!({
            "event": "subagent_spawned",
            "sessionId": format!("subagent-{}", agent.id),
            "parentSessionId": format!("{sess}"),
            "name": format!("Role: {role}"),
            "task": task,
        })
        .to_string();
        let actor_id = caller_actor(conn, caller)?;
        let mut ev = NewEvent::new(sess, EventKind::ToolCall, actor_id, payload);
        if let Some(last) = ledger::tail(conn, sess, 1)
            .map_err(|e| e.to_string())?
            .last()
        {
            ev = ev.parent(last.seq);
        }
        ledger::append(conn, &ev).map_err(|e| e.to_string())?;

        let text = format!(
            "Spawned child agent #{} (role: '{}', status: '{}').\nTask: {}\nWorktree: {}\nJSON: {}",
            agent.id,
            agent.role,
            agent.status.as_str(),
            agent.task,
            agent.worktree.as_deref().unwrap_or("none"),
            json!({
                "agent_id": agent.id,
                "role": agent.role,
                "status": agent.status.as_str(),
                "worktree": agent.worktree,
            })
        );
        return Ok(json!({ "content": [{ "type": "text", "text": text }] }));
    }

    if name == "chorus_send_message" {
        let agent_id = args
            .get("agent_id")
            .and_then(|v| {
                v.as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .ok_or("agent_id is required")?;
        let message = args
            .get("message")
            .and_then(Value::as_str)
            .ok_or("message is required")?;

        require_owned(conn, caller, agent_id)?;
        let actor_id = caller_actor(conn, caller)?;
        let msg = agent_control::send_message(conn, agent_id, actor_id, message)
            .map_err(|e| e.to_string())?;

        let text = format!(
            "Message delivered to child agent #{}: {:?}",
            msg.subagent_id, msg.content
        );
        return Ok(json!({ "content": [{ "type": "text", "text": text }] }));
    }

    if name == "chorus_wait_agent" {
        let agent_id = args.get("agent_id").and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        });
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(30);
        let sess = require_session(caller)?;
        if let Some(id) = agent_id {
            require_owned(conn, caller, id)?;
        }

        let rep = agent_control::wait_agent(conn, Some(sess), agent_id, timeout_secs)
            .map_err(|e| e.to_string())?;

        let text = format!(
            "Wait status:\nAll done: {}\nCompleted: {:?}\nRunning: {:?}\nCancelled: {:?}",
            rep.all_done, rep.completed, rep.running, rep.cancelled
        );
        return Ok(json!({
            "content": [{ "type": "text", "text": text }],
            "all_done": rep.all_done,
            "statuses": rep.statuses
        }));
    }

    if name == "chorus_collect_results" {
        let mut agent_ids = Vec::new();
        if let Some(arr) = args.get("agent_ids").and_then(Value::as_array) {
            for v in arr {
                if let Some(id) = v
                    .as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                {
                    agent_ids.push(id);
                }
            }
        } else if let Some(id) = args.get("agent_id").and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        }) {
            agent_ids.push(id);
        }
        let ids_opt = if agent_ids.is_empty() {
            None
        } else {
            Some(agent_ids.as_slice())
        };
        let join_mode = args
            .get("join_mode")
            .and_then(Value::as_str)
            .unwrap_or("compact");
        let sess = require_session(caller)?;
        for id in &agent_ids {
            require_owned(conn, caller, *id)?;
        }

        let res = agent_control::collect_results(conn, Some(sess), ids_opt, join_mode)
            .map_err(|e| e.to_string())?;

        return Ok(json!({
            "content": [{ "type": "text", "text": res.rendered_markdown }],
            "briefs": res.briefs
        }));
    }

    if name == "chorus_cancel_agent" {
        let agent_id = args
            .get("agent_id")
            .and_then(|v| {
                v.as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .ok_or("agent_id is required")?;
        let reason = args.get("reason").and_then(Value::as_str);
        require_owned(conn, caller, agent_id)?;

        let agent =
            agent_control::cancel_agent(conn, agent_id, reason).map_err(|e| e.to_string())?;

        let text = format!(
            "Cancelled child agent #{} (role: '{}')",
            agent.id, agent.role
        );
        return Ok(json!({ "content": [{ "type": "text", "text": text }] }));
    }

    if name == "chorus_update_checklist" {
        let items_val = args
            .get("items")
            .and_then(Value::as_array)
            .ok_or("items array is required")?;

        let mut items = Vec::new();
        for item in items_val {
            let text = item
                .get("text")
                .and_then(Value::as_str)
                .ok_or("item text is required")?;
            let done = item.get("done").and_then(Value::as_bool).unwrap_or(false);
            let in_progress = item
                .get("in_progress")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            items.push(agent_control::ChecklistItem {
                text: text.to_string(),
                done,
                in_progress,
            });
        }

        let sess = require_session(caller)?;
        let updated =
            agent_control::update_checklist(conn, sess, &items).map_err(|e| e.to_string())?;

        let done_count = updated.iter().filter(|i| i.done).count();
        let text = format!(
            "Updated live task checklist: {}/{} completed.",
            done_count,
            updated.len()
        );
        return Ok(json!({ "content": [{ "type": "text", "text": text }] }));
    }

    if name == "chorus_jump_to_file" {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or("path is required")?;
        let line = args.get("line").and_then(Value::as_u64).map(|v| v as u32);

        let sess = require_session(caller)?;
        let jump =
            agent_control::jump_to_file(conn, sess, path, line).map_err(|e| e.to_string())?;

        let line_str = jump.line.map(|l| format!(":{l}")).unwrap_or_default();
        let text = format!("Navigated Workbench to {}{}", jump.path, line_str);
        return Ok(json!({ "content": [{ "type": "text", "text": text }] }));
    }

    if name == "chorus_request_review" {
        let summary = args
            .get("summary")
            .and_then(Value::as_str)
            .ok_or("summary is required")?;
        let hunks = args.get("hunks").cloned().unwrap_or(json!([]));

        let sess = require_session(caller)?;
        let req = agent_control::request_review(conn, sess, summary, &hunks)
            .map_err(|e| e.to_string())?;

        let text = format!(
            "Created interactive review request #{}: {:?}. Status: pending.",
            req.id, req.summary
        );
        return Ok(json!({ "content": [{ "type": "text", "text": text }] }));
    }

    if name == "chorus_synthesize_role" {
        let task = args
            .get("task")
            .and_then(Value::as_str)
            .ok_or("task is required")?;
        let name_arg = args.get("name").and_then(Value::as_str);
        let write_mode = args
            .get("write_mode")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let role = roles::synthesize_role(task, write_mode, name_arg);
        let text = format!(
            "Synthesized custom role '{}':\n\n{}",
            role.name,
            role.to_markdown()
        );
        return Ok(json!({
            "content": [{ "type": "text", "text": text }],
            "role": role
        }));
    }

    if name == "chorus_notify_user" {
        let title = args
            .get("title")
            .and_then(Value::as_str)
            .ok_or("title is required")?;
        let message = args
            .get("message")
            .and_then(Value::as_str)
            .ok_or("message is required")?;
        let _urgency = args
            .get("urgency")
            .and_then(Value::as_str)
            .unwrap_or("normal");

        let safe_title = format!("Chorus Agent: {}", title.trim());
        let opts = notify::NotificationOptions::new(&safe_title, message);
        let _ = notify::send_notification(&opts);

        let text = format!("User notification triggered: {safe_title} - {message}");
        return Ok(json!({ "content": [{ "type": "text", "text": text }] }));
    }

    Err(format!("no tool {name}"))
}

fn attachment_list_schema() -> Value {
    json!({"name":"attachment_list","description":"List attachments belonging to this Chorus session. Deleted attachments are listed as tombstones and cannot be read.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}})
}

fn attachment_read_schema() -> Value {
    json!({"name":"attachment_read","description":"Read prepared content or original content for one live attachment in this Chorus session. Text output is bounded to 12,000 characters per call; use offset_chars and next_offset_chars to continue. Images are returned as image content.","inputSchema":{"type":"object","properties":{"attachment_id":{"type":"integer"},"max_chars":{"type":"integer","minimum":1,"maximum":12000},"offset_chars":{"type":"integer","minimum":0}},"required":["attachment_id"],"additionalProperties":false}})
}

fn spawn_agent_schema() -> Value {
    json!({
        "name": "chorus_spawn_agent",
        "description": "Spawn a child agent session with a specialized role, task prompt, weight, and optional isolated git worktree.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "role": {
                    "type": "string",
                    "description": "The agent role to spawn (e.g. 'investigator', 'builder', 'reviewer', or custom role name)"
                },
                "task": {
                    "type": "string",
                    "description": "The specific task or instruction for the child agent"
                },
                "weight": {
                    "type": "string",
                    "description": "Task weight/difficulty: 'light', 'normal', or 'heavy'",
                    "enum": ["light", "normal", "heavy"]
                },
                "worktree": {
                    "type": "boolean",
                    "description": "If true, allocates an isolated git worktree for write operations. Defaults to true for builder role."
                },
                "model": {
                    "type": "string",
                    "description": "Optional model override preference for the child agent"
                }
            },
            "required": ["role", "task"]
        }
    })
}

fn send_message_schema() -> Value {
    json!({
        "name": "chorus_send_message",
        "description": "Send a steering message, feedback, or follow-up instruction to a child agent session.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "integer",
                    "description": "The numeric ID of the child agent session"
                },
                "message": {
                    "type": "string",
                    "description": "The message or guidance to transmit to the child agent"
                }
            },
            "required": ["agent_id", "message"]
        }
    })
}

fn wait_agent_schema() -> Value {
    json!({
        "name": "chorus_wait_agent",
        "description": "Wait for a child agent or all child agents to complete execution.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "integer",
                    "description": "Optional specific child agent ID to wait for. If omitted, waits for all active child agents."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Optional timeout in seconds (default: 30)",
                    "default": 30
                }
            }
        }
    })
}

fn collect_results_schema() -> Value {
    json!({
        "name": "chorus_collect_results",
        "description": "Collect a compact structured brief (summary, diff stat, files touched) from completed child sessions, protecting parent context.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "agent_ids": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "Optional list of child agent IDs to collect. If omitted, collects from all completed child agents."
                },
                "join_mode": {
                    "type": "string",
                    "description": "Aggregation mode: 'compact' (default, brief summary + stats), 'full' (detailed logs), or 'diff_only'",
                    "enum": ["compact", "full", "diff_only"],
                    "default": "compact"
                }
            }
        }
    })
}

fn cancel_agent_schema() -> Value {
    json!({
        "name": "chorus_cancel_agent",
        "description": "Cancel a running child agent session and clean up its allocated worktree.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "integer",
                    "description": "The child agent ID to cancel"
                },
                "reason": {
                    "type": "string",
                    "description": "Optional cancellation reason"
                }
            },
            "required": ["agent_id"]
        }
    })
}

fn update_checklist_schema() -> Value {
    json!({
        "name": "chorus_update_checklist",
        "description": "Update the live task checklist displayed in the Task State pane.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "text": { "type": "string", "description": "Checklist item description" },
                            "done": { "type": "boolean", "description": "Whether the item is completed" },
                            "in_progress": { "type": "boolean", "description": "Whether the item is currently in progress", "default": false }
                        },
                        "required": ["text", "done"]
                    },
                    "description": "The complete list of current checklist items"
                }
            },
            "required": ["items"]
        }
    })
}

fn jump_to_file_schema() -> Value {
    json!({
        "name": "chorus_jump_to_file",
        "description": "Trigger the Workbench code viewer to navigate to and highlight a specific file and line for the human.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to open"
                },
                "line": {
                    "type": "integer",
                    "description": "Optional 1-indexed line number to highlight"
                }
            },
            "required": ["path"]
        }
    })
}

fn request_review_schema() -> Value {
    json!({
        "name": "chorus_request_review",
        "description": "Request an interactive human review on diff hunks before making changes or continuing.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "Summary of the changes or proposal requiring approval"
                },
                "hunks": {
                    "type": "array",
                    "description": "List of diff hunks or code changes to review"
                }
            },
            "required": ["summary"]
        }
    })
}

fn synthesize_role_schema() -> Value {
    json!({
        "name": "chorus_synthesize_role",
        "description": "Synthesize an optimized custom subagent role definition with prompt, boundaries, and tools for a specialized task.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "The specific task or domain requirements for the synthesized role"
                },
                "name": {
                    "type": "string",
                    "description": "Optional suggested role name"
                },
                "write_mode": {
                    "type": "boolean",
                    "description": "Whether this role requires file writing and bash execution capabilities",
                    "default": false
                }
            },
            "required": ["task"]
        }
    })
}

fn notify_user_schema() -> Value {
    json!({
        "name": "chorus_notify_user",
        "description": "Trigger a native macOS user notification with audio chime.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Notification title" },
                "message": { "type": "string", "description": "Notification message body" },
                "urgency": {
                    "type": "string",
                    "enum": ["low", "normal", "critical"],
                    "default": "normal"
                }
            },
            "required": ["title", "message"]
        }
    })
}

fn write_http(stream: &mut TcpStream, status: u16, body: &Value) -> std::io::Result<()> {
    let payload = serde_json::to_vec(body).unwrap_or_default();
    write!(
        stream,
        "HTTP/1.1 {status} OK\r\n\
         Content-Type: application/json\r\n\
         Mcp-Session-Id: chorus-session\r\n\
         Content-Length: {}\r\n\r\n",
        payload.len()
    )?;
    stream.write_all(&payload)?;
    stream.flush()
}
