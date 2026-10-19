//! Discovery and import of other tools' sessions, against fixtures written
//! in each tool's on-disk shape (see `external_sessions.rs` for sources).

use chorus_core::external_sessions::{self as ext, Provider, Root, TurnRole};
use chorus_core::{ledger, EventKind, Store};
use std::fs;
use std::path::Path;

fn write(path: &Path, lines: &[serde_json::Value]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
    fs::write(path, body).unwrap();
}

fn claude_fixture(home: &Path) -> std::path::PathBuf {
    let p = home.join(".claude/projects/-work-shop/1111-aaaa.jsonl");
    write(&p, &[
        serde_json::json!({"type":"user","isMeta":true,"sessionId":"1111-aaaa","cwd":"/work/shop",
            "message":{"role":"user","content":"<command-name>/clear</command-name>"}}),
        serde_json::json!({"type":"user","sessionId":"1111-aaaa","cwd":"/work/shop",
            "message":{"role":"user","content":"ทำหน้า order list ให้เร็วขึ้น"}}),
        serde_json::json!({"type":"assistant","sessionId":"1111-aaaa","cwd":"/work/shop",
            "message":{"role":"assistant","content":[
                {"type":"thinking","thinking":"secret plan"},
                {"type":"text","text":"ดู OrderRepo ก่อน"},
                {"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"src/repo.ts"}}]}}),
        serde_json::json!({"type":"user","sessionId":"1111-aaaa","cwd":"/work/shop",
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"findAll() loads rows one by one"}]}}),
        serde_json::json!({"type":"assistant","isSidechain":true,"sessionId":"1111-aaaa",
            "message":{"role":"assistant","content":[{"type":"text","text":"subagent chatter"}]}}),
        serde_json::json!({"type":"assistant","sessionId":"1111-aaaa","cwd":"/work/shop",
            "message":{"role":"assistant","content":[{"type":"text","text":"เจอ N+1 query"}]}}),
    ]);
    p
}

fn codex_fixture(home: &Path, compressed: bool) -> std::path::PathBuf {
    let dir = home.join(".codex/sessions/2026/10/18");
    let lines = [
        serde_json::json!({"timestamp":"2026-10-18T10:00:00.000Z","type":"session_meta",
            "payload":{"id":"019a-bbbb","session_id":"019a-bbbb","timestamp":"2026-10-18T10:00:00.000Z","cwd":"/work/api"}}),
        serde_json::json!({"timestamp":"2026-10-18T10:00:01.000Z","type":"response_item",
            "payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"system rules"}]}}),
        serde_json::json!({"timestamp":"2026-10-18T10:00:01.000Z","type":"response_item",
            "payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>\n<cwd>/work/api</cwd>"}]}}),
        serde_json::json!({"timestamp":"2026-10-18T10:00:02.000Z","type":"response_item",
            "payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"add rate limiting to /login"}]}}),
        serde_json::json!({"timestamp":"2026-10-18T10:00:02.000Z","type":"event_msg",
            "payload":{"type":"user_message","message":"add rate limiting to /login"}}),
        serde_json::json!({"timestamp":"2026-10-18T10:00:03.000Z","type":"response_item",
            "payload":{"type":"reasoning","summary":[]}}),
        serde_json::json!({"timestamp":"2026-10-18T10:00:04.000Z","type":"response_item",
            "payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"rg\",\"login\"]}","call_id":"c1"}}),
        serde_json::json!({"timestamp":"2026-10-18T10:00:05.000Z","type":"response_item",
            "payload":{"type":"function_call_output","call_id":"c1","output":"src/auth.rs:12: fn login"}}),
        serde_json::json!({"timestamp":"2026-10-18T10:00:06.000Z","type":"response_item",
            "payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Added a token bucket in auth.rs"}]}}),
    ];
    if compressed {
        let p = dir.join("rollout-2026-10-18T10-00-00-019a-bbbb.jsonl.zst");
        fs::create_dir_all(&dir).unwrap();
        let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
        fs::write(&p, zstd::encode_all(body.as_bytes(), 3).unwrap()).unwrap();
        p
    } else {
        let p = dir.join("rollout-2026-10-18T10-00-00-019a-bbbb.jsonl");
        write(&p, &lines);
        p
    }
}

fn gemini_fixture(home: &Path) -> std::path::PathBuf {
    let p = home.join(".gemini/tmp/abc123/chats/session-2026-10-18T09-00-cccc.jsonl");
    write(&p, &[
        serde_json::json!({"sessionId":"cccc","projectHash":"abc123","startTime":"2026-10-18T09:00:00Z","lastUpdated":"2026-10-18T09:05:00Z"}),
        serde_json::json!({"id":"m1","timestamp":"t","type":"user","content":[{"text":"explain the build script"}]}),
        serde_json::json!({"id":"m2","timestamp":"t","type":"gemini","content":"wrong answer",
            "toolCalls":[{"id":"x","name":"read_file","args":{"path":"build.sh"},"status":"success","timestamp":"t",
                "result":[{"functionResponse":{"response":{"output":"#!/bin/sh"}}}]}]}),
        serde_json::json!({"$rewindTo":"m2"}),
        serde_json::json!({"id":"m3","timestamp":"t","type":"gemini","content":[{"text":"It builds the FFI then the app"}]}),
        serde_json::json!({"id":"m4","timestamp":"t","type":"info","content":"Session saved"}),
    ]);
    p
}

fn roots(home: &Path) -> Vec<Root> {
    vec![
        Root { provider: Provider::Claude, dir: home.join(".claude"), account: None },
        Root { provider: Provider::Codex, dir: home.join(".codex"), account: None },
        Root { provider: Provider::Gemini, dir: home.join(".gemini"), account: Some("work".into()) },
    ]
}

#[test]
fn discovers_all_three_providers_with_title_and_cwd() {
    let home = tempfile::tempdir().unwrap();
    claude_fixture(home.path());
    codex_fixture(home.path(), true);
    gemini_fixture(home.path());

    let found = ext::discover(&roots(home.path()), 50);
    assert_eq!(found.len(), 3, "{found:#?}");
    let by = |p: Provider| found.iter().find(|s| s.provider == p).unwrap();

    let c = by(Provider::Claude);
    assert_eq!(c.id, "1111-aaaa");
    assert_eq!(c.cwd.as_deref(), Some("/work/shop"));
    assert_eq!(c.title, "ทำหน้า order list ให้เร็วขึ้น", "the /clear echo is not the title");

    let x = by(Provider::Codex);
    assert_eq!(x.id, "019a-bbbb");
    assert_eq!(x.cwd.as_deref(), Some("/work/api"));
    assert_eq!(x.title, "add rate limiting to /login", "environment_context is not the title");

    let g = by(Provider::Gemini);
    assert_eq!(g.id, "cccc");
    assert_eq!(g.agent_name(), "gemini@work");
    assert_eq!(g.title, "explain the build script");
}

#[test]
fn reads_each_format_into_clean_turns() {
    let home = tempfile::tempdir().unwrap();
    claude_fixture(home.path());
    codex_fixture(home.path(), false);
    gemini_fixture(home.path());
    let found = ext::discover(&roots(home.path()), 50);
    let turns = |p: Provider| ext::read_turns(found.iter().find(|s| s.provider == p).unwrap()).unwrap();

    let c = turns(Provider::Claude);
    let roles: Vec<TurnRole> = c.iter().map(|t| t.role).collect();
    assert_eq!(roles, [TurnRole::User, TurnRole::Agent, TurnRole::ToolCall, TurnRole::ToolResult, TurnRole::Agent]);
    assert!(c.iter().all(|t| !t.text.contains("secret plan")), "thinking is not imported");
    assert!(c.iter().all(|t| !t.text.contains("subagent chatter")), "sidechains are skipped");
    assert!(c[2].text.starts_with("Read: "));

    let x = turns(Provider::Codex);
    let users: Vec<&str> = x.iter().filter(|t| t.role == TurnRole::User).map(|t| t.text.as_str()).collect();
    assert_eq!(users, ["add rate limiting to /login"], "no developer text, no env block, no event_msg duplicate");
    assert!(x.iter().any(|t| t.role == TurnRole::ToolResult && t.text.contains("fn login")));

    let g = turns(Provider::Gemini);
    assert!(g.iter().all(|t| !t.text.contains("wrong answer")), "$rewindTo drops the rewound reply");
    assert!(g.iter().all(|t| !t.text.contains("#!/bin/sh")), "…and its tool calls");
    assert_eq!(g.last().unwrap().text, "It builds the FFI then the app");
    assert!(g.iter().all(|t| !t.text.contains("Session saved")), "info lines are CLI chrome");
}

#[test]
fn search_matches_title_then_content() {
    let home = tempfile::tempdir().unwrap();
    claude_fixture(home.path());
    codex_fixture(home.path(), true);
    gemini_fixture(home.path());
    let r = roots(home.path());

    let by_title = ext::search(&r, "RATE LIMIT", 10);
    assert_eq!(by_title.len(), 1);
    assert_eq!(by_title[0].provider, Provider::Codex);
    assert!(by_title[0].snippet.is_none());

    let by_content = ext::search(&r, "N+1", 10);
    assert_eq!(by_content.len(), 1);
    assert_eq!(by_content[0].provider, Provider::Claude);
    assert!(by_content[0].snippet.as_deref().unwrap().contains("N+1"));

    assert!(ext::search(&r, "nothing like this", 10).is_empty());
}

#[test]
fn import_once_then_append_only_new_turns() {
    let home = tempfile::tempdir().unwrap();
    let path = claude_fixture(home.path());
    gemini_fixture(home.path());
    let db = home.path().join("store.db");
    let store = Store::open(&db).unwrap();
    let c = store.conn();
    let r = roots(home.path());
    let s = ext::find(&r, Provider::Claude, "1111-aaaa").expect("found by id");
    assert!(ext::find(&r, Provider::Claude, "1111").is_none(), "a 4-char fragment is too short to trust");
    assert!(ext::find(&r, Provider::Gemini, "cccc").is_some(), "Gemini's full id is inside the file, not its name");

    let first = ext::import(c, &s, "/work/shop").unwrap();
    assert!(!first.reused);
    assert_eq!(first.events, 6, "5 turns + the handoff marker");
    let events = ledger::history(c, first.session_id, 100).unwrap();
    assert_eq!(events.last().unwrap().kind, EventKind::Handoff);

    // Same source again: same session, nothing written.
    let again = ext::import(c, &s, "/work/shop").unwrap();
    assert_eq!((again.session_id, again.events, again.reused), (first.session_id, 0, true));

    // The source grew: only the new turn is appended.
    let mut body = fs::read_to_string(&path).unwrap();
    body.push_str(&serde_json::json!({"type":"user","sessionId":"1111-aaaa",
        "message":{"role":"user","content":"แล้วเพิ่ม index ด้วย"}}).to_string());
    body.push('\n');
    fs::write(&path, body).unwrap();
    let grown = ext::import(c, &s, "/work/shop").unwrap();
    assert_eq!((grown.session_id, grown.events), (first.session_id, 2));

    // Once the conversation continues in Chorus, later source turns are
    // reported, not interleaved into it.
    let user = chorus_core::actor::intern(c, "user", "human").unwrap();
    ledger::append(c, &chorus_core::NewEvent::new(first.session_id, EventKind::Msg, user, "ต่อใน Chorus")).unwrap();
    let mut body = fs::read_to_string(&path).unwrap();
    body.push_str(&serde_json::json!({"type":"user","sessionId":"1111-aaaa",
        "message":{"role":"user","content":"ข้อความหลังแยกทาง"}}).to_string());
    body.push('\n');
    fs::write(&path, body).unwrap();
    let diverged = ext::import(c, &s, "/work/shop").unwrap();
    assert_eq!((diverged.events, diverged.not_appended), (0, 1));
}

#[test]
fn session_search_and_export_read_real_columns() {
    let home = tempfile::tempdir().unwrap();
    let store = Store::open(&home.path().join("store.db")).unwrap();
    let c = store.conn();
    let sess = ledger::new_session(c, "order list", "/work").unwrap();
    let user = chorus_core::actor::intern(c, "user", "human").unwrap();
    ledger::append(c, &chorus_core::NewEvent::new(sess, EventKind::Msg, user, "find the N+1 query")).unwrap();
    // A body past the inline limit lives in the blob store.
    let big = format!("big {}", "x".repeat(4096));
    ledger::append(c, &chorus_core::NewEvent::new(sess, EventKind::Msg, user, big.clone())).unwrap();

    let hits = chorus_core::search::search_sessions(c, "N+1", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].session_id, hits[0].actor.as_str()), (sess, "user"));

    let md = chorus_core::search::export_session_markdown(c, sess).unwrap();
    assert!(md.contains("find the N+1 query") && md.contains(&big));
}
