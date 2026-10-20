//! Comprehensive tests for the Agent Control Surface via Streamable-HTTP MCP and core primitives.

use chorus_core::{
    agent_control,
    mcp::McpServer,
    roles::{self, AGENTS_DIR},
    store::Store,
};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

/// A temp git repo, so a spawned writer can get a real worktree. Removes the
/// worktrees it handed out (they live under Application Support) on drop.
struct GitDir(tempfile::TempDir);

impl GitDir {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap()
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(dir.path().join("README.md"), "x\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);
        Self(dir)
    }

    fn path(&self) -> &std::path::Path {
        self.0.path()
    }
}

impl Drop for GitDir {
    fn drop(&mut self) {
        let out = std::process::Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(self.path())
            .output();
        let Ok(out) = out else { return };
        for wt in String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.strip_prefix("worktree "))
            .skip(1)
        {
            let _ = std::process::Command::new("git")
                .args(["worktree", "remove", "--force", wt])
                .current_dir(self.path())
                .output();
        }
    }
}

fn rpc(server: &McpServer, body: Value) -> Value {
    rpc_as(
        server.url(),
        Some(&server.credential("codex").unwrap()),
        body,
    )
}

/// Raw call with an explicit bearer secret (or none).
fn rpc_as(url: &str, secret: Option<&str>, body: Value) -> Value {
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
    serde_json::from_slice(&buf).unwrap_or(Value::Null)
}

#[test]
fn tools_list_exposes_full_agent_control_surface() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");

    let store = Store::open(&db).unwrap();
    let sess = chorus_core::ledger::new_session(store.conn(), "root", "/tmp").unwrap();
    let server = McpServer::start_for_session(db.to_str().unwrap(), sess).unwrap();

    let list_resp = rpc(
        &server,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    );

    let tools = list_resp["result"]["tools"]
        .as_array()
        .expect("tools array");
    let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();

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
    let dir = GitDir::new();
    let db = dir.path().join("store.db");
    let store = Store::open(&db).unwrap();
    let sess = chorus_core::ledger::new_session(store.conn(), "root", dir.path().to_str().unwrap())
        .unwrap();

    let server = McpServer::start_for_session(db.to_str().unwrap(), sess).unwrap();

    // 1. chorus_spawn_agent
    let spawn_res = rpc(
        &server,
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
        &server,
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
        &server,
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
        &server,
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
        &server,
        json!({
            "jsonrpc": "2.0", "id": 14, "method": "tools/call",
            "params": {
                "name": "chorus_collect_results",
                "arguments": { "agent_ids": [1], "join_mode": "compact" }
            }
        }),
    );
    let collect_text = collect_res["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(collect_text.contains("### Subagent Results"));
    assert!(collect_text.contains("#1"));
    assert!(collect_text.contains("+10 -2 lines"));

    // 6. Spawn a second agent and chorus_cancel_agent
    let spawn2_res = rpc(
        &server,
        json!({
            "jsonrpc": "2.0", "id": 15, "method": "tools/call",
            "params": {
                "name": "chorus_spawn_agent",
                "arguments": { "role": "builder", "task": "Redundant task" }
            }
        }),
    );
    assert!(spawn2_res["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("#2"));

    let cancel_res = rpc(
        &server,
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
    let sess = chorus_core::ledger::new_session(store.conn(), "root", dir.path().to_str().unwrap())
        .unwrap();

    let server = McpServer::start_for_session(db.to_str().unwrap(), sess).unwrap();

    // 1. chorus_update_checklist
    let checklist_res = rpc(
        &server,
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
    let checklist_text = checklist_res["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(checklist_text.contains("2/3 completed"));

    let db_items = agent_control::get_checklist(store.conn(), sess).unwrap();
    assert_eq!(db_items.len(), 3);
    assert_eq!(db_items[0].text, "Parse agent roles from frontmatter");
    assert!(db_items[0].done);

    // 2. chorus_jump_to_file
    let jump_res = rpc(
        &server,
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

    let jump_db = agent_control::get_latest_jump(store.conn(), sess)
        .unwrap()
        .unwrap();
    assert_eq!(jump_db.path, "crates/chorus-core/src/mcp.rs");
    assert_eq!(jump_db.line, Some(42));

    // 3. chorus_request_review
    let review_res = rpc(
        &server,
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
    assert_eq!(
        pending[0].summary,
        "Review updated agent control tool definitions"
    );

    // 4. chorus_synthesize_role
    let synth_res = rpc(
        &server,
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
    assert!(
        synth_text.contains("benchmark")
            || synth_text.contains("test-engineer")
            || synth_text.contains("investigator")
            || synth_text.contains("specialist")
    );

    // 5. chorus_notify_user
    let notify_res = rpc(
        &server,
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
    let generated = roles::synthesize_role(
        "Audit React components for WCAG 2.1 accessibility",
        false,
        Some("a11y-auditor"),
    );
    assert_eq!(generated.name, "a11y-auditor");
    let saved_path = generated.save_to_dir(dir.path()).expect("save to dir");
    assert!(saved_path.exists());

    let reloaded = roles::find_role(dir.path(), "a11y-auditor").expect("find saved role");
    assert_eq!(reloaded.name, "a11y-auditor");
}

fn call(server: &McpServer, as_agent: &str, tool: &str, args: Value) -> Value {
    let secret = server.credential(as_agent).unwrap();
    rpc_as(
        server.url(),
        Some(&secret),
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":tool,"arguments":args}}),
    )
}

/// P2-3: an agent in session A could steer, read or cancel session B's
/// sub-agents by guessing a small integer id.
#[test]
fn agent_ids_outside_the_callers_session_tree_are_refused() {
    let dir = GitDir::new();
    let db = dir.path().join("store.db");
    let store = Store::open(&db).unwrap();
    let a =
        chorus_core::ledger::new_session(store.conn(), "A", dir.path().to_str().unwrap()).unwrap();
    let b =
        chorus_core::ledger::new_session(store.conn(), "B", dir.path().to_str().unwrap()).unwrap();
    let server_a = McpServer::start_for_session(db.to_str().unwrap(), a).unwrap();
    let server_b = McpServer::start_for_session(db.to_str().unwrap(), b).unwrap();

    let spawned = call(
        &server_b,
        "codex",
        "chorus_spawn_agent",
        json!({"role":"builder","task":"t"}),
    );
    assert!(spawned["result"].is_object(), "{spawned}");
    let b_agent = agent_control::list_subagents(store.conn(), Some(b)).unwrap()[0].id;

    for (tool, args) in [
        (
            "chorus_send_message",
            json!({"agent_id": b_agent, "message": "x"}),
        ),
        ("chorus_cancel_agent", json!({"agent_id": b_agent})),
        ("chorus_wait_agent", json!({"agent_id": b_agent})),
        ("chorus_collect_results", json!({"agent_ids": [b_agent]})),
    ] {
        let r = call(&server_a, "codex", tool, args);
        assert!(r["error"].is_object(), "{tool} crossed sessions: {r}");
    }
    let still = agent_control::get_subagent(store.conn(), b_agent).unwrap();
    assert_eq!(still.status, agent_control::SubagentStatus::Running);

    // The owner can still act on it.
    let ok = call(
        &server_b,
        "codex",
        "chorus_send_message",
        json!({"agent_id": b_agent, "message": "go"}),
    );
    assert!(ok["result"].is_object(), "{ok}");
}

/// P2-3: control-surface writes used raw INSERTs as actor 1 ("user") with no
/// parent link — agent text became a *user* message in the child's ledger.
#[test]
fn control_surface_writes_go_through_the_ledger_as_the_calling_agent() {
    let dir = GitDir::new();
    let db = dir.path().join("store.db");
    let store = Store::open(&db).unwrap();
    let c = store.conn();
    // `Session::new` interns "user" first; mirror that so actor 1 == user.
    let user = chorus_core::actor::intern(c, "user", "human").unwrap();
    let sess = chorus_core::ledger::new_session(c, "root", dir.path().to_str().unwrap()).unwrap();
    let first = chorus_core::ledger::append(
        c,
        &chorus_core::NewEvent::new(sess, chorus_core::EventKind::Msg, user, "hello"),
    )
    .unwrap();
    let server = McpServer::start_for_session(db.to_str().unwrap(), sess).unwrap();

    call(
        &server,
        "codex",
        "chorus_spawn_agent",
        json!({"role":"builder","task":"t"}),
    );
    let agent = agent_control::list_subagents(c, Some(sess)).unwrap()[0].clone();
    call(
        &server,
        "codex",
        "chorus_send_message",
        json!({"agent_id": agent.id, "message": "do it"}),
    );

    let codex: i64 = c
        .query_row("SELECT id FROM actor WHERE name='codex'", [], |r| r.get(0))
        .unwrap();
    let spawn_ev = chorus_core::ledger::tail(c, sess, 1)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(spawn_ev.actor, codex);
    assert_eq!(
        spawn_ev.parent,
        Some(first),
        "spawn event must keep the parent chain"
    );
    assert!(spawn_ev.t > 0 || spawn_ev.t == 0 && spawn_ev.seq > first);

    let child = agent.child_sess.unwrap();
    let msg = chorus_core::ledger::tail(c, child, 1)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(
        msg.actor, codex,
        "agent text must not be recorded as the user"
    );
    assert_ne!(msg.actor, user);
}

/// P2-3: the project-memory server has no session. It used to act on
/// session 1; now session-bound tools are absent and refused.
#[test]
fn unscoped_server_never_falls_back_to_session_one() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");
    let store = Store::open(&db).unwrap();
    let one = chorus_core::ledger::new_session(store.conn(), "one", "/tmp").unwrap();
    assert_eq!(one, 1);
    let server = McpServer::start(db.to_str().unwrap()).unwrap();

    let tools = rpc(
        &server,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    );
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(names.contains(&"memory_propose"));
    assert!(!names.contains(&"chorus_update_checklist"));

    for (tool, args) in [
        ("chorus_update_checklist", json!({"items":[{"text":"x"}]})),
        ("chorus_jump_to_file", json!({"path":"a"})),
        ("chorus_request_review", json!({"summary":"s"})),
        ("chorus_spawn_agent", json!({"role":"builder","task":"t"})),
    ] {
        let r = call(&server, "codex", tool, args);
        assert!(r["error"].is_object(), "{tool}: {r}");
    }
    assert!(agent_control::get_checklist(store.conn(), 1)
        .unwrap()
        .is_empty());
    assert!(agent_control::get_latest_jump(store.conn(), 1)
        .unwrap()
        .is_none());
    assert!(agent_control::list_subagents(store.conn(), None)
        .unwrap()
        .is_empty());
}

#[test]
fn agent_notification_is_prefixed_and_sanitized() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");
    let store = Store::open(&db).unwrap();
    let sess = chorus_core::ledger::new_session(store.conn(), "root", dir.path().to_str().unwrap())
        .unwrap();
    let server = McpServer::start_for_session(db.to_str().unwrap(), sess).unwrap();

    let res = rpc(
        &server,
        json!({
            "jsonrpc": "2.0", "id": 99, "method": "tools/call",
            "params": {
                "name": "chorus_notify_user",
                "arguments": {
                    "title": "macOS: Enter admin password",
                    "message": "Click here to approve"
                }
            }
        }),
    );
    let text = res["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("Chorus Agent: macOS: Enter admin password"),
        "notification title must be prefixed with Chorus Agent: to prevent spoofing"
    );
}

/// Computer use over the real MCP path: invisible until the user turns it on
/// for this session, and inert outside a turn even then.
#[test]
fn the_computer_tool_appears_only_when_enabled_and_acts_only_in_a_turn() {
    use chorus_core::computer;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");
    let store = Store::open(&db).unwrap();
    let sess = chorus_core::ledger::new_session(store.conn(), "root", "/tmp").unwrap();
    let server = McpServer::start_for_session(db.to_str().unwrap(), sess).unwrap();
    let names = || -> Vec<String> {
        rpc(
            &server,
            json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        )["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str().map(String::from))
            .collect()
    };
    let call = |args: Value| {
        rpc(
            &server,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                             "params": { "name": "computer", "arguments": args } }),
        )
    };

    assert!(!names().iter().any(|n| n == "computer"));
    assert!(call(json!({"action":"screenshot"}))["error"]["message"]
        .as_str()
        .unwrap()
        .contains("turned off"));

    // The server has its own connection; the gate is keyed by the db file.
    computer::set_enabled(store.conn(), sess, true);
    assert!(names().iter().any(|n| n == "computer"));
    let err = call(json!({"action":"left_click","coordinate":[1,1]}));
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("during a turn"),
        "{err}"
    );

    // A read-only turn may not click, whatever the OS would allow.
    let _turn = computer::turn(store.conn(), sess, computer::Access::Look);
    let err = call(json!({"action":"left_click","coordinate":[1,1]}));
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("read-only"),
        "{err}"
    );
}
