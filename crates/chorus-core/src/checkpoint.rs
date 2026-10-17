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
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Excludes for Chorus-projected files (hidden by default unless toggle is on).
pub const PROJECTED_EXCLUDES: &[&str] = &[
    ":(exclude,top)CLAUDE.md",
    ":(exclude,top)AGENTS.md",
    ":(exclude,top)GEMINI.md",
    ":(exclude,top).claude/skills",
    ":(exclude,top).claude/skills/**",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileDiffStat {
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
    pub is_binary: bool,
    pub is_lockfile: bool,
    pub is_projected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffSummaryReport {
    pub files: Vec<FileDiffStat>,
    pub total_additions: usize,
    pub total_deletions: usize,
    pub total_files: usize,
    pub is_collapsed: bool,
}

pub fn is_lockfile(path: &str) -> bool {
    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path);
    matches!(
        filename,
        "Cargo.lock"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "bun.lockb"
            | "Package.resolved"
            | "poetry.lock"
            | "composer.lock"
            | "Gemfile.lock"
    )
}

pub fn is_projected_file(path: &str) -> bool {
    let p = path.trim_start_matches("./");
    p == "CLAUDE.md"
        || p == "AGENTS.md"
        || p == "GEMINI.md"
        || p.starts_with(".claude/skills/")
        || p == ".claude/skills"
}

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
    // Framework/tool caches that are huge and never the agent's work. Every
    // file here is hashed and copied on the first snapshot, which is what made
    // "start session" take minutes on a web project.
    ".next/",
    ".nuxt/",
    ".svelte-kit/",
    ".turbo/",
    ".parcel-cache/",
    ".cache/",
    ".gradle/",
    ".build/",
    "Pods/",
    "DerivedData/",
    "coverage/",
    ".pytest_cache/",
    ".mypy_cache/",
    ".idea/",
    "*.log",
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
            // Every turn runs `git add -A` over the whole tree. These make the
            // repeat runs stat-only instead of rescanning untracked dirs, and
            // stop git from spending turn time on housekeeping.
            this.git(&["config", "core.untrackedCache", "true"])?;
            this.git(&["config", "core.preloadindex", "true"])?;
            this.git(&["config", "gc.auto", "0"])?;
            this.git(&["config", "core.autocrlf", "false"])?;
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

    /// [`diff_since`] minus projected files unless `include_projected` is true.
    pub fn diff_since_user(&self, since: &str) -> Result<String> {
        self.diff_since_user_opts(since, false)
    }

    pub fn diff_since_user_opts(&self, since: &str, include_projected: bool) -> Result<String> {
        self.git(&["add", "-A"])?;
        let base = if since.trim().is_empty() {
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string()
        } else {
            since.trim().to_string()
        };
        let mut args = vec!["diff", "--staged", &base, "--", "."];
        if !include_projected {
            args.extend_from_slice(PROJECTED_EXCLUDES);
        }
        self.git(&args)
    }

    /// [`diff_since_user`] without writing the shared index.
    pub fn peek_diff_since_user(&self, since: &str) -> Result<String> {
        self.peek_diff_since_user_opts(since, false)
    }

    pub fn peek_diff_since_user_opts(
        &self,
        since: &str,
        include_projected: bool,
    ) -> Result<String> {
        let private = self.git_dir.join("index.peek");
        let _ = std::fs::copy(self.git_dir.join("index"), &private);
        let index = Some(private.as_path());
        self.git_with(&["add", "-A"], index)?;
        let base = if since.trim().is_empty() {
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string()
        } else {
            since.trim().to_string()
        };
        let mut args = vec!["diff", "--staged", &base, "--", "."];
        if !include_projected {
            args.extend_from_slice(PROJECTED_EXCLUDES);
        }
        self.git_with(&args, index)
    }

    /// Single-file diff against `since`.
    pub fn file_diff(&self, since: &str, path: &str) -> Result<String> {
        self.git(&["add", "-A"])?;
        let base = if since.trim().is_empty() {
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string()
        } else {
            since.trim().to_string()
        };
        self.git(&["diff", "--staged", &base, "--", path])
    }

    pub fn peek_file_diff(&self, since: &str, path: &str) -> Result<String> {
        let private = self.git_dir.join("index.peek");
        let _ = std::fs::copy(self.git_dir.join("index"), &private);
        let index = Some(private.as_path());
        self.git_with(&["add", "-A"], index)?;
        let base = if since.trim().is_empty() {
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string()
        } else {
            since.trim().to_string()
        };
        self.git_with(&["diff", "--staged", &base, "--", path], index)
    }

    /// Compute file-level stats via `git diff --numstat` fast path.
    pub fn diff_file_stats(
        &self,
        since: &str,
        include_projected: bool,
    ) -> Result<DiffSummaryReport> {
        self.git(&["add", "-A"])?;
        let base = if since.trim().is_empty() {
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string()
        } else {
            since.trim().to_string()
        };
        let mut args = vec!["diff", "--numstat", "--staged", &base, "--", "."];
        if !include_projected {
            args.extend_from_slice(PROJECTED_EXCLUDES);
        }
        let out = self.git(&args)?;
        Ok(Self::parse_numstat(&out))
    }

    pub fn peek_diff_file_stats(
        &self,
        since: &str,
        include_projected: bool,
    ) -> Result<DiffSummaryReport> {
        let private = self.git_dir.join("index.peek");
        let _ = std::fs::copy(self.git_dir.join("index"), &private);
        let index = Some(private.as_path());
        self.git_with(&["add", "-A"], index)?;
        let base = if since.trim().is_empty() {
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string()
        } else {
            since.trim().to_string()
        };
        let mut args = vec!["diff", "--numstat", "--staged", &base, "--", "."];
        if !include_projected {
            args.extend_from_slice(PROJECTED_EXCLUDES);
        }
        let out = self.git_with(&args, index)?;
        Ok(Self::parse_numstat(&out))
    }

    fn parse_numstat(numstat_output: &str) -> DiffSummaryReport {
        let mut files = Vec::new();
        let mut total_additions = 0;
        let mut total_deletions = 0;

        for line in numstat_output.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 3 {
                continue;
            }
            let add_str = parts[0];
            let del_str = parts[1];
            let path = parts[2].trim().to_string();

            let is_binary = add_str == "-" || del_str == "-";
            let additions = if is_binary {
                0
            } else {
                add_str.parse::<usize>().unwrap_or(0)
            };
            let deletions = if is_binary {
                0
            } else {
                del_str.parse::<usize>().unwrap_or(0)
            };

            total_additions += additions;
            total_deletions += deletions;

            let lock = is_lockfile(&path);
            let proj = is_projected_file(&path);

            files.push(FileDiffStat {
                path,
                additions,
                deletions,
                is_binary,
                is_lockfile: lock,
                is_projected: proj,
            });
        }

        let total_files = files.len();
        let total_lines = total_additions + total_deletions;
        let is_collapsed = total_files > 300 || total_lines > 5000;

        DiffSummaryReport {
            files,
            total_additions,
            total_deletions,
            total_files,
            is_collapsed,
        }
    }

    /// Which files changed since `since`, without staging anything.
    ///
    /// [`diff_since`] runs `git add -A` so new files show up in the patch. That
    /// is right for a patch and wrong for a question asked on the read path:
    /// assembling context happens on every turn, and mutating the index to
    /// answer "what changed" makes a cheap question expensive on a large repo.
    /// Two plumbing calls answer it without writing anything.
    pub fn files_since(&self, since: &str) -> Result<Vec<String>> {
        let base = if since.trim().is_empty() {
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_string()
        } else {
            since.trim().to_string()
        };
        let mut out: Vec<String> = self
            .git(&["diff", "--name-only", &base, "--"])?
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();
        for l in self
            .git(&["ls-files", "--others", "--exclude-standard"])?
            .lines()
        {
            let l = l.trim();
            if !l.is_empty() && !out.iter().any(|o| o == l) {
                out.push(l.to_string());
            }
        }
        Ok(out)
    }

    /// Roll the work tree back to `sha`. Destructive — the caller confirms.
    pub fn restore(&self, sha: &str) -> Result<()> {
        self.git(&["reset", "--hard", "--quiet", sha])?;
        self.git(&["clean", "-fd", "--quiet"])?;
        Ok(())
    }

    /// Reverse-apply `patch` to the work tree — undo exactly the hunks it
    /// contains. Used by per-hunk diff review to reject a selection without
    /// touching the rest. The patch must be a valid unified diff.
    pub fn apply_reverse(&self, patch: &str) -> Result<()> {
        let mut child = Command::new("git")
            .arg("--git-dir")
            .arg(&self.git_dir)
            .arg("--work-tree")
            .arg(&self.work_tree)
            .args(["apply", "--reverse", "--recount", "-"])
            .current_dir(&self.work_tree)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| ChorusError::Invalid(format!("run git apply: {e}")))?;
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(patch.as_bytes())
            .map_err(ChorusError::Io)?;
        let out = child.wait_with_output().map_err(ChorusError::Io)?;
        if !out.status.success() {
            return Err(ChorusError::Invalid(format!(
                "git apply --reverse: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(())
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        self.git_with(args, None)
    }

    /// `index`: a private index file instead of the repo's own.
    fn git_with(&self, args: &[&str], index: Option<&Path>) -> Result<String> {
        let mut cmd = Command::new("git");
        cmd.arg("--git-dir")
            .arg(&self.git_dir)
            .arg("--work-tree")
            .arg(&self.work_tree)
            .args(args);
        if let Some(index) = index {
            cmd.env("GIT_INDEX_FILE", index);
        }
        let out = cmd
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
