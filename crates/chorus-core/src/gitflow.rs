//! One-click commit → push → PR for the changes a user just accepted.
//!
//! Runs the user's own `git` and `gh` (their credentials, their hooks) in the
//! project folder. Nothing here forces, rewrites history or touches other
//! branches: the worst case is a commit the user can undo with `git reset`.
//! Secret-looking files are refused at the staging step rather than warned
//! about afterwards (CLAUDE.md 7.28).

use serde::Serialize;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    /// Two-letter porcelain code, e.g. " M", "??", "A ".
    pub code: String,
    /// Looks like a secret (.env, private key…) — never staged.
    pub blocked: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub branch: String,
    pub default_branch: Option<String>,
    pub on_default: bool,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub has_remote: bool,
    pub gh_ready: bool,
    pub files: Vec<FileChange>,
    /// Changed files left out because the session did not touch them
    /// (see [`status_scoped`]).
    pub hidden: usize,
}

fn git(ws: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(ws)
        .args(args)
        .output()
        .map_err(|e| format!("รัน git ไม่ได้: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = if err.trim().is_empty() { String::from_utf8_lossy(&out.stdout) } else { err };
        Err(msg.trim().to_string())
    }
}

/// Files that must never be committed by a one-click flow.
pub fn is_secret_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let lower = name.to_lowercase();
    if lower.starts_with(".env") {
        return !(lower.ends_with(".example") || lower.ends_with(".sample") || lower.ends_with(".template"));
    }
    ["pem", "key", "p12", "pfx", "jks", "keystore"]
        .iter()
        .any(|e| lower.ends_with(&format!(".{e}")))
        || matches!(lower.as_str(), "id_rsa" | "id_ed25519" | "id_ecdsa" | ".netrc")
}

pub fn status(ws: &Path) -> Result<Status, String> {
    status_scoped(ws, None)
}

/// Like [`status`], but with `scope` only files the session actually changed
/// are listed. Files that were already modified or untracked when the session
/// opened (a `.gitignore`, another tool's project file) stay out of the sheet,
/// so a one-file turn is not offered as "49 files". Out-of-scope files are
/// counted in `hidden`, never silently dropped.
pub fn status_scoped(ws: &Path, scope: Option<&[String]>) -> Result<Status, String> {
    git(ws, &["rev-parse", "--is-inside-work-tree"]).map_err(|_| "โฟลเดอร์นี้ไม่ใช่ git repo".to_string())?;
    let branch = git(ws, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    let default_branch = git(ws, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .ok()
        .and_then(|s| s.strip_prefix("origin/").map(str::to_string))
        .or_else(|| {
            ["main", "master"]
                .iter()
                .find(|b| git(ws, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{b}")]).is_ok())
                .map(|b| b.to_string())
        });
    let upstream = git(ws, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).ok();
    let ahead = upstream
        .as_ref()
        .and_then(|u| git(ws, &["rev-list", "--count", &format!("{u}..HEAD")]).ok())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let has_remote = git(ws, &["remote"]).map(|r| !r.trim().is_empty()).unwrap_or(false);
    let gh_ready = Command::new("gh")
        .args(["auth", "status"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let porcelain = git(ws, &["status", "--porcelain=v1", "-uall"])?;
    let files = porcelain
        .lines()
        .filter(|l| l.len() > 3)
        .map(|l| {
            let path = l[3..].rsplit(" -> ").next().unwrap_or(&l[3..]).trim_matches('"').to_string();
            FileChange { blocked: is_secret_path(&path), code: l[..2].to_string(), path }
        })
        .collect::<Vec<FileChange>>();
    let total = files.len();
    let files: Vec<FileChange> = match scope {
        Some(keep) => files.into_iter().filter(|f| in_scope(&f.path, keep)).collect(),
        None => files,
    };
    let hidden = total - files.len();
    Ok(Status {
        on_default: default_branch.as_deref() == Some(branch.as_str()),
        branch,
        default_branch,
        upstream,
        ahead,
        has_remote,
        gh_ready,
        files,
        hidden,
    })
}

/// `path` is one of `keep`, or lives under a directory entry of it.
fn in_scope(path: &str, keep: &[String]) -> bool {
    keep.iter().any(|k| {
        let k = k.trim_end_matches('/');
        path == k || path.strip_prefix(k).is_some_and(|rest| rest.starts_with('/'))
    })
}

/// A Conventional Commits draft from the changed paths. Deliberately plain —
/// the user edits it in the sheet; it only saves typing the type and scope.
pub fn suggest_message(files: &[FileChange]) -> String {
    suggest_message_with_goal(files, "")
}

/// Same, but the subject comes from what the user asked for (`goal`, the first
/// line of the session's opening request) when there is one — "update 3 files"
/// says nothing a diff does not already. Without a goal the subject names the
/// files, up to three.
pub fn suggest_message_with_goal(files: &[FileChange], goal: &str) -> String {
    let paths: Vec<&str> = files.iter().filter(|f| !f.blocked).map(|f| f.path.as_str()).collect();
    if paths.is_empty() {
        return String::new();
    }
    let all = |pred: &dyn Fn(&str) -> bool| paths.iter().all(|p| pred(p));
    let kind = if all(&|p| p.ends_with(".md") || p.starts_with("docs/")) {
        "docs"
    } else if all(&|p| p.contains("test") || p.contains("spec")) {
        "test"
    } else if all(&|p| p.starts_with(".github/") || p.ends_with(".toml") || p.ends_with(".lock") || p.ends_with(".json")) {
        "chore"
    } else if files.iter().all(|f| f.code.trim() == "??" || f.code.starts_with('A')) {
        "feat"
    } else {
        "fix"
    };
    let scope = {
        // crates/<name>/… → <name>; apps/<name>/… → <name>; else first dir.
        let first = |p: &str| {
            let mut it = p.split('/');
            match (it.next(), it.next(), it.next()) {
                (Some("crates" | "apps"), Some(n), Some(_)) => Some(n.trim_start_matches("chorus-").to_string()),
                (Some(d), Some(_), _) => Some(d.to_string()),
                _ => None,
            }
        };
        let scopes: std::collections::BTreeSet<_> = paths.iter().map(|p| first(p)).collect();
        if scopes.len() == 1 { scopes.into_iter().next().flatten() } else { None }
    };
    let subject = goal_subject(goal).unwrap_or_else(|| {
        let names: Vec<String> = paths
            .iter()
            .map(|p| Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
            .collect();
        if names.len() <= 3 {
            format!("update {}", names.join(", "))
        } else {
            format!("update {}, {} and {} more", names[0], names[1], names.len() - 2)
        }
    });
    match scope {
        Some(s) => format!("{kind}({s}): {subject}"),
        None => format!("{kind}: {subject}"),
    }
}

/// A commit subject from the user's request: first line, no trailing
/// punctuation, at most 60 characters, first letter lower-cased (Conventional
/// Commits). `None` when the request is empty.
fn goal_subject(goal: &str) -> Option<String> {
    let line = goal.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_end_matches(|c: char| ".!?:;,".contains(c) || c.is_whitespace());
    let mut chars = line.chars();
    let first = chars.next()?;
    let mut out: String = first.to_lowercase().chain(chars).collect();
    if out.chars().count() > 60 {
        out = out.chars().take(59).collect::<String>().trim_end().to_string() + "…";
    }
    Some(out)
}

/// Branch name from a commit subject: `feat(term): add tabs` → `feat/term-add-tabs`.
pub fn branch_from_message(msg: &str) -> String {
    let subject = msg.lines().next().unwrap_or("");
    let slug: String = subject
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug: String = slug.chars().take(40).collect();
    format!("chorus/{}", if slug.is_empty() { "changes" } else { slug.trim_end_matches('-') })
}

#[derive(Debug, Serialize)]
pub struct CommitOutcome {
    pub sha: String,
    pub branch: String,
    pub created_branch: bool,
    pub committed: usize,
}

/// Stage `paths` (never secret-looking ones), commit, and — when still on the
/// default branch — move the commit onto a new branch first, so one click
/// never lands on `main`.
pub fn commit(ws: &Path, message: &str, paths: &[String]) -> Result<CommitOutcome, String> {
    if message.trim().is_empty() {
        return Err("ต้องมีข้อความ commit".into());
    }
    let st = status(ws)?;
    let chosen: Vec<&FileChange> = st.files.iter().filter(|f| paths.contains(&f.path)).collect();
    if let Some(b) = chosen.iter().find(|f| f.blocked) {
        return Err(format!("{} ดูเหมือนไฟล์ลับ — ไม่ commit ให้ (เอาออกจากรายการก่อน)", b.path));
    }
    if chosen.is_empty() {
        return Err("ไม่มีไฟล์ที่เลือกให้ commit".into());
    }
    let mut created_branch = false;
    if st.on_default {
        git(ws, &["switch", "-c", &branch_from_message(message)])?;
        created_branch = true;
    }
    // Reset the index first: a previously staged file the user un-ticked must
    // not ride along.
    git(ws, &["reset", "-q"]).ok();
    let mut add = vec!["add", "--"];
    add.extend(chosen.iter().map(|f| f.path.as_str()));
    git(ws, &add)?;
    git(ws, &["commit", "-q", "-m", message])?;
    Ok(CommitOutcome {
        sha: git(ws, &["rev-parse", "--short", "HEAD"])?,
        branch: git(ws, &["rev-parse", "--abbrev-ref", "HEAD"])?,
        created_branch,
        committed: chosen.len(),
    })
}

/// `git push -u origin HEAD` — plain, never forced.
pub fn push(ws: &Path) -> Result<String, String> {
    let remote = if git(ws, &["remote", "get-url", "origin"]).is_ok() {
        "origin".to_string()
    } else {
        git(ws, &["remote"])?.lines().next().map(str::to_string).ok_or("ไม่มี remote")?
    };
    git(ws, &["push", "-u", &remote, "HEAD"])?;
    git(ws, &["rev-parse", "--abbrev-ref", "HEAD"])
}

/// `gh pr create`; returns the PR URL.
pub fn create_pr(ws: &Path, title: &str, body: &str, base: Option<&str>) -> Result<String, String> {
    let mut cmd = Command::new("gh");
    cmd.current_dir(ws).args(["pr", "create", "--title", title, "--body", body]);
    if let Some(b) = base {
        cmd.args(["--base", b]);
    }
    let out = cmd.output().map_err(|e| format!("รัน gh ไม่ได้ (ติดตั้งแล้วหรือยัง?): {e}"))?;
    if out.status.success() {
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines().rev().find(|l| l.starts_with("http")).map(str::to_string).ok_or_else(|| text.trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str]) {
        let o = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(o.status.success(), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
    }

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        sh(d.path(), &["init", "-q", "-b", "main"]);
        sh(d.path(), &["config", "user.email", "t@t"]);
        sh(d.path(), &["config", "user.name", "t"]);
        sh(d.path(), &["config", "commit.gpgsign", "false"]);
        std::fs::write(d.path().join("a.txt"), "1").unwrap();
        sh(d.path(), &["add", "."]);
        sh(d.path(), &["commit", "-qm", "init"]);
        d
    }

    #[test]
    fn secrets_are_recognised() {
        for p in [".env", "app/.env.local", "k.pem", "x/id_rsa", "a.p12"] {
            assert!(is_secret_path(p), "{p}");
        }
        for p in [".env.example", "src/main.rs", "environment.md"] {
            assert!(!is_secret_path(p), "{p}");
        }
    }

    #[test]
    fn commit_from_default_branch_makes_a_branch_and_skips_unchosen() {
        let d = repo();
        std::fs::write(d.path().join("b.txt"), "2").unwrap();
        std::fs::write(d.path().join("c.txt"), "3").unwrap();
        let out = commit(d.path(), "feat(x): add b", &["b.txt".into()]).unwrap();
        assert!(out.created_branch);
        assert_eq!(out.branch, "chorus/feat-x-add-b");
        let st = status(d.path()).unwrap();
        assert_eq!(st.files.len(), 1);
        assert_eq!(st.files[0].path, "c.txt");
    }

    #[test]
    fn a_secret_file_is_refused_not_staged() {
        let d = repo();
        std::fs::write(d.path().join(".env"), "K=v").unwrap();
        let err = commit(d.path(), "chore: env", &[".env".into()]).unwrap_err();
        assert!(err.contains(".env"));
        // and nothing was committed or branched
        assert_eq!(status(d.path()).unwrap().branch, "main");
    }

    #[test]
    fn push_to_a_bare_remote() {
        let d = repo();
        let bare = tempfile::tempdir().unwrap();
        sh(bare.path(), &["init", "-q", "--bare"]);
        sh(d.path(), &["remote", "add", "origin", bare.path().to_str().unwrap()]);
        std::fs::write(d.path().join("b.txt"), "2").unwrap();
        commit(d.path(), "feat: b", &["b.txt".into()]).unwrap();
        let b = push(d.path()).unwrap();
        assert_eq!(b, "chorus/feat-b");
        assert_eq!(status(d.path()).unwrap().ahead, 0);
    }

    #[test]
    fn scope_hides_files_the_session_did_not_touch() {
        let d = repo();
        std::fs::write(d.path().join("old.yml"), "x").unwrap();
        std::fs::write(d.path().join("new.txt"), "y").unwrap();
        let all = status(d.path()).unwrap();
        assert_eq!((all.files.len(), all.hidden), (2, 0));
        let scoped = status_scoped(d.path(), Some(&["new.txt".to_string()])).unwrap();
        assert_eq!(scoped.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["new.txt"]);
        assert_eq!(scoped.hidden, 1);
    }

    #[test]
    fn message_names_the_goal_or_the_files() {
        let f = |p: &str| FileChange { path: p.into(), code: "??".into(), blocked: false };
        let one = [f("docs/a.md")];
        assert_eq!(suggest_message_with_goal(&one, "Add the usage guide."), "docs(docs): add the usage guide");
        assert_eq!(suggest_message_with_goal(&[f("src/a.rs"), f("src/b.rs")], ""), "feat(src): update a.rs, b.rs");
        let many: Vec<_> = ["a", "b", "c", "d"].iter().map(|n| f(&format!("{n}.rs"))).collect();
        assert_eq!(suggest_message_with_goal(&many, ""), "feat: update a.rs, b.rs and 2 more");
    }

    #[test]
    fn suggestion_uses_type_and_scope() {
        let f = |p: &str, c: &str| FileChange { path: p.into(), code: c.into(), blocked: false };
        assert_eq!(suggest_message(&[f("README.md", " M")]), "docs: update README.md");
        assert_eq!(
            suggest_message(&[f("crates/chorus-core/src/a.rs", " M"), f("crates/chorus-core/src/b.rs", " M")]),
            "fix(core): update a.rs, b.rs"
        );
    }
}
