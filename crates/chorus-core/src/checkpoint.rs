//! Checkpoints: a shadow git repo that snapshots the workspace so a handoff
//! brief can carry a real "diff since checkpoint" and a turn can be rolled
//! back.
//!
//! Design §4 / trap §7.3: the shadow repo lives at `<cwd>/.chorus/ckpt`
//! with its own `GIT_DIR`, so it never touches the user's own `.git`. It
//! honours the user's `.gitignore` (git does this for us) plus our own
//! exclude list, or `.chorus/` would end up larger than the project.
//!
//! We shell out to `git` rather than link libgit2 — no new dependency, and
//! `git` is already required for the rest of the workflow. `Checkpoint`
//! methods return an error (not a panic) when `git` is missing.

use crate::{ChorusError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Patterns the shadow repo always ignores, on top of the user's
/// `.gitignore`. Covers our own state and the usual build/cache noise so a
/// checkpoint diff is about the agent's work, not artifacts.
const ALWAYS_EXCLUDE: &[&str] = &[
    ".chorus/",
    ".git/",
    "target/",
    "node_modules/",
    ".DS_Store",
    "__pycache__/",
    "*.pyc",
    ".venv/",
    "dist/",
    "build/",
    // The ledger store, in case a caller keeps it inside the workspace.
    "*.db",
    "*.db-wal",
    "*.db-shm",
    "*.sqlite",
    "*.sqlite3",
];

/// A shadow repository over one workspace directory.
pub struct Checkpoint {
    work_tree: PathBuf,
    git_dir: PathBuf,
}

impl Checkpoint {
    /// Open the shadow repo for `cwd`, creating and initialising it on first
    /// use. Idempotent. Returns whether this call created it — a caller that
    /// wants a session base sha should take a `snapshot` only when it did.
    pub fn open(cwd: &Path) -> Result<Self> {
        Self::open_reporting(cwd).map(|(c, _)| c)
    }

    /// Like [`open`], plus `true` when the repo was created by this call.
    pub fn open_reporting(cwd: &Path) -> Result<(Self, bool)> {
        let work_tree = cwd.canonicalize().map_err(ChorusError::Io)?;
        let git_dir = work_tree.join(".chorus").join("ckpt");
        let this = Self { work_tree, git_dir };

        let fresh = !this.git_dir.join("HEAD").exists();
        if fresh {
            std::fs::create_dir_all(&this.git_dir).map_err(ChorusError::Io)?;
            this.git(&["init", "--quiet"])?;
            this.git(&["config", "user.email", "checkpoint@chorus.local"])?;
            this.git(&["config", "user.name", "Chorus Checkpoint"])?;
            // Our exclude list goes in the shadow repo's private exclude
            // file, so it never appears in the user's tree.
            let exclude = this.git_dir.join("info").join("exclude");
            std::fs::create_dir_all(exclude.parent().unwrap()).map_err(ChorusError::Io)?;
            std::fs::write(&exclude, format!("{}\n", ALWAYS_EXCLUDE.join("\n")))
                .map_err(ChorusError::Io)?;
        }
        Ok((this, fresh))
    }

    /// Stage the whole work tree and commit. Returns the commit sha, or the
    /// existing `HEAD` sha when there was nothing to commit.
    pub fn snapshot(&self, message: &str) -> Result<String> {
        self.git(&["add", "-A"])?;
        // `commit` fails when the tree is clean; that's fine — return HEAD.
        let committed = self
            .git(&["commit", "--quiet", "--allow-empty-message", "-m", message])
            .is_ok();
        let _ = committed;
        self.head()
    }

    /// The current `HEAD` sha, or `None` before the first snapshot.
    pub fn head(&self) -> Result<String> {
        match self.git(&["rev-parse", "HEAD"]) {
            Ok(out) => Ok(out.trim().to_string()),
            Err(_) => Ok(String::new()),
        }
    }

    /// Unified diff of the current work tree against `since` (a sha from an
    /// earlier `snapshot`). Empty `since` diffs against the empty tree.
    pub fn diff_since(&self, since: &str) -> Result<String> {
        // Stage first so new files show up in the diff.
        self.git(&["add", "-A"])?;
        let base = if since.trim().is_empty() {
            // The empty-tree object — diff shows every tracked file as added.
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string()
        } else {
            since.trim().to_string()
        };
        self.git(&["diff", "--staged", &base, "--"])
    }

    /// Roll the work tree back to `sha`. Destructive — the caller confirms.
    pub fn restore(&self, sha: &str) -> Result<()> {
        self.git(&["reset", "--hard", "--quiet", sha])?;
        self.git(&["clean", "-fd", "--quiet"])?;
        Ok(())
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        let out = Command::new("git")
            .arg("--git-dir")
            .arg(&self.git_dir)
            .arg("--work-tree")
            .arg(&self.work_tree)
            .args(args)
            .output()
            .map_err(|e| ChorusError::Invalid(format!("run git: {e}")))?;
        if !out.status.success() {
            return Err(ChorusError::Invalid(format!(
                "git {}: {}",
                args.first().copied().unwrap_or(""),
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}
