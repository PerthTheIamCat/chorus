use chorus_core::types::EventKind;
use chorus_core::{actor, blob, ledger, memory, schema, store::Store, NewEvent};

fn setup() -> Store {
    Store::open_in_memory().expect("open in-memory store")
}

#[test]
fn migrate_is_idempotent() {
    let store = setup();
    // Calling migrate again on an already-current db must not error and
    // must not duplicate anything.
    schema::migrate(store.conn()).unwrap();
    schema::migrate(store.conn()).unwrap();

    let version: i64 = store
        .conn()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, schema::SCHEMA_VERSION);
}

#[test]
fn actor_interning_is_stable() {
    let store = setup();
    let id1 = actor::intern(store.conn(), "claude", "anthropic").unwrap();
    let id2 = actor::intern(store.conn(), "claude", "anthropic").unwrap();
    let id3 = actor::intern(store.conn(), "gpt", "openai").unwrap();

    assert_eq!(id1, id2);
    assert_ne!(id1, id3);
    assert_eq!(actor::name(store.conn(), id1).unwrap(), "claude");
}

#[test]
fn duplicate_large_payload_creates_one_blob_row() {
    let store = setup();
    let conn = store.conn();
    let actor_id = actor::intern(conn, "claude", "anthropic").unwrap();
    let sess = ledger::new_session(conn, "test session", "/tmp").unwrap();

    // 50 KB payload, well above INLINE_MAX_BYTES.
    let payload = "x".repeat(50 * 1024);

    for _ in 0..10 {
        let ev = NewEvent::new(sess, EventKind::ToolResult, actor_id, payload.clone());
        ledger::append(conn, &ev).unwrap();
    }

    let blob_count: i64 = conn.query_row("SELECT COUNT(*) FROM blob", [], |r| r.get(0)).unwrap();
    assert_eq!(blob_count, 1, "identical large payloads must dedup to a single blob row");

    let event_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM event WHERE sess = ?1", [sess], |r| r.get(0))
        .unwrap();
    assert_eq!(event_count, 10, "each append should still create its own event row");
}

#[test]
fn large_body_round_trips_through_blob_store() {
    let store = setup();
    let conn = store.conn();
    let actor_id = actor::intern(conn, "claude", "anthropic").unwrap();
    let sess = ledger::new_session(conn, "s", "/tmp").unwrap();

    let payload = "y".repeat(10 * 1024);
    let ev = NewEvent::new(sess, EventKind::Diff, actor_id, payload.clone());
    let seq = ledger::append(conn, &ev).unwrap();

    let stored = ledger::get(conn, seq).unwrap();
    assert!(stored.body.is_none(), "large body should not be stored inline");
    assert!(stored.blob.is_some(), "large body should reference a blob");

    let round_tripped = ledger::body(conn, &stored).unwrap();
    assert_eq!(round_tripped, payload);
}

#[test]
fn small_body_stays_inline() {
    let store = setup();
    let conn = store.conn();
    let actor_id = actor::intern(conn, "claude", "anthropic").unwrap();
    let sess = ledger::new_session(conn, "s", "/tmp").unwrap();

    let ev = NewEvent::new(sess, EventKind::Msg, actor_id, "hello world");
    let seq = ledger::append(conn, &ev).unwrap();

    let stored = ledger::get(conn, seq).unwrap();
    assert!(stored.blob.is_none(), "small body must stay inline (event.blob IS NULL)");
    assert_eq!(stored.body.as_deref(), Some("hello world"));

    let body_col: Option<String> = conn
        .query_row("SELECT blob FROM event WHERE seq = ?1", [seq], |r| r.get(0))
        .unwrap();
    assert!(body_col.is_none());
}

#[test]
fn tail_is_chronological_and_respects_n() {
    let store = setup();
    let conn = store.conn();
    let actor_id = actor::intern(conn, "claude", "anthropic").unwrap();
    let sess = ledger::new_session(conn, "s", "/tmp").unwrap();

    for i in 0..5 {
        let ev = NewEvent::new(sess, EventKind::Msg, actor_id, format!("msg-{i}"));
        ledger::append(conn, &ev).unwrap();
    }

    let last3 = ledger::tail(conn, sess, 3).unwrap();
    assert_eq!(last3.len(), 3);
    let bodies: Vec<String> = last3.iter().map(|e| ledger::body(conn, e).unwrap()).collect();
    assert_eq!(bodies, vec!["msg-2", "msg-3", "msg-4"]);

    // chronological: seqs strictly increasing
    for w in last3.windows(2) {
        assert!(w[0].seq < w[1].seq);
    }
}

#[test]
fn fork_creates_session_and_copies_no_events() {
    let store = setup();
    let conn = store.conn();
    let actor_id = actor::intern(conn, "claude", "anthropic").unwrap();
    let sess = ledger::new_session(conn, "parent", "/tmp").unwrap();

    let mut last_seq = 0;
    for i in 0..3 {
        let ev = NewEvent::new(sess, EventKind::Msg, actor_id, format!("m{i}"));
        last_seq = ledger::append(conn, &ev).unwrap();
    }

    let forked = ledger::fork(conn, sess, last_seq, "forked").unwrap();
    assert_ne!(forked, sess);

    let forked_events: i64 = conn
        .query_row("SELECT COUNT(*) FROM event WHERE sess = ?1", [forked], |r| r.get(0))
        .unwrap();
    assert_eq!(forked_events, 0, "fork must copy zero events");

    let (forked_from, fork_at_seq): (Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT forked_from, fork_at_seq FROM session WHERE id = ?1",
            [forked],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(forked_from, Some(sess));
    assert_eq!(fork_at_seq, Some(last_seq));
}

#[test]
fn gc_removes_only_unreferenced_blobs() {
    let store = setup();
    let conn = store.conn();
    let actor_id = actor::intern(conn, "claude", "anthropic").unwrap();
    let sess = ledger::new_session(conn, "s", "/tmp").unwrap();

    // Referenced blob (via an event).
    let referenced_payload = "a".repeat(10 * 1024);
    let ev = NewEvent::new(sess, EventKind::ToolResult, actor_id, referenced_payload);
    ledger::append(conn, &ev).unwrap();

    // Unreferenced blob (put directly, no event points at it).
    let orphan_payload = b"z".repeat(10 * 1024);
    let orphan_id = blob::put(conn, &orphan_payload).unwrap();

    let before: i64 = conn.query_row("SELECT COUNT(*) FROM blob", [], |r| r.get(0)).unwrap();
    assert_eq!(before, 2);

    let deleted = blob::gc(conn, i64::MAX).unwrap();
    assert_eq!(deleted, 1, "gc should delete exactly the unreferenced blob");

    let after: i64 = conn.query_row("SELECT COUNT(*) FROM blob", [], |r| r.get(0)).unwrap();
    assert_eq!(after, 1);

    assert!(blob::get(conn, &orphan_id).is_err());
}

#[test]
fn fact_text_over_140_chars_is_rejected() {
    let store = setup();
    let conn = store.conn();
    let long_text = "a".repeat(141);
    let result = memory::add(conn, 1, &long_text);
    assert!(result.is_err());

    let short_text = "a".repeat(140);
    let result = memory::add(conn, 1, &short_text);
    assert!(result.is_ok());
}

#[test]
fn render_output_has_no_digit_metadata() {
    let store = setup();
    let conn = store.conn();
    memory::add(conn, 1, "prefers dark mode").unwrap();
    memory::add(conn, 1, "uses zsh").unwrap();

    let facts = memory::top(conn, 1, 10).unwrap();
    let rendered = memory::render(&facts);

    assert!(rendered.contains("prefers dark mode"));
    assert!(rendered.contains("uses zsh"));
    assert!(
        !rendered.chars().any(|c| c.is_ascii_digit()),
        "render output must not leak ids/confidence/timestamps as digits: {rendered:?}"
    );
}

#[test]
fn on_disk_store_persists_and_migrates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("chorus.sqlite3");

    {
        let store = Store::open(&path).unwrap();
        let conn = store.conn();
        let actor_id = actor::intern(conn, "claude", "anthropic").unwrap();
        let sess = ledger::new_session(conn, "disk session", "/tmp").unwrap();
        let ev = NewEvent::new(sess, EventKind::Msg, actor_id, "on disk");
        ledger::append(conn, &ev).unwrap();
    }

    // Reopen: migrate must be idempotent and prior data must still be there.
    let store2 = Store::open(&path).unwrap();
    let count: i64 = store2
        .conn()
        .query_row("SELECT COUNT(*) FROM event", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}
