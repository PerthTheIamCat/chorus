//! Process lifecycle management, background detached mode, and signal handling.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Daemon execution lifecycle mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleMode {
    /// Foreground interactive execution (tied to active terminal or GUI session).
    Foreground,
    /// Background detached execution (continues running after GUI window dismisses or app quits).
    BackgroundDetached,
}

impl LifecycleMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Foreground => "foreground",
            Self::BackgroundDetached => "background",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "foreground" => Some(Self::Foreground),
            "background" | "background_detached" | "detached" => {
                Some(Self::BackgroundDetached)
            }
            _ => None,
        }
    }
}

/// Manages daemon lifecycle state, background execution mode, and graceful shutdown.
#[derive(Clone)]
pub struct LifecycleManager {
    mode: Arc<RwLock<LifecycleMode>>,
    started_ms: i64,
    shutdown_flag: Arc<AtomicBool>,
    active_sessions: Arc<AtomicUsize>,
}

impl Default for LifecycleManager {
    fn default() -> Self {
        Self::new(LifecycleMode::Foreground)
    }
}

impl LifecycleManager {
    /// Create a new LifecycleManager with the specified mode.
    pub fn new(mode: LifecycleMode) -> Self {
        Self {
            mode: Arc::new(RwLock::new(mode)),
            started_ms: now_ms(),
            shutdown_flag: Arc::new(AtomicBool::new(false)),
            active_sessions: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Trigger graceful shutdown.
    pub fn request_shutdown(&self) {
        self.shutdown_flag.store(true, Ordering::SeqCst);
    }

    /// Check if shutdown has been requested.
    pub fn is_shutting_down(&self) -> bool {
        self.shutdown_flag.load(Ordering::SeqCst)
    }

    /// Returns shared shutdown flag handle.
    pub fn shutdown_flag(&self) -> Arc<AtomicBool> {
        self.shutdown_flag.clone()
    }

    /// Set execution mode (`foreground` or `background`).
    pub fn set_mode(&self, mode: LifecycleMode) {
        let mut m = self.mode.write().unwrap();
        *m = mode;
    }

    /// Current execution mode.
    pub fn mode(&self) -> LifecycleMode {
        *self.mode.read().unwrap()
    }

    /// Increment active running session / task count.
    pub fn increment_active_sessions(&self) {
        self.active_sessions.fetch_add(1, Ordering::SeqCst);
    }

    /// Decrement active running session / task count.
    pub fn decrement_active_sessions(&self) {
        self.active_sessions.fetch_sub(1, Ordering::SeqCst);
    }

    /// Active session count.
    pub fn active_session_count(&self) -> usize {
        self.active_sessions.load(Ordering::SeqCst)
    }

    /// Produce a status snapshot.
    pub fn status_json(&self, pending_interactions: usize) -> serde_json::Value {
        let uptime_secs = (now_ms() - self.started_ms) / 1000;
        serde_json::json!({
            "status": if self.is_shutting_down() { "shutting_down" } else { "running" },
            "version": env!("CARGO_PKG_VERSION"),
            "mode": self.mode().as_str(),
            "uptime_seconds": uptime_secs,
            "active_sessions": self.active_session_count(),
            "pending_interactions": pending_interactions,
        })
    }
}

// Global hook for C signal handlers to trigger shutdown
static GLOBAL_SHUTDOWN: AtomicBool = AtomicBool::new(false);
static IS_DETACHED: AtomicBool = AtomicBool::new(false);

extern "C" fn sigint_handler(_: libc::c_int) {
    if IS_DETACHED.load(Ordering::SeqCst) {
        // In detached background mode, ignore SIGINT from terminal or window dismiss
        return;
    }
    GLOBAL_SHUTDOWN.store(true, Ordering::SeqCst);
}

extern "C" fn sigterm_handler(_: libc::c_int) {
    // SIGTERM always commands graceful shutdown
    GLOBAL_SHUTDOWN.store(true, Ordering::SeqCst);
}

/// Install signal handlers for SIGINT, SIGTERM, and ignore SIGPIPE.
pub fn install_signal_handlers(manager: LifecycleManager) {
    let mode = manager.mode();
    IS_DETACHED.store(mode == LifecycleMode::BackgroundDetached, Ordering::SeqCst);

    unsafe {
        // Ignore SIGPIPE so disconnected HTTP/SSE clients don't kill the daemon process
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        libc::signal(libc::SIGINT, sigint_handler as usize);
        libc::signal(libc::SIGTERM, sigterm_handler as usize);
    }

    // Spawn a background watcher thread that bridges GLOBAL_SHUTDOWN to the manager
    let shutdown_flag = manager.shutdown_flag();
    std::thread::spawn(move || {
        while !shutdown_flag.load(Ordering::Relaxed) {
            if GLOBAL_SHUTDOWN.load(Ordering::Relaxed) {
                shutdown_flag.store(true, Ordering::SeqCst);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
}
