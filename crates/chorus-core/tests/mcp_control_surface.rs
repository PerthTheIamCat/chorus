//! Comprehensive tests for the Agent Control Surface via Streamable-HTTP MCP and core primitives.

use chorus_core::{
    agent_control,
    mcp::McpServer,
    roles::{self, AgentRole, AGENTS_DIR},
    store::Store,
};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

fn rpc(url: &str, body: Value) -> Value {
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
    serde_json::from_slice(&buf).unwrap_or(Value::Null)
}

#[test]
fn tools_list_exposes_full_agent_control_surface() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");
    Store::open(&db).unwrap();

    let server = McpServer::start(db.to_str().unwrap()).unwrap();

    let list_resp = rpc(
        server.url(),
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    );

    let tools = list_resp["result"]["tools"].as_array().expect("tools array");
    let tool_names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();

    let expected = [
        "memory_propose",
        "chorus_spawn_agent",
        "chorus_send_message",
        "chorus_wait_agent",
        "chorus_collect_results",
        "chorus_cancel_agent",
        "chorus_update_checklist",
        "chorus_jump_to_file",
        "chorus_request_review",
        "chorus_synthesize_role",
        "chorus_notify_user",
    ];

    for name in expected {
        assert!(
            tool_names.contains(&name),
            "Expected tool '{name}' in tools/list. Found: {tool_names:?}"
        );
    }

    // Verify schema structure for each tool
    for tool in tools {
        let schema = &tool["inputSchema"];
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].is_object());
    }
}

#[test]
fn end_to_end_orchestration_tool_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");
    let store = Store::open(&db).unwrap();
    let sess = chorus_core::ledger::new_session(store.conn(), "root", dir.path().to_str().unwrap()).unwrap();

    let server = McpServer::start_for_session(db.to_str().unwrap(), sess).unwrap();

    // 1. chorus_spawn_agent
    let spawn_res = rpc(
        server.url(),
        json!({
            "jsonrpc": "2.0", "id": 10, "method": "tools/call",
            "params": {
                "name": "chorus_spawn_agent",
                "arguments": {
                    "role": "investigator",
                    "task": "Locate memory leaks in TCP connection reader",
                    "worktree": true
                }
            }
        }),
    );
    let spawn_text = spawn_res["result"]["content"][0]["text"].as_str().unwrap();
    assert!(spawn_text.contains("Spawned child agent #1"));
    assert!(spawn_text.contains("investigator"));

    // 2. chorus_send_message
    let msg_res = rpc(
        server.url(),
        json!({
            "jsonrpc": "2.0", "id": 11, "method": "tools/call",
            "params": {
                "name": "chorus_send_message",
                "arguments": {
                    "agent_id": 1,
                    "message": "Focus on the keep-alive loop in handle_conn"
                }
            }
        }),
    );
    let msg_text = msg_res["result"]["content"][0]["text"].as_str().unwrap();
    assert!(msg_text.contains("Message delivered to child agent #1"));

    // 3. chorus_wait_agent (still running)
    let wait_res = rpc(
        server.url(),
        json!({
            "jsonrpc": "2.0", "id": 12, "method": "tools/call",
            "params": {
                "name": "chorus_wait_agent",
                "arguments": { "agent_id": 1 }
            }
        }),
    );
    assert_eq!(wait_res["result"]["all_done"], false);

    // Complete child agent 1 using engine primitive
    agent_control::complete_agent(
        store.conn(),
        1,
        "Identified unbounded loop in keep-alive stream. Fixed with timeout.",
        Some("+10 -2 lines"),
        &["crates/chorus-core/src/mcp.rs".into()],
    )
    .unwrap();

    // 4. chorus_wait_agent (after completion)
    let wait_done = rpc(
        server.url(),
        json!({
            "jsonrpc": "2.0", "id": 13, "method": "tools/call",
            "params": {
                "name": "chorus_wait_agent",
                "arguments": { "agent_id": 1 }
            }
        }),
    );
    assert_eq!(wait_done["result"]["all_done"], true);

    // 5. chorus_collect_results
    let collect_res = rpc(
        server.url(),
        json!({
            "jsonrpc": "2.0", "id": 14, "method": "tools/call",
            "params": {
                "name": "chorus_collect_results",
                "arguments": { "agent_ids": [1], "join_mode": "compact" }
            }
        }),
    );
    let collect_text = collect_res["result"]["content"][0]["text"].as_str().unwrap();
    assert!(collect_text.contains("### Subagent Results"));
    assert!(collect_text.contains("#1"));
    assert!(collect_text.contains("+10 -2 lines"));

    // 6. Spawn a second agent and chorus_cancel_agent
    let spawn2_res = rpc(
        server.url(),
        json!({
            "jsonrpc": "2.0", "id": 15, "method": "tools/call",
            "params": {
                "name": "chorus_spawn_agent",
                "arguments": { "role": "builder", "task": "Redundant task" }
            }
        }),
    );
    assert!(spawn2_res["result"]["content"][0]["text"].as_str().unwrap().contains("#2"));

    let cancel_res = rpc(
        server.url(),
        json!({
            "jsonrpc": "2.0", "id": 16, "method": "tools/call",
            "params": {
                "name": "chorus_cancel_agent",
                "arguments": { "agent_id": 2, "reason": "No longer needed" }
            }
        }),
    );
    let cancel_text = cancel_res["result"]["content"][0]["text"].as_str().unwrap();
    assert!(cancel_text.contains("Cancelled child agent #2"));
}

#[test]
fn steering_and_task_visibility_tool_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");
    let store = Store::open(&db).unwrap();
    let sess = chorus_core::ledger::new_session(store.conn(), "root", dir.path().to_str().unwrap()).unwrap();

    let server = McpServer::start_for_session(db.to_str().unwrap(), sess).unwrap();

    // 1. chorus_update_checklist
    let checklist_res = rpc(
        server.url(),
        json!({
            "jsonrpc": "2.0", "id": 20, "method": "tools/call",
            "params": {
                "name": "chorus_update_checklist",
                "arguments": {
                    "items": [
                        { "text": "Parse agent roles from frontmatter", "done": true, "in_progress": false },
                        { "text": "Wire Streamable-HTTP MCP server", "done": true, "in_progress": false },
                        { "text": "Add unit tests", "done": false, "in_progress": true }
                    ]
                }
            }
        }),
    );
    let checklist_text = checklist_res["result"]["content"][0]["text"].as_str().unwrap();
    assert!(checklist_text.contains("2/3 completed"));

    let db_items = agent_control::get_checklist(store.conn(), sess).unwrap();
    assert_eq!(db_items.len(), 3);
    assert_eq!(db_items[0].text, "Parse agent roles from frontmatter");
    assert!(db_items[0].done);

    // 2. chorus_jump_to_file
    let jump_res = rpc(
        server.url(),
        json!({
            "jsonrpc": "2.0", "id": 21, "method": "tools/call",
            "params": {
                "name": "chorus_jump_to_file",
                "arguments": { "path": "crates/chorus-core/src/mcp.rs", "line": 42 }
            }
        }),
    );
    let jump_text = jump_res["result"]["content"][0]["text"].as_str().unwrap();
    assert!(jump_text.contains("Navigated Workbench to crates/chorus-core/src/mcp.rs:42"));

    let jump_db = agent_control::get_latest_jump(store.conn(), sess).unwrap().unwrap();
    assert_eq!(jump_db.path, "crates/chorus-core/src/mcp.rs");
    assert_eq!(jump_db.line, Some(42));

    // 3. chorus_request_review
    let review_res = rpc(
        server.url(),
        json!({
            "jsonrpc": "2.0", "id": 22, "method": "tools/call",
            "params": {
                "name": "chorus_request_review",
                "arguments": {
                    "summary": "Review updated agent control tool definitions",
                    "hunks": [
                        { "file": "src/mcp.rs", "hunk": "@@ -10,3 +10,4 @@" }
                    ]
                }
            }
        }),
    );
    let review_text = review_res["result"]["content"][0]["text"].as_str().unwrap();
    assert!(review_text.contains("Created interactive review request #1"));

    let pending = agent_control::list_reviews(store.conn(), sess, true).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].summary, "Review updated agent control tool definitions");

    // 4. chorus_synthesize_role
    let synth_res = rpc(
        server.url(),
        json!({
            "jsonrpc": "2.0", "id": 23, "method": "tools/call",
            "params": {
                "name": "chorus_synthesize_role",
                "arguments": {
                    "task": "Benchmark SQLite FTS5 query performance against LIKE scans",
                    "write_mode": false
                }
            }
        }),
    );
    let synth_text = synth_res["result"]["content"][0]["text"].as_str().unwrap();
    assert!(synth_text.contains("Synthesized custom role"));
    assert!(synth_text.contains("benchmark") || synth_text.contains("test-engineer") || synth_text.contains("investigator") || synth_text.contains("specialist"));

    // 5. chorus_notify_user
    let notify_res = rpc(
        server.url(),
        json!({
            "jsonrpc": "2.0", "id": 24, "method": "tools/call",
            "params": {
                "name": "chorus_notify_user",
                "arguments": {
                    "title": "Subagent Complete",
                    "message": "Investigator finished indexing codebase"
                }
            }
        }),
    );
    let notify_text = notify_res["result"]["content"][0]["text"].as_str().unwrap();
    assert!(notify_text.contains("Subagent Complete"));
}

#[test]
fn custom_agent_role_loading_and_synthesis() {
    let dir = tempfile::tempdir().unwrap();
    let agents_path = dir.path().join(AGENTS_DIR);
    std::fs::create_dir_all(&agents_path).unwrap();

    let custom_md = r#"---
name: security-sentinel
description: "Scans diffs for leaked API keys and memory safety violations"
tools: ["read", "grep", "diff"]
model: "claude-3-7-sonnet"
---

# Security Sentinel
Always ensure zero credentials, tokens, or plaintext secrets exist in proposed patches.
"#;
    std::fs::write(agents_path.join("security-sentinel.md"), custom_md).unwrap();

    let roles_list = roles::load_roles(dir.path());
    assert!(roles_list.len() >= 5);

    let sentinel = roles::find_role(dir.path(), "security-sentinel").expect("find sentinel");
    assert_eq!(sentinel.name, "security-sentinel");
    assert_eq!(sentinel.tools, vec!["read", "grep", "diff"]);
    assert_eq!(sentinel.model.as_deref(), Some("claude-3-7-sonnet"));
    assert!(sentinel.system_prompt.contains("zero credentials"));

    // Synthesis and save roundtrip
    let generated = roles::synthesize_role("Audit React components for WCAG 2.1 accessibility", false, Some("a11y-auditor"));
    assert_eq!(generated.name, "a11y-auditor");
    let saved_path = generated.save_to_dir(dir.path()).expect("save to dir");
    assert!(saved_path.exists());

    let reloaded = roles::find_role(dir.path(), "a11y-auditor").expect("find saved role");
    assert_eq!(reloaded.name, "a11y-auditor");
}
