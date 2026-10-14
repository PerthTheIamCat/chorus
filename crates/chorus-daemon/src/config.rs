//! Configuration and environment preparation for `chorusd`.

use crate::error::{DaemonError, Result};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// Default socket filename relative to chorus home.
pub const DEFAULT_SOCKET_NAME: &str = "engine.sock";
/// Default HTTP fallback address.
pub const DEFAULT_HTTP_ADDR: &str = "127.0.0.1:24678";
/// Default SQLite ledger filename relative to chorus home.
pub const DEFAULT_DB_NAME: &str = "ledger.db";
/// Default Unix permissions for the engine socket file: owner-only read/write (0600).
pub const DEFAULT_SOCKET_MODE: u32 = 0o600;
/// Default Unix permissions for the ~/.chorus directory: owner-only read/write/exec (0700).
pub const DEFAULT_DIR_MODE: u32 = 0o700;

/// Return the default chorus home directory: `$CHORUS_HOME` or `~/.chorus`.
pub fn default_chorus_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CHORUS_HOME") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.trim().is_empty() {
            return PathBuf::from(home).join(".chorus");
        }
    }
    PathBuf::from(".chorus")
}

/// Expands a leading `~` to the user's home directory.
pub fn expand_tilde(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    if path_str == "~" {
        return std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("~"));
    }
    if let Some(rest) = path_str.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    path.to_path_buf()
}

/// Daemon runtime configuration.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// Unix Domain Socket path.
    pub socket_path: PathBuf,
    /// Localhost HTTP fallback bind address (e.g. "127.0.0.1:24678").
    pub http_addr: Option<String>,
    /// Path to SQLite database file.
    pub db_path: PathBuf,
    /// File mode permissions for Unix socket (default: 0600).
    pub socket_mode: u32,
    /// Directory mode permissions for chorus home (default: 0700).
    pub dir_mode: u32,
    /// Whether daemon is running in detached background mode.
    pub detached: bool,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        let chorus_dir = default_chorus_dir();
        Self {
            socket_path: chorus_dir.join(DEFAULT_SOCKET_NAME),
            http_addr: Some(DEFAULT_HTTP_ADDR.to_string()),
            db_path: chorus_dir.join(DEFAULT_DB_NAME),
            socket_mode: DEFAULT_SOCKET_MODE,
            dir_mode: DEFAULT_DIR_MODE,
            detached: false,
        }
    }
}

impl DaemonConfig {
    /// Create a new configuration with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a custom socket path.
    pub fn with_socket_path(mut self, path: impl AsRef<Path>) -> Self {
        self.socket_path = expand_tilde(path.as_ref());
        self
    }

    /// Set or disable HTTP fallback address.
    pub fn with_http_addr(mut self, addr: Option<String>) -> Self {
        self.http_addr = addr;
        self
    }

    /// Set custom database path.
    pub fn with_db_path(mut self, path: impl AsRef<Path>) -> Self {
        self.db_path = expand_tilde(path.as_ref());
        self
    }

    /// Set detached background mode flag.
    pub fn with_detached(mut self, detached: bool) -> Self {
        self.detached = detached;
        self
    }

    /// Prepare environment: ensure directory exists with 0700 mode, and clean stale socket.
    pub fn prepare_environment(&self) -> Result<()> {
        let socket_path = expand_tilde(&self.socket_path);

        // Ensure parent directory exists
        if let Some(parent) = socket_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
                let perms = std::fs::Permissions::from_mode(self.dir_mode);
                std::fs::set_permissions(parent, perms)?;
            }
        }

        // Also ensure parent directory of db_path exists
        let db_path = expand_tilde(&self.db_path);
        if let Some(parent) = db_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
                let perms = std::fs::Permissions::from_mode(self.dir_mode);
                std::fs::set_permissions(parent, perms)?;
            }
        }

        // Check if socket file already exists
        match std::fs::symlink_metadata(&socket_path) {
            Ok(meta) => {
                if !meta.file_type().is_socket() {
                    return Err(DaemonError::InvalidRequest(
                        "socket path exists and is not a Unix socket".into(),
                    ));
                }
                // Probe if another instance is listening
                match UnixStream::connect(&socket_path) {
                    Ok(_) => {
                        // Active daemon detected
                        return Err(DaemonError::SocketAlreadyInUse(socket_path));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                        std::fs::remove_file(&socket_path)?;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }

        Ok(())
    }

    /// Apply socket file permissions (0600).
    pub fn apply_socket_permissions(&self) -> Result<()> {
        let socket_path = expand_tilde(&self.socket_path);
        if socket_path.exists() {
            let perms = std::fs::Permissions::from_mode(self.socket_mode);
            std::fs::set_permissions(&socket_path, perms)?;
        }
        Ok(())
    }
}
