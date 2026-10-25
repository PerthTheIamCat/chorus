//! Schema definition and migration.
//!
//! Design notes (see module-level design rules in the task spec):
//! - Actor names and event kinds are integers, never repeated strings.
//! - `event.t` is a millisecond offset from the session's start.
//! - `event.seq` is `INTEGER PRIMARY KEY`, i.e. sqlite rowid — monotonic,
//!   never a UUID.
//! - Blobs are content-addressed by BLAKE3 so identical payloads
//!   (agents re-reading the same file 5-10x per session) are stored once.
//!
//! `migrate` is idempotent: it reads `PRAGMA user_version` and does nothing
//! if the database is already at `SCHEMA_VERSION`.

use crate::Result;
use rusqlite::Connection;

pub const SCHEMA_VERSION: i64 = 8;

/// Create (or upgrade) the schema. Safe to call on every open.
pub fn migrate(conn: &Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current >= SCHEMA_VERSION {
        return Ok(());
    }

    conn.execute_batch(
        r#"
        BEGIN;

        CREATE TABLE IF NOT EXISTS session (
            id          INTEGER PRIMARY KEY,
            title       TEXT NOT NULL,
            cwd         TEXT NOT NULL,
            started_ms  INTEGER NOT NULL,
            forked_from INTEGER,
            fork_at_seq INTEGER
        );

        CREATE TABLE IF NOT EXISTS actor (
            id       INTEGER PRIMARY KEY,
            name     TEXT NOT NULL UNIQUE,
            provider TEXT NOT NULL
        );

        -- `zstd` records whether `bytes` is zstd-compressed (1) or stored
        -- raw (0) — some payloads (already-compressed data, tiny inputs)
        -- don't shrink under zstd, so we skip the round-trip cost.
        CREATE TABLE IF NOT EXISTS blob (
            id         TEXT PRIMARY KEY,
            bytes      BLOB NOT NULL,
            orig_len   INTEGER NOT NULL,
            tok        INTEGER,
            created_ms INTEGER NOT NULL,
            refs       INTEGER NOT NULL DEFAULT 0,
            zstd       INTEGER NOT NULL DEFAULT 1
        );

        CREATE TABLE IF NOT EXISTS event (
            seq      INTEGER PRIMARY KEY,
            t        INTEGER NOT NULL,
            sess     INTEGER NOT NULL REFERENCES session(id),
            parent   INTEGER,
            kind     INTEGER NOT NULL,
            actor    INTEGER NOT NULL REFERENCES actor(id),
            body     TEXT,
            blob     TEXT,
            tok_in   INTEGER,
            tok_out  INTEGER,
            cost_usd REAL,
            model_id TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_event_sess_seq ON event(sess, seq);
        CREATE INDEX IF NOT EXISTS idx_event_blob ON event(blob);

        CREATE TABLE IF NOT EXISTS fact (
            id           INTEGER PRIMARY KEY,
            scope        INTEGER NOT NULL,
            text         TEXT NOT NULL,
            src          INTEGER,
            conf         REAL NOT NULL DEFAULT 1.0,
            pinned       INTEGER NOT NULL DEFAULT 0,
            hits         INTEGER NOT NULL DEFAULT 0,
            last_used_ms INTEGER,
            expires_ms   INTEGER,
            created_ms   INTEGER NOT NULL
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_fact_scope_text ON fact(scope, text);

        -- v2: agents may not write `fact` directly (see design §4). They
        -- append here via `memory::propose`; a human moves rows into `fact`
        -- with `memory::approve` or drops them with `memory::reject`.
        CREATE TABLE IF NOT EXISTS fact_proposal (
            id         INTEGER PRIMARY KEY,
            scope      INTEGER NOT NULL,
            text       TEXT NOT NULL,
            src        INTEGER REFERENCES actor(id),
            created_ms INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_proposal_scope ON fact_proposal(scope, id);

        -- v8: optional embedding of a fact, from a local model
        -- (`embedding::Embedder`). Derived data — safe to drop; the n-gram
        -- retrieval needs none of it. `model` keeps vectors of different
        -- models from ever being compared.
        CREATE TABLE IF NOT EXISTS fact_embedding (
            fact  INTEGER PRIMARY KEY REFERENCES fact(id) ON DELETE CASCADE,
            model TEXT NOT NULL,
            dim   INTEGER NOT NULL,
            vec   BLOB NOT NULL
        );

        -- v3: the warm tier of the compaction ladder (storage spec, rungs).
        -- L1/L2 are *stored*, not recomputed: assembling a context must never
        -- run a summariser, or every turn pays a model call per event. Rows
        -- are filled in by `chorus compact`, off the turn's critical path.
        -- Keyed by event seq rather than being events themselves: a summary is
        -- metadata about a turn, not something anyone said, and keeping it out
        -- of the ledger keeps `history()` and the dedup counts honest.
        CREATE TABLE IF NOT EXISTS summary (
            seq        INTEGER PRIMARY KEY REFERENCES event(seq) ON DELETE CASCADE,
            l1         TEXT NOT NULL,
            l2         TEXT NOT NULL,
            model      TEXT NOT NULL,
            created_ms INTEGER NOT NULL
        );

        -- v4: session-owned attachments are separate from event.blob, whose
        -- established meaning remains oversized message body text.
        CREATE TABLE IF NOT EXISTS attachment (
            id INTEGER PRIMARY KEY,
            sess INTEGER NOT NULL REFERENCES session(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            media_type TEXT NOT NULL,
            size INTEGER NOT NULL,
            -- Hashes remain in tombstones after bytes are reclaimed.
            hash TEXT NOT NULL,
            prepared_hash TEXT,
            prepared_media_type TEXT,
            status TEXT NOT NULL CHECK(status IN ('staged','sent','deleted')),
            created_ms INTEGER NOT NULL,
            deleted_ms INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_attachment_sess ON attachment(sess, id);
        CREATE TABLE IF NOT EXISTS attachment_event (
            attachment_id INTEGER PRIMARY KEY REFERENCES attachment(id),
            event_seq INTEGER NOT NULL REFERENCES event(seq)
        );
        CREATE INDEX IF NOT EXISTS idx_attachment_event_seq ON attachment_event(event_seq);
        CREATE TABLE IF NOT EXISTS attachment_access (
            id INTEGER PRIMARY KEY,
            attachment_id INTEGER NOT NULL,
            sess INTEGER NOT NULL,
            accessed_ms INTEGER NOT NULL
        );

        -- v6: agent control surface
        CREATE TABLE IF NOT EXISTS subagent (
            id            INTEGER PRIMARY KEY,
            parent_sess   INTEGER REFERENCES session(id),
            child_sess    INTEGER REFERENCES session(id),
            role          TEXT NOT NULL,
            task          TEXT NOT NULL,
            worktree      TEXT,
            status        TEXT NOT NULL CHECK(status IN ('running','completed','failed','cancelled')),
            summary       TEXT,
            diff_stat     TEXT,
            files_touched TEXT,
            created_ms    INTEGER NOT NULL,
            updated_ms    INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_subagent_parent ON subagent(parent_sess, id);

        CREATE TABLE IF NOT EXISTS subagent_message (
            id          INTEGER PRIMARY KEY,
            subagent_id INTEGER NOT NULL REFERENCES subagent(id) ON DELETE CASCADE,
            direction   TEXT NOT NULL,
            content     TEXT NOT NULL,
            created_ms  INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_subagent_msg ON subagent_message(subagent_id, id);

        CREATE TABLE IF NOT EXISTS task_checklist (
            id         INTEGER PRIMARY KEY,
            sess       INTEGER REFERENCES session(id) ON DELETE CASCADE,
            items_json TEXT NOT NULL,
            updated_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_checklist_sess ON task_checklist(sess);

        CREATE TABLE IF NOT EXISTS review_request (
            id          INTEGER PRIMARY KEY,
            sess        INTEGER REFERENCES session(id) ON DELETE CASCADE,
            summary     TEXT NOT NULL,
            hunks_json  TEXT NOT NULL,
            status      TEXT NOT NULL CHECK(status IN ('pending','approved','rejected')),
            created_ms  INTEGER NOT NULL,
            resolved_ms INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_review_sess ON review_request(sess, id);

        CREATE TABLE IF NOT EXISTS ui_navigation (
            id         INTEGER PRIMARY KEY,
            sess       INTEGER REFERENCES session(id) ON DELETE CASCADE,
            path       TEXT NOT NULL,
            line       INTEGER,
            created_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_navigation_sess ON ui_navigation(sess, id);

        -- v7: last known usage per session. Metadata like `summary`, never a
        -- ledger event, so `history()` and the dedup counts stay honest. Seeds
        -- the context meter on reopen and feeds cost display.
        CREATE TABLE IF NOT EXISTS session_usage (
            sess           INTEGER PRIMARY KEY REFERENCES session(id) ON DELETE CASCADE,
            context_used   INTEGER,
            context_window INTEGER,
            tokens_in      INTEGER NOT NULL DEFAULT 0,
            tokens_out     INTEGER NOT NULL DEFAULT 0,
            cost_usd       REAL,
            updated_ms     INTEGER NOT NULL
        );

        COMMIT;
        "#,
    )?;

    // FTS5 external-content index over event.body. Not all sqlite builds
    // carry the FTS5 module; if it's missing, skip it entirely and let
    // `ledger::search` fall back to a LIKE scan instead of failing the
    // whole migration.
    let fts_result = conn.execute_batch(
        r#"
        CREATE VIRTUAL TABLE IF NOT EXISTS ev_fts USING fts5(
            body,
            content='event',
            content_rowid='seq'
        );

        CREATE TRIGGER IF NOT EXISTS ev_fts_ai AFTER INSERT ON event BEGIN
            INSERT INTO ev_fts(rowid, body) VALUES (new.seq, new.body);
        END;

        CREATE TRIGGER IF NOT EXISTS ev_fts_ad AFTER DELETE ON event BEGIN
            INSERT INTO ev_fts(ev_fts, rowid, body) VALUES ('delete', old.seq, old.body);
        END;

        CREATE TRIGGER IF NOT EXISTS ev_fts_au AFTER UPDATE ON event BEGIN
            INSERT INTO ev_fts(ev_fts, rowid, body) VALUES ('delete', old.seq, old.body);
            INSERT INTO ev_fts(rowid, body) VALUES (new.seq, new.body);
        END;
        "#,
    );
    if fts_result.is_err() {
        // FTS5 unavailable in this sqlite build — degrade gracefully.
        // `ledger::search` probes for `ev_fts` itself and falls back to LIKE.
    }

    // v5: preserve the model the bridge reports for each assistant message.
    // Check the column so fresh databases and interrupted upgrades are both safe.
    if !has_column(conn, "event", "model_id")? {
        conn.execute_batch("ALTER TABLE event ADD COLUMN model_id TEXT")?;
    }

    // v7: review baseline (a shadow-repo sha) lives on the session row.
    if !has_column(conn, "session", "review_base")? {
        conn.execute_batch("ALTER TABLE session ADD COLUMN review_base TEXT")?;
    }

    // v7: `subagent.kind` and a `waiting` status. SQLite cannot alter a CHECK
    // constraint, so the table is rebuilt. Foreign keys go off for the swap:
    // with them on, `DROP TABLE subagent` would cascade-delete every
    // `subagent_message`. The pragma is a no-op inside a transaction, which is
    // why the rebuild runs after the main batch has committed.
    if !has_column(conn, "subagent", "kind")? {
        let fk: i64 = conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
        conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let rebuilt = conn.execute_batch(
            r#"
            BEGIN;
            CREATE TABLE subagent_v7 (
                id            INTEGER PRIMARY KEY,
                parent_sess   INTEGER REFERENCES session(id),
                child_sess    INTEGER REFERENCES session(id),
                kind          TEXT NOT NULL DEFAULT 'chorus' CHECK(kind IN ('chorus','provider')),
                role          TEXT NOT NULL,
                task          TEXT NOT NULL,
                worktree      TEXT,
                status        TEXT NOT NULL CHECK(status IN ('running','waiting','completed','failed','cancelled')),
                summary       TEXT,
                diff_stat     TEXT,
                files_touched TEXT,
                created_ms    INTEGER NOT NULL,
                updated_ms    INTEGER NOT NULL
            );
            INSERT INTO subagent_v7(id, parent_sess, child_sess, role, task, worktree, status,
                                    summary, diff_stat, files_touched, created_ms, updated_ms)
                SELECT id, parent_sess, child_sess, role, task, worktree, status,
                       summary, diff_stat, files_touched, created_ms, updated_ms
                FROM subagent;
            DROP TABLE subagent;
            ALTER TABLE subagent_v7 RENAME TO subagent;
            CREATE INDEX IF NOT EXISTS idx_subagent_parent ON subagent(parent_sess, id);
            COMMIT;
            "#,
        );
        if rebuilt.is_err() {
            let _ = conn.execute_batch("ROLLBACK;");
        }
        conn.execute_batch(&format!("PRAGMA foreign_keys = {fk};"))?;
        rebuilt?;
    }

    conn.execute(&format!("PRAGMA user_version = {SCHEMA_VERSION}"), [])?;

    Ok(())
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
        [table, column],
        |row| row.get(0),
    )?)
}
