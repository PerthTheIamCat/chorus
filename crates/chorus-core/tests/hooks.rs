//! Tests for automated verification hooks.

use chorus_core::hooks::{
    find_hook, run_post_turn, run_pre_dispatch, HookContext, HookStage, HooksEngine,
};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn pre_dispatch_and_post_turn_discovery() {
    let dir = tempfile::tempdir().unwrap();
    let hooks_dir = dir.path().join(".chorus").join("hooks");
    fs::create_dir_all(&hooks_dir).unwrap();

    let pre_file = hooks_dir.join("pre-dispatch.sh");
    let post_file = hooks_dir.join("post-turn.sh");
    fs::write(&pre_file, "#!/bin/sh\nexit 0\n").unwrap();
    fs::write(&post_file, "#!/bin/sh\nexit 0\n").unwrap();

    let engine = HooksEngine::new(dir.path());
    assert!(engine.has_hook(HookStage::PreDispatch));
    assert!(engine.has_hook(HookStage::PostTurn));

    assert_eq!(
        find_hook(dir.path(), HookStage::PreDispatch),
        Some(pre_file)
    );
    assert_eq!(
        find_hook(dir.path(), HookStage::PostTurn),
        Some(post_file)
    );
}

#[test]
fn runs_pre_dispatch_hook_and_passes_environment() {
    let dir = tempfile::tempdir().unwrap();
    let hooks_dir = dir.path().join(".chorus").join("hooks");
    fs::create_dir_all(&hooks_dir).unwrap();

    let script = hooks_dir.join("pre-dispatch.sh");
    let script_content = r#"#!/bin/sh
echo "Stage: $CHORUS_HOOK_STAGE"
echo "Agent: $CHORUS_AGENT_NAME"
echo "Session: $CHORUS_SESSION_ID"
echo "Custom: $MY_CUSTOM_VAR"
exit 0
"#;
    fs::write(&script, script_content).unwrap();

    #[cfg(unix)]
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    let ctx = HookContext::new()
        .with_session(42)
        .with_agent("claude-sonnet")
        .with_turn(3)
        .with_env("MY_CUSTOM_VAR", "hello_chorus");

    let out = run_pre_dispatch(dir.path(), &ctx).unwrap().expect("hook should run");
    assert!(out.is_success());
    assert_eq!(out.exit_code, Some(0));
    assert!(out.stdout.contains("Stage: pre-dispatch"));
    assert!(out.stdout.contains("Agent: claude-sonnet"));
    assert!(out.stdout.contains("Session: 42"));
    assert!(out.stdout.contains("Custom: hello_chorus"));
    assert_eq!(out.verification_feedback(), None);
}

#[test]
fn post_turn_failure_captures_stdout_stderr_and_generates_feedback() {
    let dir = tempfile::tempdir().unwrap();
    let hooks_dir = dir.path().join(".chorus").join("hooks");
    fs::create_dir_all(&hooks_dir).unwrap();

    let script = hooks_dir.join("post-turn.sh");
    let script_content = r#"#!/bin/sh
echo "Running verification checks..."
echo "error[E0425]: cannot find value 'unresolved_symbol' in this scope"
>&2 echo "cargo check failed: 1 compile error found"
exit 1
"#;
    fs::write(&script, script_content).unwrap();

    #[cfg(unix)]
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    let ctx = HookContext::new().with_agent("codex");
    let out = run_post_turn(dir.path(), &ctx).unwrap().expect("hook should run");

    assert!(!out.is_success());
    assert_eq!(out.exit_code, Some(1));
    assert!(out.stdout.contains("error[E0425]"));
    assert!(out.stderr.contains("cargo check failed"));

    let feedback = out.verification_feedback().expect("feedback should be present on failure");
    assert!(feedback.contains("Verification Hook Failure: `post-turn`"));
    assert!(feedback.contains("code 1"));
    assert!(feedback.contains("error[E0425]"));
    assert!(feedback.contains("cargo check failed"));
    assert!(feedback.contains("Please inspect the verification failure details above"));
}

#[test]
fn returns_none_when_no_hook_is_defined() {
    let dir = tempfile::tempdir().unwrap();
    let engine = HooksEngine::new(dir.path());
    assert!(!engine.has_hook(HookStage::PreDispatch));
    assert!(!engine.has_hook(HookStage::PostTurn));

    let ctx = HookContext::new();
    let res = engine.run_post_turn(&ctx).unwrap();
    assert!(res.is_none());
}
