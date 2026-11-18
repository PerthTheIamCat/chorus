//! An agent's own ACP plan becomes the session's todo list (the same
//! checklist `chorus_update_checklist` feeds), with no tool call from the
//! agent. Its own test binary: it listens on the process-global push sink.
//!
//! Frames follow agentclientprotocol.com/protocol/v1/agent-plan: every
//! `plan` update carries the whole list and replaces the previous one.

use chorus_cli::session::{Policy, Session};
use chorus_core::agent_control::get_checklist;
use std::path::Path;
use std::sync::{Arc, Mutex};

mod common;

fn replay_bridge(name: &str) -> Vec<String> {
    let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    vec![
        common::probe_bin().to_string_lossy().into_owned(),
        "--serve-mock".into(),
        "--replay".into(),
        f.to_string_lossy().into_owned(),
    ]
}

#[test]
fn the_agents_plan_lands_in_the_checklist_and_streams_live() {
    let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    chorus_cli::set_push_sink(Some(Box::new(move |p: &chorus_cli::Push| {
        sink.lock().unwrap().push((p.kind.into(), p.text.into()));
    })));

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");
    let mut s = Session::create(&db, dir.path(), "plan")
        .unwrap()
        .with_bridge(replay_bridge("plan_updates.jsonl"));
    s.run_turn("codex", "openai", "fix it", Policy::AllowAll).unwrap();
    chorus_cli::set_push_sink(None);

    let store = chorus_core::store::Store::open(&db).unwrap();
    let items = get_checklist(store.conn(), s.session_id()).unwrap();
    let got: Vec<(&str, bool, bool)> =
        items.iter().map(|i| (i.text.as_str(), i.done, i.in_progress)).collect();
    assert_eq!(
        got,
        [("Read the module", true, false), ("Write the fix", false, true), ("Run the tests", false, false)]
    );

    // Both plan frames were pushed live, already in checklist shape.
    let plans: Vec<serde_json::Value> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|(k, _)| k == "plan")
        .map(|(_, t)| serde_json::from_str(t).unwrap())
        .collect();
    assert_eq!(plans.len(), 2);
    assert_eq!(plans[0][0]["in_progress"], true);
    assert_eq!(plans[1][0]["done"], true);
}
