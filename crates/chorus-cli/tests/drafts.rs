//! ✎ opens a draft; the session row appears only with the first message.

use chorus_cli::drafts::DraftBook;
use chorus_cli::session::list_sessions;

#[test]
fn no_session_row_until_the_first_send_and_plus_reuses_the_draft() {
    let dir = std::env::temp_dir().join(format!("chorus-drafts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".chorus")).unwrap();
    let db = dir.join(".chorus/store.db");
    // An earlier session in the same store, so "empty list" is not just
    // "no database".
    drop(chorus_cli::session::Session::create(&db, &dir, "earlier").unwrap());
    let before = list_sessions(&db).unwrap().len();

    let mut book = DraftBook::new();
    let first = book.open(&db, &dir);
    let second = book.open(&db, &dir);
    assert!(second.reused, "+ again must return to the unsent draft");
    assert_eq!(first.draft.id, second.draft.id);
    assert_eq!(
        list_sessions(&db).unwrap().len(),
        before,
        "a draft wrote a row"
    );

    // First message: now it is a session.
    let session = book.commit(first.draft.id, "session").unwrap();
    let rows = list_sessions(&db).unwrap();
    assert_eq!(rows.len(), before + 1);
    assert!(rows.iter().any(|s| s.id == session.session_id()));
    assert!(book.get(first.draft.id).is_none());

    // The draft is spent: + now starts a fresh one.
    let third = book.open(&db, &dir);
    assert!(!third.reused);
    assert_ne!(third.draft.id, first.draft.id);
    assert_eq!(list_sessions(&db).unwrap().len(), before + 1);

    drop(session);
    let _ = std::fs::remove_dir_all(&dir);
}
