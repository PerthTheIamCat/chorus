//! Tests for macOS notifications and audio chime helpers.

use chorus_core::notify::{
    escape_applescript_string, notify_agent_completed, notify_task_completed,
    notify_worktree_completed, play_chime, send_notification, NotificationOptions,
    CHORUS_DISABLE_NOTIFICATIONS_ENV, DEFAULT_MACOS_CHIME,
};

#[test]
fn applescript_escaping_handles_special_characters() {
    let unescaped = r#"Error in "src/main.rs": path \foo\bar
with newline"#;
    let escaped = escape_applescript_string(unescaped);

    assert!(!escaped.contains('\n'));
    assert!(escaped.contains(r#"\"src/main.rs\""#));
    assert!(escaped.contains(r#"\\foo\\bar"#));
}

#[test]
fn notification_options_builder() {
    let options = NotificationOptions::new("Chorus", "All tests passed")
        .with_subtitle("Build Successful")
        .with_chime(true)
        .with_chime_path(DEFAULT_MACOS_CHIME);

    assert_eq!(options.title, "Chorus");
    assert_eq!(options.subtitle, Some("Build Successful".to_string()));
    assert_eq!(options.body, "All tests passed");
    assert!(options.play_chime);
    assert!(options.chime_path.is_some());
}

#[test]
fn notifications_suppressed_via_environment_variable() {
    // Ensure test suite runs silently without unexpected audio or OS dialogs
    std::env::set_var(CHORUS_DISABLE_NOTIFICATIONS_ENV, "1");

    // All notification calls should succeed cleanly without spawning osascript / afplay
    assert!(play_chime().is_ok());

    let opts = NotificationOptions::new("Test", "Body");
    assert!(send_notification(&opts).is_ok());

    assert!(notify_task_completed("Refactor storage", Some("12 files changed")).is_ok());
    assert!(notify_agent_completed("codex", Some(1), Some("Finished turn")).is_ok());
    assert!(notify_worktree_completed("feat-hooks", Some("Ready for review")).is_ok());

    std::env::remove_var(CHORUS_DISABLE_NOTIFICATIONS_ENV);
}
