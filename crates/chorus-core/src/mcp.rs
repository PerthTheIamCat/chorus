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

use crate::{memory, Result, Store};
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
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let url = format!("http://127.0.0.1:{port}/mcp");

        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let db = db_path.to_string();

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
                                let _ = handle_conn(stream, store.conn(), &stop);
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
        let resp = dispatch(&req, conn);
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

fn dispatch(req: &Value, conn: &rusqlite::Connection) -> Value {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");

    let result: std::result::Result<Value, String> = match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2025-06-18",
            "serverInfo": { "name": "chorus", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": { "tools": {} }
        })),
        "notifications/initialized" | "notifications/cancelled" => return Value::Null,
        "tools/list" => Ok(json!({ "tools": [ propose_tool_schema() ] })),
        "tools/call" => call_tool(req.get("params").unwrap_or(&Value::Null), conn),
        other => Err(format!("unknown method {other}")),
    };

    match result {
        Ok(v) => json!({ "jsonrpc": "2.0", "id": id, "result": v }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": e } }),
    }
}

fn propose_tool_schema() -> Value {
    json!({
        "name": "memory_propose",
        "description": "Propose a durable fact for the human to review. Keep it under \
                        140 characters. It is NOT remembered until a person approves it.",
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

fn call_tool(params: &Value, conn: &rusqlite::Connection) -> std::result::Result<Value, String> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    if name != "memory_propose" {
        return Err(format!("no tool {name}"));
    }
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
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
