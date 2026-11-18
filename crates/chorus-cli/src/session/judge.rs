//! Judging compare lanes with evidence (gap map E4), the cross-provider
//! review step (R6a) and the stored compare results the router reads later
//! (F6).
//!
//! Compare mode used to hand the human N diffs and nothing else. With scoring
//! on, each lane is checked where it ran — the project's own verifier and its
//! tests, inside the lane's worktree and the same Seatbelt profile a turn gets
//! — and a judge from another provider reads the evidence, each lane's diff
//! and a digest of each lane's ledger, then ranks with reasons. The human
//! still picks (design: "the human picks"); the ranking is advice.
//!
//! Evidence comes before opinion. A lane whose build or tests failed never
//! ranks above one that passed, whatever the judge says: LLM judges favour
//! position and length over substance (Zheng et al., "Judging LLM-as-a-Judge
//! with MT-Bench and Chatbot Arena", 2023), so lanes reach the judge under
//! neutral labels in the order the evidence gives, with the numbers first.
//!
//! Everything here runs after the lanes finish. Nothing touches
//! `assemble_context` (CLAUDE.md 7.2b), and results are stored as `Decision`
//! rows with a `chorus:compare` marker in each lane's own session — no schema
//! change, and the parent session is not written to.

use super::*;
use chorus_core::journal;
use chorus_core::trust::{which, TrustStore};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::time::{Duration, Instant};

/// Characters of one lane's diff the judge reads.
const JUDGE_DIFF_CHARS: usize = 6_000;
/// Ledger lines of one lane the judge reads.
const JUDGE_LEDGER_LINES: usize = 40;
/// A marker row must stay inline (`INLINE_MAX_BYTES`) so `LIKE` can find it.
const MARKER_MAX_BYTES: usize = 1_900;

// ---- options and lanes ---------------------------------------------------

/// What a compare run does beyond running the lanes. All off by default: scoring
/// costs build and test time, judging costs a model call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CompareOptions {
    /// Run the project's verifier and tests in each lane's worktree.
    pub score: bool,
    /// Have a judge (another provider when one is ready) rank the lanes.
    /// Implies `score`.
    pub judge: bool,
    /// Have the judge's provider write one test that tells the lanes apart,
    /// then run it in each lane. Implies `judge`.
    pub discriminating_test: bool,
    /// Agent that judges; empty = another provider than the lanes if one is ready.
    pub judge_agent: String,
    /// ACP command for the judge instead of its default bridge (tests, pinned installs).
    pub judge_bridge: Vec<String>,
    /// ACP command for the agent that writes the discriminating test, when it
    /// is not the judge's (tests, pinned installs).
    pub test_writer_bridge: Vec<String>,
    /// Policy of the judge's turn. `None` = [`Policy::ReadOnly`], which needs
    /// Seatbelt and so refuses on Linux; tests there pass something else.
    #[serde(skip)]
    pub judge_policy: Option<Policy>,
}

impl CompareOptions {
    fn scoring(&self, lanes: &[Lane]) -> bool {
        self.score || self.judge || self.discriminating_test || lanes.iter().any(|l| l.orchestra)
    }

    fn judging(&self) -> bool {
        self.judge || self.discriminating_test
    }
}

/// One column of a compare: an agent, run alone or as the lead of an orchestra.
#[derive(Debug, Clone)]
pub struct Lane {
    pub spec: AgentSpec,
    /// F6: run the same prompt as an orchestra-mode turn (the agent leads and
    /// may split) instead of a single-agent turn.
    pub orchestra: bool,
}

impl Lane {
    pub fn single(spec: AgentSpec) -> Self {
        Self { spec, orchestra: false }
    }

    pub fn label(&self) -> String {
        if self.orchestra {
            format!("{}+orchestra", self.spec.name)
        } else {
            self.spec.name.clone()
        }
    }

    pub fn mode(&self) -> &'static str {
        if self.orchestra {
            "orchestra"
        } else {
            "single"
        }
    }
}

/// `codex,claude-opus+orchestra` → lanes. The suffix turns a lane into an
/// orchestra lane, so `codex,codex+orchestra` compares one agent both ways.
pub fn lanes_from_csv(csv: &str, provider_of: impl Fn(&str) -> String) -> Vec<Lane> {
    csv.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            let (name, orchestra) = match s.strip_suffix("+orchestra") {
                Some(n) => (n, true),
                None => (s, false),
            };
            Lane {
                spec: AgentSpec::new(name, provider_of(name)),
                orchestra,
            }
        })
        .collect()
}

// ---- evidence ------------------------------------------------------------

/// One check run in a lane's worktree.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Check {
    pub name: String,
    pub ran: bool,
    /// `None` when the check did not run.
    pub passed: Option<bool>,
    pub exit: Option<i32>,
    /// Why it did not run, or the tail of its output.
    pub note: String,
}

impl Check {
    fn skipped(name: &str, why: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            note: why.into(),
            ..Self::default()
        }
    }

    fn summary(&self) -> String {
        match (self.ran, self.passed) {
            (true, Some(true)) => format!("{}: passed", self.name),
            (true, _) => format!(
                "{}: FAILED (exit {}) {}",
                self.name,
                self.exit.map_or("?".into(), |c| c.to_string()),
                tail(&self.note, 500)
            ),
            _ => format!("{}: not run ({})", self.name, self.note),
        }
    }
}

/// Everything known about one lane after it ran.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LaneScore {
    pub lane: String,
    pub agent: String,
    pub mode: String,
    pub answered: bool,
    pub verify: Check,
    pub tests: Check,
    pub discriminating: Check,
    pub files_changed: usize,
    pub tokens: Option<i64>,
    pub elapsed_ms: u64,
    pub cost_usd: Option<f64>,
    pub rewinds: u32,
    /// `Some(true)` every check that ran passed; `Some(false)` one failed or
    /// the lane did not answer; `None` nothing could be checked.
    pub passed: Option<bool>,
    /// 1-based rank by evidence alone.
    pub evidence_rank: usize,
    /// 1-based final rank (judge's, bounded by the evidence).
    pub rank: usize,
    pub reason: String,
}

/// The judge's word on the whole compare.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Verdict {
    /// Agent that judged; empty when no model was asked.
    pub judge: String,
    /// The ranking came from the judge's reply (otherwise from evidence alone).
    pub judged: bool,
    pub summary: String,
    /// Anything the human should know: no judge available, unparsable reply, …
    pub note: String,
}

pub struct CompareReport {
    pub branches: Vec<CompareBranch>,
    pub verdict: Option<Verdict>,
}

fn tail(s: &str, max: usize) -> String {
    let s = s.trim();
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let skip = n - max;
    format!("…{}", s.chars().skip(skip).collect::<String>())
}

fn head(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max).collect::<String>())
}

/// The command that runs a project's tests, by what the folder contains. The
/// package manager follows the lockfile — never a competing one.
pub fn test_command(ws: &Path) -> Option<Vec<String>> {
    let v = |a: &[&str]| Some(a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    // The user's own word beats guessing: a monorepo, a Makefile target.
    if let Some(cmd) = std::env::var("CHORUS_TEST_COMMAND").ok().filter(|c| !c.trim().is_empty()) {
        return v(&["sh", "-c", cmd.trim()]);
    }
    if ws.join("Cargo.toml").exists() {
        return v(&["cargo", "test", "--quiet"]);
    }
    if let Ok(raw) = std::fs::read_to_string(ws.join("package.json")) {
        let has_test = serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .map(|j| j["scripts"]["test"].is_string())
            .unwrap_or(false);
        if has_test {
            return if ws.join("pnpm-lock.yaml").exists() {
                v(&["pnpm", "test"])
            } else if ws.join("yarn.lock").exists() {
                v(&["yarn", "test"])
            } else if ws.join("bun.lock").exists() || ws.join("bun.lockb").exists() {
                v(&["bun", "run", "test"])
            } else {
                v(&["npm", "test", "--silent"])
            };
        }
    }
    if ws.join("Package.swift").exists() {
        return v(&["swift", "test"]);
    }
    if ws.join("pytest.ini").exists()
        || ws.join("pyproject.toml").exists()
        || ws.join("requirements.txt").exists()
    {
        return v(&["pytest", "-q"]);
    }
    None
}

fn test_timeout() -> Duration {
    let secs = std::env::var("CHORUS_COMPARE_TEST_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(600);
    Duration::from_secs(secs.max(1))
}

/// Run `argv` under `consent` with a deadline. Output is masked before it is kept.
fn run_check(
    consent: &chorus_core::trust::Consent,
    name: &str,
    argv: &[String],
    timeout: Duration,
) -> Check {
    let Some(first) = argv.first() else {
        return Check::skipped(name, "no command");
    };
    let Some(bin) = which(first) else {
        return Check::skipped(name, format!("`{first}` is not installed"));
    };
    let mut full = vec![bin.to_string_lossy().into_owned()];
    full.extend(argv[1..].iter().cloned());
    let mut cmd = match consent.command(&full) {
        Ok(c) => c,
        Err(e) => return Check::skipped(name, format!("{e}")),
    };
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Check::skipped(name, format!("could not start: {e}")),
    };
    let pump = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out = pump(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let err = pump(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => break None,
        }
    };
    let mut text = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    text.push_str(&String::from_utf8_lossy(&err.join().unwrap_or_default()));
    let text = mask_secrets(&text);
    match status {
        Some(s) => Check {
            name: name.into(),
            ran: true,
            passed: Some(s.success()),
            exit: s.code(),
            note: tail(&text, 600),
        },
        None => Check {
            name: name.into(),
            ran: true,
            passed: Some(false),
            exit: None,
            note: format!("timed out after {}s. {}", timeout.as_secs(), tail(&text, 400)),
        },
    }
}

/// A [`TrustStore`] that trusts exactly `lane_cwd`, but only because the user
/// trusted `project`. A lane runs in a throwaway copy of the project, which is
/// not itself on the user's list, so the project's consent is carried over to
/// it; an untrusted project gets no store and nothing runs.
struct LaneTrust {
    store: TrustStore,
    file: PathBuf,
}

impl LaneTrust {
    fn derive(project: &Path, lane_cwd: &Path) -> Option<Self> {
        if !TrustStore::default_location().is_trusted(project) {
            return None;
        }
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let file = std::env::temp_dir().join(format!(
            "chorus-lane-trust-{}-{}.json",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let store = TrustStore::at(&file);
        store.set_trusted(lane_cwd, true).ok()?;
        Some(Self { store, file })
    }
}

impl Drop for LaneTrust {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
    }
}

/// The sandbox a turn that may write gets; checks run project code and are held to it.
fn seatbelt(argv: &[String], cwd: &Path) -> std::result::Result<Vec<String>, String> {
    sandbox::wrap(argv, cwd, sandbox::Profile::Workspace)
}

const UNTRUSTED: &str = "โปรเจกต์ยังไม่ได้ trust — ไม่รันเช็ค/เทสต์ของโปรเจกต์ (Settings › Trust)";

/// Verifier + tests for one lane. Neither runs for a project the user has not
/// trusted: they run the project's own code.
fn check_lane(
    project: &Path,
    lane_cwd: &Path,
    run_tests: bool,
    confine: &chorus_core::trust::Confine,
) -> (Check, Check) {
    let Some(trust) = LaneTrust::derive(project, lane_cwd) else {
        return (
            Check::skipped("verify", UNTRUSTED),
            Check::skipped("tests", UNTRUSTED),
        );
    };
    let consent = match trust.store.consent(lane_cwd, confine) {
        Ok(c) => c,
        Err(e) => {
            let why = format!("{e}");
            return (Check::skipped("verify", why.clone()), Check::skipped("tests", why));
        }
    };
    let verify = match chorus_core::verify_loop::VerifyLoop::verify_workspace(&consent) {
        Ok(r) if r.checker_used == "none" || r.checker_used.ends_with("-generic") => {
            Check::skipped("verify", "no checker for this project")
        }
        Ok(r) => Check {
            name: r.checker_used.clone(),
            ran: true,
            passed: Some(r.passed),
            exit: None,
            note: if r.passed {
                String::new()
            } else {
                tail(&r.to_agent_feedback(), 600)
            },
        },
        Err(e) => Check::skipped("verify", format!("{e}")),
    };
    let tests = if !run_tests {
        Check::skipped("tests", "tests off")
    } else {
        match test_command(lane_cwd) {
            Some(argv) => run_check(&consent, &argv.join(" "), &argv, test_timeout()),
            None => Check::skipped("tests", "no test command found"),
        }
    };
    (verify, tests)
}

/// Pass/fail/unknown from the checks. A lane that never answered fails.
fn overall(answered: bool, checks: &[&Check]) -> Option<bool> {
    if !answered {
        return Some(false);
    }
    let ran: Vec<bool> = checks.iter().filter_map(|c| c.passed).collect();
    if ran.is_empty() {
        None
    } else {
        Some(ran.iter().all(|p| *p))
    }
}

/// Order lanes by evidence alone: passed before unknown before failed, then
/// the lane that touched files, then the faster. Returns lane indexes, best first.
pub fn evidence_order(scores: &[LaneScore]) -> Vec<usize> {
    let class = |s: &LaneScore| match s.passed {
        Some(true) => 2,
        None => 1,
        Some(false) => 0,
    };
    let mut idx: Vec<usize> = (0..scores.len()).collect();
    idx.sort_by(|&a, &b| {
        class(&scores[b])
            .cmp(&class(&scores[a]))
            .then((scores[b].files_changed > 0).cmp(&(scores[a].files_changed > 0)))
            .then(scores[a].elapsed_ms.cmp(&scores[b].elapsed_ms))
    });
    idx
}

/// The judge to ask: an explicit choice, else a ready agent whose provider is
/// not among the lanes' (a lane's own provider flatters its own work), else
/// any ready agent.
pub fn pick_judge(
    explicit: &str,
    lane_providers: &[String],
    ready: &[(String, String)],
) -> Option<(String, String)> {
    if !explicit.trim().is_empty() {
        return Some((
            explicit.trim().to_string(),
            ready
                .iter()
                .find(|(n, _)| n == explicit.trim())
                .map(|(_, p)| p.clone())
                .unwrap_or_else(|| crate::orchestra::provider_for(explicit).to_string()),
        ));
    }
    ready
        .iter()
        .find(|(_, p)| !lane_providers.contains(p))
        .or_else(|| ready.first())
        .cloned()
}

// ---- the judge's prompt and reply ------------------------------------------

fn letter(i: usize) -> char {
    (b'A' + (i % 26) as u8) as char
}

/// What the judge reads: the task, then per lane the numbers, the diff and a
/// digest of that lane's ledger. Lanes are labelled A, B, … in evidence order
/// so no agent name or fixed position decides the outcome.
pub fn judge_prompt(
    task: &str,
    order: &[usize],
    scores: &[LaneScore],
    diffs: &[String],
    ledgers: &[String],
) -> String {
    let mut s = String::new();
    s.push_str(
        "You are the judge of a compare run: several agents were given the same task, each in \
         its own copy of the project. You may only read; change nothing. Rank the candidates \
         from best to worst. The test and build results below are facts measured by the \
         harness — trust them over any claim in a candidate's own text. Do not prefer a \
         candidate for being longer, or for being listed first.\n\n",
    );
    s.push_str(&format!("# Task\n{}\n", head(task, 2_000)));
    for (pos, &i) in order.iter().enumerate() {
        let sc = &scores[i];
        s.push_str(&format!("\n# Candidate {}\n", letter(pos)));
        s.push_str(&format!(
            "evidence:\n- {}\n- {}\n- {}\n- files changed: {} · rewinds: {}\n",
            sc.verify.summary(),
            sc.tests.summary(),
            if sc.discriminating.ran || !sc.discriminating.note.is_empty() {
                sc.discriminating.summary()
            } else {
                "discriminating test: none".to_string()
            },
            sc.files_changed,
            sc.rewinds
        ));
        let d = diffs[i].trim();
        if d.is_empty() {
            s.push_str("\ndiff: (no changes)\n");
        } else {
            s.push_str(&format!("\ndiff:\n```diff\n{}\n```\n", head(d, JUDGE_DIFF_CHARS)));
        }
        s.push_str(&format!("\nwhat it did (ledger digest):\n{}\n", ledgers[i]));
    }
    s.push_str(&format!(
        "\n# Reply\nReply with one JSON object and nothing else: \
         {{\"ranking\": [\"A\", ...every candidate, best first], \
         \"reasons\": {{\"A\": \"one sentence\", ...}}, \"summary\": \"one or two sentences\"}}. \
         Candidates: {}.\n",
        (0..order.len()).map(|p| letter(p).to_string()).collect::<Vec<_>>().join(", ")
    ));
    s
}

/// One line per ledger event of a lane, the first few and the last ones, so
/// the judge sees how the lane worked and not only what it produced.
fn ledger_digest(store: &Store, lane_sess: i64) -> String {
    let c = store.conn();
    let Ok(events) = ledger::history(c, lane_sess, 10_000) else {
        return "(unreadable)".into();
    };
    let mut lines: Vec<String> = events
        .iter()
        .filter(|e| e.sess == lane_sess)
        .filter_map(|e| {
            let body = ledger::body(c, e).ok()?;
            let kind = match e.kind {
                EventKind::Msg => "say",
                EventKind::ToolCall => "call",
                EventKind::ToolResult => "result",
                EventKind::Decision if !journal::is_marker(&body) => "decision",
                _ => return None,
            };
            Some(format!("- {kind}: {}", head(&body.replace('\n', " "), 160)))
        })
        .collect();
    if lines.len() > JUDGE_LEDGER_LINES {
        let keep_head = JUDGE_LEDGER_LINES / 4;
        let cut = lines.len() - JUDGE_LEDGER_LINES;
        let mut kept: Vec<String> = lines[..keep_head].to_vec();
        kept.push(format!("- … {cut} events omitted …"));
        kept.extend_from_slice(&lines[lines.len() - (JUDGE_LEDGER_LINES - keep_head)..]);
        lines = kept;
    }
    if lines.is_empty() {
        "(nothing recorded)".into()
    } else {
        lines.join("\n")
    }
}

#[derive(Debug, PartialEq)]
pub struct ParsedJudgement {
    /// Candidate positions (the A=0, B=1, … of the prompt), best first.
    pub ranking: Vec<usize>,
    pub reasons: Vec<(usize, String)>,
    pub summary: String,
}

/// Read the judge's JSON reply, tolerating prose or a code fence around it.
/// `None` when no usable ranking is in it.
pub fn parse_judgement(reply: &str, candidates: usize) -> Option<ParsedJudgement> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    let v: serde_json::Value = serde_json::from_str(reply.get(start..=end)?).ok()?;
    let pos = |s: &str| -> Option<usize> {
        let c = s.trim().trim_start_matches("Candidate").trim().chars().next()?;
        let p = (c.to_ascii_uppercase() as u8).checked_sub(b'A')? as usize;
        (p < candidates).then_some(p)
    };
    let mut ranking: Vec<usize> = Vec::new();
    for item in v["ranking"].as_array()? {
        if let Some(p) = item.as_str().and_then(pos) {
            if !ranking.contains(&p) {
                ranking.push(p);
            }
        }
    }
    if ranking.is_empty() {
        return None;
    }
    // A candidate the judge left out goes last rather than vanishing.
    for p in 0..candidates {
        if !ranking.contains(&p) {
            ranking.push(p);
        }
    }
    let reasons = v["reasons"]
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, r)| Some((pos(k)?, r.as_str()?.trim().to_string())))
                .collect()
        })
        .unwrap_or_default();
    Some(ParsedJudgement {
        ranking,
        reasons,
        summary: v["summary"].as_str().unwrap_or("").trim().to_string(),
    })
}

/// Final order: evidence classes first, the judge's order inside a class.
/// `judged` is each lane's judge position (lower = better), or `None`.
pub fn final_order(scores: &[LaneScore], judged: Option<&[usize]>) -> Vec<usize> {
    let class = |s: &LaneScore| match s.passed {
        Some(true) => 2,
        None => 1,
        Some(false) => 0,
    };
    let evidence = evidence_order(scores);
    let mut idx = evidence.clone();
    let pos_in = |order: &[usize], lane: usize| order.iter().position(|&l| l == lane).unwrap_or(usize::MAX);
    idx.sort_by(|&a, &b| {
        class(&scores[b]).cmp(&class(&scores[a])).then_with(|| match judged {
            Some(j) => pos_in(j, a).cmp(&pos_in(j, b)),
            None => pos_in(&evidence, a).cmp(&pos_in(&evidence, b)),
        })
    });
    idx
}

// ---- stored results (F6) ---------------------------------------------------

fn marker_body(run: i64, task: &str, sc: &LaneScore, of: usize) -> String {
    let b = |c: &Check| c.passed;
    let mut v = serde_json::json!({
        "run": run,
        "task": head(task, 80),
        "lane": sc.lane,
        "agent": sc.agent,
        "mode": sc.mode,
        "pass": sc.passed,
        "answered": sc.answered,
        "verify": b(&sc.verify),
        "tests": b(&sc.tests),
        "disc": b(&sc.discriminating),
        "tokens": sc.tokens,
        "ms": sc.elapsed_ms,
        "cost": sc.cost_usd,
        "rewinds": sc.rewinds,
        "files": sc.files_changed,
        "rank": sc.rank,
        "of": of,
        "reason": head(&sc.reason, 240),
    });
    let mut body = format!("{}{v}", journal::COMPARE);
    if body.len() > MARKER_MAX_BYTES {
        v["reason"] = serde_json::Value::Null;
        v["task"] = serde_json::Value::Null;
        body = format!("{}{v}", journal::COMPARE);
    }
    body
}

/// One stored lane result.
#[derive(Debug, Clone, Serialize)]
pub struct StoredResult {
    pub sess: i64,
    pub seq: i64,
    pub result: serde_json::Value,
}

/// The newest stored compare results, newest first. `rewinds` is recounted
/// now: it grows after the compare, when the human rewinds the lane.
pub fn compare_results(store: &Store, limit: usize) -> Result<Vec<StoredResult>> {
    let c = store.conn();
    let mut stmt = c.prepare(
        "SELECT sess, seq, body FROM event
         WHERE kind = ?1 AND body LIKE ?2 ORDER BY seq DESC LIMIT ?3",
    )?;
    let rows = stmt
        .query_map(
            (
                EventKind::Decision.as_i64(),
                format!("{}%", journal::COMPARE),
                limit.clamp(1, 1_000) as i64,
            ),
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)),
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut out = Vec::new();
    for (sess, seq, body) in rows {
        let Some(json) = body.strip_prefix(journal::COMPARE) else {
            continue;
        };
        let Ok(mut result) = serde_json::from_str::<serde_json::Value>(json) else {
            continue;
        };
        let rewinds: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM session WHERE forked_from = ?1 AND title LIKE '%(ย้อนกลับ)'",
                [sess],
                |r| r.get(0),
            )
            .unwrap_or(0);
        result["rewinds"] = rewinds.into();
        out.push(StoredResult { sess, seq, result });
    }
    Ok(out)
}

/// Per (agent, mode) totals over stored results: what the router wants to
/// read — how often a lane passed, what it cost, how often it was rewound.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LaneStats {
    pub agent: String,
    pub mode: String,
    pub runs: usize,
    pub passed: usize,
    /// Runs where nothing could be checked; they count for neither side.
    pub unknown: usize,
    pub avg_tokens: Option<i64>,
    pub avg_ms: u64,
    pub rewinds: i64,
}

pub fn lane_stats(results: &[StoredResult]) -> Vec<LaneStats> {
    let mut map: Vec<(LaneStats, i64, usize, u64)> = Vec::new();
    for r in results {
        let v = &r.result;
        let agent = v["agent"].as_str().unwrap_or("?").to_string();
        let mode = v["mode"].as_str().unwrap_or("single").to_string();
        let slot = match map.iter().position(|(s, ..)| s.agent == agent && s.mode == mode) {
            Some(i) => i,
            None => {
                map.push((
                    LaneStats {
                        agent,
                        mode,
                        ..LaneStats::default()
                    },
                    0,
                    0,
                    0,
                ));
                map.len() - 1
            }
        };
        let (s, tok_sum, tok_n, ms_sum) = &mut map[slot];
        s.runs += 1;
        match v["pass"].as_bool() {
            Some(true) => s.passed += 1,
            None => s.unknown += 1,
            Some(false) => {}
        }
        if let Some(t) = v["tokens"].as_i64() {
            *tok_sum += t;
            *tok_n += 1;
        }
        *ms_sum += v["ms"].as_u64().unwrap_or(0);
        s.rewinds += v["rewinds"].as_i64().unwrap_or(0);
    }
    map.into_iter()
        .map(|(mut s, tok_sum, tok_n, ms_sum)| {
            s.avg_tokens = (tok_n > 0).then(|| tok_sum / tok_n as i64);
            s.avg_ms = if s.runs > 0 { ms_sum / s.runs as u64 } else { 0 };
            s
        })
        .collect()
}

// ---- the compare run -------------------------------------------------------

impl Session {
    /// Compare mode with optional scoring and judging. [`Session::compare`] is
    /// this with every option off.
    ///
    /// Lanes run one after another, each in its own fork and worktree. When
    /// scoring is on, each lane is then checked in place, the judge (if any)
    /// reads the evidence, and one `chorus:compare` row per lane is written to
    /// that lane's session. The parent session is not written to.
    pub fn compare_with(
        &mut self,
        lanes: &[Lane],
        prompt: &str,
        policy: Policy,
        opts: &CompareOptions,
    ) -> Result<CompareReport> {
        let db_path: String = self
            .store
            .conn()
            .query_row("PRAGMA database_list", [], |r| r.get::<_, String>(2))
            .unwrap_or_default();
        anyhow::ensure!(!db_path.is_empty(), "compare needs a file-backed store");

        let head_seq = ledger::history(self.store.conn(), self.sess, 1_000_000)
            .ok()
            .and_then(|h| h.last().map(|e| e.seq))
            .unwrap_or(0);

        let mut branches = Vec::new();
        for lane in lanes {
            let spec = &lane.spec;
            let label = lane.label();
            let fork_id = ledger::fork(
                self.store.conn(),
                self.sess,
                head_seq,
                &format!("compare: {label}"),
            )?;
            // Each agent works in its own git worktree, started from the
            // user's current tree, so none sees (or edits) another's files.
            // Outside a git repo, or if the worktree cannot be made, the
            // shared folder is used as before.
            let isolated = self.compare_worktree(&label);
            if let Some((_, cwd)) = &isolated {
                self.store.conn().execute(
                    "UPDATE session SET cwd = ?1 WHERE id = ?2",
                    (cwd.to_string_lossy().as_ref(), fork_id),
                )?;
            }
            let mut branch = Session::open(Path::new(&db_path), fork_id)?;
            branch.forced_bridge = Some(self.resolve_bridge(&spec.name, &spec.bridge));
            let before = branch
                .checkpoint
                .as_ref()
                .and_then(|ck| ck.snapshot("compare base").ok());
            let started = Instant::now();
            let turn = if lane.orchestra {
                branch.set_child_candidates(self.ready_candidates());
                branch
                    .run_orchestrator_turn(prompt, &[], &spec.name, "", policy)
                    .map(|r| r.relay.turn)
            } else {
                branch.run_turn(&spec.name, &spec.provider, prompt, policy)
            };
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let diff = match (&branch.checkpoint, &before) {
                (Some(ck), Some(base)) => ck.diff_since_user(base).unwrap_or_default(),
                _ => String::new(),
            };
            let used = usage::get(self.store.conn(), fork_id).ok().flatten().unwrap_or_default();
            // An orchestra's children run in sessions of their own; their
            // context is part of what the lane cost.
            let child_ctx: i64 = self
                .store
                .conn()
                .prepare("SELECT child_sess FROM subagent WHERE parent_sess = ?1")
                .and_then(|mut s| {
                    s.query_map([fork_id], |r| r.get::<_, i64>(0))?
                        .collect::<std::result::Result<Vec<_>, _>>()
                })
                .unwrap_or_default()
                .into_iter()
                .filter_map(|id| usage::get(self.store.conn(), id).ok().flatten())
                .filter_map(|u| u.context_used)
                .sum();
            let tokens = match (used.context_used, child_ctx) {
                (None, 0) => None,
                (u, c) => Some(u.unwrap_or(0) + c),
            };
            branches.push(CompareBranch {
                agent: label,
                session_id: fork_id,
                result: turn.map(|t| (t.message, t.stop_reason)),
                diff,
                elapsed_ms,
                context_used: tokens,
                cost_usd: used.cost_usd,
                worktree: isolated.as_ref().map(|(w, _)| w.path.clone()),
                worktree_branch: isolated.as_ref().map(|(w, _)| w.branch.clone()),
                score: None,
            });
        }

        if !opts.scoring(lanes) {
            return Ok(CompareReport {
                branches,
                verdict: None,
            });
        }
        let verdict = self.score_and_judge(lanes, &mut branches, prompt, head_seq, &db_path, opts);
        Ok(CompareReport { branches, verdict })
    }

    fn score_and_judge(
        &mut self,
        lanes: &[Lane],
        branches: &mut [CompareBranch],
        prompt: &str,
        head_seq: i64,
        db_path: &str,
        opts: &CompareOptions,
    ) -> Option<Verdict> {
        let project = self.cwd.clone();
        let mut scores: Vec<LaneScore> = Vec::new();
        for (lane, b) in lanes.iter().zip(branches.iter()) {
            let lane_cwd: PathBuf = b
                .worktree
                .as_ref()
                .map(|w| {
                    // `compare_worktree` mirrors the project's sub-folder inside the copy.
                    let top = self.git_prefix();
                    PathBuf::from(w).join(top)
                })
                .unwrap_or_else(|| project.clone());
            let answered = matches!(&b.result, Ok((m, _)) if !m.trim().is_empty());
            let confine = self.lane_confine.unwrap_or(seatbelt);
            let (verify, tests) = check_lane(&project, &lane_cwd, true, &confine);
            let files = b
                .diff
                .lines()
                .filter(|l| l.starts_with("+++ b/"))
                .count();
            scores.push(LaneScore {
                lane: lane.label(),
                agent: lane.spec.name.clone(),
                mode: lane.mode().into(),
                answered,
                passed: overall(answered, &[&verify, &tests]),
                verify,
                tests,
                discriminating: Check::default(),
                files_changed: files,
                tokens: b.context_used,
                elapsed_ms: b.elapsed_ms,
                cost_usd: b.cost_usd,
                rewinds: 0,
                evidence_rank: 0,
                rank: 0,
                reason: String::new(),
            });
        }

        let mut verdict = None;
        if opts.judging() {
            verdict = Some(self.judge_lanes(lanes, branches, &mut scores, prompt, head_seq, db_path, opts));
        }

        // Ranks, after the judge: evidence class first, the judge inside it.
        let evidence = evidence_order(&scores);
        for (r, &i) in evidence.iter().enumerate() {
            scores[i].evidence_rank = r + 1;
        }
        let judged_order: Option<Vec<usize>> = verdict
            .as_ref()
            .filter(|v| v.judged)
            .map(|_| {
                let mut idx: Vec<usize> = (0..scores.len()).collect();
                idx.sort_by_key(|&i| scores[i].rank.max(1));
                idx
            });
        let fin = final_order(&scores, judged_order.as_deref());
        let had_judge_rank = scores.iter().any(|s| s.rank > 0);
        for (r, &i) in fin.iter().enumerate() {
            let before = scores[i].rank;
            scores[i].rank = r + 1;
            if had_judge_rank && before != 0 && before != r + 1 {
                scores[i].reason = format!(
                    "{} (ผลเช็คทำให้เลื่อนจากอันดับ {before} เป็น {})",
                    scores[i].reason.trim(),
                    r + 1
                );
            }
            if scores[i].reason.trim().is_empty() {
                scores[i].reason = match scores[i].passed {
                    Some(true) => "ผ่านทุกเช็คที่รัน".into(),
                    Some(false) => "ไม่ผ่านเช็คหรือไม่ตอบ".into(),
                    None => "ไม่มีเช็คที่รันได้ — ไม่มีหลักฐาน".into(),
                };
            }
        }

        // Store one row per lane, in the lane's own session.
        let of = scores.len();
        let run = head_seq;
        let c = self.store.conn();
        if let Ok(who) = actor::intern(c, "chorus", "system") {
            for (b, sc) in branches.iter().zip(scores.iter()) {
                let body = marker_body(run, prompt, sc, of);
                let _ = ledger::append(c, &NewEvent::new(b.session_id, EventKind::Decision, who, body));
            }
        }
        for (b, sc) in branches.iter_mut().zip(scores) {
            b.score = Some(sc);
        }
        verdict
    }

    /// The sub-folder of the git root the project folder sits in, mirrored
    /// inside each lane worktree (empty at the root).
    fn git_prefix(&self) -> String {
        std::process::Command::new("git")
            .args(["rev-parse", "--show-prefix"])
            .current_dir(&self.cwd)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    }

    #[allow(clippy::too_many_arguments)]
    fn judge_lanes(
        &mut self,
        lanes: &[Lane],
        branches: &[CompareBranch],
        scores: &mut [LaneScore],
        prompt: &str,
        head_seq: i64,
        db_path: &str,
        opts: &CompareOptions,
    ) -> Verdict {
        let ready: Vec<(String, String)> = crate::agents::list_agents()
            .into_iter()
            .filter(|a| matches!(a.auth, crate::agents::AuthState::Ok))
            .map(|a| {
                let p = crate::orchestra::provider_for(&a.id).to_string();
                (a.id, p)
            })
            .collect();
        let lane_providers: Vec<String> = lanes.iter().map(|l| l.spec.provider.clone()).collect();
        let Some((judge_name, judge_provider)) = pick_judge(&opts.judge_agent, &lane_providers, &ready)
        else {
            return Verdict {
                note: "ไม่มี agent ที่พร้อมเป็นผู้ตัดสิน — จัดอันดับจากผลเช็คอย่างเดียว".into(),
                ..Verdict::default()
            };
        };
        let mut note = String::new();
        if lane_providers.contains(&judge_provider) {
            note.push_str("ผู้ตัดสินเป็นเจ้าเดียวกับบางเลน ");
        }
        let mut spec = AgentSpec::new(&judge_name, &judge_provider);
        spec.bridge = opts.judge_bridge.clone();
        let opts_bridge = opts.judge_bridge.clone();

        // Optional: the judge's provider writes a test that tells the lanes apart.
        if opts.discriminating_test {
            let mut writer = spec.clone();
            if !opts.test_writer_bridge.is_empty() {
                writer.bridge = opts.test_writer_bridge.clone();
            }
            match self.discriminating_test(&writer, lanes, branches, scores, prompt, head_seq, db_path) {
                Ok(()) => {}
                Err(e) => note.push_str(&format!("เทสต์แยกผู้ชนะ: {e:#} ")),
            }
        }

        // Judge reads lanes in evidence order under neutral labels.
        let order = evidence_order(scores);
        let diffs: Vec<String> = branches.iter().map(|b| b.diff.clone()).collect();
        let ledgers: Vec<String> = branches
            .iter()
            .map(|b| ledger_digest(&self.store, b.session_id))
            .collect();
        let text = judge_prompt(prompt, &order, scores, &diffs, &ledgers);

        let fork = match ledger::fork(self.store.conn(), self.sess, head_seq, "compare: judge") {
            Ok(f) => f,
            Err(e) => {
                return Verdict {
                    judge: judge_name,
                    note: format!("{note}เปิดเซสชันผู้ตัดสินไม่ได้: {e}"),
                    ..Verdict::default()
                }
            }
        };
        let reply = Session::open(Path::new(db_path), fork).and_then(|mut js| {
            js.forced_bridge = Some(self.resolve_bridge(&spec.name, &opts_bridge));
            js.run_turn(&spec.name, &spec.provider, &text, opts.judge_policy.unwrap_or(Policy::ReadOnly))
        });
        let message = match reply {
            Ok(t) => t.message,
            Err(e) => {
                return Verdict {
                    judge: judge_name,
                    note: format!("{note}ผู้ตัดสินตอบไม่ได้: {e:#} — ใช้ผลเช็คอย่างเดียว"),
                    ..Verdict::default()
                }
            }
        };
        match parse_judgement(&message, order.len()) {
            Some(p) => {
                for (rank0, &pos) in p.ranking.iter().enumerate() {
                    let lane_idx = order[pos];
                    scores[lane_idx].rank = rank0 + 1;
                }
                for (pos, why) in p.reasons {
                    if let Some(&lane_idx) = order.get(pos) {
                        scores[lane_idx].reason = head(&why, 240);
                    }
                }
                Verdict {
                    judge: judge_name,
                    judged: true,
                    summary: head(&p.summary, 400),
                    note: note.trim().to_string(),
                }
            }
            None => Verdict {
                judge: judge_name,
                note: format!("{note}อ่านคำตัดสินไม่ออก — ใช้ผลเช็คอย่างเดียว"),
                ..Verdict::default()
            },
        }
    }

    /// Have `writer` write one test in a throwaway worktree, then run it in
    /// every lane. A lane that passed before and fails now is the one the test
    /// tells apart; the file is removed from the lane again afterwards.
    #[allow(clippy::too_many_arguments)]
    fn discriminating_test(
        &mut self,
        writer: &AgentSpec,
        lanes: &[Lane],
        branches: &[CompareBranch],
        scores: &mut [LaneScore],
        prompt: &str,
        head_seq: i64,
        db_path: &str,
    ) -> Result<()> {
        let (info, wt_cwd) = self
            .compare_worktree("test-writer")
            .context("เทสต์แยกผู้ชนะต้องใช้โปรเจกต์ที่เป็น git repo")?;
        let result = (|| -> Result<Vec<String>> {
            let fork = ledger::fork(self.store.conn(), self.sess, head_seq, "compare: test-writer")?;
            self.store.conn().execute(
                "UPDATE session SET cwd = ?1 WHERE id = ?2",
                (wt_cwd.to_string_lossy().as_ref(), fork),
            )?;
            let mut ws = Session::open(Path::new(db_path), fork)?;
            ws.forced_bridge = Some(self.resolve_bridge(&writer.name, &writer.bridge));
            let base = ws
                .checkpoint
                .as_ref()
                .and_then(|ck| ck.snapshot("test-writer base").ok());
            let ask = format!(
                "Several agents are solving this task independently:\n\n{}\n\n\
                 Write ONE new automated test, in the project's own test framework, that a \
                 correct solution passes and a plausible wrong or partial one fails. Touch only \
                 test files. Do not implement the task.",
                head(prompt, 2_000)
            );
            ws.run_turn(&writer.name, &writer.provider, &ask, Policy::AllowAll)?;
            let diff = match (&ws.checkpoint, &base) {
                (Some(ck), Some(b)) => ck.diff_since_user(b).unwrap_or_default(),
                _ => String::new(),
            };
            Ok(chorus_core::checkpoint::test_files_in_diff(&diff))
        })();
        let files = match result {
            Ok(f) => f,
            Err(e) => {
                let _ = chorus_core::worktrees::remove_worktree(&self.cwd, Path::new(&info.path), Some(&info.branch));
                return Err(e);
            }
        };
        let outcome = (|| -> Result<()> {
            anyhow::ensure!(!files.is_empty(), "ผู้เขียนเทสต์ไม่ได้สร้างไฟล์เทสต์");
            let prefix = self.git_prefix();
            for ((lane, b), sc) in lanes.iter().zip(branches.iter()).zip(scores.iter_mut()) {
                let _ = lane;
                let Some(w) = &b.worktree else { continue };
                let lane_cwd = PathBuf::from(w).join(&prefix);
                // Only a lane that was green can be tipped red by the new test.
                if sc.tests.passed != Some(true) {
                    sc.discriminating = Check::skipped(
                        "discriminating test",
                        "เทสต์เดิมของเลนนี้ไม่ผ่านอยู่แล้ว — บอกอะไรไม่ได้",
                    );
                    continue;
                }
                let mut saved: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
                for f in &files {
                    let to = lane_cwd.join(f);
                    saved.push((to.clone(), std::fs::read(&to).ok()));
                    if let Some(dir) = to.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let _ = std::fs::copy(wt_cwd.join(f), &to);
                }
                let confine = self.lane_confine.unwrap_or(seatbelt);
                let (_, tests) = check_lane(&self.cwd, &lane_cwd, true, &confine);
                for (path, original) in saved {
                    match original {
                        Some(bytes) => {
                            let _ = std::fs::write(&path, bytes);
                        }
                        None => {
                            let _ = std::fs::remove_file(&path);
                        }
                    }
                }
                sc.discriminating = Check {
                    name: format!("discriminating test ({})", files.join(", ")),
                    ..tests
                };
                sc.passed = overall(sc.answered, &[&sc.verify, &sc.tests, &sc.discriminating]);
            }
            Ok(())
        })();
        let _ = chorus_core::worktrees::remove_worktree(&self.cwd, Path::new(&info.path), Some(&info.branch));
        outcome
    }
}

// ---- cross-provider review (R6a) -------------------------------------------

/// A reviewer from another provider and the criteria agreed up front.
#[derive(Debug, Clone)]
pub struct CrossReview {
    pub reviewer: AgentSpec,
    pub criteria: Vec<String>,
    /// The reviewer's turn policy; read-only unless a test says otherwise.
    pub policy: Policy,
}

impl CrossReview {
    pub fn new(reviewer: AgentSpec, criteria: Vec<String>) -> Self {
        Self {
            reviewer,
            criteria,
            policy: Policy::ReadOnly,
        }
    }
}

/// What a review concluded.
#[derive(Debug, Clone, Serialize)]
pub struct ReviewNote {
    pub reviewer: String,
    /// `pass`, `issues`, `skipped` or `error`.
    pub verdict: String,
    pub message: String,
}

const REVIEW_DIFF_CHARS: usize = 12_000;

impl Session {
    /// Turn the cross-provider review on (or off with `None`). After a turn
    /// that changed files, the reviewer reads the diff against `criteria`
    /// and its answer goes to the ledger like any agent's message.
    pub fn set_cross_review(&mut self, review: Option<CrossReview>) {
        self.cross_review = review;
    }

    /// Review what the turn that `builder` just finished changed. `None` when
    /// review is off or nothing changed. Never fails the turn: a reviewer that
    /// breaks is recorded as `error`.
    pub(super) fn review_after_turn(
        &mut self,
        builder: &str,
        builder_provider: &str,
        request: &str,
    ) -> Option<ReviewNote> {
        let review = self.cross_review.clone()?;
        let turn_base = self.last_turn_guard.clone().unwrap_or_else(|| self.base_ckpt.clone());
        let diff = self
            .checkpoint
            .as_ref()
            .and_then(|ck| ck.diff_since_user(&turn_base).ok())
            .unwrap_or_default();
        if diff.trim().is_empty() {
            return None;
        }
        let reviewer = review.reviewer.clone();
        let note = if !builder_provider.is_empty() && builder_provider == reviewer.provider {
            ReviewNote {
                reviewer: reviewer.name.clone(),
                verdict: "skipped".into(),
                message: format!(
                    "ข้ามรีวิว: {} เป็นเจ้าเดียวกับผู้ลงมือ ({builder_provider}) — รีวิวต้องมาจากอีกเจ้า",
                    reviewer.name
                ),
            }
        } else {
            self.run_review(&review, builder, request, &diff)
        };
        let c = self.store.conn();
        if let Ok(who) = actor::intern(c, "chorus", "system") {
            let body = format!(
                "{}{}",
                journal::REVIEW,
                serde_json::json!({
                    "reviewer": note.reviewer,
                    "builder": builder,
                    "verdict": note.verdict,
                    "criteria": review.criteria.iter().map(|s| head(s, 120)).collect::<Vec<_>>(),
                })
            );
            if let Ok(seq) = ledger::append(c, &NewEvent::new(self.sess, EventKind::Decision, who, mask_secrets(&body))) {
                self.announce(seq);
            }
        }
        Some(note)
    }

    fn run_review(&mut self, review: &CrossReview, builder: &str, request: &str, diff: &str) -> ReviewNote {
        let reviewer = &review.reviewer;
        let criteria = if review.criteria.is_empty() {
            "- the change does what the request asks, and nothing else\n- no test was weakened or removed to make the change pass".to_string()
        } else {
            review.criteria.iter().map(|c| format!("- {}", c.trim())).collect::<Vec<_>>().join("\n")
        };
        let text = format!(
            "You are reviewing another agent's work ({builder}). Read only; change nothing.\n\n\
             # Request\n{}\n\n# Criteria agreed up front\n{criteria}\n\n# Change\n```diff\n{}\n```\n\n\
             # Reply\nBegin with `PASS` if every criterion is met, otherwise with `ISSUES`. Then \
             list each problem as a line naming the criterion it breaks and the file.",
            head(request, 2_000),
            head(diff, REVIEW_DIFF_CHARS)
        );
        let bridge = self.resolve_bridge(&reviewer.name, &reviewer.bridge);
        let outcome = match self.one_turn(&reviewer.name, &bridge, &text, review.policy, &[]) {
            Ok(o) => o,
            Err(e) => {
                return ReviewNote {
                    reviewer: reviewer.name.clone(),
                    verdict: "error".into(),
                    message: format!("รีวิวไม่สำเร็จ: {e:#}"),
                }
            }
        };
        let actor_id = actor::intern(self.store.conn(), &reviewer.name, &reviewer.provider).ok();
        if let Some(a) = actor_id {
            let parent = self.head_seq();
            if let Err(e) = self.persist_outcome(&outcome, a, parent) {
                eprintln!("chorus: could not record the review: {e:#}");
            }
        }
        let first = outcome.message.trim_start().to_uppercase();
        ReviewNote {
            reviewer: reviewer.name.clone(),
            verdict: if first.starts_with("PASS") { "pass" } else { "issues" }.into(),
            message: mask_secrets(&outcome.message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(lane: &str, passed: Option<bool>, files: usize, ms: u64) -> LaneScore {
        LaneScore {
            lane: lane.into(),
            agent: lane.into(),
            mode: "single".into(),
            answered: true,
            passed,
            files_changed: files,
            elapsed_ms: ms,
            ..LaneScore::default()
        }
    }

    #[test]
    fn evidence_orders_passed_before_unknown_before_failed() {
        let s = vec![
            score("fail", Some(false), 3, 10),
            score("pass-slow", Some(true), 1, 900),
            score("unknown", None, 2, 50),
            score("pass-fast", Some(true), 1, 100),
        ];
        let names: Vec<&str> = evidence_order(&s).iter().map(|&i| s[i].lane.as_str()).collect();
        assert_eq!(names, ["pass-fast", "pass-slow", "unknown", "fail"]);
    }

    #[test]
    fn the_judge_cannot_lift_a_failing_lane_over_a_passing_one() {
        let s = vec![score("failing", Some(false), 3, 10), score("passing", Some(true), 1, 10)];
        // The judge preferred lane 0.
        let order = final_order(&s, Some(&[0, 1]));
        assert_eq!(order, vec![1, 0]);
        // Inside one class the judge's order stands.
        let s = vec![score("a", Some(true), 1, 10), score("b", Some(true), 1, 5)];
        assert_eq!(final_order(&s, Some(&[0, 1])), vec![0, 1]);
        assert_eq!(final_order(&s, None), vec![1, 0]);
    }

    #[test]
    fn judge_reply_is_read_through_prose_and_fences() {
        let reply = "Here you go:\n```json\n{\"ranking\":[\"B\",\"A\"],\"reasons\":{\"A\":\"misses the edge case\",\"B\":\"handles it\"},\"summary\":\"B wins\"}\n```";
        let p = parse_judgement(reply, 2).unwrap();
        assert_eq!(p.ranking, vec![1, 0]);
        assert!(p.reasons.contains(&(1, "handles it".to_string())));
        assert_eq!(p.summary, "B wins");
        // A candidate left out goes last; unknown letters are ignored.
        let p = parse_judgement("{\"ranking\":[\"C\",\"Z\"]}", 3).unwrap();
        assert_eq!(p.ranking, vec![2, 0, 1]);
        assert!(parse_judgement("no verdict here", 2).is_none());
        assert!(parse_judgement("{\"ranking\":[]}", 2).is_none());
    }

    #[test]
    fn the_judge_is_from_another_provider_when_one_is_ready() {
        let ready = vec![
            ("claude".to_string(), "anthropic".to_string()),
            ("codex".to_string(), "openai".to_string()),
        ];
        let lanes = vec!["anthropic".to_string()];
        assert_eq!(pick_judge("", &lanes, &ready).unwrap().0, "codex");
        let both = vec!["anthropic".to_string(), "openai".to_string()];
        assert_eq!(pick_judge("", &both, &ready).unwrap().0, "claude");
        assert_eq!(pick_judge("codex", &both, &ready).unwrap().0, "codex");
        assert!(pick_judge("", &lanes, &[]).is_none());
    }

    #[test]
    fn lanes_parse_the_orchestra_suffix() {
        let lanes = lanes_from_csv("codex, codex+orchestra,claude-opus", |n| {
            crate::orchestra::provider_for(n).to_string()
        });
        assert_eq!(lanes.len(), 3);
        assert_eq!(lanes[0].label(), "codex");
        assert!(lanes[1].orchestra && lanes[1].spec.name == "codex");
        assert_eq!(lanes[1].label(), "codex+orchestra");
        assert_eq!(lanes[2].spec.provider, "anthropic");
    }

    #[test]
    fn test_command_follows_the_lockfile() {
        let d = tempfile::tempdir().unwrap();
        assert!(test_command(d.path()).is_none());
        std::fs::write(d.path().join("package.json"), "{\"scripts\":{\"test\":\"vitest\"}}").unwrap();
        assert_eq!(test_command(d.path()).unwrap()[0], "npm");
        std::fs::write(d.path().join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(test_command(d.path()).unwrap(), vec!["pnpm", "test"]);
        std::fs::write(d.path().join("Cargo.toml"), "").unwrap();
        assert_eq!(test_command(d.path()).unwrap()[0], "cargo");
    }

    #[test]
    fn the_judge_prompt_hides_names_and_carries_the_numbers() {
        let mut a = score("secret-agent-name", Some(true), 1, 10);
        a.tests = Check { name: "cargo test".into(), ran: true, passed: Some(true), ..Check::default() };
        let mut b = score("other-agent", Some(false), 1, 10);
        b.tests = Check { name: "cargo test".into(), ran: true, passed: Some(false), exit: Some(101), note: "boom".into() };
        let scores = vec![a, b];
        let p = judge_prompt(
            "fix the parser",
            &[0, 1],
            &scores,
            &["+x".to_string(), String::new()],
            &["- say: done".to_string(), "(nothing recorded)".to_string()],
        );
        assert!(!p.contains("secret-agent-name") && !p.contains("other-agent"));
        assert!(p.contains("# Candidate A") && p.contains("# Candidate B"));
        assert!(p.contains("cargo test: passed") && p.contains("FAILED (exit 101) boom"));
        assert!(p.contains("diff: (no changes)"));
    }

    #[test]
    fn a_stored_result_stays_inline_and_is_found_by_the_router() {
        let mut sc = score("codex+orchestra", Some(true), 2, 1234);
        sc.mode = "orchestra".into();
        sc.tokens = Some(900);
        sc.reason = "x".repeat(5_000);
        sc.rank = 1;
        let body = marker_body(7, &"t".repeat(5_000), &sc, 2);
        assert!(body.len() < chorus_core::types::INLINE_MAX_BYTES, "{} bytes", body.len());
        assert!(journal::is_marker(&body));

        let d = tempfile::tempdir().unwrap();
        let store = Store::open(&d.path().join("s.db")).unwrap();
        let c = store.conn();
        let s = ledger::new_session(c, "t", "/tmp").unwrap();
        let who = actor::intern(c, "chorus", "system").unwrap();
        ledger::append(c, &NewEvent::new(s, EventKind::Decision, who, body)).unwrap();
        ledger::append(c, &NewEvent::new(s, EventKind::Decision, who, "use pnpm".to_string())).unwrap();
        let rows = compare_results(&store, 10).unwrap();
        assert_eq!(rows.len(), 1, "an ordinary decision is not a compare result");
        let stats = lane_stats(&rows);
        assert_eq!((stats[0].agent.as_str(), stats[0].mode.as_str()), ("codex+orchestra", "orchestra"));
        assert_eq!((stats[0].runs, stats[0].passed), (1, 1));
        assert_eq!(stats[0].avg_tokens, Some(900));
    }
}
