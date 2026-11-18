//! Memory that survives a new session: fact scopes, earlier-session lines,
//! and the history search behind `chorus_search_history`.

use chorus_core::types::{EventKind, NewEvent};
use chorus_core::{actor, history, ledger, mcp::McpServer, memory, store::Store};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

fn say(c: &rusqlite::Connection, sess: i64, who: &str, kind: EventKind, body: &str) -> i64 {
    let a = actor::intern(c, who, if who == "user" { "human" } else { "openai" }).unwrap();
    ledger::append(c, &NewEvent::new(sess, kind, a, body))
        .unwrap()
}

#[test]
fn project_reads_also_see_global_and_legacy_session_facts() {
    let s = Store::open_in_memory().unwrap();
    let c = s.conn();
    memory::add(c, 0, "ตอบเป็นภาษาไทยเสมอ").unwrap();
    memory::add(c, 1, "ใช้ pnpm ไม่ใช่ npm").unwrap();
    memory::add(c, 2, "ไฟล์นี้ห้ามแตะ migrations").unwrap();
    memory::add(c, 7, "scope อื่นที่ไม่มีใครอ่าน").unwrap();

    let top: Vec<String> = memory::top(c, 1, 10).unwrap().into_iter().map(|f| f.text).collect();
    assert_eq!(top.len(), 3, "{top:?}");
    assert!(top.iter().any(|t| t.contains("ภาษาไทย")));
    assert!(top.iter().any(|t| t.contains("migrations")));

    // Ranked retrieval sees the same set, not just scope 1.
    let rel = memory::relevant(c, 1, "ภาษาไทย", 10).unwrap();
    assert_eq!(rel.len(), 3);
    assert!(rel[0].text.contains("ภาษาไทย"));

    // The same sentence in two scopes is listed once.
    memory::add(c, 2, "ใช้ pnpm ไม่ใช่ npm").unwrap();
    assert_eq!(memory::top(c, 1, 10).unwrap().len(), 3);
    assert_eq!(memory::relevant(c, 1, "pnpm", 10).unwrap().len(), 3);

    // Other scopes still read only themselves.
    assert_eq!(memory::top(c, 7, 10).unwrap().len(), 1);
}

#[test]
fn an_approved_proposal_of_any_scope_reaches_project_readers() {
    let s = Store::open_in_memory().unwrap();
    let c = s.conn();
    for scope in [0, 1, 2] {
        let id = memory::propose(c, scope, &format!("fact for scope {scope}"), None).unwrap();
        memory::approve_with(c, id, None).unwrap();
    }
    assert_eq!(memory::top(c, 1, 10).unwrap().len(), 3);
}

#[test]
fn previous_sessions_describe_earlier_work_in_the_same_folder_only() {
    let s = Store::open_in_memory().unwrap();
    let c = s.conn();
    let old = ledger::new_session(c, "old", "/proj").unwrap();
    say(c, old, "user", EventKind::Msg, "แก้ N+1 query ในหน้า order list");
    say(c, old, "codex", EventKind::Msg, "เพิ่ม index แล้ว");
    say(
        c,
        old,
        "codex",
        EventKind::Handoff,
        r#"{"goal":"g","pending":["เขียน eager-load","รันเทสต์ order"],"constraints":[]}"#,
    );
    let other = ledger::new_session(c, "other folder", "/elsewhere").unwrap();
    say(c, other, "user", EventKind::Msg, "งานโปรเจกต์อื่น");
    let empty = ledger::new_session(c, "never spoke", "/proj").unwrap();
    let cur = ledger::new_session(c, "now", "/proj").unwrap();
    let fork = ledger::fork(c, old, 1, "branch").unwrap();
    let _ = (empty, fork);

    let prev = history::previous_sessions(c, cur, 3).unwrap();
    assert_eq!(prev.len(), 1, "{prev:?}");
    assert_eq!(prev[0].id, old);
    assert_eq!(prev[0].agent, "codex");
    assert!(prev[0].pending.contains("eager-load"));
    assert!(!prev[0].from_summary);

    let text = history::render_previous(&prev, prev[0].last_ms + 86_400_000);
    assert!(text.contains(&format!("#{old} เมื่อวาน · codex · แก้ N+1")), "{text}");
    assert!(text.contains("ค้าง: เขียน eager-load; รันเทสต์ order"), "{text}");
    // Nobody else's sessions, and it stays small.
    assert!(!text.contains("โปรเจกต์อื่น"));
    assert!(text.chars().count() < 400);

    // A session never lists itself or the session it was forked from.
    let child = ledger::fork(c, old, 1, "child").unwrap();
    assert!(history::previous_sessions(c, child, 3).unwrap().iter().all(|p| p.id != old && p.id != child));
}

#[test]
fn previous_sessions_fall_back_to_the_stored_last_turn_summary() {
    let s = Store::open_in_memory().unwrap();
    let c = s.conn();
    let old = ledger::new_session(c, "old", "/proj").unwrap();
    say(c, old, "user", EventKind::Msg, "ทำ login");
    let last = say(c, old, "claude", EventKind::Msg, "ทำ login เสร็จ เหลือ logout");
    c.execute(
        "INSERT INTO summary(seq, l1, l2, model, created_ms) VALUES (?1, 'l1', 'login done, logout left', 'extractive', 0)",
        [last],
    )
    .unwrap();
    let cur = ledger::new_session(c, "now", "/proj").unwrap();
    let prev = history::previous_sessions(c, cur, 3).unwrap();
    assert_eq!(prev[0].pending, "login done, logout left");
    assert!(prev[0].from_summary);
    assert!(history::render_previous(&prev, prev[0].last_ms).contains("ล่าสุด: login done"));
}

#[test]
fn history_search_is_scoped_to_the_project_and_skips_tool_output() {
    let s = Store::open_in_memory().unwrap();
    let c = s.conn();
    let a = ledger::new_session(c, "a", "/proj").unwrap();
    let b = ledger::new_session(c, "b", "/proj").unwrap();
    let z = ledger::new_session(c, "z", "/else").unwrap();
    say(c, a, "user", EventKind::Msg, "we use postgres connection pooling");
    say(c, a, "codex", EventKind::Decision, "decided: pgbouncer in transaction mode");
    say(c, a, "codex", EventKind::ToolResult, "postgres noisy tool output");
    say(c, z, "user", EventKind::Msg, "postgres in another project");
    say(c, b, "user", EventKind::Msg, "ใช้ Redis เป็น cache นะ");

    let hits = history::search(c, b, "postgres", 10).unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].sess, a);
    let hits = history::search(c, b, "pgbouncer transaction", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].agent, "codex");
    // Thai has no word breaks, so a middle-of-run word still has to match.
    assert_eq!(history::search(c, a, "cache", 10).unwrap().len(), 1);
    // Quotes and operators in the query are text, not FTS syntax.
    assert!(history::search(c, a, "\"postgres\" OR (", 10).is_ok());
    assert!(history::render_hits(&[]).contains("no matches"));
}

fn rpc(server: &McpServer, secret: &str, body: serde_json::Value) -> serde_json::Value {
    let addr = server.url().trim_start_matches("http://").split('/').next().unwrap().to_string();
    let payload = serde_json::to_vec(&body).unwrap();
    let mut s = TcpStream::connect(&addr).unwrap();
    write!(
        s,
        "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nAuthorization: Bearer {secret}\r\nContent-Length: {}\r\n\r\n",
        payload.len()
    )
    .unwrap();
    s.write_all(&payload).unwrap();
    let mut r = BufReader::new(s);
    let mut len = 0usize;
    loop {
        let mut h = String::new();
        r.read_line(&mut h).unwrap();
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some(v) = h.to_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).unwrap();
    serde_json::from_slice(&buf).unwrap_or(serde_json::Value::Null)
}

#[test]
fn the_mcp_server_offers_history_search_and_folds_scope_2_into_project() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("s.db");
    let sess = {
        let st = Store::open(&db).unwrap();
        let c = st.conn();
        let old = ledger::new_session(c, "old", "/proj").unwrap();
        say(c, old, "user", EventKind::Msg, "deploy runs from the release branch");
        ledger::new_session(c, "now", "/proj").unwrap()
    };
    let server = McpServer::start_for_session(db.to_str().unwrap(), sess).unwrap();
    let secret = server.credential("codex").unwrap();

    let tools = rpc(&server, &secret, serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}));
    let names: Vec<&str> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"chorus_search_history"), "{names:?}");
    let propose = tools["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "memory_propose").unwrap();
    assert!(!propose["inputSchema"]["properties"]["scope"]["description"].as_str().unwrap().contains("session"));

    let found = rpc(
        &server,
        &secret,
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"chorus_search_history","arguments":{"query":"release branch"}}}),
    );
    let text = found["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("deploy runs from the release branch") && text.starts_with('#'), "{text}");

    // The retired scope 2 is queued as a project fact (1).
    rpc(
        &server,
        &secret,
        serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"memory_propose","arguments":{"text":"keep it small","scope":2}}}),
    );
    drop(server);
    let st = Store::open(&db).unwrap();
    assert_eq!(memory::pending(st.conn()).unwrap()[0].scope, 1);
}
