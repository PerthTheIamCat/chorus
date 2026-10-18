//! New sessions start as drafts: nothing is written until the first message.
//!
//! "New session" used to insert a `session` row on the spot, so every ✎ that
//! was never followed by a message left an empty row in the sidebar (one
//! workspace had 55 of them, 18 Oct 2026). A draft lives only in memory. It
//! becomes a real session when [`DraftBook::commit`] runs, which the shell
//! calls when the first message is sent; until then there is no row, no
//! shadow-repo snapshot and no MCP server.
//!
//! One draft per workspace: asking for a new session where an unsent draft
//! already exists returns that draft instead of stacking another.

use crate::session::Session;
use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub id: u64,
    pub db: PathBuf,
    pub cwd: PathBuf,
}

/// What [`DraftBook::open`] gave back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    pub draft: Draft,
    /// `true` when an unsent draft for this workspace already existed.
    pub reused: bool,
}

#[derive(Debug, Default)]
pub struct DraftBook {
    next: u64,
    drafts: Vec<Draft>,
}

/// Same spelling as `Session::create` stores, so `/tmp/x` and
/// `/private/tmp/x` are one workspace.
fn canonical(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

impl DraftBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// The draft for `cwd` in `db`: the existing one if it was never sent,
    /// else a new one. Touches nothing on disk.
    pub fn open(&mut self, db: &Path, cwd: &Path) -> Opened {
        let cwd = canonical(cwd);
        if let Some(d) = self.drafts.iter().find(|d| d.db == db && d.cwd == cwd) {
            return Opened {
                draft: d.clone(),
                reused: true,
            };
        }
        self.next += 1;
        let draft = Draft {
            id: self.next,
            db: db.to_path_buf(),
            cwd,
        };
        self.drafts.push(draft.clone());
        Opened {
            draft,
            reused: false,
        }
    }

    pub fn get(&self, id: u64) -> Option<&Draft> {
        self.drafts.iter().find(|d| d.id == id)
    }

    /// First message: create the real session. The draft is spent only when
    /// that worked, so a failure (folder gone, disk full) can be retried.
    pub fn commit(&mut self, id: u64, title: &str) -> Result<Session> {
        let draft = self
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no draft {id}"))?;
        let session = Session::create(&draft.db, &draft.cwd, title)?;
        self.drafts.retain(|d| d.id != id);
        Ok(session)
    }

    /// Forget a draft (its workspace was removed). Nothing to clean up.
    pub fn discard(&mut self, id: u64) {
        self.drafts.retain(|d| d.id != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plus_twice_in_one_workspace_is_one_draft() {
        let mut book = DraftBook::new();
        let db = Path::new("/nonexistent/a/.chorus/store.db");
        let a = book.open(db, Path::new("/nonexistent/a"));
        let again = book.open(db, Path::new("/nonexistent/a"));
        assert!(!a.reused);
        assert!(again.reused);
        assert_eq!(a.draft, again.draft);

        let b = book.open(
            Path::new("/nonexistent/b/.chorus/store.db"),
            Path::new("/nonexistent/b"),
        );
        assert!(!b.reused);
        assert_ne!(a.draft.id, b.draft.id);
        assert!(!db.exists(), "a draft must not create anything on disk");
    }
}
