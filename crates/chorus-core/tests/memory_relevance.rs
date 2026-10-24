use chorus_core::{memory, store::Store};

fn store() -> Store {
    let s = Store::open_in_memory().unwrap();
    let c = s.conn();
    for t in [
        "deploys go through GitHub Actions on tag push",
        "database is PostgreSQL 16 with pgbouncer",
        "ผู้ใช้ชอบให้ตอบเป็นภาษาไทย",
        "tests live under tests/ and use pytest",
        "never commit .env files",
        "frontend uses SvelteKit with Tailwind",
    ] {
        memory::add(c, 1, t).unwrap();
    }
    s
}

#[test]
fn the_fact_about_the_question_comes_first() {
    let s = store();
    let got = memory::relevant(s.conn(), 1, "how do we deploy a release with actions?", 3).unwrap();
    assert!(got[0].text.contains("GitHub Actions"), "{:?}", got.iter().map(|f| &f.text).collect::<Vec<_>>());
    let got = memory::relevant(s.conn(), 1, "ตอบเป็นภาษาไทยให้หน่อย", 2).unwrap();
    assert!(got[0].text.contains("ภาษาไทย"));
}

#[test]
fn pinned_facts_survive_an_unrelated_query() {
    let s = store();
    let id = memory::top(s.conn(), 1, 10).unwrap().into_iter().find(|f| f.text.contains(".env")).unwrap().id;
    s.conn().execute("UPDATE fact SET pinned = 1 WHERE id = ?1", [id]).unwrap();
    let got = memory::relevant(s.conn(), 1, "postgres connection pool", 2).unwrap();
    assert!(got.iter().any(|f| f.text.contains(".env")));
    assert!(got.iter().any(|f| f.text.contains("PostgreSQL")));
}

#[test]
fn empty_query_and_small_k_behave_like_top() {
    let s = store();
    assert_eq!(memory::relevant(s.conn(), 1, "  ", 4).unwrap().len(), 4);
    assert_eq!(memory::relevant(s.conn(), 1, "deploy", 0).unwrap().len(), 0);
    assert_eq!(memory::relevant(s.conn(), 1, "deploy", 100).unwrap().len(), 6);
}
