//! Skill drafts — recipes that kept working, written down for review.
//!
//! When the same run of commands shows up in three or more past sessions of
//! this project, and every one of those sessions ended with its tests passing,
//! that run is probably worth a skill. This module finds such runs in the
//! ledger and writes each as a draft `SKILL.md` into a review queue.
//!
//! **Never auto-enabled.** Self-written skills nobody reviewed do not help
//! (SkillsBench, arXiv 2602.12670), and a skill is instructions every agent
//! will follow — so a draft sits in `.chorus/skill-drafts/` where no agent
//! loader looks, until a person accepts it. Accepting copies it into
//! `.chorus/skills/`, the canonical source that [`crate::projector`] and
//! [`crate::skills`] already project for every agent. The autonomy setting of
//! the Learn screen governs lessons, not skills: this queue is always manual.
//!
//! Evidence, not imagination: a draft lists only commands that actually ran,
//! reduced to their first two words (`cargo test`, `git status`) so no
//! argument, path or secret travels into it. It is derived from the ledger
//! off the turn path, and it is a rule-based pass — no model is involved.

use crate::outcomes::{is_test_command, tests_in, Tests};
use crate::types::EventKind;
use crate::{ledger, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Sessions that must share a sequence before it becomes a draft.
pub const MIN_SUPPORT: usize = 3;
/// Shortest and longest recipe, in steps.
const MIN_LEN: usize = 2;
const MAX_LEN: usize = 6;
/// Drafts written per scan, so one pass never floods the queue.
pub const MAX_NEW_PER_PASS: usize = 3;
/// Sessions looked at per scan (newest first).
const MAX_SESSIONS: usize = 200;

/// Where drafts wait for review, relative to the workspace root.
pub const DRAFTS_DIR: &str = ".chorus/skill-drafts";
const DECIDED_FILE: &str = "decided.json";
const META_FILE: &str = "draft.json";

/// The step an edit tool call reduces to — there is no command to name.
const EDIT: &str = "edit files";

/// One tool call, reduced to something that can recur across sessions.
///
/// `execute: cd app && cargo test -p x` → `cargo test`; any edit → `edit files`;
/// reads and searches → `None` (they interleave at random and would break
/// every sequence); anything with an assignment, a pipe-to-shell or an odd
/// character → `None`, so nothing secret-shaped is ever quoted.
pub fn normalize_step(body: &str) -> Option<String> {
    let (kind, title) = body.split_once(": ")?;
    match kind.trim() {
        "edit" | "delete" | "move" => return Some(EDIT.to_string()),
        "execute" => {}
        _ => return None,
    }
    let mut title = title.trim();
    for prefix in ["Run ", "run ", "Running ", "$ "] {
        if let Some(rest) = title.strip_prefix(prefix) {
            title = rest.trim();
        }
    }
    // `cd x && cmd` and `VAR=1 cmd`: the interesting part is the last command.
    let last = title.rsplit("&&").next().unwrap_or(title).trim();
    let mut words = last.split_whitespace();
    let first = words.next()?;
    if first.contains('=') || (first.contains('/') && !first.starts_with("./")) {
        return None;
    }
    let second = words.next().filter(|w| !w.starts_with('-') && !w.contains('/'));
    let step = match second {
        Some(w) => format!("{first} {w}"),
        None => first.to_string(),
    };
    let ok = step.len() <= 40
        && step
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | ':'));
    ok.then_some(step)
}

/// Ordered, de-duplicated steps of one session, and whether it ended green.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSteps {
    pub sess: i64,
    pub steps: Vec<String>,
    pub ended_green: bool,
}

/// Read one session's own events into steps. `ended_green` is the verdict of
/// the *last* test command in the session: a session that fixed its failure
/// counts, one that gave up with a red test does not.
pub fn session_steps(c: &Connection, sess: i64) -> Result<SessionSteps> {
    let hist = ledger::history(c, sess, 1_000_000)?;
    let mut steps: Vec<String> = Vec::new();
    let mut results: Vec<String> = Vec::new();
    for e in hist.iter().filter(|e| e.sess == sess) {
        match e.kind {
            EventKind::ToolCall => {
                if let Some(s) = normalize_step(&ledger::body(c, e)?) {
                    if steps.last() != Some(&s) {
                        steps.push(s);
                    }
                }
            }
            EventKind::ToolResult => results.push(ledger::body(c, e)?),
            _ => {}
        }
    }
    let ended_green = tests_in(results.iter().map(String::as_str)) == Tests::Pass;
    Ok(SessionSteps { sess, steps, ended_green })
}

/// A recurring run and who vouches for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    pub steps: Vec<String>,
    pub sessions: Vec<i64>,
}

fn is_command(step: &str) -> bool {
    step != EDIT
}

/// Contiguous step runs shared by at least `min_support` green sessions.
///
/// A run qualifies only if it has a test command in it and at least two
/// different commands: `edit files → cargo test` alone is just "change code,
/// run the tests", which every agent already does. Maximal runs only — when a
/// longer run has the same support, its parts are dropped.
pub fn recipes_from(sessions: &[SessionSteps], min_support: usize) -> Vec<Recipe> {
    let mut seen: HashMap<Vec<String>, BTreeSet<i64>> = HashMap::new();
    for s in sessions.iter().filter(|s| s.ended_green) {
        let mut mine: HashSet<&[String]> = HashSet::new();
        for len in MIN_LEN..=MAX_LEN {
            for w in s.steps.windows(len) {
                mine.insert(w);
            }
        }
        for w in mine {
            seen.entry(w.to_vec()).or_default().insert(s.sess);
        }
    }
    let mut found: Vec<Recipe> = seen
        .into_iter()
        .filter(|(steps, who)| {
            who.len() >= min_support
                && steps.iter().any(|s| is_test_command(s))
                && steps
                    .iter()
                    .filter(|s| is_command(s))
                    .collect::<HashSet<_>>()
                    .len()
                    >= 2
        })
        .map(|(steps, who)| Recipe { steps, sessions: who.into_iter().collect() })
        .collect();
    // Drop a run when a longer one contains it with the same support.
    let all = found.clone();
    found.retain(|r| {
        !all.iter().any(|o| {
            o.steps.len() > r.steps.len()
                && o.sessions.len() == r.sessions.len()
                && o.steps.windows(r.steps.len()).any(|w| w == r.steps.as_slice())
        })
    });
    // Best first: more sessions, then more steps, then alphabetical — a total
    // order, so two scans over the same ledger write the same drafts.
    found.sort_by(|a, b| {
        b.sessions
            .len()
            .cmp(&a.sessions.len())
            .then(b.steps.len().cmp(&a.steps.len()))
            .then(a.steps.cmp(&b.steps))
    });
    found
}

/// A draft waiting for review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    /// Directory-safe name: `[a-z0-9-]+`.
    pub name: String,
    pub description: String,
    /// `a → b → c`, the identity used to remember a decision.
    pub signature: String,
    pub steps: Vec<String>,
    pub sessions: Vec<i64>,
    /// The `SKILL.md` that will be written on accept.
    pub body: String,
}

fn slug(steps: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for s in steps.iter().filter(|s| is_command(s)) {
        let p: String = s
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let p = p.trim_matches('-').to_string();
        if !p.is_empty() && !parts.contains(&p) {
            parts.push(p);
        }
        if parts.len() == 3 {
            break;
        }
    }
    let joined = parts.join("-").replace("--", "-");
    let hash = blake3::hash(steps.join("\u{1f}").as_bytes()).to_hex();
    format!("recipe-{}-{}", joined, &hash[..6])
}

/// Turn a recipe into a draft `SKILL.md`.
pub fn draft_of(r: &Recipe) -> Draft {
    let name = slug(&r.steps);
    let signature = r.steps.join(" → ");
    let n = r.sessions.len();
    let description = format!(
        "Sequence that ended with passing tests in {n} past sessions of this project: {signature}. \
         Use when changing code here and you need to verify the change the way those sessions did."
    );
    let mut body = format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n\n");
    body.push_str(
        "Drafted by Chorus from the ledger. It lists commands that actually ran in the sessions below, \
         in the order they ran, and says nothing about why. Read it before you keep it.\n\n",
    );
    for (i, s) in r.steps.iter().enumerate() {
        if s == EDIT {
            body.push_str(&format!("{}. Make the code change.\n", i + 1));
        } else {
            body.push_str(&format!("{}. Run `{s}`.\n", i + 1));
        }
    }
    let ids: Vec<String> = r.sessions.iter().map(|s| s.to_string()).collect();
    body.push_str(&format!("\nEvidence: sessions {} (each ended with its tests passing).\n", ids.join(", ")));
    Draft {
        name,
        description,
        signature,
        steps: r.steps.clone(),
        sessions: r.sessions.clone(),
        body,
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Decided {
    #[serde(default)]
    rejected: Vec<String>,
    #[serde(default)]
    accepted: Vec<String>,
}

fn drafts_root(cwd: &Path) -> PathBuf {
    cwd.join(DRAFTS_DIR)
}

fn load_decided(cwd: &Path) -> Decided {
    std::fs::read_to_string(drafts_root(cwd).join(DECIDED_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_decided(cwd: &Path, d: &Decided) -> Result<()> {
    let root = drafts_root(cwd);
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join(DECIDED_FILE), serde_json::to_string_pretty(d)?)?;
    Ok(())
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Drafts waiting for review in `cwd`, sorted by name.
pub fn list(cwd: &Path) -> Vec<Draft> {
    let Ok(rd) = std::fs::read_dir(drafts_root(cwd)) else {
        return Vec::new();
    };
    let mut out: Vec<Draft> = rd
        .flatten()
        .filter_map(|e| {
            let meta = std::fs::read_to_string(e.path().join(META_FILE)).ok()?;
            let d: Draft = serde_json::from_str(&meta).ok()?;
            // The directory name is the identity; a hand-edited meta file must
            // not point an accept at some other path.
            (valid_name(&d.name) && e.file_name().to_str() == Some(d.name.as_str())).then_some(d)
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Find new recipes in the project's past sessions and queue drafts for them.
/// Returns only the drafts written by this pass.
///
/// Skips a recipe that is already queued, was rejected or was accepted, and
/// writes at most [`MAX_NEW_PER_PASS`]. Off the turn path.
pub fn scan(c: &Connection, cwd: &Path) -> Result<Vec<Draft>> {
    let cwd_s = cwd.to_string_lossy().to_string();
    let ids: Vec<i64> = {
        let mut stmt = c.prepare(
            "SELECT s.id FROM session s WHERE s.cwd = ?1
             ORDER BY (SELECT COALESCE(MAX(seq), 0) FROM event WHERE sess = s.id) DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![cwd_s, MAX_SESSIONS as i64], |r| r.get(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let mut sessions = Vec::new();
    for id in ids {
        sessions.push(session_steps(c, id)?);
    }
    let recipes = recipes_from(&sessions, MIN_SUPPORT);

    let decided = load_decided(cwd);
    let queued: HashSet<String> = list(cwd).into_iter().map(|d| d.signature).collect();
    let mut written = Vec::new();
    for r in recipes {
        if written.len() >= MAX_NEW_PER_PASS {
            break;
        }
        let d = draft_of(&r);
        if queued.contains(&d.signature)
            || decided.rejected.contains(&d.signature)
            || decided.accepted.contains(&d.signature)
        {
            continue;
        }
        let dir = drafts_root(cwd).join(&d.name);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("SKILL.md"), &d.body)?;
        std::fs::write(dir.join(META_FILE), serde_json::to_string_pretty(&d)?)?;
        written.push(d);
    }
    Ok(written)
}

/// Accept a draft: copy it into `.chorus/skills/<name>/SKILL.md` — where the
/// projector picks it up — and take it off the queue. Refuses to overwrite a
/// skill that already exists under that name.
pub fn accept(cwd: &Path, name: &str) -> Result<PathBuf> {
    let d = find(cwd, name)?;
    let target_dir = cwd.join(crate::skills::SKILLS_DIR).join(&d.name);
    if target_dir.exists() {
        return Err(crate::ChorusError::Invalid(format!(
            "skill {} already exists",
            d.name
        )));
    }
    std::fs::create_dir_all(&target_dir)?;
    let target = target_dir.join("SKILL.md");
    std::fs::write(&target, &d.body)?;
    let mut decided = load_decided(cwd);
    decided.accepted.push(d.signature.clone());
    save_decided(cwd, &decided)?;
    std::fs::remove_dir_all(drafts_root(cwd).join(&d.name))?;
    Ok(target)
}

/// Reject a draft and remember it, so the same recipe is not drafted again.
pub fn reject(cwd: &Path, name: &str) -> Result<()> {
    let d = find(cwd, name)?;
    let mut decided = load_decided(cwd);
    decided.rejected.push(d.signature.clone());
    save_decided(cwd, &decided)?;
    std::fs::remove_dir_all(drafts_root(cwd).join(&d.name))?;
    Ok(())
}

fn find(cwd: &Path, name: &str) -> Result<Draft> {
    if !valid_name(name) {
        return Err(crate::ChorusError::Invalid(format!("bad draft name: {name}")));
    }
    list(cwd)
        .into_iter()
        .find(|d| d.name == name)
        .ok_or_else(|| crate::ChorusError::Invalid(format!("no draft named {name}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NewEvent;
    use crate::{actor, Store};

    fn steps(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn steps_are_reduced_to_command_heads() {
        let n = normalize_step;
        assert_eq!(n("execute: cargo test -p chorus-core --lib").as_deref(), Some("cargo test"));
        assert_eq!(n("execute: cd app && npm run build").as_deref(), Some("npm run"));
        assert_eq!(n("execute: Run git status").as_deref(), Some("git status"));
        assert_eq!(n("edit: src/main.rs").as_deref(), Some(EDIT));
        assert_eq!(n("read: src/main.rs"), None);
        assert_eq!(n("search: foo"), None);
        // Nothing secret-shaped is ever quoted.
        assert_eq!(n("execute: API_KEY=abc123 curl https://x"), None);
        assert_eq!(n("execute: /usr/local/bin/tool run"), None);
        assert_eq!(n("execute: curl -H 'Authorization: x' url").as_deref(), Some("curl"));
    }

    fn green(sess: i64, list: &[&str]) -> SessionSteps {
        SessionSteps { sess, steps: steps(list), ended_green: true }
    }

    #[test]
    fn a_recipe_needs_three_green_sessions() {
        let run = ["edit files", "cargo build", "cargo test"];
        let two = [green(1, &run), green(2, &run)];
        assert!(recipes_from(&two, MIN_SUPPORT).is_empty());

        let three = [green(1, &run), green(2, &run), green(3, &run)];
        let r = recipes_from(&three, MIN_SUPPORT);
        assert_eq!(r.len(), 1, "only the maximal run: {r:?}");
        assert_eq!(r[0].steps, steps(&run));
        assert_eq!(r[0].sessions, vec![1, 2, 3]);
    }

    #[test]
    fn red_sessions_do_not_vouch_for_a_recipe() {
        let run = ["edit files", "cargo build", "cargo test"];
        let mut red = green(3, &run);
        red.ended_green = false;
        assert!(recipes_from(&[green(1, &run), green(2, &run), red], MIN_SUPPORT).is_empty());
    }

    #[test]
    fn trivial_and_testless_runs_are_not_recipes() {
        // Just "edit then test" is what every agent does anyway.
        let trivial = ["edit files", "cargo test"];
        let s = [green(1, &trivial), green(2, &trivial), green(3, &trivial)];
        assert!(recipes_from(&s, MIN_SUPPORT).is_empty());
        // No test command in the run: nothing says it worked.
        let testless = ["git status", "git diff", "cargo test"];
        let s = [green(1, &testless), green(2, &testless), green(3, &testless)];
        let r = recipes_from(&s, MIN_SUPPORT);
        assert!(r.iter().all(|x| x.steps.iter().any(|t| is_test_command(t))));
    }

    #[test]
    fn draft_is_a_valid_skill_with_evidence() {
        let r = Recipe {
            steps: steps(&["edit files", "cargo build", "cargo test"]),
            sessions: vec![4, 7, 9],
        };
        let d = draft_of(&r);
        assert!(valid_name(&d.name), "{}", d.name);
        assert!(d.body.starts_with("---\nname: recipe-cargo-build-cargo-test-"));
        assert!(d.body.contains("2. Run `cargo build`."));
        assert!(d.body.contains("sessions 4, 7, 9"));
        // Same recipe, same name: re-scans do not duplicate.
        assert_eq!(draft_of(&r).name, d.name);
    }

    fn seed_session(st: &Store, cwd: &str, run: &[(&str, &str)], status: &str) {
        let c = st.conn();
        let sess = ledger::new_session(c, "t", cwd).unwrap();
        let a = actor::intern(c, "codex", "codex").unwrap();
        for (call, _) in run {
            ledger::append(c, &NewEvent::new(sess, EventKind::ToolCall, a, *call)).unwrap();
        }
        ledger::append(
            c,
            &NewEvent::new(sess, EventKind::ToolResult, a, format!("cargo test → {status}")),
        )
        .unwrap();
    }

    #[test]
    fn scan_queues_then_accept_and_reject_are_remembered() {
        let st = Store::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path();
        let cwd_s = cwd.to_string_lossy().to_string();
        let run = [
            ("edit: src/a.rs", ""),
            ("execute: cargo build --release", ""),
            ("execute: cargo test -p x", ""),
        ];
        for _ in 0..3 {
            seed_session(&st, &cwd_s, &run, "completed");
        }
        // A session in another project, and a red one, add nothing.
        seed_session(&st, "/elsewhere", &run, "completed");
        seed_session(&st, &cwd_s, &run, "failed");

        let new = scan(st.conn(), cwd).unwrap();
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].sessions.len(), 3);
        // Queued, not enabled: nothing is in the skills dir.
        assert_eq!(list(cwd).len(), 1);
        assert!(!cwd.join(crate::skills::SKILLS_DIR).exists());
        // A second scan finds nothing new.
        assert!(scan(st.conn(), cwd).unwrap().is_empty());

        let name = new[0].name.clone();
        let path = accept(cwd, &name).unwrap();
        assert!(path.ends_with(format!("{name}/SKILL.md")));
        assert!(std::fs::read_to_string(&path).unwrap().contains("cargo build"));
        assert!(list(cwd).is_empty());
        // The accepted recipe is not drafted again, and the projector sees it.
        assert!(scan(st.conn(), cwd).unwrap().is_empty());
        assert_eq!(crate::skills::discover(cwd).len(), 1);
        // Accepting twice must not overwrite.
        assert!(accept(cwd, &name).is_err());
    }

    #[test]
    fn a_rejected_recipe_stays_rejected() {
        let st = Store::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path();
        let cwd_s = cwd.to_string_lossy().to_string();
        let run = [("execute: git status", ""), ("execute: cargo test", "")];
        for _ in 0..3 {
            seed_session(&st, &cwd_s, &run, "completed");
        }
        let new = scan(st.conn(), cwd).unwrap();
        assert_eq!(new.len(), 1);
        reject(cwd, &new[0].name).unwrap();
        assert!(list(cwd).is_empty());
        assert!(scan(st.conn(), cwd).unwrap().is_empty());
    }

    #[test]
    fn draft_names_cannot_escape_the_queue() {
        let dir = tempfile::tempdir().unwrap();
        assert!(accept(dir.path(), "../../etc").is_err());
        assert!(reject(dir.path(), "a/b").is_err());
        assert!(accept(dir.path(), "").is_err());
    }
}
