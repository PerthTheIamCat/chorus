//! The Antigravity bridge end to end: `CodexAgent` → `antigravity-acp` → a
//! fake `agy` on PATH that records its argv and replays a real agy 1.4.2
//! stream. Checks the two things the app depends on: the picked model and
//! effort reach agy, and agy's token counts reach the context meter.

#![cfg(unix)]

use chorus_acp::codex::{CodexAgent, CodexConfig, Permission, PermissionAsk};
use std::os::unix::fs::PermissionsExt;

const FAKE_AGY: &str = r#"#!/bin/sh
if [ "$1" = "models" ]; then
  echo "Fetching available models..." >&2
  printf 'gemini-4-argon-medium\tGemini 4 Argon (Medium)\n'
  printf 'gemini-4-argon-high\tGemini 4 Argon (High)\n'
  printf 'claude-opus-5-5-low\tClaude Opus 5.5 (Low)\n'
  exit 0
fi
printf '%s\n' "$@" > "$(dirname "$0")/argv.txt"
cat > "$(dirname "$0")/stdin.txt"
printf '%s\n' '{"event":"init","init":{"model":"gemini-4-argon"}}'
printf '%s\n' '{"event":"step_update","step_update":{"step_index":1,"state":"ACTIVE","step_type":"agent_response","text_delta":"ok"}}'
printf '%s\n' '{"event":"step_update","step_update":{"step_index":1,"state":"DONE","step_type":"agent_response","text_delta":"\n","usage":{"input_tokens":15206,"output_tokens":1,"thinking_tokens":0,"cache_read_tokens":0,"total_tokens":15207}}}'
printf '%s\n' '{"event":"result","result":{"status":"SUCCESS","response":"ok\n","usage":{"input_tokens":15206,"output_tokens":1,"thinking_tokens":0,"cache_read_tokens":0,"total_tokens":15207}}}'
"#;

#[test]
fn picked_model_and_effort_reach_agy_and_usage_reaches_the_meter() {
    let root = std::env::temp_dir().join(format!("chorus-agy-bridge-{}", std::process::id()));
    let bin = root.join("bin");
    let cwd = root.join("ws");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    let agy = bin.join("agy");
    std::fs::write(&agy, FAKE_AGY).unwrap();
    std::fs::set_permissions(&agy, std::fs::Permissions::from_mode(0o755)).unwrap();

    let path = format!("{}:/usr/bin:/bin", bin.display());
    let cfg = CodexConfig {
        command: vec![env!("CARGO_BIN_EXE_antigravity-acp").to_string()],
        mode: None,
        model: Some("gemini-4-argon[high]".into()),
        env: vec![("PATH".into(), path)],
        ..Default::default()
    };
    let mut agent = CodexAgent::start(&cwd, cfg).expect("start bridge");

    // `agy models`' own ids come back split, the stderr banner skipped.
    let ids: Vec<&str> = agent.models().iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "gemini-4-argon[medium]",
            "gemini-4-argon[high]",
            "claude-opus-5-5[low]"
        ]
    );
    assert_eq!(agent.model(), Some("gemini-4-argon[high]"));

    let mut allow = |_: &PermissionAsk| Permission::Allow;
    let out = agent.prompt("hello", &mut allow).expect("turn");
    assert!(out.message.contains("ok"), "{:?}", out.message);
    assert_eq!(out.context_used, Some(15_207));
    assert_eq!(out.context_window, Some(1_048_576));

    let argv = std::fs::read_to_string(bin.join("argv.txt")).unwrap();
    let argv: Vec<&str> = argv.lines().collect();
    assert!(
        argv.windows(4)
            .any(|w| w == ["--model", "gemini-4-argon", "--effort", "high"]),
        "{argv:?}"
    );
    assert!(
        !argv.iter().any(|a| a.contains("hello")),
        "prompt leaked into argv: {argv:?}"
    );
    assert!(std::fs::read_to_string(bin.join("stdin.txt"))
        .unwrap()
        .contains("hello"));

    drop(agent);
    let _ = std::fs::remove_dir_all(&root);
}
