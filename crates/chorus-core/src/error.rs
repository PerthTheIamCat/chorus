use thiserror::Error;

#[derive(Debug, Error)]
pub enum ChorusError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("blob {0} not found")]
    BlobMissing(String),
    #[error("event {0} not found")]
    EventMissing(i64),
    #[error("bad schema version {found}, expected <= {expected}")]
    SchemaVersion { found: i64, expected: i64 },
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, ChorusError>;
