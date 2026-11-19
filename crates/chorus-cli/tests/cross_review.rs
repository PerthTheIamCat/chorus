//! R6a: after a builder turn that changed files, a reviewer from another
//! provider checks the change against the criteria agreed up front, and the
//! result lands in the ledger.

use chorus_cli::session::{AgentSpec, CrossReview, Policy, Session};
use chorus_core::{journal, ledger, EventKind, Store};
use std::path::{Path, PathBuf};

mod common;

fn bridge(extra: &[&str]) -> Vec<String> {
    let mut b = vec![common::probe_bin().to_string_lossy().into_owned(), "--serve-mock".into()];
    b.extend(extra.iter().map(|s| s.to_string()));
    b
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name).to_string_lossy().into_owned()
}

fn spec(name: &str, provider: &str, extra: &[&str]) -> AgentSpec {
    let mut s = AgentSpec::new(name, provider);
    s.bridge = bridge(extra);
    s
}

/// `(actor, kind, body)` of the whole session.
fn rows(db: &Path, sess: i64) -> Vec<(String, EventKind, String)> {
    let store = Store::open(db).unwrap();
    let c = store.conn();
    ledger::history(c, sess, 1_000)
        .unwrap()
        .iter()
        .map(|e| {
            (
                chorus_core::actor::name(c, e.actor).unwrap_or_default(),
                e.kind,
                ledger::body(c, e).unwrap(),
            )
        })
        .collect()
}

fn setup() -> (tempfile::TempDir, PathBuf, Session) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");
    let s = Session::create(&db, dir.path(), "review").unwrap();
    (dir, db, s)
}

#[test]
fn a_reviewer_from_another_provider_judges_the_change_against_the_criteria() {
    let (_dir, db, mut s) = setup();
    let mut review = CrossReview::new(
        spec("codex", "openai", &["--replay", &fixture("review_issues.jsonl")]),
        vec!["every new file has a test".into()],
    );
    review.policy = Policy::AllowAll; // read-only needs Seatbelt (CLAUDE.md 7.22)
    s.set_cross_review(Some(review));

    let builder = spec("claude", "anthropic", &["--rogue", "good.txt"]);
    s.run_relay(&[builder], "add good.txt", Policy::AllowAll).unwrap();

    let rows = rows(&db, s.session_id());
    let said = rows.iter().find(|(who, k, _)| who == "codex" && *k == EventKind::Msg).expect("reviewer's message is in the ledger");
    assert!(said.2.starts_with("ISSUES"), "{}", said.2);
    let marker = rows
        .iter()
        .find(|(_, k, b)| *k == EventKind::Decision && b.starts_with(journal::REVIEW))
        .expect("review marker");
    assert!(marker.2.contains("\"verdict\":\"issues\"") && marker.2.contains("every new file has a test"), "{}", marker.2);
}

#[test]
fn a_reviewer_from_the_builders_own_provider_is_skipped() {
    let (_dir, db, mut s) = setup();
    let mut review = CrossReview::new(spec("claude-sonnet", "anthropic", &[]), vec![]);
    review.policy = Policy::AllowAll;
    s.set_cross_review(Some(review));
    s.run_relay(&[spec("claude", "anthropic", &["--rogue", "good.txt"])], "add good.txt", Policy::AllowAll).unwrap();

    let rows = rows(&db, s.session_id());
    assert!(rows.iter().all(|(who, _, _)| who != "claude-sonnet"), "the same provider did not review");
    let marker = rows.iter().find(|(_, _, b)| b.starts_with(journal::REVIEW)).expect("the skip is recorded");
    assert!(marker.2.contains("\"verdict\":\"skipped\""), "{}", marker.2);
}

#[test]
fn nothing_is_reviewed_when_nothing_changed() {
    let (_dir, db, mut s) = setup();
    let mut review = CrossReview::new(spec("codex", "openai", &[]), vec![]);
    review.policy = Policy::AllowAll;
    s.set_cross_review(Some(review));
    s.run_relay(&[spec("claude", "anthropic", &[])], "just talk", Policy::AllowAll).unwrap();
    assert!(rows(&db, s.session_id()).iter().all(|(_, _, b)| !b.starts_with(journal::REVIEW)));
}
