//! Project notes — a few lines about how this project is worked on, written
//! at idle by a local model and kept in the warm tier.
//!
//! **Off by default.** Nothing here runs unless `CHORUS_SUMMARY_MODEL` names an
//! Ollama model, the same switch that turns on model-written compaction. With
//! it off, no note is generated, stored or sent to an agent.
//!
//! What goes in is facts the ledger already holds — which test and build
//! commands ran and passed, which command keeps failing, which files get
//! edited most — never raw conversation. The local model only phrases them.
//! What comes out is masked like every other path into a context (7.28) and
//! capped. Notes live in their own table, not the ledger (2.1), and
//! [`for_context`] only *reads* it, so `assemble_context` still makes
//! nothing (7.2b). Whether the notes earn their tokens is for the live
//! handoff eval to say (R4); until then they are opt-in.

use crate::outcomes::is_test_command;
use crate::skill_drafts::normalize_step;
use crate::types::EventKind;
use crate::{ledger, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;

/// Longest note stored and shown, in characters.
pub const MAX_CHARS: usize = 1_200;
/// A rewrite is attempted at most this often (ms), however busy the ledger.
const MIN_INTERVAL_MS: i64 = 30 * 60 * 1000;
/// Sessions read per pass.
const MAX_SESSIONS: usize = 100;

/// The Ollama model named by `CHORUS_SUMMARY_MODEL`, if any.
pub fn model_from_env() -> Option<String> {
    std::env::var("CHORUS_SUMMARY_MODEL")
        .ok()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
}

fn init(c: &Connection) -> Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS project_notes (
            cwd        TEXT PRIMARY KEY,
            notes      TEXT NOT NULL,
            basis      TEXT NOT NULL,
            model      TEXT NOT NULL,
            created_ms INTEGER NOT NULL
        );",
    )?;
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// What the ledger knows about working in a project, as plain counts.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Facts {
    /// Test commands whose result was a pass, most used first.
    pub test_commands: Vec<(String, usize)>,
    /// Other commands that finished fine at least twice.
    pub build_commands: Vec<(String, usize)>,
    /// Commands that failed at least twice.
    pub failing_commands: Vec<(String, usize)>,
    /// Files edited most, relative to the project.
    pub hot_files: Vec<(String, usize)>,
}

impl Facts {
    pub fn is_thin(&self) -> bool {
        self.test_commands.is_empty()
            && self.build_commands.is_empty()
            && self.failing_commands.is_empty()
            && self.hot_files.len() < 2
    }

    /// The text both the model prompt and the change-detection hash use.
    pub fn render(&self) -> String {
        fn block(title: &str, rows: &[(String, usize)]) -> String {
            if rows.is_empty() {
                return String::new();
            }
            let mut s = format!("{title}:\n");
            for (name, n) in rows {
                s.push_str(&format!("- {name} ({n}x)\n"));
            }
            s
        }
        [
            block("Test commands that passed", &self.test_commands),
            block("Other commands that worked", &self.build_commands),
            block("Commands that keep failing", &self.failing_commands),
            block("Most edited files", &self.hot_files),
        ]
        .concat()
    }
}

fn top(map: HashMap<String, usize>, min: usize, k: usize) -> Vec<(String, usize)> {
    let mut v: Vec<(String, usize)> = map.into_iter().filter(|(_, n)| *n >= min).collect();
    // Count, then name: a total order keeps the hash stable between passes.
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.truncate(k);
    v
}

/// Gather [`Facts`] from the project's sessions in this ledger.
pub fn gather(c: &Connection, cwd: &str) -> Result<Facts> {
    let ids: Vec<i64> = {
        let mut stmt = c.prepare(
            "SELECT s.id FROM session s WHERE s.cwd = ?1
             ORDER BY (SELECT COALESCE(MAX(seq), 0) FROM event WHERE sess = s.id) DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![cwd, MAX_SESSIONS as i64], |r| r.get(0))?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let (mut tests, mut worked, mut failed, mut files) = (
        HashMap::<String, usize>::new(),
        HashMap::<String, usize>::new(),
        HashMap::<String, usize>::new(),
        HashMap::<String, usize>::new(),
    );
    let prefix = format!("{}/", cwd.trim_end_matches('/'));
    for sess in ids {
        for e in ledger::history(c, sess, 1_000_000)?.iter().filter(|e| e.sess == sess) {
            match e.kind {
                EventKind::ToolResult => {
                    let body = ledger::body(c, e)?;
                    if body.starts_with('{') {
                        continue;
                    }
                    let Some((title, status)) = body.rsplit_once(" → ") else { continue };
                    let Some(step) = normalize_step(&format!("execute: {}", title.trim())) else {
                        continue;
                    };
                    let ok = matches!(status.trim(), "completed" | "success" | "ok");
                    match (ok, is_test_command(&step)) {
                        (true, true) => *tests.entry(step).or_default() += 1,
                        (true, false) => *worked.entry(step).or_default() += 1,
                        (false, _) => *failed.entry(step).or_default() += 1,
                    }
                }
                EventKind::Diff => {
                    let body = ledger::body(c, e)?;
                    let path = body
                        .lines()
                        .next()
                        .and_then(|l| l.strip_prefix("--- "))
                        .unwrap_or("")
                        .trim();
                    let rel = path.strip_prefix(&prefix).unwrap_or(path);
                    let projected = matches!(rel, "CLAUDE.md" | "AGENTS.md" | "GEMINI.md")
                        || rel.starts_with(".claude/")
                        || rel.starts_with(".agents/")
                        || rel.starts_with(".chorus/");
                    if !rel.is_empty() && !rel.starts_with('/') && !projected {
                        *files.entry(rel.to_string()).or_default() += 1;
                    }
                }
                _ => {}
            }
        }
    }
    Ok(Facts {
        test_commands: top(tests, 1, 3),
        build_commands: top(worked, 2, 5),
        failing_commands: top(failed, 2, 3),
        hot_files: top(files, 2, 8),
    })
}

/// The prompt a local model rewrites [`Facts`] with. Refuses to invent: the
/// facts are the only source it is allowed.
pub fn prompt_for(facts: &Facts) -> String {
    format!(
        "You write short working notes for a coding agent that is new to this project.\n\
         Use ONLY the facts below. Do not guess, do not add advice, do not repeat the facts \
         verbatim as a list of counts. At most 8 short lines of plain text: how to build, how \
         to run the tests, what tends to fail, which files change most. Never include secrets \
         or absolute home paths.\n\nFacts:\n{}",
        facts.render()
    )
}

fn clamp(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() <= MAX_CHARS {
        return t.to_string();
    }
    let cut: String = t.chars().take(MAX_CHARS).collect();
    format!("{}…", cut.trim_end())
}

/// What a pass did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refresh {
    /// A new note was stored.
    Written,
    /// Facts unchanged since the stored note, or it is too recent to redo.
    Fresh,
    /// Not enough history to say anything.
    Thin,
    /// The model gave nothing (down, not pulled, timed out); old note kept.
    NoModel,
}

/// The testable half of [`refresh`]: the writer and the masker are injected.
pub fn refresh_with(
    c: &Connection,
    cwd: &str,
    model: &str,
    generate: &dyn Fn(&str) -> Option<String>,
    mask: &dyn Fn(&str) -> String,
) -> Result<Refresh> {
    init(c)?;
    let facts = gather(c, cwd)?;
    if facts.is_thin() {
        return Ok(Refresh::Thin);
    }
    let basis = blake3::hash(facts.render().as_bytes()).to_hex().to_string();
    let stored: Option<(String, i64)> = c
        .query_row(
            "SELECT basis, created_ms FROM project_notes WHERE cwd = ?1",
            [cwd],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((old_basis, at)) = &stored {
        if *old_basis == basis || now_ms() - at < MIN_INTERVAL_MS {
            return Ok(Refresh::Fresh);
        }
    }
    let Some(text) = generate(&prompt_for(&facts)).filter(|t| !t.trim().is_empty()) else {
        return Ok(Refresh::NoModel);
    };
    let notes = clamp(&mask(&text));
    c.execute(
        "INSERT INTO project_notes (cwd, notes, basis, model, created_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(cwd) DO UPDATE SET notes = excluded.notes, basis = excluded.basis,
            model = excluded.model, created_ms = excluded.created_ms",
        params![cwd, notes, basis, model, now_ms()],
    )?;
    Ok(Refresh::Written)
}

/// Rewrite the project's notes with the Ollama model from the environment.
/// `None` when no model is configured — the feature is off.
pub fn refresh(c: &Connection, cwd: &str) -> Result<Option<Refresh>> {
    let Some(model) = model_from_env() else {
        return Ok(None);
    };
    let ollama = crate::compaction::Ollama::new(model.clone());
    let r = refresh_with(
        c,
        cwd,
        &format!("ollama:{model}"),
        &|p| ollama.generate(p),
        &|t| crate::secrets::user_vault().mask(t).text,
    )?;
    Ok(Some(r))
}

/// The stored note for a project, for a context. Read-only; returns `None`
/// unless the feature is switched on, so turning it off removes the notes
/// from every prompt at once without deleting anything.
pub fn for_context(c: &Connection, cwd: &str) -> Option<String> {
    model_from_env()?;
    load(c, cwd)
}

/// The stored note regardless of the switch (for the Learn screen).
pub fn load(c: &Connection, cwd: &str) -> Option<String> {
    init(c).ok()?;
    c.query_row("SELECT notes FROM project_notes WHERE cwd = ?1", [cwd], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
        .filter(|n: &String| !n.trim().is_empty())
}

/// Forget the stored note for a project.
pub fn clear(c: &Connection, cwd: &str) -> Result<()> {
    init(c)?;
    c.execute("DELETE FROM project_notes WHERE cwd = ?1", [cwd])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NewEvent;
    use crate::{actor, Store};
    use std::cell::Cell;

    fn seed(st: &Store, cwd: &str) {
        let c = st.conn();
        let sess = ledger::new_session(c, "t", cwd).unwrap();
        let a = actor::intern(c, "codex", "codex").unwrap();
        for _ in 0..2 {
            for (body, kind) in [
                ("cargo test -p x → completed", EventKind::ToolResult),
                ("cargo build → completed", EventKind::ToolResult),
                ("cargo clippy → failed", EventKind::ToolResult),
                (&format!("--- {cwd}/src/a.rs\n+++ x\n+ y"), EventKind::Diff),
                (&format!("--- {cwd}/CLAUDE.md\n+++ x\n+ y"), EventKind::Diff),
            ] {
                ledger::append(c, &NewEvent::new(sess, kind, a, body)).unwrap();
            }
        }
    }

    #[test]
    fn gathers_facts_without_raw_conversation() {
        let st = Store::open_in_memory().unwrap();
        seed(&st, "/p");
        let f = gather(st.conn(), "/p").unwrap();
        assert_eq!(f.test_commands, vec![("cargo test".to_string(), 2)]);
        assert_eq!(f.build_commands, vec![("cargo build".to_string(), 2)]);
        assert_eq!(f.failing_commands, vec![("cargo clippy".to_string(), 2)]);
        // Files Chorus writes itself are not "hot files".
        assert_eq!(f.hot_files, vec![("src/a.rs".to_string(), 2)]);
        let prompt = prompt_for(&f);
        assert!(prompt.contains("cargo clippy") && prompt.contains("src/a.rs"));
        assert!(!prompt.contains("CLAUDE.md"));
    }

    #[test]
    fn thin_history_writes_nothing_and_never_calls_the_model() {
        let st = Store::open_in_memory().unwrap();
        let called = Cell::new(false);
        let r = refresh_with(
            st.conn(),
            "/p",
            "m",
            &|_| {
                called.set(true);
                Some("x".into())
            },
            &|t| t.to_string(),
        )
        .unwrap();
        assert_eq!(r, Refresh::Thin);
        assert!(!called.get());
    }

    #[test]
    fn writes_masks_caps_and_skips_unchanged_facts() {
        let st = Store::open_in_memory().unwrap();
        seed(&st, "/p");
        let calls = Cell::new(0);
        let gen = |_: &str| {
            calls.set(calls.get() + 1);
            Some(format!("run cargo test. token=SECRET {}", "x".repeat(3_000)))
        };
        let mask = |t: &str| t.replace("SECRET", "$CHORUS_SECRET_TOKEN_1");

        assert_eq!(refresh_with(st.conn(), "/p", "m", &gen, &mask).unwrap(), Refresh::Written);
        let note = load(st.conn(), "/p").unwrap();
        assert!(note.contains("$CHORUS_SECRET_TOKEN_1") && !note.contains("SECRET "));
        assert!(note.chars().count() <= MAX_CHARS + 1);

        // Same facts: no second model call.
        assert_eq!(refresh_with(st.conn(), "/p", "m", &gen, &mask).unwrap(), Refresh::Fresh);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn a_silent_model_keeps_the_old_note() {
        let st = Store::open_in_memory().unwrap();
        seed(&st, "/p");
        let id = |t: &str| t.to_string();
        refresh_with(st.conn(), "/p", "m", &|_| Some("first".into()), &id).unwrap();
        // Facts change, but the stored note is recent → still fresh.
        seed(&st, "/p");
        assert_eq!(
            refresh_with(st.conn(), "/p", "m", &|_| None, &id).unwrap(),
            Refresh::Fresh
        );
        assert_eq!(load(st.conn(), "/p").as_deref(), Some("first"));
        clear(st.conn(), "/p").unwrap();
        assert!(load(st.conn(), "/p").is_none());

        // With no stored note and no model, nothing is written.
        assert_eq!(refresh_with(st.conn(), "/p", "m", &|_| None, &id).unwrap(), Refresh::NoModel);
    }

    #[test]
    fn context_is_empty_unless_the_model_switch_is_on() {
        // CHORUS_SUMMARY_MODEL is unset in the test environment.
        if model_from_env().is_some() {
            return;
        }
        let st = Store::open_in_memory().unwrap();
        seed(&st, "/p");
        refresh_with(st.conn(), "/p", "m", &|_| Some("note".into()), &|t| t.to_string()).unwrap();
        assert!(load(st.conn(), "/p").is_some());
        assert!(for_context(st.conn(), "/p").is_none());
    }
}
