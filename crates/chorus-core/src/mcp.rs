//! A tiny MCP server Chorus hands to every agent, so the agent can reach
//! back into the app for the few things it must not do directly.
//!
//! Right now it exposes exactly one tool: `memory.propose`. An agent may
//! *propose* a durable fact; a human approves it later (design §4, trap
//! §7.2). The agent never writes `fact` itself.
//!
//! Transport is plain HTTP JSON-RPC on `127.0.0.1:<port>` — the Codex
//! bridge advertises `mcpCapabilities.http`, and we don't need SSE for a
//! request/response tool server. No HTTP-crate dependency: the protocol
//! surface is `initialize` / `tools/list` / `tools/call` and the framing is
//! a dozen lines of `std::net`.

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
            // One connection for the life of the server thread.
            let store = match Store::open(std::path::Path::new(&db)) {
                Ok(s) => s,
                Err(_) => return,
            };
            while !stop_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = handle_conn(stream, store.conn());
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

fn handle_conn(mut stream: TcpStream, conn: &rusqlite::Connection) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    let mut reader = BufReader::new(stream.try_clone()?);

    // Request line + headers.
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if !line.starts_with("POST") {
        return write_http(&mut stream, 405, &json!({"error": "POST only"}));
    }
    let mut content_length = 0usize;
    loop {
        let mut h = String::new();
        reader.read_line(&mut h)?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some(v) = h.to_lowercase().strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body)?;

    let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let resp = dispatch(&req, conn);
    write_http(&mut stream, 200, &resp)
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
        "name": "memory.propose",
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
    if name != "memory.propose" {
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
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    )?;
    stream.write_all(&payload)?;
    stream.flush()
}
