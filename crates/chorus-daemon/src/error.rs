//! Error types for the Chorus daemon.

use std::path::PathBuf;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, DaemonError>;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("Chorus core error: {0}")]
    Chorus(#[from] chorus_core::ChorusError),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Socket already in use by active daemon at {0}")]
    SocketAlreadyInUse(PathBuf),

    #[error("Interaction not found: {0}")]
    InteractionNotFound(String),

    #[error("Interaction already resolved: {0}")]
    InteractionAlreadyResolved(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("HTTP protocol error: {0}")]
    HttpProtocol(String),

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Timeout waiting for resolution: {0}")]
    Timeout(String),

    #[error("{0}")]
    Other(String),
}
