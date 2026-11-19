//! Memory that learns (schema v9): conflict-aware proposals, the review queue's
//! kinds, per-fact usage counters and retirement suggestions.

use chorus_core::embedding::Embedder;
use chorus_core::memory::{self, NewProposal, ProposalKind, Proposed};
use chorus_core::{schema, store::Store};

fn store() -> Store {
    Store::open_in_memory().unwrap()
}

fn agent_fact<'a>(text: &'a str) -> NewProposal<'a> {
    NewProposal::fact(1, text)
}

#[test]
fn a_new_subject_is_queued_as_a_new_row() {
    let s = store();
    let c = s.conn();
    memory::add(c, 1, "ใช้ pnpm ไม่ใช่ npm").unwrap();
    let r = memory::propose_with(c, &agent_fact("ตอบเป็นภาษาไทยเสมอ"), None).unwrap();
    assert!(matches!(r, Proposed::Queued(_)), "{r:?}");
}

#[test]
fn a_conflicting_sentence_becomes_replace_not_a_second_row() {
    let s = store();
    let c = s.conn();
    let old = memory::add(c, 1, "ใช้ pnpm ไม่ใช่ npm").unwrap();
    let r = memory::propose_with(c, &agent_fact("ใช้ yarn ไม่ใช่ npm"), None).unwrap();
    let Proposed::Replaces { proposal, fact } = r else { panic!("{r:?}") };
    assert_eq!(fact, old);
    let q = memory::pending(c).unwrap();
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].id, proposal);
    assert_eq!(q[0].replaces, Some(old));

    // Approving rewrites the fact in place: one fact, new wording, same id.
    let added = memory::approve_with(c, proposal, None).unwrap();
    assert_eq!(added.id, old);
    let facts = memory::top(c, 1, 10).unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].text, "ใช้ yarn ไม่ใช่ npm");
}

#[test]
fn something_already_remembered_or_queued_adds_nothing() {
    let s = store();
    let c = s.conn();
    let f = memory::add(c, 1, "test: cargo test --workspace").unwrap();
    assert_eq!(memory::propose_with(c, &agent_fact("test: cargo test --workspace"), None).unwrap(), Proposed::Known(f));
    let Proposed::Queued(p) = memory::propose_with(c, &agent_fact("deploy: fly deploy"), None).unwrap() else { panic!() };
    assert_eq!(memory::propose_with(c, &agent_fact("deploy: fly deploy"), None).unwrap(), Proposed::AlreadyQueued(p));
    assert_eq!(memory::pending_count(c).unwrap(), 1);
}

/// One axis per concept, like `semantic_memory.rs`: "Postgres" and "database"
/// are the same axis, so the sentences are near-identical to the model but not
/// to the n-gram embedding.
struct Axes;
impl Embedder for Axes {
    fn id(&self) -> String {
        "fake:axes".into()
    }
    fn embed(&self, text: &str) -> Option<Vec<f32>> {
        let t = text.to_lowercase();
        let hit = |ws: &[&str]| ws.iter().filter(|w| t.contains(*w)).count() as f32;
        Some(vec![hit(&["postgres", "database", "db"]), hit(&["tabs", "indent"]), 0.05])
    }
}

#[test]
fn with_embeddings_a_reworded_fact_is_the_one_to_replace() {
    let s = store();
    let c = s.conn();
    let e = Axes;
    memory::add_with(c, 1, "the database is Postgres 15", Some(&e)).unwrap();
    memory::add_with(c, 1, "indent with tabs", Some(&e)).unwrap();
    let r = memory::propose_with(c, &agent_fact("our db moved to Postgres 16"), Some(&e)).unwrap();
    let Proposed::Replaces { proposal, .. } = r else { panic!("{r:?}") };
    memory::approve_with(c, proposal, Some(&e)).unwrap();
    let texts: Vec<_> = memory::top(c, 1, 10).unwrap().into_iter().map(|f| f.text).collect();
    assert_eq!(texts.len(), 2);
    assert!(texts.contains(&"our db moved to Postgres 16".to_string()));
    // The vector followed the new wording.
    let again = memory::propose_with(c, &agent_fact("db: Postgres 17 now"), Some(&e)).unwrap();
    assert!(matches!(again, Proposed::Replaces { .. }), "{again:?}");
}

#[test]
fn the_queue_knows_what_each_entry_is() {
    let s = store();
    let c = s.conn();
    memory::enqueue(
        c,
        &NewProposal {
            kind: ProposalKind::Lesson,
            evidence: Some("session 3 · seq 41"),
            suspicious: true,
            ..NewProposal::fact(1, "`pnpm test` ล้มแล้วผ่านหลังแก้ src/a.ts")
        },
    )
    .unwrap();
    let q = memory::pending(c).unwrap();
    assert_eq!(q[0].kind, ProposalKind::Lesson);
    assert_eq!(q[0].evidence.as_deref(), Some("session 3 · seq 41"));
    assert!(q[0].suspicious);
    // An ordinary proposal reads back as a plain, trusted fact.
    memory::propose(c, 1, "plain", None).unwrap();
    let q = memory::pending(c).unwrap();
    assert_eq!((q[1].kind, q[1].suspicious, q[1].replaces), (ProposalKind::Fact, false, None));
}

#[test]
fn counters_move_only_when_told_and_reset_on_rewording() {
    let s = store();
    let c = s.conn();
    let id = memory::add(c, 1, "ใช้ pnpm ไม่ใช่ npm").unwrap();
    // Reading facts for a prompt does not count as injecting them.
    memory::relevant(c, 1, "pnpm", 5).unwrap();
    let get = || -> (i64, i64) {
        c.query_row("SELECT injected, referenced FROM fact WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
    };
    assert_eq!(get(), (0, 0));
    memory::record_injected(c, &[id]).unwrap();
    memory::record_injected(c, &[id]).unwrap();
    memory::record_referenced(c, &[id]).unwrap();
    assert_eq!(get(), (2, 1));

    let Proposed::Replaces { proposal, .. } = memory::propose_with(c, &agent_fact("ใช้ yarn ไม่ใช่ npm"), None).unwrap() else { panic!() };
    memory::approve_with(c, proposal, None).unwrap();
    assert_eq!(get(), (0, 0));
}

#[test]
fn an_output_mentions_a_fact_when_it_names_what_the_fact_is_about() {
    assert!(memory::mentions("ran `pnpm test` and it passed", "ใช้ pnpm ไม่ใช่ npm"));
    assert!(memory::mentions("edited src/order/repo.ts", "eager load อยู่ใน src/order/repo.ts"));
    assert!(!memory::mentions("fixed the typo in README", "ใช้ pnpm ไม่ใช่ npm"));
    // Thai-only facts have no Latin terms: matched by overlapping text.
    assert!(memory::mentions("รับทราบครับ ตอบเป็นภาษาไทยเสมอ ตามที่ตั้งไว้", "ตอบเป็นภาษาไทยเสมอ"));
    assert!(!memory::mentions("all done, tests pass", "ตอบเป็นภาษาไทยเสมอ"));
    // Stop words alone prove nothing.
    assert!(!memory::mentions("use the thing", "use the"));
}

#[test]
fn retirement_is_suggested_for_unused_and_dead_facts_only() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/live.rs"), "").unwrap();
    let s = store();
    let c = s.conn();
    let unused = memory::add(c, 1, "always run the linter first").unwrap();
    let used = memory::add(c, 1, "tests live in tests/").unwrap();
    let dead = memory::add(c, 1, "entry point is src/gone.rs").unwrap();
    let live = memory::add(c, 1, "entry point is src/live.rs").unwrap();
    let pinned = memory::add(c, 1, "never push to main").unwrap();
    c.execute("UPDATE fact SET pinned = 1 WHERE id = ?1", [pinned]).unwrap();
    for _ in 0..memory::RETIRE_AFTER_INJECTIONS {
        memory::record_injected(c, &[unused, used, pinned]).unwrap();
    }
    memory::record_referenced(c, &[used]).unwrap();

    let made = memory::suggest_retirements(c, dir.path(), 10).unwrap();
    assert_eq!(made.len(), 2);
    let q = memory::pending(c).unwrap();
    let targets: Vec<_> = q.iter().map(|p| p.replaces.unwrap()).collect();
    assert!(targets.contains(&unused) && targets.contains(&dead), "{targets:?}");
    assert!(!targets.contains(&live) && !targets.contains(&used) && !targets.contains(&pinned));
    assert!(q.iter().all(|p| p.kind == ProposalKind::Retire && p.evidence.is_some()));

    // Asking again does not queue the same fact twice, and the cap holds.
    assert!(memory::suggest_retirements(c, dir.path(), 10).unwrap().is_empty());

    // Approving a retirement deletes the fact.
    let p = q.iter().find(|p| p.replaces == Some(dead)).unwrap();
    memory::approve_with(c, p.id, None).unwrap();
    assert!(!memory::top(c, 1, 50).unwrap().iter().any(|f| f.id == dead));
}

#[test]
fn schema_v9_is_additive_over_a_v8_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v8.sqlite3");
    let id;
    {
        let s = Store::open(&path).unwrap();
        let c = s.conn();
        id = memory::add(c, 1, "survives the upgrade").unwrap();
        memory::propose(c, 1, "so does this proposal", None).unwrap();
        c.execute_batch(
            "ALTER TABLE fact DROP COLUMN injected; ALTER TABLE fact DROP COLUMN referenced;
             ALTER TABLE fact DROP COLUMN last_injected_ms;
             ALTER TABLE fact_proposal DROP COLUMN kind; ALTER TABLE fact_proposal DROP COLUMN replaces;
             ALTER TABLE fact_proposal DROP COLUMN evidence; ALTER TABLE fact_proposal DROP COLUMN suspicious;
             ALTER TABLE session DROP COLUMN lessons_seq; ALTER TABLE session DROP COLUMN lessons_ms;
             PRAGMA user_version = 8;",
        )
        .unwrap();
    }
    let s = Store::open(&path).unwrap();
    let c = s.conn();
    let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(v, schema::SCHEMA_VERSION);
    assert_eq!(v, 9);
    let q = memory::pending(c).unwrap();
    assert_eq!((q.len(), q[0].kind), (1, ProposalKind::Fact));
    memory::record_injected(c, &[id]).unwrap();
    // Opening a v9 database again changes nothing.
    drop(s);
    assert!(Store::open(&path).is_ok());
}
