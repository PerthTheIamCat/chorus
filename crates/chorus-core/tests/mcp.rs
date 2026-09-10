//! The in-process MCP server, driven over real HTTP.

use chorus_core::{mcp::McpServer, memory, store::Store};
use std::io::{Read, Write};
use std::net::TcpStream;

fn rpc(url: &str, body: serde_json::Value) -> serde_json::Value {
    use std::io::{BufRead, BufReader};
    let addr = url.trim_start_matches("http://").split('/').next().unwrap();
    let payload = serde_json::to_vec(&body).unwrap();
    let mut s = TcpStream::connect(addr).unwrap();
    write!(
        s,
        "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
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
        server.url(),
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}
        }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "chorus");

    let tools = rpc(
        server.url(),
        serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/list"
        }),
    );
    assert_eq!(tools["result"]["tools"][0]["name"], "memory_propose");

    let call = rpc(
        server.url(),
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
        server.url(),
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
