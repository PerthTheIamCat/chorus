//! HTTP/1.1 protocol handling, request routing, and SSE event streaming.

use crate::events::{
    format_sse, format_sse_comment, replay_ledger_events, DaemonEvent, EventBroadcaster,
};
use crate::interaction::{
    CreateInteractionRequest, InteractionManager, InteractionStatus, ResolveInteractionRequest,
};
use crate::lifecycle::LifecycleManager;
use crate::pairing::PairingManager;
use rusqlite::Connection;
use serde_json::json;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

/// Maximum allowable request body size (1 MB).
const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Bound allocations before reading untrusted request lines and headers.
const MAX_HEADER_BYTES: usize = 16 * 1024;
/// Connection read timeout for standard requests.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Unified stream type supporting both Unix domain sockets and TCP streams.
pub enum DaemonStream {
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Read for DaemonStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Unix(s) => s.read(buf),
            Self::Tcp(s) => s.read(buf),
        }
    }
}

impl Write for DaemonStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Unix(s) => s.write(buf),
            Self::Tcp(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Unix(s) => s.flush(),
            Self::Tcp(s) => s.flush(),
        }
    }
}

impl DaemonStream {
    pub fn try_clone(&self) -> std::io::Result<Self> {
        match self {
            Self::Unix(s) => s.try_clone().map(DaemonStream::Unix),
            Self::Tcp(s) => s.try_clone().map(DaemonStream::Tcp),
        }
    }

    pub fn set_read_timeout(&self, dur: Option<Duration>) -> std::io::Result<()> {
        match self {
            Self::Unix(s) => s.set_read_timeout(dur),
            Self::Tcp(s) => s.set_read_timeout(dur),
        }
    }

    pub fn set_write_timeout(&self, dur: Option<Duration>) -> std::io::Result<()> {
        match self {
            Self::Unix(s) => s.set_write_timeout(dur),
            Self::Tcp(s) => s.set_write_timeout(dur),
        }
    }
}

/// Parsed HTTP request.
#[derive(Debug)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub query_params: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    pub fn last_event_id(&self) -> Option<i64> {
        self.headers
            .get("last-event-id")
            .and_then(|v| v.trim().parse::<i64>().ok())
            .or_else(|| {
                self.query_params
                    .get("since")
                    .and_then(|v| v.trim().parse::<i64>().ok())
            })
    }
}

/// Parse query string like `?since=10&status=pending`.
fn parse_query_params(query_str: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    for part in query_str.split('&') {
        if part.is_empty() {
            continue;
        }
        let mut kv = part.splitn(2, '=');
        let key = kv.next().unwrap_or("").trim().to_string();
        let val = kv.next().unwrap_or("").trim().to_string();
        if !key.is_empty() {
            params.insert(key, val);
        }
    }
    params
}

/// Write an HTTP response to stream.
pub fn write_response<W: Write>(
    w: &mut W,
    code: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let headers = format!(
        "HTTP/1.1 {code} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    w.write_all(headers.as_bytes())?;
    w.write_all(body)?;
    w.flush()
}

/// Write JSON response.
pub fn write_json<W: Write>(w: &mut W, code: u16, val: &serde_json::Value) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(val).unwrap_or_default();
    let reason = match code {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        _ => "Response",
    };
    write_response(w, code, reason, "application/json", &bytes)
}

/// Write error JSON response.
pub fn write_error<W: Write>(w: &mut W, code: u16, msg: &str) -> std::io::Result<()> {
    write_json(w, code, &json!({ "ok": false, "error": msg }))
}

/// Handle a single incoming client connection.
pub fn handle_client(
    mut stream: DaemonStream,
    conn: Arc<Mutex<Connection>>,
    interactions: InteractionManager,
    broadcaster: EventBroadcaster,
    lifecycle: LifecycleManager,
    pairing: PairingManager,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(DEFAULT_TIMEOUT))?;
    stream.set_write_timeout(Some(DEFAULT_TIMEOUT))?;
    let mut reader = BufReader::new(stream.try_clone()?);

    // Read request line
    let mut req_line = String::new();
    if Read::by_ref(&mut reader)
        .take((MAX_HEADER_BYTES + 1) as u64)
        .read_line(&mut req_line)?
        == 0
    {
        return Ok(()); // Client closed
    }
    let mut header_bytes = req_line.len();
    if header_bytes > MAX_HEADER_BYTES {
        return write_error(&mut stream, 431, "Request headers exceed limit");
    }

    let req_line = req_line.trim_end();
    let mut parts = req_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_uppercase();
    let full_path = parts.next().unwrap_or("/").to_string();
    if !matches!(parts.next(), Some("HTTP/1.1" | "HTTP/1.0")) || parts.next().is_some() {
        return write_error(&mut stream, 400, "Invalid request line");
    }

    let (path, query_params) = if let Some(idx) = full_path.find('?') {
        let (p, q) = full_path.split_at(idx);
        (p.to_string(), parse_query_params(&q[1..]))
    } else {
        (full_path, HashMap::new())
    };

    // Read headers
    let mut headers = HashMap::new();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        let remaining = MAX_HEADER_BYTES - header_bytes;
        if Read::by_ref(&mut reader)
            .take((remaining + 1) as u64)
            .read_line(&mut line)?
            == 0
        {
            return write_error(&mut stream, 400, "Incomplete request headers");
        }
        header_bytes += line.len();
        if header_bytes > MAX_HEADER_BYTES {
            return write_error(&mut stream, 431, "Request headers exceed limit");
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_lowercase();
            let val = v.trim().to_string();
            if headers.contains_key(&key) {
                return write_error(&mut stream, 400, "Duplicate request header");
            }
            if key == "content-length" {
                content_length = match val.parse() {
                    Ok(length) => length,
                    Err(_) => return write_error(&mut stream, 400, "Invalid Content-Length"),
                };
            }
            if key == "transfer-encoding" {
                return write_error(&mut stream, 400, "Transfer-Encoding is unsupported");
            }
            headers.insert(key, val);
        } else {
            return write_error(&mut stream, 400, "Invalid request header");
        }
    }

    // Browser content has no authority to call the native control surface.
    if headers.contains_key("origin") {
        return write_error(&mut stream, 403, "Browser origins are not allowed");
    }
    if matches!(&stream, DaemonStream::Tcp(_)) {
        let public = matches!(
            (method.as_str(), path.as_str()),
            ("GET", "/health" | "/ping") | ("POST", "/api/pairing/complete")
        );
        if !public {
            let token = headers
                .get("authorization")
                .and_then(|h| h.strip_prefix("Bearer "));
            if token.and_then(|t| pairing.verify_token(t)).is_none() {
                return write_error(&mut stream, 401, "A paired device bearer token is required");
            }
        }
        // Pairing and host lifecycle are managed only through the owner-only socket.
        if path.starts_with("/api/pairing/") && path != "/api/pairing/complete"
            || matches!(path.as_str(), "/shutdown" | "/lifecycle/mode")
        {
            return write_error(&mut stream, 403, "Host management requires the Unix socket");
        }
    }

    if method == "OPTIONS" {
        return write_response(&mut stream, 204, "No Content", "text/plain", b"");
    }

    // Read body if present
    let mut body = Vec::new();
    if content_length > 0 {
        if content_length > MAX_BODY_BYTES {
            return write_error(&mut stream, 413, "Request body exceeds 1 MB limit");
        }
        body.resize(content_length, 0);
        reader.read_exact(&mut body)?;
    }

    let req = HttpRequest {
        method,
        path,
        query_params,
        headers,
        body,
    };

    dispatch_request(
        stream,
        req,
        conn,
        interactions,
        broadcaster,
        lifecycle,
        pairing,
    )
}

fn dispatch_request(
    mut stream: DaemonStream,
    req: HttpRequest,
    conn: Arc<Mutex<Connection>>,
    interactions: InteractionManager,
    broadcaster: EventBroadcaster,
    lifecycle: LifecycleManager,
    pairing: PairingManager,
) -> std::io::Result<()> {
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/health") | ("GET", "/ping") => write_json(
            &mut stream,
            200,
            &json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }),
        ),

        ("GET", "/lifecycle/status") => {
            let pending_count = interactions
                .list(Some(InteractionStatus::Pending), None)
                .map(|list| list.len())
                .unwrap_or(0);
            let status = lifecycle.status_json(pending_count);
            write_json(&mut stream, 200, &status)
        }

        ("POST", "/lifecycle/mode") => {
            #[derive(serde::Deserialize)]
            struct ModeReq {
                mode: String,
            }
            match serde_json::from_slice::<ModeReq>(&req.body) {
                Ok(m) => match crate::lifecycle::LifecycleMode::from_str(&m.mode) {
                    Some(new_mode) => {
                        lifecycle.set_mode(new_mode);
                        write_json(
                            &mut stream,
                            200,
                            &json!({ "ok": true, "mode": new_mode.as_str() }),
                        )
                    }
                    None => write_error(
                        &mut stream,
                        400,
                        "Invalid mode: expected 'foreground' or 'background'",
                    ),
                },
                Err(e) => write_error(&mut stream, 400, &format!("Invalid JSON: {e}")),
            }
        }

        ("POST", "/shutdown") => {
            lifecycle.request_shutdown();
            write_json(
                &mut stream,
                200,
                &json!({ "ok": true, "message": "Graceful shutdown initiated" }),
            )
        }

        ("GET", "/events") | ("GET", "/v1/events") => {
            serve_events_stream(stream, req, conn, broadcaster, lifecycle, pairing)
        }

        ("POST", "/events") => {
            #[derive(serde::Deserialize)]
            struct PostEventReq {
                session_id: Option<i64>,
                kind: String,
                actor: Option<String>,
                body: String,
                model_id: Option<String>,
            }
            match serde_json::from_slice::<PostEventReq>(&req.body) {
                Ok(p) => {
                    let seq = broadcaster.next_seq();
                    let t = 0i64;
                    let actor = p.actor.unwrap_or_else(|| "user".to_string());
                    let event = DaemonEvent {
                        seq,
                        t,
                        session_id: p.session_id,
                        kind: p.kind,
                        actor,
                        body: p.body,
                        model_id: p.model_id,
                        interaction: None,
                    };
                    broadcaster.broadcast(event.clone());
                    write_json(
                        &mut stream,
                        201,
                        &serde_json::to_value(&event).unwrap_or_default(),
                    )
                }
                Err(e) => write_error(&mut stream, 400, &format!("Invalid event JSON: {e}")),
            }
        }

        ("GET", "/interactions") | ("GET", "/v1/interactions") => {
            let status = req
                .query_params
                .get("status")
                .and_then(|s| InteractionStatus::from_str(s));
            let session_id = req
                .query_params
                .get("session_id")
                .and_then(|s| s.parse::<i64>().ok());

            match interactions.list(status, session_id) {
                Ok(list) => write_json(
                    &mut stream,
                    200,
                    &serde_json::to_value(&list).unwrap_or_default(),
                ),
                Err(e) => write_error(&mut stream, 500, &format!("{e}")),
            }
        }

        ("POST", "/interactions") | ("POST", "/v1/interactions") => {
            match serde_json::from_slice::<CreateInteractionRequest>(&req.body) {
                Ok(create_req) => match interactions.create(create_req) {
                    Ok(interaction) => {
                        // Broadcast interaction.created event
                        let seq = broadcaster.next_seq();
                        let event = DaemonEvent {
                            seq,
                            t: 0,
                            session_id: interaction.session_id,
                            kind: "interaction.created".to_string(),
                            actor: "system".to_string(),
                            body: interaction.detail.clone(),
                            model_id: None,
                            interaction: Some(interaction.clone()),
                        };
                        broadcaster.broadcast(event);
                        write_json(
                            &mut stream,
                            201,
                            &serde_json::to_value(&interaction).unwrap_or_default(),
                        )
                    }
                    Err(e) => write_error(&mut stream, 500, &format!("{e}")),
                },
                Err(e) => write_error(&mut stream, 400, &format!("Invalid interaction JSON: {e}")),
            }
        }

        (method, path) if path.starts_with("/interactions/") => {
            let remainder = &path["/interactions/".len()..];
            if let Some(id) = remainder.strip_suffix("/resolve") {
                if method == "POST" {
                    match serde_json::from_slice::<ResolveInteractionRequest>(&req.body) {
                        Ok(resolve_req) => match interactions.resolve(id, resolve_req) {
                            Ok(resolved) => {
                                // Broadcast interaction.resolved event
                                let seq = broadcaster.next_seq();
                                let event = DaemonEvent {
                                    seq,
                                    t: 0,
                                    session_id: resolved.session_id,
                                    kind: "interaction.resolved".to_string(),
                                    actor: "system".to_string(),
                                    body: format!(
                                        "decision: {}",
                                        resolved.decision.map(|d| d.as_str()).unwrap_or("unknown")
                                    ),
                                    model_id: None,
                                    interaction: Some(resolved.clone()),
                                };
                                broadcaster.broadcast(event);
                                write_json(
                                    &mut stream,
                                    200,
                                    &serde_json::to_value(&resolved).unwrap_or_default(),
                                )
                            }
                            Err(crate::error::DaemonError::InteractionNotFound(_)) => {
                                write_error(&mut stream, 404, "Interaction not found")
                            }
                            Err(crate::error::DaemonError::InteractionAlreadyResolved(_)) => {
                                write_error(&mut stream, 409, "Interaction already resolved")
                            }
                            Err(e) => write_error(&mut stream, 500, &format!("{e}")),
                        },
                        Err(e) => {
                            write_error(&mut stream, 400, &format!("Invalid resolve JSON: {e}"))
                        }
                    }
                } else {
                    write_error(&mut stream, 405, "Method not allowed")
                }
            } else if method == "GET" {
                match interactions.get(remainder) {
                    Ok(Some(item)) => write_json(
                        &mut stream,
                        200,
                        &serde_json::to_value(&item).unwrap_or_default(),
                    ),
                    Ok(None) => write_error(&mut stream, 404, "Interaction not found"),
                    Err(e) => write_error(&mut stream, 500, &format!("{e}")),
                }
            } else {
                write_error(&mut stream, 404, "Not found")
            }
        }

        ("GET", "/api/pairing/initiate") => match pairing.initiate_host_session() {
            Ok(session) => write_json(
                &mut stream,
                200,
                &serde_json::to_value(&session).unwrap_or_default(),
            ),
            Err(_) => write_error(
                &mut stream,
                500,
                "Unable to create a secure pairing session",
            ),
        },

        ("POST", "/api/pairing/complete") => {
            #[derive(serde::Deserialize)]
            struct CompleteReq {
                pairing_token: String,
                device_name: String,
            }
            match serde_json::from_slice::<CompleteReq>(&req.body) {
                Ok(c) => match pairing.complete_pairing(&c.pairing_token, &c.device_name) {
                    Ok(dev) => write_json(
                        &mut stream,
                        200,
                        &serde_json::to_value(&dev).unwrap_or_default(),
                    ),
                    Err(e) => write_error(&mut stream, 400, &format!("{e}")),
                },
                Err(e) => write_error(&mut stream, 400, &format!("Invalid JSON: {e}")),
            }
        }

        ("GET", "/api/pairing/devices") => {
            let devs: Vec<_> = pairing
                .list_devices()
                .iter()
                .map(|dev| {
                    json!({
                        "device_id": dev.device_id,
                        "device_name": dev.device_name,
                        "paired_at": dev.paired_at,
                        "last_seen_at": dev.last_seen_at,
                    })
                })
                .collect();
            write_json(&mut stream, 200, &json!(devs))
        }

        ("POST", "/api/pairing/revoke") => {
            #[derive(serde::Deserialize)]
            struct RevokeReq {
                device_id: String,
            }
            match serde_json::from_slice::<RevokeReq>(&req.body) {
                Ok(r) => {
                    let ok = pairing.revoke_device(&r.device_id);
                    write_json(&mut stream, 200, &json!({ "ok": ok }))
                }
                Err(e) => write_error(&mut stream, 400, &format!("Invalid JSON: {e}")),
            }
        }

        ("GET", "/api/mobile/interactions") => {
            let auth = req
                .headers
                .get("authorization")
                .map(|s| s.as_str())
                .unwrap_or("");
            let token = auth.strip_prefix("Bearer ").unwrap_or(auth);
            if pairing.verify_token(token).is_none() {
                write_error(&mut stream, 401, "Unauthorized mobile companion token")
            } else {
                match interactions.list(Some(InteractionStatus::Pending), None) {
                    Ok(list) => write_json(
                        &mut stream,
                        200,
                        &serde_json::to_value(&list).unwrap_or_default(),
                    ),
                    Err(e) => write_error(&mut stream, 500, &format!("{e}")),
                }
            }
        }

        _ => write_error(&mut stream, 404, "Endpoint not found"),
    }
}

/// Serve live and replayed events via Server-Sent Events (SSE).
fn serve_events_stream(
    mut stream: DaemonStream,
    req: HttpRequest,
    conn: Arc<Mutex<Connection>>,
    broadcaster: EventBroadcaster,
    lifecycle: LifecycleManager,
    pairing: PairingManager,
) -> std::io::Result<()> {
    let token = if matches!(&stream, DaemonStream::Tcp(_)) {
        req.headers
            .get("authorization")
            .and_then(|h| h.strip_prefix("Bearer "))
            .map(str::to_owned)
    } else {
        None
    };
    let authorized = || {
        token
            .as_deref()
            .is_none_or(|t| pairing.verify_token(t).is_some())
    };
    // Clear timeouts for long-lived streaming
    let _ = stream.set_read_timeout(None);
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));

    // Write SSE HTTP headers
    let sse_headers = "HTTP/1.1 200 OK\r\n\
                       Content-Type: text/event-stream\r\n\
                       Cache-Control: no-cache\r\n\
                       Connection: keep-alive\r\n\r\n";
    stream.write_all(sse_headers.as_bytes())?;
    stream.flush()?;

    // Catch-up replay: read events > Last-Event-ID from SQLite ledger
    let after_seq = req.last_event_id().unwrap_or(0);
    let session_id = req
        .query_params
        .get("session_id")
        .and_then(|s| s.parse::<i64>().ok());

    // Subscribe to live broadcaster first so no events are lost between replay and subscription
    let rx = broadcaster.subscribe();

    let mut max_seen_seq = after_seq;
    let replayed = {
        let c = conn.lock().unwrap();
        replay_ledger_events(&c, after_seq, session_id).map_err(std::io::Error::other)?
    };
    for ev in replayed {
        if !authorized() {
            return Ok(());
        }
        if ev.seq > max_seen_seq {
            max_seen_seq = ev.seq;
        }
        let chunk = format_sse(&ev);
        if stream.write_all(chunk.as_bytes()).is_err() {
            return Ok(()); // Client hung up
        }
    }
    let _ = stream.flush();

    // Stream live events as they arrive
    while !lifecycle.is_shutting_down() {
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(ev) => {
                if !authorized() {
                    break;
                }
                if session_id.is_some() && ev.session_id != session_id {
                    continue;
                }
                // If event was already replayed from DB, skip duplicate
                if ev.seq <= max_seen_seq {
                    continue;
                }
                max_seen_seq = ev.seq;
                let chunk = format_sse(&ev);
                if stream.write_all(chunk.as_bytes()).is_err() {
                    break; // Client disconnected
                }
                if stream.flush().is_err() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !authorized() {
                    break;
                }
                // Send periodic keepalive comment
                let ping = format_sse_comment("keepalive");
                if stream.write_all(ping.as_bytes()).is_err() {
                    break;
                }
                if stream.flush().is_err() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(())
}
