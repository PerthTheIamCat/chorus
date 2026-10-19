//! Turn-Indexed Git Checkpoint DAG and 3-Way Selective Rewind Engine.
//!
//! Ties SQLite `event.seq` to cryptographic Git commit hashes in the shadow repo
//! (`.chorus/ckpt`), providing 3-Way Selective Time-Travel Rewind:
//! - CodeAndLedger: Roll back working tree + rewind conversation history.
//! - CodeOnly: Roll back working tree files; keep full chat transcript.
//! - LedgerOnly: Keep all written code; rewind conversation history.

use crate::checkpoint::Checkpoint;
use crate::store::Store;
use crate::{ChorusError, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// The mode for time-travel rewind execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RewindMode {
    /// Roll back working tree + rewind ledger history after target seq.
    CodeAndLedger,
    /// Roll back working tree files; preserve chat context.
    CodeOnly,
    /// Keep working tree edits; roll back conversation to target seq.
    LedgerOnly,
}

/// Record linking a ledger sequence number to a shadow git commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnCheckpointRecord {
    pub seq: i64,
    pub session_id: i64,
    pub commit_sha: String,
    pub message: String,
    pub created_ms: i64,
}

/// Summary of a completed rewind operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewindSummary {
    pub session_id: i64,
    pub target_seq: i64,
    pub mode: RewindMode,
    pub restored_commit: Option<String>,
    pub events_rolled_back: usize,
}

pub struct CheckpointDag;

impl CheckpointDag {
    /// Ensure the checkpoint_turn table exists in the SQLite store.
    pub fn ensure_schema(store: &Store) -> Result<()> {
        let conn = store.conn();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS checkpoint_turn (
                seq         INTEGER PRIMARY KEY,
                session_id  INTEGER NOT NULL,
                commit_sha  TEXT NOT NULL,
                message     TEXT NOT NULL,
                created_ms  INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_checkpoint_session ON checkpoint_turn(session_id, seq);",
            [],
        )
        .map_err(ChorusError::Sqlite)?;
        Ok(())
    }

    /// Commit a turn snapshot in the shadow git repo and bind it to SQLite `seq`.
    pub fn record_turn(
        store: &Store,
        ckpt: &Checkpoint,
        session_id: i64,
        seq: i64,
        message: &str,
    ) -> Result<String> {
        let commit_msg = format!("turn:{}:{} {}", session_id, seq, message);
        let sha = ckpt.snapshot(&commit_msg)?;
        Self::bind(store, session_id, seq, &sha, message)?;
        Ok(sha)
    }

    /// Link `seq` to a snapshot that was already taken — the session engine
    /// snapshots before every turn anyway ("turn guard"), so binding that sha
    /// to the user's message costs one row instead of a second commit.
    pub fn bind(store: &Store, session_id: i64, seq: i64, sha: &str, message: &str) -> Result<()> {
        Self::ensure_schema(store)?;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let conn = store.conn();
        conn.execute(
            "INSERT OR REPLACE INTO checkpoint_turn (seq, session_id, commit_sha, message, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![seq, session_id, sha, message, now_ms],
        )
        .map_err(ChorusError::Sqlite)?;
        Ok(())
    }

    /// Retrieve the checkpoint record for a given sequence number.
    pub fn get_turn_checkpoint(store: &Store, seq: i64) -> Result<Option<TurnCheckpointRecord>> {
        Self::ensure_schema(store)?;
        let conn = store.conn();
        let mut stmt = conn
            .prepare(
                "SELECT seq, session_id, commit_sha, message, created_ms
                 FROM checkpoint_turn WHERE seq = ?1",
            )
            .map_err(ChorusError::Sqlite)?;

        let mut rows = stmt
            .query_map(params![seq], |row| {
                Ok(TurnCheckpointRecord {
                    seq: row.get(0)?,
                    session_id: row.get(1)?,
                    commit_sha: row.get(2)?,
                    message: row.get(3)?,
                    created_ms: row.get(4)?,
                })
            })
            .map_err(ChorusError::Sqlite)?;

        match rows.next() {
            Some(Ok(rec)) => Ok(Some(rec)),
            Some(Err(e)) => Err(ChorusError::Sqlite(e)),
            None => Ok(None),
        }
    }

    /// List all turn checkpoints for a given session, ordered by sequence number.
    pub fn list_turn_checkpoints(store: &Store, session_id: i64) -> Result<Vec<TurnCheckpointRecord>> {
        Self::ensure_schema(store)?;
        let conn = store.conn();
        let mut stmt = conn
            .prepare(
                "SELECT seq, session_id, commit_sha, message, created_ms
                 FROM checkpoint_turn WHERE session_id = ?1 ORDER BY seq ASC",
            )
            .map_err(ChorusError::Sqlite)?;

        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok(TurnCheckpointRecord {
                    seq: row.get(0)?,
                    session_id: row.get(1)?,
                    commit_sha: row.get(2)?,
                    message: row.get(3)?,
                    created_ms: row.get(4)?,
                })
            })
            .map_err(ChorusError::Sqlite)?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(ChorusError::Sqlite)?);
        }
        Ok(out)
    }

    /// Execute a 3-Way Selective Rewind to a target sequence number.
    pub fn rewind_to_seq(
        store: &Store,
        ckpt: &Checkpoint,
        session_id: i64,
        target_seq: i64,
        mode: RewindMode,
    ) -> Result<RewindSummary> {
        Self::ensure_schema(store)?;

        // 1. Find checkpoint commit if mode requires code restoration
        let mut restored_commit = None;
        if matches!(mode, RewindMode::CodeAndLedger | RewindMode::CodeOnly) {
            let record = Self::get_turn_checkpoint(store, target_seq)?
                .ok_or_else(|| ChorusError::Invalid(format!("No checkpoint found for seq {}", target_seq)))?;
            ckpt.restore(&record.commit_sha)?;
            restored_commit = Some(record.commit_sha);
        }

        // 2. Roll back ledger events if mode requires ledger restoration
        let mut events_rolled_back = 0;
        if matches!(mode, RewindMode::CodeAndLedger | RewindMode::LedgerOnly) {
            let conn = store.conn();
            // Count affected events
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM event WHERE sess = ?1 AND seq > ?2",
                    params![session_id, target_seq],
                    |row| row.get(0),
                )
                .map_err(ChorusError::Sqlite)?;

            events_rolled_back = count as usize;

            // Prune events after target_seq
            conn.execute(
                "DELETE FROM event WHERE sess = ?1 AND seq > ?2",
                params![session_id, target_seq],
            )
            .map_err(ChorusError::Sqlite)?;

            // Prune checkpoint_turn records after target_seq
            conn.execute(
                "DELETE FROM checkpoint_turn WHERE session_id = ?1 AND seq > ?2",
                params![session_id, target_seq],
            )
            .map_err(ChorusError::Sqlite)?;
        }

        Ok(RewindSummary {
            session_id,
            target_seq,
            mode,
            restored_commit,
            events_rolled_back,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EventKind, NewEvent};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn checkpoint_dag_three_way_rewind() {
        use crate::ledger;

        let dir = tempdir().unwrap();
        let ws = dir.path();

        let store = Store::open_in_memory().unwrap();
        let ckpt = Checkpoint::open(ws).unwrap();

        let session_id = ledger::new_session(store.conn(), "Test Rewind Session", ws.to_str().unwrap()).unwrap();
        let actor_id = crate::actor::intern(store.conn(), "user", "human").unwrap();

        // Turn 1: Add a.txt
        fs::write(ws.join("a.txt"), "Version 1").unwrap();
        let ev1_seq = ledger::append(
            store.conn(),
            &NewEvent::new(session_id, EventKind::Msg, actor_id, "Turn 1"),
        )
        .unwrap();

        let sha1 = CheckpointDag::record_turn(&store, &ckpt, session_id, ev1_seq, "T1 commit").unwrap();
        assert!(!sha1.is_empty());

        // Turn 2: Edit a.txt, add b.txt
        fs::write(ws.join("a.txt"), "Version 2").unwrap();
        fs::write(ws.join("b.txt"), "File B").unwrap();
        let ev2_seq = ledger::append(
            store.conn(),
            &NewEvent::new(session_id, EventKind::Msg, actor_id, "Turn 2"),
        )
        .unwrap();

        let _sha2 = CheckpointDag::record_turn(&store, &ckpt, session_id, ev2_seq, "T2 commit").unwrap();

        // Test 1: CodeOnly rewind to Turn 1 (Code rolls back, chat intact)
        let sum_code = CheckpointDag::rewind_to_seq(&store, &ckpt, session_id, ev1_seq, RewindMode::CodeOnly).unwrap();
        assert_eq!(sum_code.events_rolled_back, 0);
        assert_eq!(fs::read_to_string(ws.join("a.txt")).unwrap(), "Version 1");
        assert!(!ws.join("b.txt").exists());

        // Verify ledger still has 2 events
        let count: i64 = store.conn().query_row("SELECT COUNT(*) FROM event WHERE sess = ?1", params![session_id], |r: &rusqlite::Row| r.get(0)).unwrap();
        assert_eq!(count, 2);

        // Turn 3: Write code again, then test CodeAndLedger rewind
        fs::write(ws.join("a.txt"), "Version 3").unwrap();
        let sum_both = CheckpointDag::rewind_to_seq(&store, &ckpt, session_id, ev1_seq, RewindMode::CodeAndLedger).unwrap();
        assert_eq!(sum_both.events_rolled_back, 1); // ev2 pruned
        assert_eq!(fs::read_to_string(ws.join("a.txt")).unwrap(), "Version 1");

        let count2: i64 = store.conn().query_row("SELECT COUNT(*) FROM event WHERE sess = ?1", params![session_id], |r: &rusqlite::Row| r.get(0)).unwrap();
        assert_eq!(count2, 1);
    }
}
