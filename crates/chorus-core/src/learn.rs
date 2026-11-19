//! The Learn screen's rules: how much the app may learn on its own, and the
//! idle pass that does the learning.
//!
//! Swift draws the screen; every rule lives here (CLAUDE.md 2.3).
//!
//! Four autonomy modes, one of them the default:
//!
//! - [`Autonomy::Auto`] — *default.* Lessons and agent-proposed facts are
//!   accepted without asking. Anything that might carry planted instructions
//!   (an imported session, web content) is still queued.
//! - [`Autonomy::Ask`] — everything learned waits in a queue for a person to
//!   accept or reject.
//! - [`Autonomy::AutoWithTests`] — a lesson whose evidence includes a passing
//!   test run is accepted without asking. Anything that might carry planted
//!   instructions (an imported session, web content) is still queued.
//! - [`Autonomy::Off`] — nothing is drafted or queued.
//!
//! **Skill drafts ignore the mode**: they are always queued for review, even
//! in `AutoWithTests`. A skill is instructions every agent will follow, and
//! unreviewed self-written skills do not help (SkillsBench, arXiv 2602.12670).
//! The router's use of measured outcomes is a separate switch
//! (`CHORUS_ROUTER_OUTCOMES=off`).
//!
//! The idle pass runs off the turn path, on its own connection, so it never
//! holds the engine queue (7.7).

use crate::{outcomes, project_notes, skill_drafts, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Autonomy {
    /// *Default.* Lessons and facts an agent proposes are accepted without a
    /// queue; anything that may carry planted instructions still waits.
    #[default]
    Auto,
    Ask,
    AutoWithTests,
    Off,
}

impl Autonomy {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Self::Auto),
            "ask" => Some(Self::Ask),
            "auto_with_tests" => Some(Self::AutoWithTests),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Ask => "ask",
            Self::AutoWithTests => "auto_with_tests",
            Self::Off => "off",
        }
    }
}

/// What a lesson rests on.
#[derive(Debug, Clone, Copy, Default)]
pub struct LessonEvidence {
    /// A test command passed after the thing the lesson is about.
    pub tests_passed: bool,
    /// The lesson came from content an attacker could have written: an
    /// imported session or something read from the web.
    pub tainted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Put it in the review queue.
    Queue,
    /// Accept it without asking.
    AutoAccept,
    /// Do not keep it at all.
    Drop,
}

/// What to do with a freshly derived lesson under `mode`.
pub fn decide(mode: Autonomy, ev: &LessonEvidence) -> Decision {
    match mode {
        Autonomy::Off => Decision::Drop,
        Autonomy::Auto if !ev.tainted => Decision::AutoAccept,
        Autonomy::Auto => Decision::Queue,
        Autonomy::Ask => Decision::Queue,
        Autonomy::AutoWithTests if ev.tests_passed && !ev.tainted => Decision::AutoAccept,
        Autonomy::AutoWithTests => Decision::Queue,
    }
}

/// Persisted Learn settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub autonomy: Autonomy,
}

/// `~/.chorus/learn.json`.
pub fn settings_path() -> Option<PathBuf> {
    // Tests (and anyone who wants a different file) can point elsewhere.
    if let Some(p) = std::env::var_os("CHORUS_LEARN_FILE") {
        return Some(PathBuf::from(p));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".chorus").join("learn.json"))
}

/// Read the settings; a missing or unreadable file means the defaults, which
/// ask before learning anything.
pub fn load_settings(path: &Path) -> Settings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_settings(path: &Path, s: &Settings) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(s)?)?;
    Ok(())
}

/// What one idle pass did.
#[derive(Debug, Default, Clone, Serialize)]
pub struct IdleReport {
    /// Turns scored for the router.
    pub outcome_turns: usize,
    /// Names of skill drafts newly queued.
    pub new_drafts: Vec<String>,
    /// `written` / `fresh` / `thin` / `no_model` / `off`.
    pub notes: String,
    /// The pass stopped early on its time budget.
    pub hit_budget: bool,
}

/// Sessions scored per pass; a long history catches up over several passes.
const OUTCOME_SESSIONS_PER_PASS: usize = 40;

/// Run the learning that belongs at idle: score recent turns for the router,
/// look for recipes worth a skill draft, refresh the project notes.
///
/// `ledger` is the project's own database; `router_store` is the global store
/// the router reads (they may be the same connection in tests). Each step is
/// optional work — one failing leaves the others untouched.
pub fn idle_pass(
    ledger: &Connection,
    router_store: &Connection,
    src: &str,
    cwd: &Path,
    settings: &Settings,
    budget: Duration,
) -> IdleReport {
    let started = Instant::now();
    let mut report = IdleReport { notes: "off".into(), ..Default::default() };

    match outcomes::refresh(ledger, router_store, src, OUTCOME_SESSIONS_PER_PASS) {
        Ok(n) => report.outcome_turns = n,
        Err(e) => eprintln!("chorus: learn: outcomes: {e}"),
    }

    if settings.autonomy != Autonomy::Off && started.elapsed() < budget {
        match skill_drafts::scan(ledger, cwd) {
            Ok(new) => report.new_drafts = new.into_iter().map(|d| d.name).collect(),
            Err(e) => eprintln!("chorus: learn: skill drafts: {e}"),
        }
    }

    if started.elapsed() < budget {
        match project_notes::refresh(ledger, &cwd.to_string_lossy()) {
            Ok(Some(r)) => {
                report.notes = match r {
                    project_notes::Refresh::Written => "written",
                    project_notes::Refresh::Fresh => "fresh",
                    project_notes::Refresh::Thin => "thin",
                    project_notes::Refresh::NoModel => "no_model",
                }
                .into()
            }
            Ok(None) => {}
            Err(e) => eprintln!("chorus: learn: project notes: {e}"),
        }
    } else {
        report.hit_budget = true;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    #[test]
    fn auto_is_the_default() {
        assert_eq!(Settings::default().autonomy, Autonomy::Auto);
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_settings(&dir.path().join("none.json")).autonomy, Autonomy::Auto);
        std::fs::write(dir.path().join("bad.json"), "{nope").unwrap();
        assert_eq!(load_settings(&dir.path().join("bad.json")).autonomy, Autonomy::Auto);
    }

    #[test]
    fn settings_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a/b/learn.json");
        save_settings(&p, &Settings { autonomy: Autonomy::AutoWithTests }).unwrap();
        assert_eq!(load_settings(&p).autonomy, Autonomy::AutoWithTests);
        for m in [Autonomy::Auto, Autonomy::Ask, Autonomy::AutoWithTests, Autonomy::Off] {
            assert_eq!(Autonomy::parse(m.as_str()), Some(m));
        }
        assert_eq!(Autonomy::parse("yolo"), None);
    }

    #[test]
    fn autonomy_rules() {
        let proven = LessonEvidence { tests_passed: true, tainted: false };
        let unproven = LessonEvidence { tests_passed: false, tainted: false };
        let planted = LessonEvidence { tests_passed: true, tainted: true };
        assert_eq!(decide(Autonomy::Ask, &proven), Decision::Queue);
        assert_eq!(decide(Autonomy::Auto, &unproven), Decision::AutoAccept);
        assert_eq!(decide(Autonomy::Auto, &planted), Decision::Queue);
        assert_eq!(decide(Autonomy::AutoWithTests, &proven), Decision::AutoAccept);
        assert_eq!(decide(Autonomy::AutoWithTests, &unproven), Decision::Queue);
        // Imported or web-derived lessons are never accepted unseen.
        assert_eq!(decide(Autonomy::AutoWithTests, &planted), Decision::Queue);
        assert_eq!(decide(Autonomy::Off, &proven), Decision::Drop);
    }

    #[test]
    fn off_stops_drafting_but_not_the_router_scoring() {
        let st = Store::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let off = Settings { autonomy: Autonomy::Off };
        let r = idle_pass(st.conn(), st.conn(), "t", dir.path(), &off, Duration::from_secs(5));
        assert!(r.new_drafts.is_empty());
        assert!(!dir.path().join(skill_drafts::DRAFTS_DIR).exists());
        assert_eq!(r.notes, "off");
    }
}
