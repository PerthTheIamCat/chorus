//! The Session Engine, driven against the in-repo mock agent (no network).
//! Proves the core invariant: one prompt turn, every structured thing the
//! agent did, lands in one app-owned ledger with an actor.

use chorus_cli::session::{AgentSpec, Permission, Policy, Session};
use chorus_core::handoff::HandoffReason;

fn probe_bin() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("acp-probe")
}

fn mock_bridge() -> Vec<String> {
    vec![
        probe_bin().to_string_lossy().into_owned(),
        "--serve-mock".into(),
    ]
}

/// A mock that writes `name` into the workspace mid-turn without asking.
fn rogue_bridge(name: &str) -> Vec<String> {
    vec![
        probe_bin().to_string_lossy().into_owned(),
        "--serve-mock".into(),
        "--rogue".into(),
        name.into(),
    ]
}

/// A mock that fails every prompt with `msg` in a JSON-RPC error.
fn failing_bridge(msg: &str) -> Vec<String> {
    vec![
        probe_bin().to_string_lossy().into_owned(),
        "--serve-mock".into(),
        "--fail".into(),
        msg.into(),
    ]
}

#[test]
fn one_turn_persists_user_and_every_agent_event_to_the_ledger() {
    let tmp = std::env::temp_dir().join(format!("chorus-sess-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let db = tmp.join("store.db");
    let _ = std::fs::remove_file(&db);

    let mut s = Session::create(&db, &tmp, "mock task")
        .unwrap()
        .with_bridge(mock_bridge());

    let report = s
        .run_turn("codex", "openai", "look at the readme", Policy::AllowAll)
        .expect("turn runs");

    assert_eq!(report.stop_reason, "end_turn");
    assert!(!report.agent_seqs.is_empty(), "no agent events persisted");

    // Everything is in one session, user first, agent events parented to it.
    let transcript = s.transcript().unwrap();
    let kinds: Vec<_> = transcript.iter().map(|(e, _)| e.kind).collect();
    assert_eq!(
        kinds.first(),
        Some(&chorus_core::EventKind::Msg),
        "user msg leads"
    );
    assert!(
        kinds.contains(&chorus_core::EventKind::ToolCall),
        "tool call not in ledger: {kinds:?}"
    );
    assert!(
        kinds.contains(&chorus_core::EventKind::Diff),
        "diff not in ledger: {kinds:?}"
    );

    // The diff event carries a path files_touched can recover.
    let diff_body = transcript
        .iter()
        .find(|(e, _)| e.kind == chorus_core::EventKind::Diff)
        .map(|(_, b)| b.clone())
        .unwrap();
    let files =
        chorus_core::handoff::files_touched(&[(transcript[0].0.clone(), diff_body.clone())]);
    assert!(
        files.iter().any(|f| f.ends_with("README.md")),
        "diff path lost: {diff_body}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn a_second_turn_reconstructs_history_from_the_ledger() {
    let tmp = std::env::temp_dir().join(format!("chorus-multi-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let db = tmp.join("store.db");
    let _ = std::fs::remove_file(&db);

    let mut s = Session::create(&db, &tmp, "multi turn")
        .unwrap()
        .with_bridge(mock_bridge());

    s.run_turn(
        "codex",
        "openai",
        "first: look at the readme",
        Policy::AllowAll,
    )
    .unwrap();

    // Context assembled from the ledger carries the first turn back.
    let ctx = s.assemble_context(120_000).unwrap();
    assert!(
        ctx.contains("first: look at the readme"),
        "history missing from context:\n{ctx}"
    );
    assert!(ctx.contains("user:") || ctx.contains("user :") || ctx.contains("codex:"));

    let r2 = s
        .run_turn("codex", "openai", "second: now summarise", Policy::AllowAll)
        .unwrap();
    assert_eq!(r2.stop_reason, "end_turn");

    let user_msgs: Vec<_> = s
        .transcript()
        .unwrap()
        .into_iter()
        .filter(|(e, _)| e.kind == chorus_core::EventKind::Msg && e.body.is_some())
        .filter(|(_, b)| b.starts_with("first:") || b.starts_with("second:"))
        .map(|(_, b)| b)
        .collect();
    assert_eq!(
        user_msgs,
        vec!["first: look at the readme", "second: now summarise"]
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn handoff_records_the_switch_and_runs_the_incoming_agent() {
    let tmp = std::env::temp_dir().join(format!("chorus-ho-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let db = tmp.join("store.db");
    let _ = std::fs::remove_file(&db);

    let mut s = Session::create(&db, &tmp, "n+1 query")
        .unwrap()
        .with_bridge(mock_bridge());

    // A first turn by "claude", so the handoff has a `from`.
    s.run_turn(
        "claude-opus",
        "anthropic",
        "the order page is slow",
        Policy::AllowAll,
    )
    .unwrap();

    let r = s
        .handoff(
            "codex",
            "openai",
            HandoffReason::RateLimit,
            "get the order list under 200ms",
            vec!["found the N+1 in findAll".into()],
            vec!["add eager-load".into(), "run the order tests".into()],
            vec!["do not touch the schema".into()],
            "--- a/repo.ts\n+++ b/repo.ts\n@@\n-x\n+y\n".into(),
            Policy::AllowAll,
        )
        .expect("handoff runs");

    assert_eq!(r.from, "claude-opus");
    assert_eq!(r.to, "codex");
    assert_eq!(r.brief.goal, "get the order list under 200ms");
    assert_eq!(
        r.brief.pending,
        vec!["add eager-load", "run the order tests"]
    );
    assert!(!r.echo_back.is_empty(), "incoming agent didn't respond");

    // The ledger holds: claude's turn, a Handoff event, then codex's events.
    let t = s.transcript().unwrap();
    let handoff_at = t
        .iter()
        .position(|(e, _)| e.kind == chorus_core::EventKind::Handoff)
        .expect("handoff event in ledger");
    // The handoff body is the full brief JSON.
    assert!(t[handoff_at]
        .1
        .contains("\"goal\": \"get the order list under 200ms\""));
    // The handoff event and everything after share one actor (codex), and
    // it differs from claude's actor id on the first turn.
    let claude_actor = t[0].0.actor;
    let codex_actor = t[handoff_at].0.actor;
    assert_ne!(claude_actor, codex_actor, "handoff should switch actor");
    assert!(
        t[handoff_at + 1..]
            .iter()
            .all(|(e, _)| e.actor == codex_actor),
        "post-handoff events must belong to the incoming agent"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn the_ledger_survives_when_the_agent_says_no() {
    let tmp = std::env::temp_dir().join(format!("chorus-sess-deny-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let db = tmp.join("store.db");
    let _ = std::fs::remove_file(&db);

    let mut s = Session::create(&db, &tmp, "deny task")
        .unwrap()
        .with_bridge(mock_bridge());

    // DenyAll: the mock still finishes the turn, marking its tool call failed.
    let report = s
        .run_turn("codex", "openai", "try to edit something", Policy::DenyAll)
        .expect("turn still completes");

    assert_eq!(report.stop_reason, "end_turn");
    let transcript = s.transcript().unwrap();
    assert!(
        transcript
            .iter()
            .any(|(e, b)| e.kind == chorus_core::EventKind::ToolResult && b.contains("failed")),
        "denied tool call should be recorded as failed"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn a_rate_limited_agent_relays_to_the_next_in_the_chain() {
    let tmp = std::env::temp_dir().join(format!("chorus-relay-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let db = tmp.join("store.db");
    let _ = std::fs::remove_file(&db);

    let mut s = Session::create(&db, &tmp, "relay").unwrap();

    let chain = vec![
        AgentSpec {
            name: "claude-opus".into(),
            provider: "anthropic".into(),
            bridge: failing_bridge("Error: 429 rate limit exceeded"),
        },
        AgentSpec {
            name: "codex".into(),
            provider: "openai".into(),
            bridge: mock_bridge(),
        },
    ];

    let r = s
        .run_relay(&chain, "do the work", Policy::AllowAll)
        .expect("relay resolves");

    assert_eq!(r.answered_by, "codex");
    assert_eq!(r.handoffs.len(), 1);
    assert_eq!(
        r.handoffs[0],
        (
            "claude-opus".into(),
            "codex".into(),
            HandoffReason::RateLimit
        )
    );

    // The ledger tells the story: user turn, then a Handoff event, then the
    // incoming agent's events — all in the one session.
    let t = s.transcript().unwrap();
    let kinds: Vec<_> = t.iter().map(|(e, _)| e.kind).collect();
    assert_eq!(
        kinds.first(),
        Some(&chorus_core::EventKind::Msg),
        "user turn leads"
    );
    let ho = kinds
        .iter()
        .position(|k| *k == chorus_core::EventKind::Handoff)
        .expect("handoff event recorded");
    assert!(
        ho > 0 && ho < kinds.len() - 1,
        "handoff is mid-ledger: {kinds:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn a_real_error_is_not_routed_around() {
    let tmp = std::env::temp_dir().join(format!("chorus-relay-err-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let db = tmp.join("store.db");
    let _ = std::fs::remove_file(&db);

    let mut s = Session::create(&db, &tmp, "relay err").unwrap();
    let chain = vec![
        AgentSpec {
            name: "claude-opus".into(),
            provider: "anthropic".into(),
            bridge: failing_bridge("TypeError: cannot read property of undefined"),
        },
        AgentSpec {
            name: "codex".into(),
            provider: "openai".into(),
            bridge: mock_bridge(),
        },
    ];

    // A bug in the first agent is not a reason to burn the chain.
    assert!(s
        .run_relay(&chain, "do the work", Policy::AllowAll)
        .is_err());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn memory_is_projected_into_the_agent_file_but_a_hand_edit_is_left_alone() {
    let tmp = std::env::temp_dir().join(format!("chorus-proj-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let db = tmp.join("store.db");
    let _ = std::fs::remove_file(&db);

    let mut s = Session::create(&db, &tmp, "proj")
        .unwrap()
        .with_bridge(mock_bridge());

    // Seed a project-scope fact, then run a codex turn.
    chorus_core::memory::add(
        chorus_core::store::Store::open(&db).unwrap().conn(),
        1,
        "ใช้ pnpm ไม่ใช่ npm",
    )
    .unwrap();

    s.run_turn("codex", "openai", "hello", Policy::AllowAll)
        .unwrap();

    let agents_md = std::fs::read_to_string(tmp.join("AGENTS.md")).expect("AGENTS.md projected");
    assert!(agents_md.starts_with("<!-- chorus:generated b3:"));
    assert!(agents_md.contains("ใช้ pnpm ไม่ใช่ npm"));

    // The user takes it over.
    std::fs::write(tmp.join("AGENTS.md"), "# my own notes, hands off\n").unwrap();
    s.run_turn("codex", "openai", "again", Policy::AllowAll)
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(tmp.join("AGENTS.md")).unwrap(),
        "# my own notes, hands off\n",
        "a foreign file must not be overwritten"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn compare_mode_forks_the_session_per_agent() {
    let tmp = std::env::temp_dir().join(format!("chorus-cmp-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let db = tmp.join("store.db");
    let _ = std::fs::remove_file(&db);

    let mut s = Session::create(&db, &tmp, "compare").unwrap().with_bridge(mock_bridge());
    s.run_turn("user", "human", "here is the task", Policy::AllowAll).unwrap();
    let base_events = s.transcript().unwrap().len();

    let agents = vec![AgentSpec::new("codex", "openai"), AgentSpec::new("claude-opus", "anthropic")];
    let branches = s.compare(&agents, "propose a fix", Policy::AllowAll).expect("compare runs");

    assert_eq!(branches.len(), 2);
    assert_ne!(branches[0].session_id, branches[1].session_id, "each agent gets its own fork");
    assert_ne!(branches[0].session_id, s.session_id(), "forks are not the parent");
    for b in &branches {
        assert!(b.result.is_ok(), "{} failed: {:?}", b.agent, b.result.as_ref().err());
    }

    // The parent session is untouched by the comparison.
    assert_eq!(s.transcript().unwrap().len(), base_events, "compare mutated the parent");

    // A branch's history includes the pre-fork parent turn.
    let store = chorus_core::store::Store::open(&db).unwrap();
    let branch_hist = chorus_core::ledger::history(store.conn(), branches[0].session_id, 100).unwrap();
    assert!(
        branch_hist
            .iter()
            .any(|e| chorus_core::ledger::body(store.conn(), e).unwrap().contains("here is the task")),
        "branch lost the pre-fork history"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}


// ---- the warm tier and the task-state tier -------------------------------
//
// These three guard the regression that a hand-fed bench could not see: for
// months `assemble_context` passed an empty task state and regenerated every
// rung on the read path. The tests below assert against the real method.

#[test]
fn assemble_context_carries_the_task_state() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut s = Session::create(&db, &cwd, "task state").unwrap();
    s = s.with_bridge(mock_bridge());
    s.run_turn("codex", "openai", "ทำ order list ให้เร็วกว่า 200ms", Policy::AllowAll)
        .unwrap();

    let ts = s.task_state().unwrap();
    assert!(!ts.is_empty(), "task state empty right after a turn");
    assert!(ts.goal.contains("order list"), "goal not taken from the first user message: {ts:?}");

    // Even squeezed, the tier the design never trims must survive.
    let tight = s.assemble_context(800).unwrap();
    assert!(
        tight.contains("order list"),
        "the goal was trimmed out of a tight context:\n{tight}"
    );
}

#[test]
fn assemble_context_reads_the_stored_rung_instead_of_regenerating_it() {
    // A short workspace path on purpose. Every event body in this fixture
    // repeats the file's absolute path, and macOS tempdirs
    // (/private/var/folders/xx/<28 chars>/T/...) are ~70 characters longer than
    // Linux's /tmp, which at a fixed budget pushes the oldest turn off the end
    // before it can be demoted to its stored rung \u2014 the test then fails on one
    // OS only and looks like a regression in the read path. The budget here is
    // measuring the assembler, so the paths must not be the variable.
    let dir = ShortDir::new("warm-tier");
    let db = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut s = Session::create(&db, &cwd, "warm tier").unwrap();
    s = s.with_bridge(mock_bridge());
    for i in 0..3 {
        s.run_turn("codex", "openai", &format!("turn {i}"), Policy::AllowAll)
            .unwrap();
    }
    s.compact(500).unwrap();
    assert_eq!(s.uncompacted_count().unwrap(), 0);

    // Put a value in the table that no summariser would ever produce. If it
    // reaches the assembled text, the read path really is reading the store.
    {
        let store = chorus_core::Store::open(&db).unwrap();
        let c = store.conn();
        // The *oldest* event: at a tight budget the assembler demotes the
        // oldest turns down the rungs first, so that is where a stored rung
        // has to show up.
        let seq: i64 = c
            .query_row("SELECT min(seq) FROM event", [], |r| r.get(0))
            .unwrap();
        chorus_core::compaction::save_rungs(
            c,
            seq,
            &chorus_core::compaction::Rungs {
                l1: "ZZTOP-L1".into(),
                l2: "ZZTOP-L2".into(),
            },
            "test",
        )
        .unwrap();
    }

    let s = Session::open(&db, s.session_id()).unwrap();
    let tight = s.assemble_context(700).unwrap();

    assert!(
        tight.contains("ZZTOP"),
        "stored rung never used — assemble_context is regenerating:\n{tight}"
    );
}

#[test]
fn compact_is_incremental_and_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut s = Session::create(&db, &cwd, "compact").unwrap();
    s = s.with_bridge(mock_bridge());
    for i in 0..4 {
        s.run_turn("codex", "openai", &format!("turn {i}"), Policy::AllowAll)
            .unwrap();
    }

    let total = s.uncompacted_count().unwrap();
    assert!(total > 4, "expected several events, got {total}");

    // A bounded pass leaves the rest for the next one.
    let first = s.compact(2).unwrap();
    assert_eq!(first.compacted, 2);
    assert_eq!(first.remaining as i64, total - 2);

    let rest = s.compact(500).unwrap();
    assert_eq!(rest.remaining, 0);

    // Running again does nothing rather than rewriting every row.
    let again = s.compact(500).unwrap();
    assert_eq!(again.compacted, 0);
    assert_eq!(s.uncompacted_count().unwrap(), 0);
}

#[test]
fn a_relay_brief_carries_the_session_goal_not_the_last_instruction() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut s = Session::create(&db, &cwd, "relay goal").unwrap();
    s = s.with_bridge(mock_bridge());
    // Turn 1 sets what this session is actually for.
    s.run_turn("codex", "openai", "ทำหน้า order list ให้ต่ำกว่า 200ms", Policy::AllowAll)
        .unwrap();

    let chain = vec![
        AgentSpec {
            name: "claude-opus".into(),
            provider: "anthropic".into(),
            bridge: failing_bridge("Error: 429 rate limit exceeded"),
        },
        AgentSpec {
            name: "codex".into(),
            provider: "openai".into(),
            bridge: mock_bridge(),
        },
    ];
    let r = s
        .run_relay(&chain, "เขียน eager-load ใน findAll", Policy::AllowAll)
        .unwrap();
    assert_eq!(r.answered_by, "codex");

    // The handoff brief is the ledger's record of what was handed over.
    let t = s.transcript().unwrap();
    let (_, body) = t
        .iter()
        .rev()
        .find(|(e, _)| e.kind == chorus_core::EventKind::Handoff)
        .expect("handoff recorded");
    let brief: serde_json::Value = serde_json::from_str(body).unwrap();

    let goal = brief["goal"].as_str().unwrap_or_default();
    assert!(
        goal.contains("order list"),
        "the brief's goal is the in-flight instruction, not the session's objective: {goal:?}"
    );
    let pending: Vec<&str> = brief["pending"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        pending.iter().any(|p| p.contains("eager-load")),
        "the instruction that was in flight should survive as pending: {pending:?}"
    );
}

#[test]
fn list_sessions_reports_newest_first_with_agents_and_goal() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("s.db");

    let cwd_a = dir.path().join("a");
    std::fs::create_dir_all(&cwd_a).unwrap();
    let mut a = Session::create(&db, &cwd_a, "first session").unwrap();
    a = a.with_bridge(mock_bridge());
    a.run_turn(
        "codex",
        "openai",
        "look at the readme first",
        Policy::AllowAll,
    )
    .unwrap();
    let id_a = a.session_id();

    // Sleep a tick so the two sessions don't share `started_ms`,
    // otherwise "newest first" has nothing to distinguish on.
    std::thread::sleep(std::time::Duration::from_millis(50));

    let cwd_b = dir.path().join("b");
    std::fs::create_dir_all(&cwd_b).unwrap();
    let mut b = Session::create(&db, &cwd_b, "second session").unwrap();
    b = b.with_bridge(mock_bridge());
    b.run_turn(
        "codex",
        "openai",
        "now look at the readme too",
        Policy::AllowAll,
    )
    .unwrap();
    let id_b = b.session_id();

    let rows = chorus_cli::session::list_sessions(&db).expect("list_sessions runs");
    assert!(
        rows.len() >= 2,
        "expected at least the two sessions just created, got {}",
        rows.len()
    );

    // Newest activity first: session b was the last to have an event land.
    let pos_a = rows
        .iter()
        .position(|r| r.id == id_a)
        .expect("session a listed");
    let pos_b = rows
        .iter()
        .position(|r| r.id == id_b)
        .expect("session b listed");
    assert!(
        pos_b < pos_a,
        "newest session should sort first: a at {pos_a}, b at {pos_b}"
    );

    let row_a = &rows[pos_a];
    assert!(
        row_a.events >= 4,
        "expected at least user msg + tool_call + tool_result + diff, got {}",
        row_a.events
    );
    assert_eq!(
        row_a.agents,
        vec!["codex".to_string()],
        "user itself must not appear as an agent"
    );
    assert!(
        row_a.goal.contains("look at the readme first"),
        "goal should be the first user message: {:?}",
        row_a.goal
    );
    assert_eq!(row_a.forked_from, None);
}

#[test]
fn handoff_auto_carries_the_session_goal_with_no_argument_passed_in() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut s = Session::create(&db, &cwd, "auto handoff").unwrap();
    s = s.with_bridge(mock_bridge());
    s.run_turn(
        "codex",
        "openai",
        "make the order list load under 200ms",
        Policy::AllowAll,
    )
    .unwrap();

    // No goal, no done/pending/constraints — the whole point of the feature.
    let r = s
        .handoff_auto("codex", "openai", Policy::AllowAll)
        .expect("handoff_auto runs");
    assert_eq!(r.to, "codex");
    assert!(
        r.brief.goal.contains("order list"),
        "handoff_auto's brief should carry the session goal untouched: {:?}",
        r.brief.goal
    );

    // Regression guard: the Handoff event itself must carry the goal, with
    // no caller having typed it — this is what a UI button click looks like.
    let t = s.transcript().unwrap();
    let (_, body) = t
        .iter()
        .rev()
        .find(|(e, _)| e.kind == chorus_core::EventKind::Handoff)
        .expect("handoff event recorded");
    assert!(
        body.contains("order list"),
        "the Handoff event body must carry the session goal: {body}"
    );
    assert!(
        body.contains("\"reason\": \"manual\"") || body.contains("\"manual\""),
        "handoff_auto should reason Manual: {body}"
    );
}

#[test]
fn list_agents_always_returns_both_known_agents_and_never_panics() {
    // CI has neither `claude` nor `codex` on PATH — assert on shape, not on
    // this machine's actual auth state.
    let agents = chorus_cli::agents::list_agents();
    assert_eq!(
        agents.len(),
        2,
        "expected both known agents, got {}",
        agents.len()
    );

    let ids: Vec<&str> = agents.iter().map(|a| a.id.as_str()).collect();
    assert!(ids.contains(&"claude"));
    assert!(ids.contains(&"codex"));

    for a in &agents {
        assert!(!a.label.is_empty());
        assert!(!a.models.is_empty(), "{} has no curated models", a.id);
        assert!(a.models.iter().filter(|m| m.is_default).count() <= 1);
        assert!(
            !a.hint.is_empty(),
            "{} has no hint for its current state",
            a.id
        );
    }

    // auth_status must agree with list_agents for the same id, and never
    // panic no matter what's installed on the machine running the test.
    let claude = chorus_cli::agents::auth_status("claude");
    assert_eq!(claude.id, "claude");
    let codex = chorus_cli::agents::auth_status("codex");
    assert_eq!(codex.id, "codex");
}

/// A summariser that is deliberately slow, so the time bound can be observed
/// rather than assumed. This is the shape of the real problem: with Ollama
/// configured, one event costs a model call.
struct SlowSummarizer;

impl chorus_core::compaction::Summarizer for SlowSummarizer {
    fn summarize(&self, text: &str, target_chars: usize) -> String {
        std::thread::sleep(std::time::Duration::from_millis(60));
        text.chars().take(target_chars).collect()
    }
    fn label(&self) -> String {
        "slow-test".into()
    }
}

#[test]
fn a_compaction_pass_stops_on_the_clock_and_leaves_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut s = Session::create(&db, &cwd, "budget").unwrap();
    s = s.with_bridge(mock_bridge());
    for i in 0..4 {
        s.run_turn("codex", "openai", &format!("turn {i}"), Policy::AllowAll)
            .unwrap();
    }
    let total = s.uncompacted_count().unwrap();
    assert!(total >= 8, "expected a decent backlog, got {total}");

    // Each event costs ~120ms (two rungs × 60ms), so a 200ms budget must stop
    // well short of the batch — but never at zero.
    let r = s
        .compact_with(&SlowSummarizer, 500, std::time::Duration::from_millis(200))
        .unwrap();
    assert!(r.hit_budget, "pass should have stopped on the clock: {r:?}");
    assert!(r.compacted >= 1, "a pass must always make progress: {r:?}");
    assert!(
        (r.compacted as i64) < total,
        "a 200ms budget somehow did the whole backlog: {r:?}"
    );
    assert_eq!(r.remaining as i64, total - r.compacted as i64);

    // Running it again picks up where it left off rather than starting over.
    let again = s
        .compact_with(&SlowSummarizer, 500, std::time::Duration::from_secs(30))
        .unwrap();
    assert_eq!(again.remaining, 0);
    assert!(!again.hit_budget);
    assert_eq!(s.uncompacted_count().unwrap(), 0);
}

#[test]
fn a_finished_backlog_costs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut s = Session::create(&db, &cwd, "idle").unwrap();
    s = s.with_bridge(mock_bridge());
    s.run_turn("codex", "openai", "one", Policy::AllowAll).unwrap();
    s.compact(500).unwrap();

    // The pass the shell fires after *every* turn must be free once caught up,
    // even with a summariser that would otherwise take a minute.
    let r = s
        .compact_with(&SlowSummarizer, 500, std::time::Duration::from_millis(50))
        .unwrap();
    assert_eq!(r.compacted, 0);
    assert_eq!(r.took_ms, 0);
    assert!(!r.hit_budget);
}

#[test]
fn doctor_reports_every_agent_without_touching_credentials() {
    // The CLI and the Settings pane must never disagree about whether an agent
    // can run, so they share `agents::list_agents`. This asserts the shape the
    // doctor output is built from, on a machine with neither CLI installed.
    let agents = chorus_cli::agents::list_agents();
    assert_eq!(agents.len(), 2, "both known agents must always be listed");
    for a in &agents {
        assert!(!a.label.is_empty());
        assert!(!a.hint.is_empty(), "{} has no way to fix it", a.id);
        assert!(!a.detail.is_empty(), "{} explains nothing", a.id);
        assert!(!a.bridge.is_empty(), "{} has no bridge command", a.id);
        assert!(!a.models.is_empty(), "{} offers no model", a.id);
        assert!(
            a.models.iter().filter(|m| m.is_default).count() <= 1,
            "{} has more than one default model",
            a.id
        );
    }
}

/// A scratch directory with a deliberately short path, cleaned up on drop.
/// `tempfile::tempdir` is the right default everywhere else; this exists only
/// for fixtures whose assertions are sensitive to how long a path is.
struct ShortDir(std::path::PathBuf);

impl ShortDir {
    fn new(tag: &str) -> Self {
        let p = std::path::PathBuf::from(format!("/tmp/chorus-t-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for ShortDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The safety promise `--read-only` makes, tested against an agent that breaks
/// it. Answering `session/request_permission` is not enough on its own: Codex
/// writes files without ever sending one, so the guard has to be a check on the
/// workspace afterwards, not a policy the agent is trusted to consult.
#[test]
fn a_read_only_turn_that_writes_is_reverted_and_reported() {
    let dir = ShortDir::new("readonly");
    let db = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(cwd.join("keep.txt"), "mine\n").unwrap();

    let mut s = Session::create(&db, &cwd, "no writes please").unwrap();
    s = s.with_bridge(rogue_bridge("SNUCK_IN.txt"));

    let msg = match s.run_turn("codex", "openai", "อ่านอย่างเดียวนะ", Policy::ReadOnly) {
        Err(e) => format!("{e:#}"),
        Ok(_) => panic!("a write under --read-only must not be reported as success"),
    };
    assert!(msg.contains("SNUCK_IN.txt"), "the error must name the file: {msg}");

    assert!(!cwd.join("SNUCK_IN.txt").exists(), "the write was not reverted");
    assert_eq!(
        std::fs::read_to_string(cwd.join("keep.txt")).unwrap(),
        "mine\n",
        "reverting the agent's write must not disturb the user's own files"
    );
}

/// The same agent under `--yes` is left alone: the guard is a promise-keeper,
/// not a policy of its own.
#[test]
fn an_allowed_turn_keeps_what_the_agent_wrote() {
    let dir = ShortDir::new("allowed");
    let db = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut s = Session::create(&db, &cwd, "go ahead").unwrap();
    s = s.with_bridge(rogue_bridge("WANTED.txt"));
    s.run_turn("codex", "openai", "เขียนได้เลย", Policy::AllowAll)
        .expect("allowed turn");

    assert!(cwd.join("WANTED.txt").exists(), "an allowed write was reverted");
}

/// The conversation tier should not spend a line on "the tool call finished".
#[test]
fn a_bare_tool_result_never_reaches_the_context_but_a_failure_does() {
    let dir = ShortDir::new("noise");
    let db = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut s = Session::create(&db, &cwd, "noise").unwrap();
    s = s.with_bridge(mock_bridge());
    s.run_turn("codex", "openai", "ดูไฟล์ให้ที", Policy::AllowAll)
        .unwrap();

    // A result the agent reported as failed — the next agent needs to know.
    {
        let store = chorus_core::Store::open(&db).unwrap();
        let c = store.conn();
        let who = chorus_core::actor::intern(c, "codex", "openai").unwrap();
        chorus_core::ledger::append(
            c,
            &chorus_core::NewEvent::new(
                s.session_id(),
                chorus_core::EventKind::ToolResult,
                who,
                "Run tests → failed",
            ),
        )
        .unwrap();
    }

    let s = Session::open(&db, s.session_id()).unwrap();
    let ctx = s.assemble_context(100_000).unwrap();
    assert!(
        !ctx.contains("→ completed"),
        "a contentless result reached the context:\n{ctx}"
    );
    assert!(
        ctx.contains("Run tests → failed"),
        "a failed result was dropped — that one changes what to do next:\n{ctx}"
    );
}

/// The app's permission dialog has to mean something even when the agent
/// never asks. A deny plus a write is the case that makes the dialog a lie.
#[test]
fn denying_in_the_dialog_undoes_what_the_agent_wrote_anyway() {
    let dir = ShortDir::new("dialog");
    let db = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut s = Session::create(&db, &cwd, "interactive").unwrap();
    s = s.with_bridge(rogue_bridge("BEHIND_YOUR_BACK.txt"));

    let mut say_no = |_: &chorus_cli::session::PermissionAsk| Permission::Deny;
    let msg = match s.run_turn_decide("codex", "openai", "ห้ามเขียนนะ", &mut say_no) {
        Err(e) => format!("{e:#}"),
        Ok(_) => panic!("a write after a deny must not be reported as success"),
    };
    assert!(msg.contains("BEHIND_YOUR_BACK.txt"), "the error must name the file: {msg}");
    assert!(
        !cwd.join("BEHIND_YOUR_BACK.txt").exists(),
        "a write the user denied survived the turn"
    );
}

/// The model pin is a per-agent statement that survives a handoff: "use the
/// cheap model" is about the task, not about one provider, and a failover that
/// silently upgrades is a surprise on the bill.
#[test]
fn a_model_pin_is_per_agent_and_clearable() {
    let dir = ShortDir::new("model");
    let db = dir.path().join("s.db");
    let cwd = dir.path().join("w");
    std::fs::create_dir_all(&cwd).unwrap();

    let mut s = Session::create(&db, &cwd, "pick a model").unwrap();
    assert_eq!(s.model_for("codex"), None, "nothing pinned by default");

    s.set_model("codex", "gpt-5.5[low]");
    s.set_model("claude", "haiku");
    assert_eq!(s.model_for("codex"), Some("gpt-5.5[low]"));
    assert_eq!(s.model_for("claude"), Some("haiku"));

    s.set_model("codex", "");
    assert_eq!(s.model_for("codex"), None, "an empty id clears the pin");
    assert_eq!(s.model_for("claude"), Some("haiku"), "clearing one must not clear the other");
}

/// Two folders in one project see each other's facts: the first folder's own
/// facts are carried into the shared store, and a fact approved from the
/// second folder shows up in the first.
#[test]
fn project_memory_is_shared_across_folders() {
    let root = std::env::temp_dir().join(format!("chorus-projmem-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (a, b) = (root.join("a"), root.join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let shared = root.join("project/memory.db");

    let db_a = a.join("store.db");
    let mut sa = Session::create(&db_a, &a, "a").unwrap();
    chorus_core::memory::add(chorus_core::store::Store::open(&db_a).unwrap().conn(), 1, "folder A fact")
        .unwrap();
    sa.set_memory_db(&shared).unwrap();

    let mut sb = Session::create(&b.join("store.db"), &b, "b").unwrap();
    sb.set_memory_db(&shared).unwrap();
    assert!(sb.memory_state().unwrap().0.iter().any(|f| f == "folder A fact"));

    let id = chorus_core::memory::propose(
        chorus_core::store::Store::open(&shared).unwrap().conn(), 1, "from B", None).unwrap();
    sb.review_proposal(id, true).unwrap();
    assert!(sa.memory_state().unwrap().0.iter().any(|f| f == "from B"));
}
