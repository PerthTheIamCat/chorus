//! Actor interning: names/providers are stored once in `actor` and referenced
//! everywhere else by integer id — never repeated as strings.

use crate::Result;
use rusqlite::{Connection, OptionalExtension};

/// Look up or create the actor row for `name`, returning its id. Calling
/// this twice with the same name is a no-op the second time and returns the
/// same id (an upsert, not an insert-or-error).
pub fn intern(c: &Connection, name: &str, provider: &str) -> Result<i64> {
    c.execute(
        "INSERT INTO actor(name, provider) VALUES (?1, ?2)
         ON CONFLICT(name) DO UPDATE SET provider = excluded.provider",
        (name, provider),
    )?;
    let id: i64 = c.query_row("SELECT id FROM actor WHERE name = ?1", [name], |r| r.get(0))?;
    Ok(id)
}

/// Look up an actor's name by id.
pub fn name(c: &Connection, id: i64) -> Result<String> {
    let name: Option<String> = c
        .query_row("SELECT name FROM actor WHERE id = ?1", [id], |r| r.get(0))
        .optional()?;
    match name {
        Some(n) => Ok(n),
        None => Err(crate::ChorusError::Invalid(format!("no actor with id {id}"))),
    }
}
