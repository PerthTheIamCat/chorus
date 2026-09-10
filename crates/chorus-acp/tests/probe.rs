//! Run the probe against the mock agent end to end. This is what stops us from
//! shipping a spike that hangs the first time a real agent asks us something.

use std::path::PathBuf;
use std::process::Command;

fn bin(name: &str) -> PathBuf {
    // target/debug/deps/<test> → target/debug/<bin>
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join(name)
}

#[test]
fn probe_completes_a_full_conversation_against_the_mock_agent() {
    let dir = std::env::temp_dir().join(format!("chorus-acp-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("README.md"), "# hello\n").unwrap();
    let out = dir.join("transcript.jsonl");

    let result = Command::new(bin("acp-probe"))
        .args(["--cwd", dir.to_str().unwrap(), "--out", out.to_str().unwrap(), "--mock"])
        .output()
        .expect("run acp-probe");

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "probe failed\nstdout:{stdout}\nstderr:{stderr}");

    // It got through initialize / new / prompt / load without deadlocking.
    assert!(stdout.contains("stopReason"), "no stopReason in report:\n{stdout}");
    assert!(stdout.contains("\"end_turn\""), "prompt did not finish:\n{stdout}");

    // Q1 — load replayed history, and the probe noticed.
    assert!(stdout.contains("replay update หลัง load: 2 เฟรม"), "load replay miscounted:\n{stdout}");

    // Q2 — the permission round-trip completed and the options were captured.
    assert!(stdout.contains("session/request_permission"), "permission not seen:\n{stdout}");
    assert!(stdout.contains("kind=reject_once"), "permission options not captured:\n{stdout}");

    // Q3 — structured updates were recognised as structured.
    assert!(stdout.contains("tool_call_update"), "structured updates missing:\n{stdout}");
    assert!(stdout.contains("ได้ของมีโครงสร้าง"), "structured verdict missing:\n{stdout}");

    // The transcript is the real deliverable: every frame, both directions.
    let log = std::fs::read_to_string(&out).expect("transcript written");
    let lines: Vec<&str> = log.lines().collect();
    assert!(lines.len() >= 12, "transcript too short: {} lines", lines.len());
    assert!(lines.iter().all(|l| serde_json::from_str::<serde_json::Value>(l).is_ok()));
    assert!(log.contains("\"dir\":\"out\"") && log.contains("\"dir\":\"in\""));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn probe_refuses_to_run_without_an_agent_command() {
    let out = Command::new(bin("acp-probe")).output().expect("run acp-probe");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--mock"), "usage hint missing: {err}");
    assert!(err.contains("codex acp"), "usage hint missing: {err}");
}
