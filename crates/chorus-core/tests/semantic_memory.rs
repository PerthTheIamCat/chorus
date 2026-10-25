//! Embedding-based memory (storage spec §6): cosine dedup and vector
//! retrieval, with the n-gram path as the fallback.

use chorus_core::embedding::Embedder;
use chorus_core::{memory, store::Store};

/// A "model" with a hand-made meaning space: each concept is an axis, and the
/// text maps to the axes whose trigger words it mentions. Deterministic, so
/// the tests pin behaviour, not a real model's numbers.
struct Concepts {
    id: &'static str,
    up: bool,
}

const AXES: [&[&str]; 3] = [
    &["docker", "compose", "container"],
    &["oauth", "sso", "authenticat", "log in", "login", "sign in"],
    &["tabs", "indent", "spaces"],
];

impl Embedder for Concepts {
    fn id(&self) -> String {
        self.id.into()
    }
    fn embed(&self, text: &str) -> Option<Vec<f32>> {
        if !self.up {
            return None;
        }
        let t = text.to_lowercase();
        let mut v: Vec<f32> = AXES.iter().map(|ws| ws.iter().filter(|w| t.contains(*w)).count() as f32).collect();
        // A little shared noise so "close" is not "identical".
        v.push(0.2);
        Some(v)
    }
}

const MODEL: Concepts = Concepts { id: "fake:concepts", up: true };

fn store() -> Store {
    let s = Store::open_in_memory().unwrap();
    for t in ["we deploy with docker compose", "authentication uses OAuth via SSO", "the team indents with tabs"] {
        memory::add_with(s.conn(), 1, t, Some(&MODEL)).unwrap();
    }
    s
}

#[test]
fn a_fact_is_found_by_meaning_without_sharing_words() {
    let s = store();
    // Not one word in common with "authentication uses OAuth via SSO".
    let q = "how do I log in?";
    let semantic = memory::relevant_with(s.conn(), 1, q, 1, Some(&MODEL)).unwrap();
    assert_eq!(semantic[0].text, "authentication uses OAuth via SSO");
}

#[test]
fn a_restated_fact_is_merged_not_stored_twice() {
    let s = store();
    let before: i64 = s.conn().query_row("SELECT COUNT(*) FROM fact", [], |r| r.get(0)).unwrap();
    let a = memory::add_with(s.conn(), 1, "deployment: docker compose containers", Some(&MODEL)).unwrap();
    assert!(a.merged, "same meaning should merge");
    let after: i64 = s.conn().query_row("SELECT COUNT(*) FROM fact", [], |r| r.get(0)).unwrap();
    assert_eq!(before, after);
    let kept: String = s.conn().query_row("SELECT text FROM fact WHERE id = ?1", [a.id], |r| r.get(0)).unwrap();
    assert_eq!(kept, "we deploy with docker compose", "the first wording wins");
    // A different fact is not merged.
    let b = memory::add_with(s.conn(), 1, "the database is PostgreSQL", Some(&MODEL)).unwrap();
    assert!(!b.merged);
}

#[test]
fn approving_a_proposal_merges_too() {
    let s = store();
    let p = memory::propose(s.conn(), 1, "docker compose is how we ship", None).unwrap();
    let a = memory::approve_with(s.conn(), p, Some(&MODEL)).unwrap();
    assert!(a.merged);
    assert!(memory::pending(s.conn()).unwrap().is_empty(), "the proposal is consumed either way");
}

#[test]
fn without_an_embedder_everything_is_the_old_ngram_path() {
    let s = store();
    let down = Concepts { id: "fake:concepts", up: false };
    // Down: no merge, and retrieval equals plain `relevant`.
    let a = memory::add_with(s.conn(), 1, "deployment: docker compose containers", Some(&down)).unwrap();
    assert!(!a.merged);
    let q = "docker compose deploy";
    let with = memory::relevant_with(s.conn(), 1, q, 3, Some(&down)).unwrap();
    let without = memory::relevant(s.conn(), 1, q, 3).unwrap();
    assert_eq!(with.iter().map(|f| f.id).collect::<Vec<_>>(), without.iter().map(|f| f.id).collect::<Vec<_>>());
}

#[test]
fn vectors_from_another_model_are_never_compared() {
    let s = store();
    let other = Concepts { id: "fake:other-model", up: true };
    // Nothing stored under `other` yet: it cannot rank, so n-gram decides.
    let got = memory::relevant_with(s.conn(), 1, "how do I log in?", 3, Some(&other)).unwrap();
    assert_eq!(got.len(), 3);
    // And `other` does not merge with facts embedded by MODEL.
    assert!(!memory::add_with(s.conn(), 1, "deployment: docker compose containers", Some(&other)).unwrap().merged);
}

#[test]
fn backfill_embeds_old_facts_and_stops_when_the_model_goes_away() {
    let s = Store::open_in_memory().unwrap();
    for t in ["old fact about docker", "old fact about oauth"] {
        memory::add(s.conn(), 1, t).unwrap(); // before any model: no vector
    }
    let down = Concepts { id: "fake:concepts", up: false };
    assert_eq!(memory::backfill(s.conn(), &down, 10).unwrap(), 0);
    assert_eq!(memory::backfill(s.conn(), &MODEL, 1).unwrap(), 1, "respects the limit");
    assert_eq!(memory::backfill(s.conn(), &MODEL, 10).unwrap(), 1);
    assert_eq!(memory::backfill(s.conn(), &MODEL, 10).unwrap(), 0, "nothing left");
    let got = memory::relevant_with(s.conn(), 1, "sign in flow", 1, Some(&MODEL)).unwrap();
    assert_eq!(got[0].text, "old fact about oauth");
}

#[test]
fn a_v7_database_gains_the_embedding_table() {
    let dir = std::env::temp_dir().join(format!("chorus-v7-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("s.db");
    let _ = std::fs::remove_file(&db);
    {
        let s = Store::open(&db).unwrap();
        s.conn().execute_batch("DROP TABLE fact_embedding; PRAGMA user_version = 7;").unwrap();
    }
    let s = Store::open(&db).unwrap();
    let n: i64 = s
        .conn()
        .query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'fact_embedding'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
    let v: i64 = s.conn().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(v, chorus_core::schema::SCHEMA_VERSION);
    let _ = std::fs::remove_dir_all(&dir);
}
