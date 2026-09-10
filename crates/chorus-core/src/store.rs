//! Top-level handle: owns the sqlite connection and applies pragmas/migrations
//! on open.

use crate::{schema, Result};
use rusqlite::Connection;
use std::path::Path;

pub struct Store {
    pub(crate) conn: Connection,
}

impl Store {
    /// Open (creating if needed) a database file on disk, in WAL mode.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        schema::migrate(&conn)?;
        Ok(Self { conn })
    }

    /// Open a private, in-memory database. Useful for tests.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        // WAL is meaningless (and rejected) for `:memory:` databases.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        schema::migrate(&conn)?;
        Ok(Self { conn })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}
