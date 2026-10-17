//! The in-process MCP server, driven over real HTTP.

use chorus_core::{mcp::McpServer, memory, store::Store};
use std::io::{Read, Write};
use std::net::TcpStream;

fn rpc(server: &McpServer, body: serde_json::Value) -> serde_json::Value {
    rpc_as(server.url(), Some(&server.credential("codex").unwrap()), body)
}

/// Raw call with an explicit bearer secret (or none).
fn rpc_as(url: &str, secret: Option<&str>, body: serde_json::Value) -> serde_json::Value {
    use std::io::{BufRead, BufReader};
    let addr = url.trim_start_matches("http://").split('/').next().unwrap();
    let payload = serde_json::to_vec(&body).unwrap();
    let mut s = TcpStream::connect(addr).unwrap();
    write!(
        s,
        "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n{}Content-Length: {}\r\n\r\n",
        secret.map(|s| format!("Authorization: Bearer {s}\r\n")).unwrap_or_default(),
        payload.len()
    )
    .unwrap();
    s.write_all(&payload).unwrap();

    let mut reader = BufReader::new(s);
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).unwrap();
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some(v) = h.to_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).unwrap();
    serde_json::from_slice(&buf).unwrap_or(serde_json::Value::Null)
}

#[test]
fn an_agent_can_list_and_call_memory_propose() {
    let dir = std::env::temp_dir().join(format!("chorus-mcp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("store.db");
    let _ = std::fs::remove_file(&db);
    // Create the schema up front.
    Store::open(&db).unwrap();

    let server = McpServer::start(db.to_str().unwrap()).unwrap();

    let init = rpc(
        &server,
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}
        }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "chorus");
    // The agent must be told when to call the tool, not just that it exists.
    assert!(init["result"]["instructions"].as_str().unwrap().contains("memory_propose"));

    let tools = rpc(
        &server,
        serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/list"
        }),
    );
    assert_eq!(tools["result"]["tools"][0]["name"], "memory_propose");

    let call = rpc(
        &server,
        serde_json::json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "memory_propose", "arguments": { "text": "ใช้ pnpm ไม่ใช่ npm" } }
        }),
    );
    assert!(call["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("#1"));

    // It went to the queue, not straight into `fact`.
    let store = Store::open(&db).unwrap();
    assert!(memory::top(store.conn(), 1, 10).unwrap().is_empty());
    let pending = memory::pending(store.conn()).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].text, "ใช้ pnpm ไม่ใช่ npm");

    // An over-long proposal is rejected by the tool.
    let bad = rpc(
        &server,
        serde_json::json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "memory_propose", "arguments": { "text": "x".repeat(200) } }
        }),
    );
    assert!(
        bad["error"].is_object(),
        "over-long proposal should error: {bad}"
    );

    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}


#[test]
fn an_absurd_content_length_is_refused_not_allocated() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("s.db");
    Store::open(&db).unwrap();
    let server = McpServer::start(db.to_str().unwrap()).unwrap();
    let addr = server
        .url()
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap()
        .to_string();

    // Claim 8 GiB. The old code did `vec![0u8; content_length]` and died.
    let secret = server.credential("codex").unwrap();
    let mut s = TcpStream::connect(&addr).unwrap();
    write!(
        s,
        "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {secret}\r\nContent-Length: 8589934592\r\n\r\n"
    )
    .unwrap();
    s.flush().unwrap();

    let mut buf = String::new();
    s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let _ = s.read_to_string(&mut buf);
    assert!(buf.contains("413"), "expected 413, got: {buf:?}");
}

#[test]
fn a_browser_origin_is_turned_away() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("s.db");
    Store::open(&db).unwrap();
    let server = McpServer::start(db.to_str().unwrap()).unwrap();
    let addr = server
        .url()
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap()
        .to_string();

    let body = b"{}";
    let mut s = TcpStream::connect(&addr).unwrap();
    write!(
        s,
        "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nOrigin: https://evil.example\r\n\
         Content-Length: {}\r\n\r\n",
        body.len()
    )
    .unwrap();
    s.write_all(body).unwrap();
    s.flush().unwrap();

    let mut buf = String::new();
    s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let _ = s.read_to_string(&mut buf);
    assert!(buf.contains("403"), "expected 403, got: {buf:?}");
}

/// P2-4: loopback is reachable by every local process and user. No secret,
/// or a wrong one, gets nothing — not even `tools/list`.
#[test]
fn requests_without_the_session_secret_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("s.db");
    let store = Store::open(&db).unwrap();
    let sess = chorus_core::ledger::new_session(store.conn(), "s", "/tmp").unwrap();
    let server = McpServer::start_for_session(db.to_str().unwrap(), sess).unwrap();
    let good = server.credential("codex").unwrap();
    let list = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/list"});

    assert!(rpc_as(server.url(), None, list.clone()).is_null());
    assert!(rpc_as(server.url(), Some("not-the-secret"), list.clone()).is_null());
    let mut near = good.clone();
    near.pop();
    near.push('0');
    if near != good {
        assert!(rpc_as(server.url(), Some(&near), list.clone()).is_null());
    }
    // Another session's server secret does not work here either.
    let other = McpServer::start_for_session(db.to_str().unwrap(), sess).unwrap();
    let foreign = other.credential("codex").unwrap();
    assert!(rpc_as(server.url(), Some(&foreign), list.clone()).is_null());

    assert!(rpc_as(server.url(), Some(&good), list)["result"]["tools"].is_array());

    // The ACP entry carries the secret as an `{name, value}` header.
    let entry = server.mcp_server_entry("codex").unwrap();
    assert_eq!(entry["headers"][0]["name"], "Authorization");
    assert_eq!(entry["headers"][0]["value"], format!("Bearer {good}"));

    // Raw status is 401.
    let addr = server.url().trim_start_matches("http://").split('/').next().unwrap().to_string();
    let mut s = TcpStream::connect(&addr).unwrap();
    write!(s, "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nContent-Length: 2\r\n\r\n{{}}").unwrap();
    let mut buf = String::new();
    s.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    let _ = s.read_to_string(&mut buf);
    assert!(buf.starts_with("HTTP/1.1 401"), "{buf:?}");
}
