//! The Codex adapter, exercised against the in-repo mock agent — no network,
//! no API key, no bridge. Confirms the update stream folds into a
//! `TurnOutcome` and the permission callback is actually consulted.

use chorus_acp::codex::{AgentEvent, CodexAgent, CodexConfig, Permission, PermissionAsk};
use std::path::PathBuf;

/// `target/debug/deps/<test>` → `target/debug/acp-probe` (which serves the
/// mock via `--serve-mock`).
fn mock_command() -> Vec<String> {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    let probe = p.join("acp-probe");
    vec![probe.to_string_lossy().into_owned(), "--serve-mock".into()]
}

fn config() -> CodexConfig {
    CodexConfig { command: mock_command(), ..Default::default() }
}

#[test]
fn start_then_prompt_folds_the_update_stream_into_an_outcome() {
    let dir = std::env::temp_dir().join(format!("chorus-codex-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let mut agent = CodexAgent::start(&dir, config()).expect("start against mock");
    assert_eq!(agent.session_id(), "mock-session-1");

    let mut allow_all = |_: &PermissionAsk| Permission::Allow;
    let outcome = agent.prompt("look at the readme", &mut allow_all).expect("prompt");

    assert_eq!(outcome.stop_reason, "end_turn");
    assert!(outcome.message.contains("กำลังดูไฟล์"), "message text lost: {:?}", outcome.message);

    // The structured update stream became structured events, in order.
    let kinds: Vec<&str> = outcome
        .events
        .iter()
        .map(|e| match e {
            AgentEvent::Message(_) => "msg",
            AgentEvent::ToolCall { .. } => "call",
            AgentEvent::ToolResult { .. } => "result",
            AgentEvent::Diff { .. } => "diff",
        })
        .collect();
    assert!(kinds.contains(&"call"), "no tool call: {kinds:?}");
    assert!(kinds.contains(&"diff"), "diff content not extracted: {kinds:?}");

    match outcome.events.iter().find(|e| matches!(e, AgentEvent::Diff { .. })) {
        Some(AgentEvent::Diff { path, new, .. }) => {
            assert!(path.ends_with("README.md"));
            assert_eq!(new, "# new");
        }
        _ => panic!("expected a Diff event"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn permission_callback_is_consulted_and_its_decision_is_sent() {
    let dir = std::env::temp_dir().join(format!("chorus-codex-deny-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let mut agent = CodexAgent::start(&dir, config()).expect("start");

    let mut asks: Vec<String> = Vec::new();
    let mut deny = |ask: &PermissionAsk| {
        asks.push(ask.tool.clone());
        Permission::Deny
    };
    let outcome = agent.prompt("do the thing", &mut deny).expect("prompt");

    // The mock asks once, to read README.md.
    assert_eq!(asks, vec!["Read README.md"]);
    // The mock marks the tool call failed when we don't allow it.
    assert!(
        outcome
            .events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolResult { status, .. } if status == "failed")),
        "deny should have failed the tool call: {:?}",
        outcome.events
    );

    let _ = std::fs::remove_dir_all(&dir);
}
