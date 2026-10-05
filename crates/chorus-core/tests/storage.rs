use chorus_core::types::EventKind;
use chorus_core::compaction;
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

#[test]
fn proposed_facts_wait_for_approval() {
    let store = setup();
    let c = store.conn();
    let agent = actor::intern(c, "claude", "anthropic").unwrap();

    let p1 = memory::propose(c, 1, "user prefers tabs over spaces", Some(agent)).unwrap();
    let _p2 = memory::propose(c, 1, "the build is flaky on CI", Some(agent)).unwrap();

    // Nothing reached `fact` yet.
    assert!(memory::top(c, 1, 10).unwrap().is_empty());
    assert_eq!(memory::pending(c).unwrap().len(), 2);

    // Approve one, reject the other.
    memory::approve(c, p1).unwrap();
    let pending = memory::pending(c).unwrap();
    assert_eq!(pending.len(), 1);
    memory::reject(c, pending[0].id).unwrap();

    assert!(memory::pending(c).unwrap().is_empty());
    let facts = memory::top(c, 1, 10).unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].text, "user prefers tabs over spaces");
}

#[test]
fn an_over_long_proposal_is_rejected_at_propose_time() {
    let store = setup();
    let long = "x".repeat(200);
    assert!(memory::propose(store.conn(), 0, &long, None).is_err());
}

#[test]
fn schema_v1_database_upgrades_to_v2() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v1.sqlite3");

    // Hand-build a v1 database: run the current migration, then stamp it
    // back to version 1 and drop the v2 table, as an old build would leave it.
    {
        let store = Store::open(&path).unwrap();
        store
            .conn()
            .execute_batch("DROP TABLE fact_proposal; PRAGMA user_version = 1;")
            .unwrap();
    }

    // Reopening runs migrate again and must add the v2 table without losing data.
    let store = Store::open(&path).unwrap();
    let v: i64 = store.conn().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(v, schema::SCHEMA_VERSION);
    assert!(memory::propose(store.conn(), 0, "works after upgrade", None).is_ok());
}

#[test]
fn a_v2_database_gains_the_summary_table_without_losing_anything() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v2.sqlite3");

    // Hand-build a v2 database: migrate, then drop the v3 table and stamp
    // the version back, exactly as a build from before the warm tier left it.
    let sess;
    let seq;
    {
        let store = Store::open(&path).unwrap();
        let c = store.conn();
        let a = actor::intern(c, "codex", "openai").unwrap();
        sess = ledger::new_session(c, "old", "/tmp").unwrap();
        seq = ledger::append(c, &NewEvent::new(sess, EventKind::Msg, a, "said before upgrade"))
            .unwrap();
        c.execute_batch("DROP TABLE summary; PRAGMA user_version = 2;")
            .unwrap();
    }

    let store = Store::open(&path).unwrap();
    let c = store.conn();
    let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(v, schema::SCHEMA_VERSION);

    // The old event survived, and the warm tier now works for it.
    let e = ledger::get(c, seq).unwrap();
    assert_eq!(ledger::body(c, &e).unwrap(), "said before upgrade");
    assert_eq!(compaction::uncompacted_count(c, sess).unwrap(), 1);
    compaction::save_rungs(
        c,
        seq,
        &compaction::Rungs { l1: "one".into(), l2: "two".into() },
        "test",
    )
    .unwrap();
    assert_eq!(compaction::uncompacted_count(c, sess).unwrap(), 0);
    assert_eq!(compaction::load_rungs(c, seq).unwrap().unwrap().l2, "two");
}

#[test]
fn history_walks_the_fork_chain() {
    let store = setup();
    let c = store.conn();
    let a = actor::intern(c, "claude", "anthropic").unwrap();

    let root = ledger::new_session(c, "root", "/tmp").unwrap();
    let s1 = ledger::append(c, &NewEvent::new(root, EventKind::Msg, a, "root turn 1")).unwrap();
    let s2 = ledger::append(c, &NewEvent::new(root, EventKind::Msg, a, "root turn 2")).unwrap();
    let _s3 = ledger::append(c, &NewEvent::new(root, EventKind::Msg, a, "root turn 3 (after fork)")).unwrap();

    // Fork at s2: the branch should see turns 1-2 but not 3.
    let branch = ledger::fork(c, root, s2, "branch").unwrap();
    ledger::append(c, &NewEvent::new(branch, EventKind::Msg, a, "branch turn A")).unwrap();

    let h: Vec<String> = ledger::history(c, branch, 100)
        .unwrap()
        .iter()
        .map(|e| ledger::body(c, e).unwrap())
        .collect();

    assert_eq!(h, vec!["root turn 1", "root turn 2", "branch turn A"]);
    assert!(!h.iter().any(|b| b.contains("after fork")), "branch saw a post-fork parent event");
    let _ = (s1, s2);

    // The root's own history is unaffected.
    let rh: Vec<String> = ledger::history(c, root, 100)
        .unwrap()
        .iter()
        .map(|e| ledger::body(c, e).unwrap())
        .collect();
    assert_eq!(rh.len(), 3);
}


#[test]
fn rename_and_delete_session() {
    let st = setup();
    let c = st.conn();
    let user = actor::intern(c, "user", "human").unwrap();
    let keep = ledger::new_session(c, "keep", "/tmp").unwrap();
    let gone = ledger::new_session(c, "gone", "/tmp").unwrap();
    let big = "x".repeat(10_000);
    ledger::append(c, &NewEvent::new(keep, EventKind::Msg, user, big.clone())).unwrap();
    ledger::append(c, &NewEvent::new(gone, EventKind::Msg, user, big)).unwrap();
    ledger::append(c, &NewEvent::new(gone, EventKind::Msg, user, "only here")).unwrap();

    ledger::rename_session(c, keep, "renamed").unwrap();
    let t: String = c.query_row("SELECT title FROM session WHERE id = ?1", [keep], |r| r.get(0)).unwrap();
    assert_eq!(t, "renamed");

    // A fork blocks deleting its parent.
    let fork = ledger::fork(c, gone, 0, "fork").unwrap();
    assert!(ledger::delete_session(c, gone).is_err());
    ledger::delete_session(c, fork).unwrap();

    ledger::delete_session(c, gone).unwrap();
    let n: i64 = c.query_row("SELECT COUNT(*) FROM event WHERE sess = ?1", [gone], |r| r.get(0)).unwrap();
    assert_eq!(n, 0);
    // The shared blob is still referenced by `keep`, so it survives.
    let blobs: i64 = c.query_row("SELECT COUNT(*) FROM blob", [], |r| r.get(0)).unwrap();
    assert_eq!(blobs, 1);
    assert!(ledger::delete_session(c, gone).is_err());
}
