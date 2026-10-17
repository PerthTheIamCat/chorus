//! Provider subagents, replayed from fixture streams (issue 11).
//!
//! `tests/fixtures/*.jsonl` hold `session/update` frames in the shapes the
//! two bridges send (research 06: claude-agent-acp tags child frames with
//! `_meta.claudeCode.parentToolUseId`; codex-acp puts them on the child's own
//! `sessionId` between `subagent_spawned` and `subagent_state_update`). The
//! mock replays them verbatim, so the whole path runs: normalizer → ledger →
//! assembled context.
//!
//! Prose is marked: `PARENT-PROSE-*` is the parent's, `CHILD-PROSE-*` the
//! child's, `FINAL-RESULT` the child's answer. The parent must hold the first
//! and the last but never the middle one; the child must hold all of its own.

use chorus_cli::session::{Policy, Session};
use chorus_core::agent_control::{list_subagents, SubagentKind, SubagentStatus};
use chorus_core::EventKind;
use std::path::{Path, PathBuf};

fn probe_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("acp-probe")
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn replay_bridge(name: &str) -> Vec<String> {
    vec![
        probe_bin().to_string_lossy().into_owned(),
        "--serve-mock".into(),
        "--replay".into(),
        fixture(name).to_string_lossy().into_owned(),
    ]
}

struct Replayed {
    _dir: tempfile::TempDir,
    db: PathBuf,
    session: Session,
}

fn replay(name: &str, agent: &str, provider: &str) -> Replayed {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");
    let mut session = Session::create(&db, dir.path(), "fixture")
        .unwrap()
        .with_bridge(replay_bridge(name));
    session
        .run_turn(agent, provider, "check the code", Policy::AllowAll)
        .unwrap();
    Replayed {
        _dir: dir,
        db,
        session,
    }
}

fn bodies(s: &Session) -> Vec<(EventKind, String)> {
    s.transcript()
        .unwrap()
        .into_iter()
        .map(|(e, b)| (e.kind, b))
        .collect()
}

fn joined(events: &[(EventKind, String)]) -> String {
    events
        .iter()
        .map(|(_, b)| b.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The checks both bridges must pass: no child prose in the parent's ledger
/// or context, the final result in both, and one complete provider child.
fn assert_split(r: &Replayed) -> (Session, Vec<(EventKind, String)>) {
    let parent = bodies(&r.session);
    let text = joined(&parent);
    assert!(
        text.contains("PARENT-PROSE-1") && text.contains("PARENT-PROSE-2"),
        "{text}"
    );
    assert!(
        !text.contains("CHILD-PROSE"),
        "child prose leaked into the parent:\n{text}"
    );
    assert_eq!(
        text.matches("FINAL-RESULT").count(),
        1,
        "result once, as the tool result:\n{text}"
    );

    let ctx = r.session.assemble_context(200_000).unwrap();
    assert!(
        !ctx.contains("CHILD-PROSE"),
        "child prose in the parent's context:\n{ctx}"
    );
    assert!(
        ctx.contains("FINAL-RESULT"),
        "parent context lacks the child's result:\n{ctx}"
    );

    let subs = list_subagents(r.session.store().conn(), Some(r.session.session_id())).unwrap();
    assert_eq!(subs.len(), 1, "{subs:?}");
    let sub = &subs[0];
    assert_eq!(sub.kind, SubagentKind::Provider);
    assert_eq!(sub.status, SubagentStatus::Completed);
    assert!(
        sub.summary
            .as_deref()
            .unwrap_or("")
            .contains("FINAL-RESULT"),
        "{sub:?}"
    );

    let child = Session::open(&r.db, sub.child_sess.unwrap()).unwrap();
    let events = bodies(&child);
    let child_text = joined(&events);
    for want in ["CHILD-PROSE-1", "FINAL-RESULT"] {
        assert!(
            child_text.contains(want),
            "child transcript lacks {want}:\n{child_text}"
        );
    }
    assert!(!child_text.contains("PARENT-PROSE"), "{child_text}");

    // The spawn names the child session, so the sidebar can open it.
    let spawn = parent
        .iter()
        .find(|(_, b)| b.contains("subagent_spawned"))
        .expect("spawn event");
    let v: serde_json::Value = serde_json::from_str(&spawn.1).unwrap();
    assert_eq!(v["childSession"].as_i64(), sub.child_sess);
    (child, events)
}

#[test]
fn claude_task_subagent_is_split_into_its_own_session() {
    let r = replay("claude_task_subagent.jsonl", "claude", "anthropic");
    let (child, events) = assert_split(&r);
    let text = joined(&events);
    assert!(text.contains("CHILD-PROSE-2"), "{text}");
    // Both child tools, including the one whose update came without `_meta`.
    assert!(text.contains("read: Read src/auth.rs"), "{text}");
    assert!(text.contains("Read src/auth.rs → completed"), "{text}");
    assert!(text.contains("grep bcrypt → failed"), "{text}");
    assert_eq!(events[0].0, EventKind::Msg, "child starts from its task");
    assert!(events[0].1.contains("src/auth.rs hashes passwords"));
    assert_eq!(child.transcript().unwrap().len(), events.len());

    let parent = joined(&bodies(&r.session));
    assert!(
        !parent.contains("Read src/auth.rs"),
        "child tool in the parent:\n{parent}"
    );
}

#[test]
fn codex_child_session_is_split_and_keeps_its_tool_calls() {
    let r = replay("codex_subagent.jsonl", "codex", "openai");
    let (_child, events) = assert_split(&r);
    let text = joined(&events);
    assert!(
        text.contains("execute: ls tests"),
        "codex child tool calls were dropped:\n{text}"
    );
    assert!(text.contains("ls tests → completed"), "{text}");
    assert!(
        events
            .iter()
            .any(|(k, b)| *k == EventKind::Diff && b.contains("tests/README.md")),
        "child diff missing:\n{text}"
    );
    let parent = joined(&bodies(&r.session));
    assert!(!parent.contains("ls tests"), "{parent}");
    // The child's copied usage must not become the parent's context meter.
    let usage = r.session.usage().unwrap().expect("usage recorded");
    assert_eq!(usage.context_used, Some(4100));
}
