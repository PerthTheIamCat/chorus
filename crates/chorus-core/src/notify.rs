//! macOS Native Notifications and Audio Chimes.
//!
//! Tickets 20 & 21: Dispatches system notifications and plays a subtle audio
//! chime (e.g. via `afplay /System/Library/Sounds/Ping.aiff`) when a task or
//! worktree agent completes.

use crate::Result;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;

/// Default subtle macOS system chime.
pub const DEFAULT_MACOS_CHIME: &str = "/System/Library/Sounds/Ping.aiff";

/// Environment variable to suppress notifications and chimes during automated tests or CI.
pub const CHORUS_DISABLE_NOTIFICATIONS_ENV: &str = "CHORUS_DISABLE_NOTIFICATIONS";

/// Check if notifications are disabled via environment variable.
pub fn notifications_disabled() -> bool {
    std::env::var(CHORUS_DISABLE_NOTIFICATIONS_ENV)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Notification configuration options.
#[derive(Debug, Clone)]
pub struct NotificationOptions {
    pub title: String,
    pub subtitle: Option<String>,
    pub body: String,
    pub play_chime: bool,
    pub chime_path: Option<PathBuf>,
}

impl NotificationOptions {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            body: body.into(),
            play_chime: true,
            chime_path: None,
        }
    }

    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn with_chime(mut self, chime: bool) -> Self {
        self.play_chime = chime;
        self
    }

    pub fn with_chime_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.chime_path = Some(path.into());
        self
    }
}

/// Play a subtle audio chime (non-blocking).
///
/// On macOS, uses `afplay` to play `/System/Library/Sounds/Ping.aiff`.
/// Returns `Ok(())` silently if notifications are disabled or on non-macOS systems.
pub fn play_chime() -> Result<()> {
    play_sound(Path::new(DEFAULT_MACOS_CHIME))
}

/// Play a specified audio file via `afplay` in the background (non-blocking).
pub fn play_sound(path: &Path) -> Result<()> {
    if notifications_disabled() {
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        if path.exists() {
            // Spawn detached process so audio playback doesn't block caller turn
            let _ = Command::new("afplay").arg(path).spawn();
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
    }

    Ok(())
}

/// Safely escape characters for AppleScript double-quoted strings.
pub fn escape_applescript_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\r', '\n'], " ")
}

/// Trigger a macOS system notification via AppleScript (`osascript`).
///
/// Safely escapes parameters and degrades gracefully if `osascript` is unavailable
/// or if notifications are disabled.
pub fn display_system_notification(
    title: &str,
    subtitle: Option<&str>,
    body: &str,
) -> Result<()> {
    if notifications_disabled() {
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let esc_title = escape_applescript_string(title);
        let esc_body = escape_applescript_string(body);

        let script = if let Some(sub) = subtitle {
            let esc_sub = escape_applescript_string(sub);
            format!(
                "display notification \"{esc_body}\" with title \"{esc_title}\" subtitle \"{esc_sub}\""
            )
        } else {
            format!("display notification \"{esc_body}\" with title \"{esc_title}\"")
        };

        // Best effort: execute osascript
        let _ = Command::new("osascript").arg("-e").arg(&script).output();
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (title, subtitle, body);
    }

    Ok(())
}

/// Send a complete notification with optional chime.
pub fn send_notification(options: &NotificationOptions) -> Result<()> {
    if notifications_disabled() {
        return Ok(());
    }

    if options.play_chime {
        if let Some(chime_path) = &options.chime_path {
            play_sound(chime_path)?;
        } else {
            play_chime()?;
        }
    }

    display_system_notification(
        &options.title,
        options.subtitle.as_deref(),
        &options.body,
    )
}

/// Alias for `send_notification`.
pub fn notify(options: &NotificationOptions) -> Result<()> {
    send_notification(options)
}

/// Trigger a notification when a user task finishes.
pub fn notify_task_completed(task_title: &str, summary: Option<&str>) -> Result<()> {
    let body = summary.unwrap_or(task_title);
    let options = NotificationOptions::new("Chorus", body)
        .with_subtitle("Task Completed")
        .with_chime(true);
    send_notification(&options)
}

/// Trigger a notification when an agent completes a run.
pub fn notify_agent_completed(
    agent_name: &str,
    session_id: Option<i64>,
    summary: Option<&str>,
) -> Result<()> {
    let subtitle = format!("Agent '{agent_name}' Finished");
    let default_body = if let Some(sid) = session_id {
        format!("Session #{sid}: turn finished successfully")
    } else {
        format!("{agent_name} completed turn successfully")
    };
    let body = summary.unwrap_or(&default_body);

    let options = NotificationOptions::new("Chorus", body)
        .with_subtitle(subtitle)
        .with_chime(true);
    send_notification(&options)
}

/// Trigger a notification when worktree agent work is ready.
pub fn notify_worktree_completed(worktree_name: &str, summary: Option<&str>) -> Result<()> {
    let subtitle = format!("Worktree '{worktree_name}' Ready");
    let default_body = format!("Changes in worktree '{worktree_name}' ready for diff review");
    let body = summary.unwrap_or(&default_body);

    let options = NotificationOptions::new("Chorus", body)
        .with_subtitle(subtitle)
        .with_chime(true);
    send_notification(&options)
}
