//! How often the app may interrupt the person with a question.
//!
//! Three knobs, all defaulting to "fewer questions":
//!
//! - [`ChildQuestions::LeadFirst`] — *default.* An agent working under an
//!   orchestrator does not get `chorus_ask_user`; it asks its lead with
//!   `chorus_ask_lead`, and the lead decides.
//! - [`LeadEscalation::WhenStuck`] — *default.* The lead (and any agent working
//!   alone) is told to settle ordinary ambiguity itself, say what it assumed,
//!   and ask the person only when a wrong guess is costly or hard to undo.
//! - Memory and lessons are accepted without a queue by default: that is the
//!   Learn setting (`learn::Autonomy::Auto`), not this file. Anything that may
//!   carry planted instructions is still queued.
//!
//! Swift draws the screen; every rule lives here (CLAUDE.md 2.3).

use crate::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildQuestions {
    /// Children ask their lead; only the lead may escalate to the person.
    #[default]
    LeadFirst,
    /// Children may ask the person directly (the old behaviour).
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeadEscalation {
    /// Decide, state the assumption, ask only when a wrong guess is costly.
    #[default]
    WhenStuck,
    /// Ask whenever something is unclear (the old behaviour).
    Often,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AskPolicy {
    #[serde(default)]
    pub children: ChildQuestions,
    #[serde(default)]
    pub lead: LeadEscalation,
}

impl AskPolicy {
    /// `~/.chorus/ask_policy.json`; `CHORUS_ASK_POLICY_FILE` overrides (tests).
    pub fn path() -> Option<PathBuf> {
        if let Some(p) = std::env::var_os("CHORUS_ASK_POLICY_FILE") {
            return Some(PathBuf::from(p));
        }
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".chorus").join("ask_policy.json"))
    }

    /// A missing or unreadable file means the defaults.
    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn current() -> Self {
        Self::path()
            .map(|p| Self::load_from(&p))
            .unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    /// May an agent with a lead above it call `chorus_ask_user`?
    pub fn child_may_ask_user(&self) -> bool {
        self.children == ChildQuestions::User
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_quiet_ones() {
        let p = AskPolicy::default();
        assert_eq!(p.children, ChildQuestions::LeadFirst);
        assert_eq!(p.lead, LeadEscalation::WhenStuck);
        assert!(!p.child_may_ask_user());
    }

    #[test]
    fn round_trips_and_tolerates_junk() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("p.json");
        assert_eq!(AskPolicy::load_from(&f), AskPolicy::default());
        let p = AskPolicy {
            children: ChildQuestions::User,
            lead: LeadEscalation::Often,
        };
        p.save_to(&f).unwrap();
        assert_eq!(AskPolicy::load_from(&f), p);
        std::fs::write(&f, "not json").unwrap();
        assert_eq!(AskPolicy::load_from(&f), AskPolicy::default());
        std::fs::write(&f, r#"{"lead":"often"}"#).unwrap();
        assert_eq!(AskPolicy::load_from(&f).lead, LeadEscalation::Often);
    }
}
