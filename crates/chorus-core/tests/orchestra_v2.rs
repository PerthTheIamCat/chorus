//! Multi-agent v2 over MCP: the lead's plan holds spawns to it (F4), children
//! ask the lead and share a board (F5), one-shot opinions are capped.

use chorus_core::agent_control::{self, SubagentStatus, TaskSpec};
use chorus_core::orchestra_plan;
use chorus_core::{ledger, mcp::McpServer, store::Store};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

fn call(server: &McpServer, tool: &str, args: Value) -> Value {
    call_as(server, "codex", tool, args)
}

fn call_as(server: &McpServer, agent: &str, tool: &str, args: Value) -> Value {
    let secret = server.credential(agent).unwrap();
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

fn git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap()
    };
    run(&["init"]);
    run(&["config", "user.email", "t@test.local"]);
    run(&["config", "user.name", "T"]);
    std::fs::write(dir.path().join("README.md"), "x\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "init"]);
    dir
}

struct Lead {
    _repo: tempfile::TempDir,
    _db: tempfile::TempDir,
    store: Store,
    sess: i64,
    server: McpServer,
}

fn lead() -> Lead {
    let repo = git_repo();
    let dbdir = tempfile::tempdir().unwrap();
    let db = dbdir.path().join("store.db");
    let store = Store::open(&db).unwrap();
    let sess = ledger::new_session(store.conn(), "lead", repo.path().to_str().unwrap()).unwrap();
    let server = McpServer::start_for_session(db.to_str().unwrap(), sess).unwrap();
    Lead {
        _repo: repo,
        _db: dbdir,
        store,
        sess,
        server,
    }
}

fn plan_json() -> Value {
    json!({
        "tasks": [
            {"id":"s1","goal":"parser","files_owned":["src/parser"],"done_when":"cargo test -p parser"},
            {"id":"s2","goal":"lexer","files_owned":["src/lexer"],"done_when":"cargo test -p lexer"},
            {"id":"s3","goal":"wire them","depends_on":["s1","s2"],"files_owned":["src/main.rs"],"done_when":"cargo test"}
        ],
        "signatures": ["fn parse(&str) -> Ast"]
    })
}

#[test]
fn a_second_writer_needs_a_plan_and_then_must_follow_it() {
    let l = lead();
    let spawn = |args: Value| text(&call(&l.server, "chorus_spawn_agent", args));

    // One writer alone needs no plan.
    let a = spawn(json!({"role":"builder","goal":"first"}));
    assert!(a.contains("Spawned child agent #1"), "{a}");
    // A second one next to it does.
    let b = spawn(json!({"role":"builder","goal":"second"}));
    assert!(
        b.contains("write the plan first") && b.contains("chorus_plan_orchestra"),
        "{b}"
    );
    // Readers stay free.
    let r = spawn(json!({"role":"investigator","goal":"look","worktree":false}));
    assert!(r.contains("Spawned child agent #2"), "{r}");

    // A plan the agent got wrong is refused with the reason.
    let bad = text(&call(
        &l.server,
        "chorus_plan_orchestra",
        json!({"tasks":[{"id":"x","goal":"g","depends_on":["nope"]}]}),
    ));
    assert!(bad.contains("unknown"), "{bad}");

    let ok = text(&call(&l.server, "chorus_plan_orchestra", plan_json()));
    assert!(ok.contains("Plan recorded (3 subtasks"), "{ok}");
    // It is in the ledger and in the checklist the Task pane shows.
    assert_eq!(
        orchestra_plan::latest_plan(l.store.conn(), l.sess)
            .unwrap()
            .tasks
            .len(),
        3
    );
    let list = agent_control::get_checklist(l.store.conn(), l.sess).unwrap();
    assert_eq!(list.len(), 3);
    assert!(
        list[0].text.contains("s1") && list[0].text.contains("src/parser"),
        "{:?}",
        list[0]
    );

    // A writer must name its subtask, and an unknown one is refused.
    let c = spawn(json!({"role":"builder","goal":"x"}));
    assert!(c.contains("plan_task") && c.contains("s1"), "{c}");
    let c = spawn(json!({"role":"builder","goal":"x","plan_task":"zz"}));
    assert!(c.contains("no subtask"), "{c}");
    // s3 waits for s1 and s2.
    let c = spawn(json!({"role":"builder","goal":"x","plan_task":"s3"}));
    assert!(c.contains("depends on s1"), "{c}");

    // Cancel the plan-less first writer so the plan has the floor.
    agent_control::cancel_agent(l.store.conn(), 1, None).unwrap();

    // s1: files, done_when and signatures come from the plan.
    let s1 = spawn(json!({"role":"builder","goal":"write the parser","plan_task":"s1"}));
    assert!(s1.contains("Spawned child agent #3"), "{s1}");
    let row = agent_control::get_subagent(l.store.conn(), 3).unwrap();
    let spec = TaskSpec::split(&row.task).1.unwrap();
    assert_eq!(spec.files_owned, vec!["src/parser".to_string()]);
    assert_eq!(spec.done_when, "cargo test -p parser");
    assert_eq!(spec.signatures, vec!["fn parse(&str) -> Ast".to_string()]);
    assert!(spec.render().contains("fn parse(&str) -> Ast"));

    // The same subtask twice, or files the plan did not give it, are refused.
    let again = spawn(json!({"role":"builder","goal":"x","plan_task":"s1"}));
    assert!(again.contains("already being done by agent #3"), "{again}");
    let wide = spawn(
        json!({"role":"builder","goal":"x","plan_task":"s2","files_owned":["src/parser/x.rs"]}),
    );
    assert!(wide.contains("outside what the plan gives s2"), "{wide}");

    let s2 = spawn(json!({"role":"builder","goal":"write the lexer","plan_task":"s2"}));
    assert!(s2.contains("Spawned child agent #4"), "{s2}");

    // Once both are done, s3 may start.
    for id in [3, 4] {
        agent_control::complete_agent(l.store.conn(), id, "done", None, &[]).unwrap();
    }
    let s3 = spawn(json!({"role":"builder","goal":"wire","plan_task":"s3"}));
    assert!(s3.contains("Spawned child agent #5"), "{s3}");
    let done = spawn(json!({"role":"builder","goal":"again","plan_task":"s1"}));
    assert!(done.contains("already done"), "{done}");
}

fn child_server(l: &Lead, agent_id: i64) -> McpServer {
    let row = agent_control::get_subagent(l.store.conn(), agent_id).unwrap();
    let db = l.store.conn().path().unwrap().to_string();
    McpServer::start_for_session(&db, row.child_sess.unwrap()).unwrap()
}

#[test]
fn a_child_question_reaches_the_lead_and_the_lead_answers_it() {
    std::env::set_var("CHORUS_CHILD_ASK_WAIT_S", "3");
    let l = lead();
    let w = text(&call(
        &l.server,
        "chorus_spawn_agent",
        json!({"role":"builder","goal":"build"}),
    ));
    assert!(w.contains("#1"), "{w}");
    let child = child_server(&l, 1);

    // The child has the child tools; the lead has the plan tool, not those.
    let names = call_as_list;
    let kid = names(&child, "claude");
    assert!(
        kid.contains(&"chorus_ask_lead".to_string())
            && kid.contains(&"chorus_board_post".to_string()),
        "{kid:?}"
    );
    assert!(
        !kid.contains(&"chorus_plan_orchestra".to_string()),
        "{kid:?}"
    );
    let boss = names(&l.server, "codex");
    assert!(
        boss.contains(&"chorus_plan_orchestra".to_string()),
        "{boss:?}"
    );
    assert!(!boss.contains(&"chorus_ask_lead".to_string()), "{boss:?}");

    // The lead answers while the child waits.
    let db = l.store.conn().path().unwrap().to_string();
    let answerer = std::thread::spawn(move || {
        let store = Store::open(std::path::Path::new(&db)).unwrap();
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if !orchestra_plan::open_questions(store.conn(), 1).is_empty() {
                let lead = chorus_core::actor::intern(store.conn(), "codex", "agent").unwrap();
                agent_control::send_message(store.conn(), 1, lead, "use the v2 signature").unwrap();
                return;
            }
        }
    });
    let answer = text(&call_as(
        &child,
        "claude",
        "chorus_ask_lead",
        json!({"question":"Which signature do I keep?"}),
    ));
    answerer.join().unwrap();
    assert!(
        answer.contains("Lead answered: use the v2 signature"),
        "{answer}"
    );

    // It is a child_to_parent row, mirrored into the lead's ledger.
    let n: i64 = l
        .store
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM subagent_message WHERE direction = 'child_to_parent'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
    let thread: Vec<String> = ledger::tail(l.store.conn(), l.sess, 20)
        .unwrap()
        .iter()
        .map(|e| ledger::body(l.store.conn(), e).unwrap())
        .collect();
    assert!(
        thread
            .iter()
            .any(|b| b.contains("ถาม lead") && b.contains("Which signature")),
        "{thread:?}"
    );
    assert!(
        orchestra_plan::open_questions(l.store.conn(), l.sess).is_empty(),
        "answered"
    );
}

fn call_as_list(server: &McpServer, agent: &str) -> Vec<String> {
    let secret = server.credential(agent).unwrap();
    let addr = server
        .url()
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap()
        .to_string();
    let payload =
        serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).unwrap();
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
    let v: Value = serde_json::from_slice(&buf).unwrap();
    v["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn questions_are_capped_and_deduplicated() {
    let l = lead();
    text(&call(
        &l.server,
        "chorus_spawn_agent",
        json!({"role":"builder","goal":"build"}),
    ));
    let conn = l.store.conn();
    let child = agent_control::get_subagent(conn, 1).unwrap();
    let who = chorus_core::actor::intern(conn, "claude", "agent").unwrap();

    let first = orchestra_plan::ask_lead(conn, &child, who, "which crate?").unwrap();
    let id = match first {
        orchestra_plan::Asked::New(id) => id,
        other => panic!("{other:?}"),
    };
    // Asking the same thing again points at the open question.
    assert_eq!(
        orchestra_plan::ask_lead(conn, &child, who, "Which crate?").unwrap(),
        orchestra_plan::Asked::Duplicate(id)
    );
    // What the lead is told next turn: where each agent stands and who waits.
    let digest = orchestra_plan::lead_digest(conn, l.sess);
    assert!(
        digest.contains("#1 builder · running")
            && digest.contains("Waiting for your answer")
            && digest.contains("which crate?"),
        "{digest}"
    );
    orchestra_plan::ask_lead(conn, &child, who, "second").unwrap();
    orchestra_plan::ask_lead(conn, &child, who, "third").unwrap();
    let err = orchestra_plan::ask_lead(conn, &child, who, "fourth").unwrap_err();
    assert!(err.contains("already asked 3"), "{err}");
}

#[test]
fn the_board_is_shared_between_siblings_and_visible_in_the_thread() {
    let l = lead();
    for goal in ["a", "b"] {
        text(&call(
            &l.server,
            "chorus_spawn_agent",
            json!({"role":"investigator","goal":goal,"worktree":false}),
        ));
    }
    let (a, b) = (child_server(&l, 1), child_server(&l, 2));

    let posted = text(&call_as(
        &a,
        "claude",
        "chorus_board_post",
        json!({"note":"parse() now returns Result<Ast>"}),
    ));
    assert!(posted.contains("Posted (1/5)"), "{posted}");
    // The sibling sees it, and sees none of its own.
    let seen = text(&call_as(&b, "codex", "chorus_board_read", json!({})));
    assert!(
        seen.contains("parse() now returns Result<Ast>") && seen.contains("#1"),
        "{seen}"
    );
    let mine = text(&call_as(
        &b,
        "codex",
        "chorus_board_post",
        json!({"note":"lexer: Token is now Copy"}),
    ));
    assert!(
        mine.contains("parse() now returns") && !mine.contains("Token is now Copy"),
        "{mine}"
    );

    // The lead reads it in collect_results and in the thread.
    let collected = text(&call(&l.server, "chorus_collect_results", json!({})));
    assert!(
        collected.contains("Board") && collected.contains("Token is now Copy"),
        "{collected}"
    );
    let thread: Vec<String> = ledger::tail(l.store.conn(), l.sess, 20)
        .unwrap()
        .iter()
        .map(|e| ledger::body(l.store.conn(), e).unwrap())
        .collect();
    assert!(
        thread
            .iter()
            .any(|b| b.contains("📌 board") && b.contains("Result<Ast>")),
        "{thread:?}"
    );

    // A sixth note is refused.
    let conn = l.store.conn();
    let kid = agent_control::get_subagent(conn, 1).unwrap();
    let who = chorus_core::actor::intern(conn, "claude", "agent").unwrap();
    for i in 0..4 {
        orchestra_plan::post_board(conn, &kid, who, &format!("note {i}")).unwrap();
    }
    assert!(orchestra_plan::post_board(conn, &kid, who, "one too many").is_err());
}

#[test]
fn an_outside_opinion_is_a_read_only_child_for_another_provider_and_is_capped() {
    std::env::set_var("CHORUS_OPINION_WAIT_S", "1");
    let l = lead();
    let ask = |q: &str| {
        text(&call(
            &l.server,
            "chorus_ask_provider",
            json!({"question": q}),
        ))
    };

    let first = ask("any holes in this plan?");
    // No runner is registered in this test, so the other agent never answers;
    // what matters is how it was started.
    assert!(first.contains("has not answered yet"), "{first}");
    let kids = agent_control::list_subagents(l.store.conn(), Some(l.sess)).unwrap();
    assert_eq!(kids.len(), 1);
    assert!(kids[0].worktree.is_none(), "read-only: no worktree");
    assert_eq!(kids[0].role, "reviewer");
    let spec = TaskSpec::split(&kids[0].task).1.unwrap();
    assert_eq!(spec.prefer_provider, "other_than:codex");

    // The same question again is a loop, not a second opinion.
    let dup = ask("any holes in this plan?");
    assert!(dup.contains("already asked"), "{dup}");
    assert_eq!(
        agent_control::list_subagents(l.store.conn(), Some(l.sess))
            .unwrap()
            .len(),
        1
    );

    ask("second question");
    ask("third question");
    let capped = ask("fourth question");
    assert!(capped.contains("at most 3 outside opinions"), "{capped}");

    // An advisor cannot pass the question on.
    agent_control::complete_agent(l.store.conn(), 1, "no holes", None, &[]).unwrap();
    let advisor = child_server(&l, 1);
    let r = text(&call_as(
        &advisor,
        "claude",
        "chorus_ask_provider",
        json!({"question":"and you?"}),
    ));
    assert!(r.contains("cannot ask another one"), "{r}");
    assert_eq!(
        agent_control::get_subagent(l.store.conn(), 1)
            .unwrap()
            .status,
        SubagentStatus::Completed
    );
}
