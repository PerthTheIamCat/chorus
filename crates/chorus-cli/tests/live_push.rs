//! Live signals while a turn runs (ux-v2 issue 03). Its own test binary: the
//! push sink is process-global, and other tests' turns would land in it.

use chorus_cli::session::{Policy, Session, SessionReader};
use std::sync::{Arc, Mutex};

fn probe_bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("acp-probe")
}

fn mock_bridge() -> Vec<String> {
    vec![probe_bin().to_string_lossy().into_owned(), "--serve-mock".into()]
}

/// One sink per test, one test at a time.
static SERIAL: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
struct Seen {
    session_id: i64,
    seq: i64,
    kind: String,
    text: String,
}

fn collect() -> Arc<Mutex<Vec<Seen>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    chorus_cli::set_push_sink(Some(Box::new(move |p: &chorus_cli::Push| {
        sink.lock().unwrap().push(Seen {
            session_id: p.session_id,
            seq: p.seq,
            kind: p.kind.into(),
            text: p.text.into(),
        });
    })));
    seen
}

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("chorus-push-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn concurrent_sessions_get_pushes_routed_by_session_id() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // Two stores so the turns never contend for one write lock; a throwaway
    // session in the second gives the real one a different id.
    let (dir_a, dir_b) = (scratch("a"), scratch("b"));
    let a = Session::create(&dir_a.join("s.db"), &dir_a, "a").unwrap().with_bridge(mock_bridge());
    let _pad = Session::create(&dir_b.join("s.db"), &dir_b, "pad").unwrap();
    let b = Session::create(&dir_b.join("s.db"), &dir_b, "b").unwrap().with_bridge(mock_bridge());
    let (id_a, id_b) = (a.session_id(), b.session_id());
    assert_ne!(id_a, id_b);
    let (read_a, read_b) = (a.reader().unwrap(), b.reader().unwrap());

    let seen = collect();
    let run = |mut s: Session, text: &'static str| {
        std::thread::spawn(move || {
            s.run_turn("codex", "openai", text, Policy::AllowAll).expect("turn runs");
        })
    };
    let (ta, tb) = (run(a, "look in a"), run(b, "look in b"));
    ta.join().unwrap();
    tb.join().unwrap();
    chorus_cli::set_push_sink(None);
    let seen = seen.lock().unwrap().clone();

    for (id, dir, reader) in [(id_a, &dir_a, &read_a), (id_b, &dir_b, &read_b)] {
        let mine: Vec<&Seen> = seen.iter().filter(|p| p.session_id == id).collect();
        let kinds: Vec<&str> = mine.iter().map(|p| p.kind.as_str()).collect();
        for k in ["status", "message", "usage", "tool", "tool_done", "events"] {
            assert!(kinds.contains(&k), "session {id} got no `{k}`: {kinds:?}");
        }
        // The mock names its cwd in the tool title: a token pushed under the
        // wrong session would carry the other workspace's path.
        let canon = dir.canonicalize().unwrap();
        for p in mine.iter().filter(|p| p.kind == "tool") {
            assert!(p.text.contains(&*canon.to_string_lossy()), "{id} got {p:?}");
        }
        let statuses: Vec<&str> =
            mine.iter().filter(|p| p.kind == "status").map(|p| p.text.as_str()).collect();
        assert_eq!(statuses, ["running", "idle"], "session {id}");
        // Every `events` seq is a row of this session, and the last one is
        // its head.
        let rows: Vec<i64> = reader.events_since(0).unwrap().iter().map(|r| r.0).collect();
        let announced: Vec<i64> =
            mine.iter().filter(|p| p.kind == "events").map(|p| p.seq).collect();
        assert!(announced.iter().all(|s| rows.contains(s)), "{announced:?} ⊄ {rows:?}");
        assert_eq!(announced.last(), rows.last());
        assert_eq!(reader.head_seq(), *rows.last().unwrap());
    }

    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

#[test]
fn a_dropped_push_is_recovered_by_the_next_pull() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = scratch("drop");
    let mut s = Session::create(&dir.join("s.db"), &dir, "drop").unwrap().with_bridge(mock_bridge());
    let reader: Arc<Mutex<SessionReader>> = Arc::new(Mutex::new(s.reader().unwrap()));

    // A shell that loses the first `events` push (the user's own message),
    // and pulls on the read-only handle from inside the turn when usage moves.
    let state = Arc::new(Mutex::new((0usize, Vec::<i64>::new(), None::<Option<i64>>)));
    let (st, rd) = (state.clone(), reader.clone());
    chorus_cli::set_push_sink(Some(Box::new(move |p: &chorus_cli::Push| {
        let mut st = st.lock().unwrap();
        match p.kind {
            "events" => {
                st.0 += 1;
                if st.0 > 1 {
                    st.1.push(p.seq);
                }
            }
            "usage" if st.2.is_none() => {
                // Mid-turn: the turn's connection is busy writing this session,
                // the reader still sees the stored meter.
                let u = rd.lock().unwrap().usage().unwrap();
                st.2 = Some(u.and_then(|u| u.context_used));
            }
            _ => {}
        }
    })));
    let report = s.run_turn("codex", "openai", "look", Policy::AllowAll).unwrap();
    chorus_cli::set_push_sink(None);

    let (dropped, delivered, mid_turn_usage) = state.lock().unwrap().clone();
    assert!(dropped >= 2, "expected the user push and an agent push");
    assert_eq!(mid_turn_usage, Some(Some(12_000)), "reader saw no usage mid-turn");
    // The shell had seen nothing (last seq 0); the push it did get says how
    // far to pull, and the pull brings back the row whose push was lost.
    let up_to = *delivered.last().unwrap();
    let rows = reader.lock().unwrap().events_since(0).unwrap();
    assert_eq!(rows.last().unwrap().0, up_to);
    assert!(rows.iter().any(|r| r.0 == report.user_seq && r.3 == "look"), "{rows:?}");
    // And a pull from where it left off is empty: nothing is pulled twice.
    assert!(reader.lock().unwrap().events_since(up_to).unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}
