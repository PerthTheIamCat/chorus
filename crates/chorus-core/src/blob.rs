//! Content-addressed blob store. Payloads at or above `INLINE_MAX_BYTES`
//! live here, keyed by BLAKE3 hash (`BlobId::of`), so that storing the same
//! bytes twice — an agent re-reading the same file 5-10x per session — costs
//! one row lookup and a `refs` bump, not a second copy.

use crate::{ChorusError, BlobId, Result};
use rusqlite::{Connection, OptionalExtension};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Store `bytes`, deduplicated by content hash. If a blob with this hash
/// already exists, no new row is written and no re-compression happens —
/// only `refs` is bumped.
pub fn put(c: &Connection, bytes: &[u8]) -> Result<BlobId> {
    let id = BlobId::of(bytes);

    let existing: Option<i64> = c
        .query_row("SELECT 1 FROM blob WHERE id = ?1", [id.as_str()], |r| r.get(0))
        .optional()?;
    if existing.is_some() {
        c.execute("UPDATE blob SET refs = refs + 1 WHERE id = ?1", [id.as_str()])?;
        return Ok(id);
    }

    // Compress; if that doesn't actually shrink the payload (already-
    // compressed data, tiny inputs), store the raw bytes and record that in
    // the `zstd` column so `get` knows not to try to decode it.
    let compressed = zstd::encode_all(bytes, 3)?;
    let (stored, is_zstd): (&[u8], i64) = if compressed.len() < bytes.len() {
        (&compressed, 1)
    } else {
        (bytes, 0)
    };

    c.execute(
        "INSERT INTO blob(id, bytes, orig_len, tok, created_ms, refs, zstd)
         VALUES (?1, ?2, ?3, NULL, ?4, 1, ?5)",
        (id.as_str(), stored, bytes.len() as i64, now_ms(), is_zstd),
    )?;

    Ok(id)
}

/// Fetch and decompress (if needed) the raw bytes for a blob.
pub fn get(c: &Connection, id: &BlobId) -> Result<Vec<u8>> {
    let row: Option<(Vec<u8>, i64)> = c
        .query_row(
            "SELECT bytes, zstd FROM blob WHERE id = ?1",
            [id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (bytes, is_zstd) = row.ok_or_else(|| ChorusError::BlobMissing(id.as_str().to_string()))?;
    if is_zstd != 0 {
        Ok(zstd::decode_all(&bytes[..])?)
    } else {
        Ok(bytes)
    }
}

/// Like `get`, but interpret the bytes as UTF-8 text.
pub fn get_text(c: &Connection, id: &BlobId) -> Result<String> {
    let bytes = get(c, id)?;
    String::from_utf8(bytes).map_err(|e| ChorusError::Invalid(format!("blob {id} is not valid utf-8: {e}")))
}

pub fn token_count(c: &Connection, id: &BlobId) -> Result<Option<i64>> {
    let tok: Option<i64> = c
        .query_row("SELECT tok FROM blob WHERE id = ?1", [id.as_str()], |r| r.get(0))
        .optional()?
        .flatten();
    Ok(tok)
}

/// Cache a token count against a blob's hash so it never needs recomputing.
pub fn set_token_count(c: &Connection, id: &BlobId, n: i64) -> Result<()> {
    let updated = c.execute("UPDATE blob SET tok = ?1 WHERE id = ?2", (n, id.as_str()))?;
    if updated == 0 {
        return Err(ChorusError::BlobMissing(id.as_str().to_string()));
    }
    Ok(())
}

/// Delete blobs older than `older_than_ms` that no event references.
/// Returns how many rows were deleted.
pub fn gc(c: &Connection, older_than_ms: i64) -> Result<usize> {
    let n = c.execute(
        "DELETE FROM blob
         WHERE created_ms < ?1
           AND id NOT IN (SELECT DISTINCT blob FROM event WHERE blob IS NOT NULL)",
        [older_than_ms],
    )?;
    Ok(n)
}
