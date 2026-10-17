//! Client helper for communicating with `chorusd` over Unix socket or HTTP.

use crate::error::{DaemonError, Result};
use crate::events::DaemonEvent;
use crate::interaction::{
    CreateInteractionRequest, Interaction, ResolveInteractionRequest,
};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Largest response body the client will buffer. The server's
/// `Content-Length` is not trusted: whatever answers on the socket could
/// otherwise make the app allocate without bound.
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Transport target for the daemon client.
#[derive(Debug, Clone)]
pub enum ClientTarget {
    UnixSocket(PathBuf),
    Http(String),
}

/// Client helper for interacting with a running `chorusd` instance.
#[derive(Debug, Clone)]
pub struct DaemonClient {
    target: ClientTarget,
    timeout: Duration,
}

impl DaemonClient {
    /// Connect to daemon via Unix domain socket.
    pub fn from_uds(path: impl AsRef<Path>) -> Self {
        Self {
            target: ClientTarget::UnixSocket(path.as_ref().to_path_buf()),
            timeout: Duration::from_secs(10),
        }
    }

    /// Connect to daemon via HTTP address (e.g. "127.0.0.1:24678").
    pub fn from_http(addr: impl Into<String>) -> Self {
        let mut s = addr.into();
        if let Some(stripped) = s.strip_prefix("http://") {
            s = stripped.to_string();
        }
        Self {
            target: ClientTarget::Http(s),
            timeout: Duration::from_secs(10),
        }
    }

    /// Set communication timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn open_stream(&self) -> Result<Box<dyn ReadWrite>> {
        match &self.target {
            ClientTarget::UnixSocket(p) => {
                let stream = UnixStream::connect(p)?;
                stream.set_read_timeout(Some(self.timeout))?;
                stream.set_write_timeout(Some(self.timeout))?;
                Ok(Box::new(stream))
            }
            ClientTarget::Http(addr) => {
                let stream = TcpStream::connect(addr)?;
                stream.set_read_timeout(Some(self.timeout))?;
                stream.set_write_timeout(Some(self.timeout))?;
                Ok(Box::new(stream))
            }
        }
    }

    /// Send an HTTP request and receive status code and response body.
    pub fn request(
        &self,
        method: &str,
        path: &str,
        extra_headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<(u16, Vec<u8>)> {
        let mut stream = self.open_stream()?;

        let body_len = body.map(|b| b.len()).unwrap_or(0);
        let mut req_str = format!("{method} {path} HTTP/1.1\r\nContent-Length: {body_len}\r\nConnection: close\r\n");
        for (k, v) in extra_headers {
            req_str.push_str(&format!("{k}: {v}\r\n"));
        }
        if body.is_some() {
            req_str.push_str("Content-Type: application/json\r\n");
        }
        req_str.push_str("\r\n");

        stream.write_all(req_str.as_bytes())?;
        if let Some(b) = body {
            stream.write_all(b)?;
        }
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut status_line = String::new();
        if reader.read_line(&mut status_line)? == 0 {
            return Err(DaemonError::ConnectionClosed);
        }

        let code = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse::<u16>().ok())
            .unwrap_or(500);

        // Read headers
        let mut content_len: Option<usize> = None;
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h)? == 0 {
                break;
            }
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                if k.trim().eq_ignore_ascii_case("content-length") {
                    content_len = v.trim().parse().ok();
                }
            }
        }

        // Read body
        let mut resp_body = Vec::new();
        if let Some(len) = content_len {
            if len > MAX_RESPONSE_BYTES {
                return Err(DaemonError::HttpProtocol(format!(
                    "response Content-Length {len} exceeds {MAX_RESPONSE_BYTES} bytes"
                )));
            }
            resp_body.resize(len, 0);
            reader.read_exact(&mut resp_body)?;
        } else {
            reader
                .by_ref()
                .take(MAX_RESPONSE_BYTES as u64 + 1)
                .read_to_end(&mut resp_body)?;
            if resp_body.len() > MAX_RESPONSE_BYTES {
                return Err(DaemonError::HttpProtocol(format!(
                    "response body exceeds {MAX_RESPONSE_BYTES} bytes"
                )));
            }
        }

        Ok((code, resp_body))
    }

    /// Check if daemon is reachable.
    pub fn ping(&self) -> Result<bool> {
        match self.request("GET", "/ping", &[], None) {
            Ok((200, _)) => Ok(true),
            _ => Ok(false),
        }
    }

    /// Fetch daemon status JSON.
    pub fn status(&self) -> Result<Value> {
        let (code, body) = self.request("GET", "/lifecycle/status", &[], None)?;
        if code == 200 {
            Ok(serde_json::from_slice(&body)?)
        } else {
            Err(DaemonError::HttpProtocol(format!("status returned HTTP {code}")))
        }
    }

    /// Change lifecycle mode.
    pub fn set_lifecycle_mode(&self, mode: &str) -> Result<Value> {
        let payload = serde_json::to_vec(&serde_json::json!({ "mode": mode }))?;
        let (code, body) = self.request("POST", "/lifecycle/mode", &[], Some(&payload))?;
        if code == 200 {
            Ok(serde_json::from_slice(&body)?)
        } else {
            Err(DaemonError::HttpProtocol(format!("set mode returned HTTP {code}")))
        }
    }

    /// Command daemon to gracefully shut down.
    pub fn shutdown(&self) -> Result<()> {
        let (code, _) = self.request("POST", "/shutdown", &[], None)?;
        if code == 200 {
            Ok(())
        } else {
            Err(DaemonError::HttpProtocol(format!("shutdown returned HTTP {code}")))
        }
    }

    /// Create a pending interaction.
    pub fn create_interaction(&self, req: &CreateInteractionRequest) -> Result<Interaction> {
        let payload = serde_json::to_vec(req)?;
        let (code, body) = self.request("POST", "/interactions", &[], Some(&payload))?;
        if code == 201 {
            Ok(serde_json::from_slice(&body)?)
        } else {
            Err(DaemonError::HttpProtocol(format!("create interaction returned HTTP {code}")))
        }
    }

    /// List interactions.
    pub fn list_interactions(
        &self,
        status: Option<&str>,
        session_id: Option<i64>,
    ) -> Result<Vec<Interaction>> {
        let mut path = String::from("/interactions");
        let mut query = Vec::new();
        if let Some(s) = status {
            query.push(format!("status={s}"));
        }
        if let Some(id) = session_id {
            query.push(format!("session_id={id}"));
        }
        if !query.is_empty() {
            path.push('?');
            path.push_str(&query.join("&"));
        }

        let (code, body) = self.request("GET", &path, &[], None)?;
        if code == 200 {
            Ok(serde_json::from_slice(&body)?)
        } else {
            Err(DaemonError::HttpProtocol(format!("list interactions returned HTTP {code}")))
        }
    }

    /// Fetch interaction by ID.
    pub fn get_interaction(&self, id: &str) -> Result<Option<Interaction>> {
        let path = format!("/interactions/{id}");
        let (code, body) = self.request("GET", &path, &[], None)?;
        if code == 200 {
            Ok(Some(serde_json::from_slice(&body)?))
        } else if code == 404 {
            Ok(None)
        } else {
            Err(DaemonError::HttpProtocol(format!("get interaction returned HTTP {code}")))
        }
    }

    /// Resolve an interaction.
    pub fn resolve_interaction(
        &self,
        id: &str,
        req: &ResolveInteractionRequest,
    ) -> Result<Interaction> {
        let path = format!("/interactions/{id}/resolve");
        let payload = serde_json::to_vec(req)?;
        let (code, body) = self.request("POST", &path, &[], Some(&payload))?;
        if code == 200 {
            Ok(serde_json::from_slice(&body)?)
        } else if code == 404 {
            Err(DaemonError::InteractionNotFound(id.to_string()))
        } else if code == 409 {
            Err(DaemonError::InteractionAlreadyResolved(id.to_string()))
        } else {
            Err(DaemonError::HttpProtocol(format!("resolve interaction returned HTTP {code}")))
        }
    }

    /// Publish a custom event to the daemon.
    pub fn publish_event(
        &self,
        kind: &str,
        body_text: &str,
        session_id: Option<i64>,
        actor: Option<&str>,
    ) -> Result<DaemonEvent> {
        let payload = serde_json::to_vec(&serde_json::json!({
            "kind": kind,
            "body": body_text,
            "session_id": session_id,
            "actor": actor,
        }))?;
        let (code, body) = self.request("POST", "/events", &[], Some(&payload))?;
        if code == 201 {
            Ok(serde_json::from_slice(&body)?)
        } else {
            Err(DaemonError::HttpProtocol(format!("publish event returned HTTP {code}")))
        }
    }

    /// Open an SSE streaming connection.
    pub fn open_events_stream(
        &self,
        last_event_id: Option<i64>,
        session_id: Option<i64>,
    ) -> Result<EventStreamReader> {
        let mut stream = self.open_stream()?;

        let mut path = String::from("/events");
        if let Some(sess) = session_id {
            path.push_str(&format!("?session_id={sess}"));
        }

        let mut req_str = format!("GET {path} HTTP/1.1\r\nAccept: text/event-stream\r\n");
        if let Some(id) = last_event_id {
            req_str.push_str(&format!("Last-Event-ID: {id}\r\n"));
        }
        req_str.push_str("\r\n");

        stream.write_all(req_str.as_bytes())?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut status_line = String::new();
        reader.read_line(&mut status_line)?;

        if !status_line.contains("200") {
            return Err(DaemonError::HttpProtocol(format!("SSE request failed: {status_line}")));
        }

        // Consume headers until blank line
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h)? == 0 {
                break;
            }
            if h.trim().is_empty() {
                break;
            }
        }

        Ok(EventStreamReader { reader })
    }

    /// Initiate a QR pairing session on the daemon.
    pub fn initiate_pairing(&self) -> Result<Value> {
        let (code, body) = self.request("GET", "/api/pairing/initiate", &[], None)?;
        if code == 200 {
            Ok(serde_json::from_slice(&body)?)
        } else {
            Err(DaemonError::HttpProtocol(format!("initiate pairing returned HTTP {code}")))
        }
    }

    /// List paired companion devices.
    pub fn list_paired_devices(&self) -> Result<Vec<Value>> {
        let (code, body) = self.request("GET", "/api/pairing/devices", &[], None)?;
        if code == 200 {
            Ok(serde_json::from_slice(&body)?)
        } else {
            Err(DaemonError::HttpProtocol(format!("list devices returned HTTP {code}")))
        }
    }

    /// Revoke a paired companion device.
    pub fn revoke_paired_device(&self, device_id: &str) -> Result<bool> {
        let payload = serde_json::to_vec(&serde_json::json!({ "device_id": device_id }))?;
        let (code, body) = self.request("POST", "/api/pairing/revoke", &[], Some(&payload))?;
        if code == 200 {
            let res: Value = serde_json::from_slice(&body)?;
            Ok(res.get("ok").and_then(|v| v.as_bool()).unwrap_or(false))
        } else {
            Err(DaemonError::HttpProtocol(format!("revoke device returned HTTP {code}")))
        }
    }
}

/// Helper trait for readable/writable streams.
pub trait ReadWrite: Read + Write + Send {}
impl ReadWrite for UnixStream {}
impl ReadWrite for TcpStream {}

/// Reader yielding parsed `DaemonEvent` instances from an SSE stream.
pub struct EventStreamReader {
    reader: BufReader<Box<dyn ReadWrite>>,
}

impl EventStreamReader {
    /// Read the next event from the SSE stream (blocking).
    pub fn next_event(&mut self) -> Result<Option<DaemonEvent>> {
        let mut _event_type = String::new();
        let mut _event_id = 0i64;
        let mut data_lines = Vec::new();

        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                return Ok(None); // EOF
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                // Dispatch event boundary
                if !data_lines.is_empty() {
                    let data_combined = data_lines.join("\n");
                    if let Ok(event) = serde_json::from_str::<DaemonEvent>(&data_combined) {
                        return Ok(Some(event));
                    }
                    data_lines.clear();
                }
                continue;
            }

            if trimmed.starts_with(':') {
                // Comment/keepalive line, ignore
                continue;
            }

            if let Some(rest) = trimmed.strip_prefix("id:") {
                _event_id = rest.trim().parse().unwrap_or(0);
            } else if let Some(rest) = trimmed.strip_prefix("event:") {
                _event_type = rest.trim().to_string();
            } else if let Some(rest) = trimmed.strip_prefix("data:") {
                data_lines.push(rest.trim().to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    /// Serve one canned HTTP response on a fresh socket and return a client for it.
    fn serve_once(resp: &'static [u8]) -> (tempfile::TempDir, DaemonClient) {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("d.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let _ = s.write_all(resp);
            }
        });
        (dir, DaemonClient::from_uds(&sock).with_timeout(Duration::from_secs(2)))
    }

    #[test]
    fn huge_content_length_is_refused_without_allocating() {
        let (_d, c) = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 999999999999\r\n\r\n");
        let e = c.request("GET", "/x", &[], None).unwrap_err();
        assert!(e.to_string().contains("exceeds"), "{e}");
    }

    #[test]
    fn small_response_still_reads() {
        let (_d, c) = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
        let (code, body) = c.request("GET", "/x", &[], None).unwrap();
        assert_eq!((code, body.as_slice()), (200, &b"ok"[..]));
    }
}
