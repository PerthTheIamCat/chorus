//! Structured spawns (G1) and child forking (F3): what a child is told, which
//! files it may own, and that it inherits the lead's history.

use chorus_core::agent_control::{self, TaskSpec};
use chorus_core::{ledger, mcp::McpServer, store::Store};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

fn call(server: &McpServer, tool: &str, args: Value) -> Value {
    let secret = server.credential("codex").unwrap();
    let addr = server
        .url()
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap()
        .to_string();
    let payload = serde_json::to_vec(&json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":tool,"arguments":args}
    }))
    .unwrap();
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
    serde_json::from_slice(&buf).unwrap_or(Value::Null)
}

fn text(v: &Value) -> String {
    v["result"]["content"][0]["text"]
        .as_str()
        .or_else(|| v["error"]["message"].as_str())
        .unwrap_or_default()
        .to_string()
}

#[test]
fn task_spec_round_trips_and_plain_tasks_stay_plain() {
    let spec = TaskSpec {
        goal: "fix the parser".into(),
        files_owned: vec!["src/parser.rs".into()],
        out_of_scope: vec!["src/lexer.rs".into()],
        output: "a summary".into(),
        done_when: "cargo test -p parser".into(),
        ..Default::default()
    };
    let stored = spec.encode();
    let (shown, back) = TaskSpec::split(&stored);
    assert_eq!(shown, "fix the parser");
    assert_eq!(back.as_ref(), Some(&spec));
    let r = spec.render();
    for want in ["fix the parser", "src/parser.rs", "src/lexer.rs", "cargo test -p parser"] {
        assert!(r.contains(want), "{want} missing from:\n{r}");
    }

    // A bare goal (or an old task) is stored untouched.
    let plain = TaskSpec {
        goal: "just do it".into(),
        ..Default::default()
    };
    assert_eq!(plain.encode(), "just do it");
    assert_eq!(TaskSpec::split("just do it"), ("just do it", None));
    assert_eq!(agent_control::task_text(&stored), "fix the parser");
}

#[test]
fn spawn_rejects_files_owned_that_overlap_a_running_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");
    let store = Store::open(&db).unwrap();
    let sess = ledger::new_session(store.conn(), "lead", "/tmp").unwrap();
    let server = McpServer::start_for_session(db.to_str().unwrap(), sess).unwrap();

    let a = text(&call(
        &server,
        "chorus_spawn_agent",
        json!({"role":"investigator","goal":"edit parser","files_owned":["src/parser"],"worktree":false}),
    ));
    assert!(a.contains("Spawned child agent #1"), "{a}");

    // Same directory, a file inside it, and a glob over it all clash.
    for owned in ["src/parser/mod.rs", "src/parser/**", "src"] {
        let b = text(&call(
            &server,
            "chorus_spawn_agent",
            json!({"role":"investigator","goal":"also edit","files_owned":[owned],"worktree":false}),
        ));
        assert!(
            b.contains("overlaps") && b.contains("#1"),
            "{owned} should be refused, got: {b}"
        );
    }

    // A disjoint set, and a legacy `task`-only spawn, are fine.
    let c = text(&call(
        &server,
        "chorus_spawn_agent",
        json!({"role":"investigator","goal":"edit lexer","files_owned":["src/lexer.rs"],"worktree":false}),
    ));
    assert!(c.contains("Spawned child agent #2"), "{c}");
    let d = text(&call(
        &server,
        "chorus_spawn_agent",
        json!({"role":"reviewer","task":"look around","worktree":false}),
    ));
    assert!(d.contains("Spawned child agent #3"), "{d}");

    // The fields are stored with the row, and the display text is the goal.
    let rows = agent_control::list_subagents(store.conn(), Some(sess)).unwrap();
    assert_eq!(rows.len(), 3);
    let (goal, spec) = TaskSpec::split(&rows[0].task);
    assert_eq!(goal, "edit parser");
    assert_eq!(spec.unwrap().files_owned, vec!["src/parser".to_string()]);

    // Once the sibling finishes, its files are free again.
    agent_control::complete_agent(store.conn(), rows[0].id, "done", None, &[]).unwrap();
    let e = text(&call(
        &server,
        "chorus_spawn_agent",
        json!({"role":"investigator","goal":"edit parser again","files_owned":["src/parser"],"worktree":false}),
    ));
    assert!(e.contains("Spawned child agent #4"), "{e}");
}

#[test]
fn spawn_needs_a_goal_or_a_task() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");
    let store = Store::open(&db).unwrap();
    let sess = ledger::new_session(store.conn(), "lead", "/tmp").unwrap();
    let server = McpServer::start_for_session(db.to_str().unwrap(), sess).unwrap();
    let r = text(&call(
        &server,
        "chorus_spawn_agent",
        json!({"role":"investigator","worktree":false}),
    ));
    assert!(r.contains("goal is required"), "{r}");
}

#[test]
fn a_child_is_forked_from_the_leads_latest_event() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("store.db")).unwrap();
    let c = store.conn();
    let lead = ledger::new_session(c, "lead", "/tmp").unwrap();
    let actor = chorus_core::actor::intern(c, "user", "user").unwrap();
    let mut last = 0;
    for i in 0..3 {
        last = ledger::append(
            c,
            &chorus_core::types::NewEvent::new(
                lead,
                chorus_core::types::EventKind::Msg,
                actor,
                format!("lead said {i}"),
            ),
        )
        .unwrap();
    }
    let sub = agent_control::spawn_agent(c, Some(lead), "investigator", "look", false, "/tmp")
        .unwrap();
    let child = sub.child_sess.unwrap();
    let (from, at): (i64, Option<i64>) = c
        .query_row(
            "SELECT forked_from, fork_at_seq FROM session WHERE id = ?1",
            [child],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((from, at), (lead, Some(last)));
    let history = ledger::history(c, child, 100).unwrap();
    assert_eq!(history.len(), 3, "child should see the lead's history");

    // A lead with no events yet has nothing to inherit.
    let empty = ledger::new_session(c, "empty", "/tmp").unwrap();
    let sub = agent_control::spawn_agent(c, Some(empty), "investigator", "look", false, "/tmp")
        .unwrap();
    let at: Option<i64> = c
        .query_row(
            "SELECT fork_at_seq FROM session WHERE id = ?1",
            [sub.child_sess.unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(at, None);
}
