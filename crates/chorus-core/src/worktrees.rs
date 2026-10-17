//! Git Worktree Manager for multi-agent isolation.
//! Provisions isolated worktrees in sub-200ms with .worktreeinclude synchronization
//! and dynamic port allocation.

use crate::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub id: String,
    pub branch: String,
    pub path: String,
    pub role: String,
    pub allocated_port: u16,
    pub status: String, // "active", "merged", "conflict", "discarded"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeOutcome {
    pub success: bool,
    pub fast_forward: bool,
    pub has_conflicts: bool,
    pub message: String,
}

/// Where agent worktrees live: `~/Library/Application Support/Chorus/worktrees/` (fixes D4).
pub fn default_worktrees_dir() -> std::path::PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        std::path::PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Chorus")
            .join("worktrees")
    } else {
        std::path::PathBuf::from(".chorus").join("worktrees")
    }
}

/// Provision an isolated worktree under `~/Library/Application Support/Chorus/worktrees/<role>-<short_id>`.
pub fn create_worktree(root: &Path, role: &str) -> Result<WorktreeInfo> {
    create_worktree_ext(root, role, false)
}

/// Provision an isolated worktree with opt-in `.env*` copying.
pub fn create_worktree_ext(root: &Path, role: &str, copy_env: bool) -> Result<WorktreeInfo> {
    if !crate::roles::is_valid_role_name(role) {
        return Err(crate::ChorusError::Invalid(format!(
            "invalid role name {role:?}: must match [a-z0-9-]{{1,64}}"
        )));
    }

    let short_id = secure_short_id(role);
    let branch = format!("chorus/{role}-{short_id}");
    let worktrees_dir = default_worktrees_dir();
    fs::create_dir_all(&worktrees_dir)?;

    let wt_path = worktrees_dir.join(format!("{role}-{short_id}"));
    if wt_path.exists() {
        let _ = fs::remove_dir_all(&wt_path);
    }

    // Run git worktree add -b <branch> <path> HEAD
    let output = Command::new("git")
        .args([
            "worktree",
            "add",
            "-b",
            &branch,
            wt_path.to_str().unwrap(),
            "HEAD",
        ])
        .current_dir(root)
        .output()?;

    if !output.status.success() {
        return Err(crate::ChorusError::Invalid(format!(
            "failed to create git worktree: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    // Synchronize confined .worktreeinclude files (.env is opt-in)
    sync_worktree_includes(root, &wt_path, copy_env)?;

    // Allocate dynamic port
    let allocated_port = allocate_dynamic_port();

    Ok(WorktreeInfo {
        id: short_id,
        branch,
        path: wt_path.display().to_string(),
        role: role.to_string(),
        allocated_port,
        status: "active".into(),
    })
}

/// Remove worktree and delete branch
pub fn remove_worktree(root: &Path, worktree_path: &Path, branch: Option<&str>) -> Result<()> {
    let _ = Command::new("git")
        .args([
            "worktree",
            "remove",
            "--force",
            worktree_path.to_str().unwrap(),
        ])
        .current_dir(root)
        .output();

    if let Some(b) = branch {
        let _ = Command::new("git")
            .args(["branch", "-D", b])
            .current_dir(root)
            .output();
    }

    Ok(())
}

/// Attempt to merge worktree branch into current HEAD without committing automatically.
pub fn merge_worktree(root: &Path, branch: &str) -> Result<MergeOutcome> {
    merge_worktree_ext(root, branch, false)
}

/// Attempt to merge worktree branch into current HEAD, with optional auto-commit.
pub fn merge_worktree_ext(root: &Path, branch: &str, auto_commit: bool) -> Result<MergeOutcome> {
    let output = Command::new("git")
        .args(["merge", "--no-commit", "--no-ff", branch])
        .current_dir(root)
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        if auto_commit {
            let _ = Command::new("git")
                .args([
                    "commit",
                    "-m",
                    &format!("Merge agent worktree branch {branch}"),
                ])
                .current_dir(root)
                .output();
            Ok(MergeOutcome {
                success: true,
                fast_forward: stdout.contains("Fast-forward"),
                has_conflicts: false,
                message: "Merged cleanly and committed".into(),
            })
        } else {
            Ok(MergeOutcome {
                success: true,
                fast_forward: stdout.contains("Fast-forward"),
                has_conflicts: false,
                message: "Merge staged cleanly for user review".into(),
            })
        }
    } else {
        let has_conflicts = stdout.contains("CONFLICT") || stderr.contains("CONFLICT");
        // Abort merge to keep workspace clean
        let _ = Command::new("git")
            .args(["merge", "--abort"])
            .current_dir(root)
            .output();

        Ok(MergeOutcome {
            success: false,
            fast_forward: false,
            has_conflicts,
            message: if has_conflicts {
                "Merge conflict detected".into()
            } else {
                format!("{stderr} {stdout}")
            },
        })
    }
}

/// List active worktrees registered in git
pub fn list_active_worktrees(root: &Path) -> Result<Vec<WorktreeInfo>> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(root)
        .output()?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut worktrees = Vec::new();

    let mut current_path = String::new();
    let mut current_branch = String::new();

    for line in text.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            current_path = p.trim().to_string();
        } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
            current_branch = b.trim().to_string();
        } else if line.is_empty() && !current_path.is_empty() {
            if current_branch.starts_with("chorus/") {
                let role = current_branch
                    .strip_prefix("chorus/")
                    .and_then(|s| s.split('-').next())
                    .unwrap_or("agent")
                    .to_string();
                worktrees.push(WorktreeInfo {
                    id: current_branch.clone(),
                    branch: current_branch.clone(),
                    path: current_path.clone(),
                    role,
                    allocated_port: 3001,
                    status: "active".into(),
                });
            }
            current_path.clear();
            current_branch.clear();
        }
    }

    Ok(worktrees)
}

fn sync_worktree_includes(root: &Path, target: &Path, copy_env: bool) -> Result<()> {
    if copy_env {
        let default_files = [".env", ".env.local", ".env.development"];
        for file in default_files {
            let src = root.join(file);
            if src.exists() {
                if let Ok(meta) = fs::symlink_metadata(&src) {
                    if !meta.file_type().is_symlink() && meta.is_file() {
                        let dst = target.join(file);
                        let _ = fs::copy(&src, &dst);
                    }
                }
            }
        }
    }

    // Check user-configured .worktreeinclude
    let include_path = root.join(".worktreeinclude");
    if include_path.exists() {
        if let Ok(content) = fs::read_to_string(&include_path) {
            for line in content.lines() {
                let pattern = line.trim();
                if pattern.is_empty() || pattern.starts_with('#') {
                    continue;
                }
                // Confinement check: strictly reject '..', absolute paths, or null bytes
                if pattern.contains("..")
                    || pattern.starts_with('/')
                    || pattern.starts_with('\\')
                    || pattern.contains('\0')
                {
                    continue;
                }
                let src = root.join(pattern);
                if let Ok(meta) = fs::symlink_metadata(&src) {
                    // Refuse symlinks to prevent copying files outside the repository
                    if meta.file_type().is_symlink() || !meta.is_file() {
                        continue;
                    }
                    let dst = target.join(pattern);
                    if let Some(parent) = dst.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::copy(&src, &dst);
                }
            }
        }
    }

    Ok(())
}

fn allocate_dynamic_port() -> u16 {
    use std::net::TcpListener;
    for port in 3001..3100 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    3001
}

fn secure_short_id(role: &str) -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    let hash = blake3::hash(format!("{role}-{t}-{pid}").as_bytes());
    hash.to_hex()[..10].to_string()
}

/// Discard and cleanly remove a git worktree by its branch name.
pub fn discard_worktree_by_branch(root: &Path, branch: &str) -> Result<()> {
    let active = list_active_worktrees(root)?;
    if let Some(w) = active.iter().find(|w| w.branch == branch) {
        remove_worktree(root, Path::new(&w.path), Some(branch))
    } else {
        remove_worktree(root, Path::new(branch), Some(branch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_role_names_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(create_worktree(dir.path(), "../../etc/passwd").is_err());
        assert!(create_worktree(dir.path(), "INVALID_ROLE").is_err());
        assert!(create_worktree(dir.path(), "role/with/slashes").is_err());
    }

    #[test]
    fn worktree_includes_are_strictly_confined_and_env_is_opt_in() {
        let root = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();

        // Create sample files
        fs::write(root.path().join(".env"), "SECRET=true").unwrap();
        fs::write(root.path().join("normal.txt"), "hello").unwrap();
        fs::write(root.path().join("safe.json"), "{}").unwrap();

        // Traversal and outside files
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("outside.txt"), "stolen").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            outside.path().join("outside.txt"),
            root.path().join("symlink_outside"),
        )
        .unwrap();

        fs::write(
            root.path().join(".worktreeinclude"),
            "normal.txt\n../secret.txt\n/etc/passwd\nsymlink_outside\nsafe.json\n",
        )
        .unwrap();

        // 1. Without copy_env: .env is NOT copied
        sync_worktree_includes(root.path(), target.path(), false).unwrap();
        assert!(
            !target.path().join(".env").exists(),
            ".env must not be copied by default"
        );
        assert!(target.path().join("normal.txt").exists());
        assert!(target.path().join("safe.json").exists());
        assert!(!target.path().join("secret.txt").exists());
        assert!(!target.path().join("passwd").exists());
        assert!(
            !target.path().join("symlink_outside").exists(),
            "symlinks must not be followed"
        );

        // 2. With copy_env: .env IS copied
        let target2 = tempfile::tempdir().unwrap();
        sync_worktree_includes(root.path(), target2.path(), true).unwrap();
        assert!(
            target2.path().join(".env").exists(),
            ".env must be copied when copy_env is true"
        );
    }
}
