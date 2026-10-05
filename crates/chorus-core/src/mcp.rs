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

use crate::{attachments, memory, Result, Store};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

/// A running MCP server. Drop it (or call `stop`) to shut down.
pub struct McpServer {
    url: String,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
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
        let scope = session_scope;

        let handle = std::thread::spawn(move || {
            while !stop_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        // One SQLite connection per connection thread — the
                        // GET/SSE stream is long-lived, so a shared handle
                        // would serialise everything behind it.
                        let db = db.clone();
                        let stop = stop_thread.clone();
                        std::thread::spawn(move || {
                            if let Ok(store) = Store::open(std::path::Path::new(&db)) {
                                let _ = handle_conn(stream, store.conn(), &stop, scope);
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
        })
    }

    /// The entry to put in `session/new`'s `mcpServers` (ACP HTTP shape:
    /// `type` + `name` + `url` + `headers`).
    pub fn mcp_server_entry(&self) -> Value {
        json!({ "type": "http", "name": "chorus", "url": self.url, "headers": [] })
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
    session_scope: Option<i64>,
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
        }

        if has_origin {
            write_http(
                &mut stream,
                403,
                &json!({"error": "this endpoint does not serve browser origins"}),
            )?;
            return Ok(());
        }

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
        let resp = dispatch(&req, conn, session_scope);
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

fn dispatch(req: &Value, conn: &rusqlite::Connection, session_scope: Option<i64>) -> Value {
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
            if session_scope.is_some() {
                tools.extend([attachment_list_schema(), attachment_read_schema()]);
            }
            Ok(json!({ "tools": tools }))
        }
        "tools/call" => call_tool(
            req.get("params").unwrap_or(&Value::Null),
            conn,
            session_scope,
        ),
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

fn call_tool(
    params: &Value,
    conn: &rusqlite::Connection,
    session_scope: Option<i64>,
) -> std::result::Result<Value, String> {
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
            json!({"type":"text","text":format!("Attachment {} ({}); characters {}–{} of {}; next_offset_chars={}\n{}",meta.name,mime,start,end,total,next_offset,excerpt)})
        };
        return Ok(json!({"content":[block]}));
    }
    if name != "memory_propose" {
        return Err(format!("no tool {name}"));
    }
    let text = args
        .get("text")
        .and_then(Value::as_str)
        .ok_or("text is required")?;
    let scope = args.get("scope").and_then(Value::as_i64).unwrap_or(1);

    match memory::propose(conn, scope, text, None) {
        Ok(id) => Ok(json!({
            "content": [{
                "type": "text",
                "text": format!("Queued proposal #{id} for review: {text:?}")
            }]
        })),
        Err(e) => Err(e.to_string()),
    }
}

fn attachment_list_schema() -> Value {
    json!({"name":"attachment_list","description":"List attachments belonging to this Chorus session. Deleted attachments are listed as tombstones and cannot be read.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}})
}

fn attachment_read_schema() -> Value {
    json!({"name":"attachment_read","description":"Read prepared content or original content for one live attachment in this Chorus session. Text output is bounded to 12,000 characters per call; use offset_chars and next_offset_chars to continue. Images are returned as image content.","inputSchema":{"type":"object","properties":{"attachment_id":{"type":"integer"},"max_chars":{"type":"integer","minimum":1,"maximum":12000},"offset_chars":{"type":"integer","minimum":0}},"required":["attachment_id"],"additionalProperties":false}})
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
