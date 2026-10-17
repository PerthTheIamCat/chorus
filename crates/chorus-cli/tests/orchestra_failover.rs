//! The orchestrator relays to its backup with a handoff brief when the lead
//! hits a limit (orchestra decision 3). Its own binary: it points the
//! per-agent bridge variables at the mock, which every test in a binary sees.

use chorus_cli::session::{Policy, Session};
use chorus_core::router::Candidate;

fn probe_bin() -> String {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("acp-probe").to_string_lossy().into_owned()
}

fn cand(agent: &str, id: &str, tier: u8) -> Candidate {
    let (base, effort) = chorus_cli::orchestra::split_effort(id);
    Candidate {
        id: format!("{agent}:{id}"),
        agent: agent.into(),
        model: base.into(),
        effort: effort.unwrap_or("default").into(),
        media_types: vec![],
        integrations: vec![],
        ready: true,
        tier: Some(tier),
    }
}

#[test]
fn lead_at_its_limit_hands_off_to_the_backup() {
    let bin = probe_bin();
    // Claude (the default lead) is out of usage; Codex answers.
    std::env::set_var(
        "CHORUS_CLAUDE_BRIDGE",
        format!("{bin} --serve-mock --fail usageLimitExceeded"),
    );
    std::env::set_var("CHORUS_CODEX_BRIDGE", format!("{bin} --serve-mock"));

    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::create(&dir.path().join("s.db"), dir.path(), "orch").unwrap();
    s.set_child_candidates(vec![
        cand("claude", "opus", 2),
        cand("codex", "gpt-6-astra[medium]", 2),
    ]);
    s.set_model("claude", "haiku");

    let r = s
        .run_orchestrator_turn("plan the refactor", &[], "", "", Policy::AllowAll)
        .unwrap();
    assert_eq!(r.lead[0].agent, "claude");
    assert_eq!(r.lead[1].model, "gpt-6-astra[medium]");
    assert_eq!(r.relay.answered_by, "codex");
    assert_eq!(r.relay.handoffs.len(), 1, "{:?}", r.relay.handoffs);
    assert_eq!(r.relay.handoffs[0].0, "claude");

    // The switch is in the thread, as a handoff brief carrying the goal.
    let handoff = s
        .transcript()
        .unwrap()
        .into_iter()
        .find(|(e, _)| e.kind == chorus_core::EventKind::Handoff)
        .expect("handoff event");
    assert!(handoff.1.contains("plan the refactor"), "{}", handoff.1);

    // The orchestrator's model pins do not leak into the user's next send.
    assert_eq!(s.model_for("claude"), Some("haiku"));
    assert_eq!(s.model_for("codex"), None);
}
