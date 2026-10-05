//! Session-scoped attachment persistence. Original bytes live in the shared
//! content-addressed blob store; attachment rows carry ownership and lifecycle.

use crate::{blob, ledger, BlobId, ChorusError, NewEvent, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_FILE_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_MESSAGE_BYTES: usize = 50 * 1024 * 1024;
pub const MAX_PER_MESSAGE: usize = 5;

/// RFC 4648 standard base64 encoder used for MCP image content and ACP blocks.
pub fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = ((chunk[0] as u32) << 16)
            | ((chunk.get(1).copied().unwrap_or(0) as u32) << 8)
            | chunk.get(2).copied().unwrap_or(0) as u32;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[derive(Debug, Clone)]
pub struct Attachment {
    pub id: i64,
    pub session_id: i64,
    pub name: String,
    pub media_type: String,
    pub size: i64,
    pub hash: BlobId,
    pub prepared_hash: Option<BlobId>,
    pub prepared_media_type: Option<String>,
    /// `staged`, `sent`, or `deleted`.
    pub status: String,
    pub event_seq: Option<i64>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Attachment> {
    Ok(Attachment {
        id: row.get(0)?,
        session_id: row.get(1)?,
        name: row.get(2)?,
        media_type: row.get(3)?,
        size: row.get(4)?,
        hash: BlobId(row.get(5)?),
        prepared_hash: row.get::<_, Option<String>>(6)?.map(BlobId),
        prepared_media_type: row.get(7)?,
        status: row.get(8)?,
        event_seq: row.get(9)?,
    })
}

const SELECT: &str = "SELECT a.id,a.sess,a.name,a.media_type,a.size,a.hash,a.prepared_hash,a.prepared_media_type,a.status,e.event_seq FROM attachment a LEFT JOIN attachment_event e ON e.attachment_id=a.id";

/// Copy bytes into session-owned storage as soon as they are selected.
pub fn import_staged(
    conn: &Connection,
    session_id: i64,
    name: &str,
    media_type: Option<&str>,
    bytes: &[u8],
) -> Result<Attachment> {
    import_staged_prepared(conn, session_id, name, media_type, bytes, None, None)
}

/// Import original bytes and optional locally prepared content. A normalized
/// image or extracted PDF text is stored separately so the original survives.
pub fn import_staged_prepared(
    conn: &Connection,
    session_id: i64,
    name: &str,
    media_type: Option<&str>,
    bytes: &[u8],
    derived_text: Option<&str>,
    normalized_image: Option<(&str, &[u8])>,
) -> Result<Attachment> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ChorusError::Invalid(format!(
            "attachment exceeds {} MiB",
            MAX_FILE_BYTES / 1024 / 1024
        )));
    }
    if name.is_empty() || name.len() > 1024 {
        return Err(ChorusError::Invalid(
            "attachment name must be 1–1024 bytes".into(),
        ));
    }
    let media_type = media_type
        .filter(|s| !s.is_empty())
        .unwrap_or("application/octet-stream");
    let tx = conn.unchecked_transaction()?;
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM session WHERE id=?1)",
        [session_id],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(ChorusError::Invalid(format!("no session {session_id}")));
    }
    let hash = blob::put(&tx, bytes)?;
    if derived_text.is_some() && normalized_image.is_some() {
        return Err(ChorusError::Invalid(
            "provide extracted text or normalized image, not both".into(),
        ));
    }
    let prepared = if let Some(text) = derived_text {
        if text.len() > MAX_FILE_BYTES {
            return Err(ChorusError::Invalid("prepared text exceeds 20 MiB".into()));
        }
        Some((blob::put(&tx, text.as_bytes())?, "text/plain".to_string()))
    } else if let Some((mime, image)) = normalized_image {
        if image.len() > MAX_FILE_BYTES {
            return Err(ChorusError::Invalid(
                "normalized image exceeds 20 MiB".into(),
            ));
        }
        Some((blob::put(&tx, image)?, mime.to_string()))
    } else {
        None
    };
    tx.execute("INSERT INTO attachment(sess,name,media_type,size,hash,prepared_hash,prepared_media_type,status,created_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,'staged',?8)", params![session_id,name,media_type,bytes.len() as i64,hash.as_str(),prepared.as_ref().map(|p| p.0.as_str()),prepared.as_ref().map(|p| p.1.as_str()),now_ms()])?;
    let id = tx.last_insert_rowid();
    tx.commit()?;
    get(conn, session_id, id)
}

/// Return prepared agent content when available, otherwise original bytes.
pub fn get_for_agent(
    conn: &Connection,
    session_id: i64,
    attachment_id: i64,
) -> Result<(Attachment, Vec<u8>, String)> {
    let (meta, original) = read(conn, session_id, attachment_id)?;
    if let Some(hash) = &meta.prepared_hash {
        let bytes = blob::get(conn, hash)?;
        return Ok((
            meta.clone(),
            bytes,
            meta.prepared_media_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".into()),
        ));
    }
    let mime = meta.media_type.clone();
    Ok((meta, original, mime))
}

pub fn list(conn: &Connection, session_id: i64) -> Result<Vec<Attachment>> {
    let mut stmt = conn.prepare(&format!("{SELECT} WHERE a.sess=?1 ORDER BY a.id"))?;
    let attachments = stmt
        .query_map([session_id], row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(attachments)
}

pub fn get(conn: &Connection, session_id: i64, attachment_id: i64) -> Result<Attachment> {
    conn.query_row(
        &format!("{SELECT} WHERE a.sess=?1 AND a.id=?2"),
        params![session_id, attachment_id],
        row,
    )
    .optional()?
    .ok_or_else(|| ChorusError::Invalid("attachment not found in this session".into()))
}

/// Read only a live attachment owned by this session and record the access.
pub fn read(
    conn: &Connection,
    session_id: i64,
    attachment_id: i64,
) -> Result<(Attachment, Vec<u8>)> {
    let meta = get(conn, session_id, attachment_id)?;
    if meta.status != "sent" {
        return Err(ChorusError::Invalid(if meta.status == "deleted" {
            "attachment was deleted".into()
        } else {
            "attachment has not been sent".into()
        }));
    }
    let bytes = blob::get(conn, &meta.hash)?;
    conn.execute(
        "INSERT INTO attachment_access(attachment_id,sess,accessed_ms) VALUES(?1,?2,?3)",
        params![attachment_id, session_id, now_ms()],
    )?;
    Ok((meta, bytes))
}

/// Append a user message and link staged attachments in one SQLite transaction.
pub fn link_to_event(
    conn: &mut Connection,
    session_id: i64,
    event: &NewEvent,
    attachment_ids: &[i64],
) -> Result<i64> {
    if event.sess != session_id {
        return Err(ChorusError::Invalid(
            "event session does not match attachment scope".into(),
        ));
    }
    if event.kind != crate::EventKind::Msg {
        return Err(ChorusError::Invalid(
            "attachments can only be linked to a message event".into(),
        ));
    }
    if attachment_ids.len() > MAX_PER_MESSAGE {
        return Err(ChorusError::Invalid(format!(
            "at most {MAX_PER_MESSAGE} attachments per message"
        )));
    }
    let mut unique = attachment_ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != attachment_ids.len() {
        return Err(ChorusError::Invalid("duplicate attachment ID".into()));
    }
    let tx = conn.transaction()?;
    let mut total = 0i64;
    for id in attachment_ids {
        let (size, status): (i64, String) = tx
            .query_row(
                "SELECT size,status FROM attachment WHERE id=?1 AND sess=?2",
                params![id, session_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| {
                ChorusError::Invalid(format!("attachment {id} is not available in this session"))
            })?;
        if status != "staged" {
            return Err(ChorusError::Invalid(format!(
                "attachment {id} is not staged"
            )));
        }
        total += size;
    }
    if total > MAX_MESSAGE_BYTES as i64 {
        return Err(ChorusError::Invalid(
            "combined attachments exceed 50 MiB".into(),
        ));
    }
    let seq = ledger::append(&tx, event)?;
    for id in attachment_ids {
        tx.execute(
            "INSERT INTO attachment_event(attachment_id,event_seq) VALUES(?1,?2)",
            params![id, seq],
        )?;
        tx.execute("UPDATE attachment SET status='sent' WHERE id=?1", [id])?;
    }
    tx.commit()?;
    Ok(seq)
}

/// Tombstone a session attachment. Its bytes are reclaimed by blob::gc when
/// no event body or other live attachment references the content hash.
pub fn delete(conn: &Connection, session_id: i64, attachment_id: i64) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let changed = tx.execute("UPDATE attachment SET status='deleted',deleted_ms=?1 WHERE id=?2 AND sess=?3 AND status!='deleted'", params![now_ms(),attachment_id,session_id])?;
    if changed == 0 {
        return Err(ChorusError::Invalid(
            "attachment not found or already deleted".into(),
        ));
    }
    // Revoke and reclaim immediately when these content addresses have no
    // remaining event-body or live attachment references.
    tx.execute(
        "DELETE FROM blob WHERE id IN (
             SELECT hash FROM attachment WHERE id=?1
             UNION SELECT prepared_hash FROM attachment WHERE id=?1 AND prepared_hash IS NOT NULL
         )
         AND id NOT IN (SELECT blob FROM event WHERE blob IS NOT NULL)
         AND id NOT IN (SELECT hash FROM attachment WHERE status!='deleted'
                        UNION SELECT prepared_hash FROM attachment WHERE status!='deleted' AND prepared_hash IS NOT NULL)",
        [attachment_id],
    )?;
    tx.commit()?;
    Ok(())
}
