//! E4 + F6: a scored compare runs the project's checks in every lane's
//! worktree, a judge ranks with the evidence in front of it, and each lane's
//! result is stored for the router. Its own test binary: it sets process-wide
//! environment (`CHORUS_HOME`, `CHORUS_TEST_COMMAND`).
//!
//! The project's "tests" are `test -f good.txt`; the lane whose mock agent
//! writes `good.txt` passes, the other fails.

use chorus_cli::session::judge::{compare_results, lane_stats, CompareOptions, Lane};
use chorus_cli::session::{AgentSpec, Policy, Session};
use chorus_core::router::Candidate;
use chorus_core::trust::TrustStore;
use chorus_core::Store;
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;

fn bridge(extra: &[&str]) -> Vec<String> {
    let mut b = vec![common::probe_bin().to_string_lossy().into_owned(), "--serve-mock".into()];
    b.extend(extra.iter().map(|s| s.to_string()));
    b
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn identity(argv: &[String], _: &Path) -> Result<Vec<String>, String> {
    Ok(argv.to_vec())
}

fn git(dir: &Path, args: &[&str]) {
    let o = Command::new("git").args(args).current_dir(dir).output().unwrap();
    assert!(o.status.success(), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

fn cand(agent: &str, id: &str, tier: u8) -> Candidate {
    Candidate {
        id: format!("{agent}:{id}"),
        agent: agent.into(),
        model: id.into(),
        effort: "default".into(),
        media_types: vec![],
        integrations: vec![],
        ready: true,
        tier: Some(tier),
    }
}

fn lane(name: &str, provider: &str, extra: &[&str], orchestra: bool) -> Lane {
    let mut spec = AgentSpec::new(name, provider);
    spec.bridge = bridge(extra);
    Lane { spec, orchestra }
}

#[test]
fn a_scored_compare_ranks_by_evidence_and_stores_what_the_router_reads() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("CHORUS_HOME", home.path());
    std::env::set_var("CHORUS_TEST_COMMAND", "test -f good.txt");

    let tmp: PathBuf = std::env::temp_dir().join(format!("chorus-judge-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    git(&tmp, &["init", "-q", "-b", "main"]);
    git(&tmp, &["config", "user.email", "t@t"]);
    git(&tmp, &["config", "user.name", "t"]);
    git(&tmp, &["config", "commit.gpgsign", "false"]);
    std::fs::write(tmp.join("a.txt"), "one\n").unwrap();
    git(&tmp, &["add", "."]);
    git(&tmp, &["commit", "-qm", "init"]);

    let db = tmp.join("store.db");
    let mut s = Session::create(&db, &tmp, "judge")
        .unwrap()
        .with_bridge(bridge(&[]))
        .with_lane_confine(identity);
    s.set_child_candidates(vec![cand("codex", "gpt-5.6-luna", 0), cand("claude", "opus", 2)]);
    s.run_turn("user", "human", "add good.txt", Policy::AllowAll).unwrap();
    let parent_events = s.transcript().unwrap().len();

    let lanes = vec![
        lane("claude-opus", "anthropic", &[], false), // never writes good.txt
        lane("codex", "openai", &["--rogue", "good.txt"], false),
    ];
    let judge = |reply: &str| CompareOptions {
        judge: true,
        judge_agent: "mockjudge".into(),
        judge_bridge: bridge(&["--replay", &fixture(reply)]),
        // Read-only needs Seatbelt (CLAUDE.md 7.22); the mock judge writes nothing.
        judge_policy: Some(Policy::AllowAll),
        ..CompareOptions::default()
    };

    // 1. Untrusted project: nothing of the project's runs, and nobody passes by default.
    let r = s
        .compare_with(&lanes, "add good.txt", Policy::AllowAll, &judge("judge_prefers_b.jsonl"))
        .expect("compare runs");
    for b in &r.branches {
        let sc = b.score.as_ref().expect("scored");
        assert!(!sc.tests.ran, "{}: tests ran in an untrusted project", b.agent);
        assert_eq!(sc.passed, None, "no evidence is not a pass");
    }

    // 2. Trusted: the checks run; the judge prefers the failing lane, the evidence overrules it.
    TrustStore::default_location().set_trusted(&tmp, true).unwrap();
    let r = s
        .compare_with(&lanes, "add good.txt", Policy::AllowAll, &judge("judge_prefers_b.jsonl"))
        .expect("compare runs");
    let by = |name: &str| r.branches.iter().find(|b| b.agent == name).unwrap().score.clone().unwrap();
    let (claude, codex) = (by("claude-opus"), by("codex"));
    assert_eq!(codex.tests.passed, Some(true), "codex wrote good.txt: {:?}", codex.tests);
    assert_eq!(claude.tests.passed, Some(false), "claude did not: {:?}", claude.tests);
    assert_eq!((codex.rank, claude.rank), (1, 2), "a failing lane must not outrank a passing one");
    assert!(claude.reason.contains("ผลเช็ค"), "the override is explained: {}", claude.reason);
    let v = r.verdict.as_ref().expect("verdict");
    assert!(v.judged && v.judge == "mockjudge", "{v:?}");
    assert!(!v.summary.is_empty());
    for b in &r.branches {
        assert!(b.worktree.is_some(), "lanes ran in their own worktrees");
    }

    // 3. The parent session is untouched by all of it.
    assert_eq!(s.transcript().unwrap().len(), parent_events);

    // 4. Results are stored per lane, in the lane's session, for the router.
    let store = Store::open(&db).unwrap();
    let rows = compare_results(&store, 50).unwrap();
    assert_eq!(rows.len(), 2 /* lanes of run 2 */ + 2 /* lanes of run 1 */, "{rows:?}");
    let stats = lane_stats(&rows[..2]);
    let codex_stats = stats.iter().find(|s| s.agent == "codex").unwrap();
    assert_eq!((codex_stats.runs, codex_stats.passed), (1, 1));
    let claude_stats = stats.iter().find(|s| s.agent == "claude-opus").unwrap();
    assert_eq!((claude_stats.runs, claude_stats.passed), (1, 0));

    // 5. F6: the same agent, alone and leading an orchestra, on one prompt.
    let lanes = vec![lane("codex", "openai", &["--rogue", "good.txt"], false), lane("codex", "openai", &["--rogue", "good.txt"], true)];
    let r = s
        .compare_with(&lanes, "add good.txt", Policy::AllowAll, &CompareOptions::default())
        .expect("compare runs");
    let modes: Vec<(&str, &str)> = r
        .branches
        .iter()
        .map(|b| (b.agent.as_str(), b.score.as_ref().expect("an orchestra lane is always scored").mode.as_str()))
        .collect();
    assert_eq!(modes, [("codex", "single"), ("codex+orchestra", "orchestra")]);
    assert!(r.verdict.is_none(), "no judge was asked for");
    let rows = compare_results(&store, 2).unwrap();
    let stats = lane_stats(&rows);
    assert!(stats.iter().any(|s| s.mode == "orchestra") && stats.iter().any(|s| s.mode == "single"));

    // 6. A discriminating test: written by the judge's provider in a throwaway
    // worktree, run in every lane that was green, and removed from the lane
    // again. Here the project's check turns red when the file is present.
    std::env::set_var("CHORUS_TEST_COMMAND", "test -f good.txt && test ! -f disc_test.py");
    let lanes = vec![
        lane("claude-opus", "anthropic", &["--rogue", "good.txt"], false),
        lane("codex", "openai", &["--rogue", "good.txt"], false),
    ];
    let opts = CompareOptions {
        discriminating_test: true,
        test_writer_bridge: bridge(&["--rogue", "disc_test.py"]),
        ..judge("judge_prefers_a.jsonl")
    };
    let r = s.compare_with(&lanes, "add good.txt", Policy::AllowAll, &opts).expect("compare runs");
    for b in &r.branches {
        let sc = b.score.as_ref().unwrap();
        assert!(sc.discriminating.ran, "{}: {:?}", b.agent, sc.discriminating);
        assert!(sc.discriminating.name.contains("disc_test.py"), "{:?}", sc.discriminating);
        assert_eq!(sc.discriminating.passed, Some(false), "the new test file tips the lane red");
        assert_eq!(sc.passed, Some(false), "and the lane no longer counts as passing");
        let wt = PathBuf::from(b.worktree.as_ref().unwrap());
        assert!(!wt.join("disc_test.py").exists(), "the test file is not left in the candidate");
    }

    for b in &r.branches {
        if let Some(w) = &b.worktree {
            let _ = chorus_core::worktrees::remove_worktree(&tmp, Path::new(w), b.worktree_branch.as_deref());
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
