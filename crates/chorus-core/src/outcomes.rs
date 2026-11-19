//! What actually happened after a turn — the evidence the router learns from.
//!
//! `local_router` used to learn from one thing only: the model a person picked
//! by hand. That says what they prefer, not what works. This module reads the
//! ledger and scores each past turn on three things nobody has to type in:
//!
//! - **tests** — a test command ran during the turn, and its last result
//!   passed or failed;
//! - **rewound** — the person rewound to before the turn (`ย้อนกลับ` fork);
//! - **handed off** — the turn had to leave the agent because of a quality
//!   failure (context exhausted / error), not just a spent quota.
//!
//! Rules that keep this honest (CLAUDE.md 7.24: a harness that feeds itself
//! only measures itself):
//!
//! 1. **No evidence, no learning.** A turn counts only if tests ran, or it was
//!    rewound, or it needed a quality handoff. A turn where nothing was
//!    checked says nothing about the agent and is not stored.
//! 2. **Quota is not quality.** A hand-off for a rate limit / spent quota /
//!    "I asked for the switch" is neutral: the whole turn is skipped.
//! 3. **Small samples don't steer.** A cell (task type × agent × model) needs
//!    [`MIN_SAMPLES`] counted turns, and its Wilson lower bound — not its raw
//!    pass rate — has to beat the router's own pick.
//! 4. **Policy wins.** A suggestion stays in the same cost tier as the
//!    router's pick: heavy thinking stays on the big model, plan-following
//!    coding stays on the small one. Outcomes pick *which* model within a
//!    tier, never which tier.
//!
//! Everything here is derived from the ledger and can be rebuilt from it. The
//! `router_outcome` table is created lazily like `router_override_memory`, so
//! this adds no schema version. Each project keeps its own ledger
//! (`<project>/.chorus/store.db`) but the router is global, so the idle learn
//! pass copies scored turns into the router store (`~/.chorus/store.db`),
//! tagged with the ledger they came from. It is written off the turn path and
//! only *read* by the router.

use crate::types::{Event, EventKind};
use crate::{ledger, Result};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashMap;

/// Counted turns a cell needs before it may influence routing.
pub const MIN_SAMPLES: usize = 5;
/// The suggestion's Wilson lower bound must beat the router's own pick by this.
pub const MIN_MARGIN: f64 = 0.05;
/// …and must itself be at least this good in absolute terms.
pub const MIN_LOWER_BOUND: f64 = 0.50;

/// Title suffix `Session::rewind_before` gives the fork it creates.
const REWIND_SUFFIX: &str = "(ย้อนกลับ)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    /// Planning, design, architecture — heavy thinking.
    Plan,
    Review,
    Refactor,
    Fix,
    Docs,
    /// Everything else: implement / add / create.
    Build,
}

impl TaskType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Review => "review",
            Self::Refactor => "refactor",
            Self::Fix => "fix",
            Self::Docs => "docs",
            Self::Build => "build",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Self::Plan, Self::Review, Self::Refactor, Self::Fix, Self::Docs, Self::Build]
            .into_iter()
            .find(|t| t.as_str() == s)
    }

    /// Short Thai label for reasons shown to the person.
    pub fn label_th(self) -> &'static str {
        match self {
            Self::Plan => "วางแผน/ออกแบบ",
            Self::Review => "รีวิว",
            Self::Refactor => "รื้อโครงสร้าง",
            Self::Fix => "แก้บั๊ก",
            Self::Docs => "เอกสาร",
            Self::Build => "เขียนฟีเจอร์",
        }
    }

    /// Heavy thinking belongs on the big model (the owner's routing policy).
    pub fn is_heavy(self) -> bool {
        matches!(self, Self::Plan | Self::Review | Self::Refactor)
    }
}

/// Bucket a prompt by what is being asked. Keyword based and deliberately
/// coarse: six buckets keep every cell big enough to ever reach
/// [`MIN_SAMPLES`]. First match wins, heaviest first.
pub fn classify(prompt: &str) -> TaskType {
    let p = prompt.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| p.contains(w));
    if has(&["plan", "design", "architect", "strategy", "propose", "วางแผน", "ออกแบบ", "สถาปัตย"]) {
        TaskType::Plan
    } else if has(&["review", "audit", "รีวิว", "ตรวจโค้ด"]) {
        TaskType::Review
    } else if has(&["refactor", "redesign", "migrate", "รื้อ", "ปรับโครงสร้าง"]) {
        TaskType::Refactor
    } else if has(&["fix", "bug", "crash", "error", "failing", "broken", "แก้", "บั๊ก", "พัง"]) {
        TaskType::Fix
    } else if has(&["readme", "docs", "documentation", "comment", "เอกสาร"]) {
        TaskType::Docs
    } else {
        TaskType::Build
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Big,
    Small,
}

/// Cost tier of a model, read off the same price table the router uses
/// (`calculate_cost`) so there is one definition of "big": output ≥ $15/M.
pub fn tier_of(model: &str) -> Tier {
    if crate::local_router::calculate_cost(model, 0, 1_000_000).cost_usd >= 15.0 {
        Tier::Big
    } else {
        Tier::Small
    }
}

/// Test runners an agent's shell call title is matched against.
const TEST_COMMANDS: &[&str] = &[
    "cargo test",
    "cargo nextest",
    "npm test",
    "npm run test",
    "pnpm test",
    "pnpm run test",
    "yarn test",
    "bun test",
    "bun run test",
    "pytest",
    "python -m pytest",
    "python -m unittest",
    "go test",
    "swift test",
    "xcodebuild test",
    "vitest",
    "jest",
    "make test",
    "mvn test",
    "gradle test",
    "gradlew test",
    "rspec",
    "phpunit",
    "dotnet test",
    "ctest",
];

/// Does this tool title look like a test run? `bun test` inside a longer
/// word (`bun testing`) is not matched: a command must start at a word
/// boundary and the runner must be followed by a boundary.
pub fn is_test_command(title: &str) -> bool {
    let t = title.to_lowercase();
    TEST_COMMANDS.iter().any(|c| {
        let mut from = 0;
        while let Some(i) = t[from..].find(c) {
            let at = from + i;
            let before_ok = at == 0 || !t.as_bytes()[at - 1].is_ascii_alphanumeric();
            let end = at + c.len();
            let after_ok = end >= t.len() || !t.as_bytes()[end].is_ascii_alphanumeric();
            if before_ok && after_ok {
                return true;
            }
            from = at + 1;
        }
        false
    })
}

/// Result of the last test command in a span of events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tests {
    Pass,
    Fail,
    None,
}

/// `"<title> → <status>"` as `session.rs::event_row` writes a tool result.
fn parse_result(body: &str) -> Option<(&str, &str)> {
    if body.starts_with('{') {
        return None; // orchestra child-state JSON, not a tool result line
    }
    let (title, status) = body.rsplit_once(" → ")?;
    Some((title.trim(), status.trim()))
}

fn is_pass_status(status: &str) -> bool {
    matches!(status, "completed" | "success" | "ok")
}

/// Last test verdict among `ToolResult` bodies, in order.
pub fn tests_in<'a>(results: impl Iterator<Item = &'a str>) -> Tests {
    let mut verdict = Tests::None;
    for body in results {
        if let Some((title, status)) = parse_result(body) {
            if is_test_command(title) {
                verdict = if is_pass_status(status) { Tests::Pass } else { Tests::Fail };
            }
        }
    }
    verdict
}

/// One scored turn: the user's message and what followed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnOutcome {
    pub sess: i64,
    pub user_seq: i64,
    pub task: TaskType,
    pub agent: String,
    /// Empty when the bridge never reported one.
    pub model: String,
    pub tests: Tests,
    pub rewound: bool,
    pub handed_off: bool,
}

impl TurnOutcome {
    /// Passed means tests passed and nothing undid or abandoned the turn.
    pub fn passed(&self) -> bool {
        self.tests == Tests::Pass && !self.rewound && !self.handed_off
    }
}

/// How a handoff inside a turn reads for the agent that was leaving.
enum HandoffSignal {
    /// Quota, rate limit, a person's request — says nothing about quality.
    Neutral,
    /// Context ran out or the agent errored.
    Quality,
}

fn handoff_signal(body: &str) -> Option<HandoffSignal> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    if v.get("source").is_some() {
        return None; // an import marker, not a switch between agents
    }
    Some(match v["handoff"]["reason"].as_str()? {
        "context_exhausted" | "failover" => HandoffSignal::Quality,
        _ => HandoffSignal::Neutral,
    })
}

/// Score every turn *owned by* `sess` (events of its own, not inherited from
/// a parent it was forked from — those belong to the parent, and counting them
/// twice would make a rewind look like two samples).
pub fn extract_turns(c: &Connection, sess: i64) -> Result<Vec<TurnOutcome>> {
    let user: Option<i64> = c
        .query_row("SELECT id FROM actor WHERE name = 'user'", [], |r| r.get(0))
        .ok();
    let Some(user) = user else {
        return Ok(Vec::new());
    };
    let hist: Vec<Event> = ledger::history(c, sess, 1_000_000)?
        .into_iter()
        .filter(|e| e.sess == sess)
        .collect();

    // Seqs where a rewind forked this session, i.e. everything after was undone.
    let mut stmt = c.prepare(
        "SELECT fork_at_seq FROM session
         WHERE forked_from = ?1 AND fork_at_seq IS NOT NULL AND title LIKE ?2",
    )?;
    let rewinds: Vec<i64> = stmt
        .query_map(params![sess, format!("%{REWIND_SUFFIX}")], |r| r.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    let mut out = Vec::new();
    let mut i = 0;
    while i < hist.len() {
        let e = &hist[i];
        if !(e.kind == EventKind::Msg && e.actor == user) {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < hist.len() && !(hist[j].kind == EventKind::Msg && hist[j].actor == user) {
            j += 1;
        }
        let span = &hist[i + 1..j];
        if let Some(t) = score_turn(c, sess, e, span, user, &rewinds)? {
            out.push(t);
        }
        i = j;
    }
    Ok(out)
}

fn score_turn(
    c: &Connection,
    sess: i64,
    user_msg: &Event,
    span: &[Event],
    user: i64,
    rewinds: &[i64],
) -> Result<Option<TurnOutcome>> {
    let Some(first) = span.iter().find(|e| {
        e.actor != user && matches!(e.kind, EventKind::Msg | EventKind::ToolCall)
    }) else {
        return Ok(None); // nobody answered
    };
    let agent = crate::actor::name(c, first.actor).unwrap_or_default();
    let model = span
        .iter()
        .rev()
        .find(|e| e.kind == EventKind::Msg && e.actor != user)
        .and_then(|e| e.model_id.clone())
        .unwrap_or_default();

    let mut handed_off = false;
    for e in span.iter().filter(|e| e.kind == EventKind::Handoff) {
        match handoff_signal(&ledger::body(c, e)?) {
            Some(HandoffSignal::Neutral) => return Ok(None), // rule 2
            Some(HandoffSignal::Quality) => handed_off = true,
            None => {}
        }
    }

    let mut results: Vec<String> = Vec::new();
    for e in span.iter().filter(|e| e.kind == EventKind::ToolResult) {
        results.push(ledger::body(c, e)?);
    }
    let tests = tests_in(results.iter().map(String::as_str));
    let rewound = rewinds.iter().any(|&at| at < user_msg.seq);

    // Rule 1: no evidence, no sample.
    if tests == Tests::None && !rewound && !handed_off {
        return Ok(None);
    }
    let prompt = ledger::body(c, user_msg)?;
    Ok(Some(TurnOutcome {
        sess,
        user_seq: user_msg.seq,
        task: classify(&prompt),
        agent,
        model,
        tests,
        rewound,
        handed_off,
    }))
}

fn init(c: &Connection) -> Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS router_outcome (
            src       TEXT NOT NULL,
            sess      INTEGER NOT NULL,
            user_seq  INTEGER NOT NULL,
            task      TEXT NOT NULL,
            agent     TEXT NOT NULL,
            model     TEXT NOT NULL,
            tests     INTEGER NOT NULL,
            rewound   INTEGER NOT NULL,
            handed_off INTEGER NOT NULL,
            seen_ms   INTEGER NOT NULL,
            PRIMARY KEY (src, sess, user_seq)
        );
        CREATE INDEX IF NOT EXISTS idx_router_outcome_task ON router_outcome(task);",
    )?;
    Ok(())
}

/// Re-derive outcomes for the `max_sessions` most recently active sessions of
/// the ledger in `source` and store them in `dest` under the label `src`
/// (usually the ledger's path; the two connections may be the same one).
///
/// Idempotent: a session's rows are replaced wholesale, so a turn that is
/// rewound later flips from pass to fail on the next pass. Returns how many
/// turns were written. Off the turn path — call it from the idle learn pass.
pub fn refresh(source: &Connection, dest: &Connection, src: &str, max_sessions: usize) -> Result<usize> {
    init(dest)?;
    let sessions: Vec<i64> = {
        let mut stmt = source.prepare(
            "SELECT sess FROM event GROUP BY sess ORDER BY MAX(seq) DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([max_sessions as i64], |r| r.get(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let mut written = 0;
    for sess in sessions {
        let turns = extract_turns(source, sess)?;
        dest.execute(
            "DELETE FROM router_outcome WHERE src = ?1 AND sess = ?2",
            params![src, sess],
        )?;
        for t in turns {
            dest.execute(
                "INSERT OR REPLACE INTO router_outcome
                 (src, sess, user_seq, task, agent, model, tests, rewound, handed_off, seen_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    src,
                    t.sess,
                    t.user_seq,
                    t.task.as_str(),
                    t.agent,
                    t.model,
                    match t.tests {
                        Tests::Pass => 1,
                        Tests::Fail => 0,
                        Tests::None => -1,
                    },
                    t.rewound as i64,
                    t.handed_off as i64,
                    now
                ],
            )?;
            written += 1;
        }
    }
    Ok(written)
}

/// Counted turns of one (task, agent, model).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Cell {
    pub task: String,
    /// Thai label of `task`, for the Learn screen.
    pub task_label: String,
    pub agent: String,
    pub model: String,
    pub n: usize,
    pub passed: usize,
    /// Wilson lower bound of the pass rate (what routing compares).
    pub lower_bound: f64,
    /// Enough evidence, and good enough, to steer routing today.
    pub steers: bool,
}

/// Wilson score lower bound, one-sided 90% (z = 1.2816). With few samples it
/// is far below the raw rate — 5 of 5 passes is ~0.78, not 1.0 — which is the
/// point: a lucky streak must not outrank a long record.
pub fn wilson_lower(passed: usize, n: usize) -> f64 {
    if n == 0 {
        return 0.0;
    }
    let z = 1.2816_f64;
    let (p, n) = (passed as f64 / n as f64, n as f64);
    let denom = 1.0 + z * z / n;
    let centre = p + z * z / (2.0 * n);
    let margin = z * ((p * (1.0 - p) + z * z / (4.0 * n)) / n).sqrt();
    ((centre - margin) / denom).max(0.0)
}

/// All cells with a known model, most-sampled first.
pub fn cells(c: &Connection) -> Result<Vec<Cell>> {
    init(c)?;
    let mut stmt = c.prepare(
        "SELECT task, agent, model, COUNT(*),
                SUM(CASE WHEN tests = 1 AND rewound = 0 AND handed_off = 0 THEN 1 ELSE 0 END)
         FROM router_outcome WHERE model <> ''
         GROUP BY task, agent, model ORDER BY COUNT(*) DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        let n: i64 = r.get(3)?;
        let passed: i64 = r.get(4)?;
        let task: String = r.get(0)?;
        let lower_bound = wilson_lower(passed as usize, n as usize);
        Ok(Cell {
            task_label: TaskType::parse(&task).map(TaskType::label_th).unwrap_or("อื่น ๆ").into(),
            task,
            agent: r.get(1)?,
            model: r.get(2)?,
            n: n as usize,
            passed: passed as usize,
            lower_bound,
            steers: n as usize >= MIN_SAMPLES && lower_bound >= MIN_LOWER_BOUND,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// An outcome-based preference over the router's own pick.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub agent: String,
    pub model: String,
    pub n: usize,
    pub passed: usize,
    pub reason: String,
}

/// Is learning from outcomes switched off (`CHORUS_ROUTER_OUTCOMES=off`)?
pub fn disabled() -> bool {
    matches!(
        std::env::var("CHORUS_ROUTER_OUTCOMES").as_deref(),
        Ok("off") | Ok("0") | Ok("false")
    )
}

/// Should the router prefer something over `base`? `None` is the common
/// answer: not enough evidence, nobody clearly better, or learning is off.
pub fn suggest(
    c: &Connection,
    task: TaskType,
    base_agent: &str,
    base_model: &str,
) -> Option<Suggestion> {
    if disabled() {
        return None;
    }
    let all = cells(c).ok()?;
    let in_task: Vec<&Cell> = all.iter().filter(|x| x.task == task.as_str()).collect();
    let base_lb = in_task
        .iter()
        .find(|x| x.agent == base_agent && x.model == base_model)
        .map(|x| x.lower_bound)
        .unwrap_or(0.0);
    let tier = tier_of(base_model);
    let best = in_task
        .iter()
        .filter(|x| x.n >= MIN_SAMPLES && tier_of(&x.model) == tier)
        .filter(|x| !(x.agent == base_agent && x.model == base_model))
        .max_by(|a, b| a.lower_bound.total_cmp(&b.lower_bound))?;
    if best.lower_bound < MIN_LOWER_BOUND || best.lower_bound < base_lb + MIN_MARGIN {
        return None;
    }
    Some(Suggestion {
        agent: best.agent.clone(),
        model: best.model.clone(),
        n: best.n,
        passed: best.passed,
        reason: format!(
            "เรียนรู้จากผลจริง: {}/{} ผ่านเทสต์ {}/{} เทิร์นในงาน{} (ไม่ถูกย้อนกลับ/ส่งต่อ) ดีกว่า {}",
            best.agent,
            best.model,
            best.passed,
            best.n,
            task.label_th(),
            if base_model.is_empty() { base_agent } else { base_model },
        ),
    })
}

/// Model ids carry an effort suffix in some bridges (`gpt-5.5[low]`); the
/// quality record belongs to the model, so compare without it.
fn base_model(model: &str) -> &str {
    model.split('[').next().unwrap_or(model)
}

/// Apply outcome evidence to a proposal the router already made: if a *ready*
/// candidate of the same cost tier has a clearly better record for this kind
/// of task, propose it instead and say why. Otherwise the proposal is returned
/// untouched — this never makes a proposal worse than "no data".
pub fn refine_proposal(
    conn: &Connection,
    task: &crate::router::TaskSummary,
    candidates: &crate::router::CandidateSet,
    proposal: crate::router::RouteProposal,
) -> crate::router::RouteProposal {
    let Some(picked) = candidates.candidates.iter().find(|c| c.id == proposal.candidate_id) else {
        return proposal;
    };
    let Some(s) = suggest(conn, classify(&task.text), &picked.agent, base_model(&picked.model))
    else {
        return proposal;
    };
    let better = candidates
        .candidates
        .iter()
        .filter(|c| {
            c.ready
                && crate::router::candidate_supports_task(c, task)
                && c.agent == s.agent
                && base_model(&c.model) == base_model(&s.model)
        })
        // Keep the effort the router chose when the better model offers it.
        .min_by_key(|c| (c.effort != picked.effort) as u8);
    match better {
        Some(c) => crate::router::RouteProposal {
            candidate_id: c.id.clone(),
            reason: s.reason,
            ..proposal
        },
        None => proposal,
    }
}

/// Per-task rollup for the Learn screen: how many turns were counted and
/// whether any cell has enough evidence to steer routing yet.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub cells: Vec<Cell>,
    pub counted_turns: usize,
    pub min_samples: usize,
    pub enabled: bool,
    /// Task type → cells that may steer routing today.
    pub steering: HashMap<String, usize>,
}

pub fn summary(c: &Connection) -> Result<Summary> {
    let cells = cells(c)?;
    let mut steering: HashMap<String, usize> = HashMap::new();
    for x in &cells {
        if x.steers {
            *steering.entry(x.task.clone()).or_default() += 1;
        }
    }
    let counted_turns = cells.iter().map(|x| x.n).sum();
    Ok(Summary {
        cells,
        counted_turns,
        min_samples: MIN_SAMPLES,
        enabled: !disabled(),
        steering,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NewEvent;
    use crate::{actor, Store};

    struct Fx {
        store: Store,
        sess: i64,
        user: i64,
    }

    impl Fx {
        fn new() -> Self {
            let store = Store::open_in_memory().unwrap();
            let c = store.conn();
            let sess = ledger::new_session(c, "t", "/p").unwrap();
            let user = actor::intern(c, "user", "human").unwrap();
            Self { store, sess, user }
        }

        fn say(&self, text: &str) -> i64 {
            ledger::append(
                self.store.conn(),
                &NewEvent::new(self.sess, EventKind::Msg, self.user, text),
            )
            .unwrap()
        }

        /// One agent turn: optional test result, final message with `model`.
        fn turn(&self, agent: &str, model: &str, prompt: &str, test: Option<&str>) -> i64 {
            let c = self.store.conn();
            let a = actor::intern(c, agent, agent).unwrap();
            let seq = self.say(prompt);
            if let Some(status) = test {
                ledger::append(
                    c,
                    &NewEvent::new(self.sess, EventKind::ToolCall, a, "execute: cargo test"),
                )
                .unwrap();
                ledger::append(
                    c,
                    &NewEvent::new(
                        self.sess,
                        EventKind::ToolResult,
                        a,
                        format!("cargo test → {status}"),
                    ),
                )
                .unwrap();
            }
            ledger::append(
                c,
                &NewEvent::new(self.sess, EventKind::Msg, a, "done").model(Some(model.into())),
            )
            .unwrap();
            seq
        }
    }

    #[test]
    fn classifies_by_what_is_asked() {
        assert_eq!(classify("วางแผน migration"), TaskType::Plan);
        assert_eq!(classify("review this PR"), TaskType::Review);
        assert_eq!(classify("refactor the store"), TaskType::Refactor);
        assert_eq!(classify("แก้บั๊ก login"), TaskType::Fix);
        assert_eq!(classify("update the README docs"), TaskType::Docs);
        assert_eq!(classify("add a --json flag"), TaskType::Build);
        assert!(TaskType::Plan.is_heavy() && !TaskType::Fix.is_heavy());
    }

    #[test]
    fn test_commands_match_on_word_boundaries() {
        assert!(is_test_command("cargo test -p chorus-core"));
        assert!(is_test_command("cd app && npm run test"));
        assert!(is_test_command("Run pytest -q"));
        assert!(!is_test_command("cargo testing-tools build"));
        assert!(!is_test_command("cat jester.txt"));
        assert!(!is_test_command("git status"));
    }

    #[test]
    fn last_test_result_wins() {
        let r = ["cargo test → failed", "edit → completed", "cargo test → completed"];
        assert_eq!(tests_in(r.into_iter()), Tests::Pass);
        let r = ["cargo test → completed", "cargo test → failed"];
        assert_eq!(tests_in(r.into_iter()), Tests::Fail);
        let r = ["git status → completed"];
        assert_eq!(tests_in(r.into_iter()), Tests::None);
    }

    #[test]
    fn tiers_follow_the_price_table() {
        assert_eq!(tier_of("claude-opus-4-6"), Tier::Big);
        assert_eq!(tier_of("claude-sonnet-4-6"), Tier::Big);
        assert_eq!(tier_of("gpt-5.1-codex-mini"), Tier::Small);
        assert_eq!(tier_of("gpt-5.6-luna"), Tier::Small);
    }

    #[test]
    fn wilson_does_not_trust_small_samples() {
        assert!(wilson_lower(5, 5) < 0.85);
        assert!(wilson_lower(5, 5) > 0.6);
        assert!(wilson_lower(50, 50) > wilson_lower(5, 5));
        assert_eq!(wilson_lower(0, 0), 0.0);
    }

    #[test]
    fn turns_without_evidence_are_not_stored() {
        let f = Fx::new();
        f.turn("codex", "m1", "add a flag", None);
        f.turn("codex", "m1", "add another flag", Some("completed"));
        let t = extract_turns(f.store.conn(), f.sess).unwrap();
        assert_eq!(t.len(), 1, "only the turn where tests ran counts");
        assert!(t[0].passed());
        assert_eq!(t[0].model, "m1");
    }

    #[test]
    fn a_rewound_turn_is_a_failure_even_without_tests() {
        let f = Fx::new();
        let first = f.turn("codex", "m1", "add a flag", None);
        f.turn("codex", "m1", "now another", Some("completed"));
        // The person rewound to before the first message.
        let c = f.store.conn();
        let fork = ledger::fork(c, f.sess, first - 1, "t (ย้อนกลับ)").unwrap();
        assert!(fork > 0);
        let t = extract_turns(c, f.sess).unwrap();
        assert_eq!(t.len(), 2);
        assert!(t.iter().all(|x| x.rewound && !x.passed()));
    }

    #[test]
    fn a_compare_fork_is_not_a_rewind() {
        let f = Fx::new();
        f.turn("codex", "m1", "add a flag", Some("completed"));
        let c = f.store.conn();
        ledger::fork(c, f.sess, 1, "compare: claude").unwrap();
        let t = extract_turns(c, f.sess).unwrap();
        assert!(!t[0].rewound);
    }

    fn handoff(f: &Fx, reason: &str) {
        let c = f.store.conn();
        let to = actor::intern(c, "claude", "claude").unwrap();
        let body = serde_json::json!({"handoff": {"from": "codex", "to": "claude", "reason": reason}});
        ledger::append(c, &NewEvent::new(f.sess, EventKind::Handoff, to, body.to_string())).unwrap();
    }

    #[test]
    fn quota_handoffs_are_neutral_but_context_exhaustion_is_a_failure() {
        let f = Fx::new();
        let c = f.store.conn();
        let a = actor::intern(c, "codex", "codex").unwrap();
        f.say("add a flag");
        ledger::append(c, &NewEvent::new(f.sess, EventKind::ToolCall, a, "execute: cargo test")).unwrap();
        handoff(&f, "quota_exhausted");
        f.say("add another flag");
        ledger::append(c, &NewEvent::new(f.sess, EventKind::ToolCall, a, "execute: cargo test")).unwrap();
        ledger::append(c, &NewEvent::new(f.sess, EventKind::ToolResult, a, "cargo test → completed")).unwrap();
        handoff(&f, "context_exhausted");
        let t = extract_turns(c, f.sess).unwrap();
        assert_eq!(t.len(), 1, "the quota turn is skipped entirely");
        assert!(t[0].handed_off && !t[0].passed());
    }

    #[test]
    fn forked_sessions_do_not_recount_the_parents_turns() {
        let f = Fx::new();
        f.turn("codex", "m1", "add a flag", Some("completed"));
        let c = f.store.conn();
        let fork = ledger::fork(c, f.sess, 1_000, "compare: claude").unwrap();
        assert!(extract_turns(c, fork).unwrap().is_empty());
        assert_eq!(extract_turns(c, f.sess).unwrap().len(), 1);
    }

    /// Write `n` fix-turns for `agent`/`model`, `fails` of them with a failing test.
    fn seed(f: &Fx, agent: &str, model: &str, n: usize, fails: usize) {
        for i in 0..n {
            let status = if i < fails { "failed" } else { "completed" };
            f.turn(agent, model, "fix the failing parser", Some(status));
        }
    }

    #[test]
    fn small_samples_never_steer_routing() {
        let f = Fx::new();
        seed(&f, "codex", "gpt-5.1-codex", 4, 0);
        refresh(f.store.conn(), f.store.conn(), "t", 10).unwrap();
        assert!(suggest(f.store.conn(), TaskType::Fix, "codex", "gpt-5.1-codex-mini").is_none());
    }

    #[test]
    fn enough_evidence_prefers_the_better_model_within_the_tier() {
        let f = Fx::new();
        seed(&f, "codex", "gpt-5.1-codex", 12, 0);
        seed(&f, "codex", "gpt-5.1-codex-mini", 12, 7);
        // A big-tier model with a perfect record must not be offered for a
        // small-tier pick: outcomes choose within a tier, never across it.
        seed(&f, "claude", "claude-opus-4-6", 20, 0);
        refresh(f.store.conn(), f.store.conn(), "t", 10).unwrap();

        let s = suggest(f.store.conn(), TaskType::Fix, "codex", "gpt-5.1-codex-mini")
            .expect("the codex model with 12/12 should win");
        assert_eq!((s.agent.as_str(), s.model.as_str()), ("codex", "gpt-5.1-codex"));
        assert!(s.reason.contains("12/12"), "{}", s.reason);

        // Nothing beats the best model, so nothing is suggested over it.
        assert!(suggest(f.store.conn(), TaskType::Fix, "claude", "claude-opus-4-6").is_none());
        // Other task types have no evidence.
        assert!(suggest(f.store.conn(), TaskType::Plan, "codex", "gpt-5.1-codex-mini").is_none());
    }

    fn cand(id: &str, agent: &str, model: &str, effort: &str, ready: bool) -> crate::router::Candidate {
        crate::router::Candidate {
            id: id.into(),
            agent: agent.into(),
            model: model.into(),
            effort: effort.into(),
            media_types: vec![],
            integrations: vec![],
            ready,
            tier: None,
        }
    }

    fn proposal(id: &str) -> crate::router::RouteProposal {
        crate::router::RouteProposal {
            candidate_id: id.into(),
            reason: "heuristic".into(),
            confidence: crate::router::Confidence::Medium,
            router_model: "in-process-heuristic".into(),
            router_version: None,
        }
    }

    #[test]
    fn refine_swaps_in_a_better_ready_candidate_of_the_same_tier() {
        let f = Fx::new();
        seed(&f, "codex", "gpt-5.1-codex", 12, 0);
        seed(&f, "codex", "gpt-5.1-codex-mini", 12, 7);
        refresh(f.store.conn(), f.store.conn(), "t", 10).unwrap();
        let task = crate::router::TaskSummary {
            text: "fix the failing parser".into(),
            attachments: vec![],
            enabled_integrations: vec![],
            required_integrations: vec![],
        };
        let set = crate::router::CandidateSet::new(vec![
            cand("mini", "codex", "gpt-5.1-codex-mini", "low", true),
            cand("full-hi", "codex", "gpt-5.1-codex[high]", "high", true),
            cand("full-lo", "codex", "gpt-5.1-codex[low]", "low", true),
            cand("opus", "claude", "claude-opus-4-6", "low", true),
        ]);
        let out = refine_proposal(f.store.conn(), &task, &set, proposal("mini"));
        // Same model family wins; the effort the router chose is kept.
        assert_eq!(out.candidate_id, "full-lo");
        assert!(out.reason.starts_with("เรียนรู้จากผลจริง"), "{}", out.reason);
        assert_eq!(out.router_model, "in-process-heuristic");

        // If the better model is not ready, the proposal is left alone.
        let set = crate::router::CandidateSet::new(vec![
            cand("mini", "codex", "gpt-5.1-codex-mini", "low", true),
            cand("full", "codex", "gpt-5.1-codex", "low", false),
        ]);
        let kept = refine_proposal(f.store.conn(), &task, &set, proposal("mini"));
        assert_eq!(kept.candidate_id, "mini");
        assert_eq!(kept.reason, "heuristic");
    }

    #[test]
    fn a_mediocre_record_is_not_promoted() {
        let f = Fx::new();
        seed(&f, "codex", "gpt-5.1-codex", 10, 5);
        refresh(f.store.conn(), f.store.conn(), "t", 10).unwrap();
        assert!(suggest(f.store.conn(), TaskType::Fix, "codex", "gpt-5.1-codex-mini").is_none());
    }

    #[test]
    fn refresh_is_idempotent_and_picks_up_a_later_rewind() {
        let f = Fx::new();
        let first = f.turn("codex", "m1", "fix the parser", Some("completed"));
        let c = f.store.conn();
        assert_eq!(refresh(c, c, "t", 10).unwrap(), 1);
        assert_eq!(refresh(c, c, "t", 10).unwrap(), 1);
        assert_eq!(cells(c).unwrap()[0].passed, 1);
        ledger::fork(c, f.sess, first - 1, "t (ย้อนกลับ)").unwrap();
        refresh(c, c, "t", 10).unwrap();
        assert_eq!(cells(c).unwrap()[0].passed, 0);
    }
}
