//! Fast virtualized file tree builder for Chorus Workbench.
//! Reads workspace files, respects .gitignore, computes Git status badges,
//! and marks the active agent focus file.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileNode {
    pub name: String,
    pub rel_path: String,
    pub is_dir: bool,
    /// File-type icon key (`file_icons::icon_for`); `None` for folders and unknown files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub size: u64,
    pub git_status: Option<String>,
    pub is_focus: bool,
    pub children: Vec<FileNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTreeResult {
    pub root_path: String,
    pub total_files: usize,
    pub nodes: Vec<FileNode>,
}

/// Standard ignore folders for performance
const IGNORED_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".build",
    ".cache",
    ".DS_Store",
    ".chorus",
    ".scratch",
    "graft",
];

/// Collect git statuses: path -> status flag (e.g. "M", "A", "?")
pub fn collect_git_status(root: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let output = match Command::new("git")
        .args(["status", "--porcelain", "-uall"])
        .current_dir(root)
        .output()
    {
        Ok(o) => o,
        Err(_) => return map,
    };

    if !output.status.success() {
        return map;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if line.len() < 4 {
            continue;
        }
        let status = line[..2].trim().to_string();
        let path = line[3..].trim().trim_matches('"').to_string();
        map.insert(path, status);
    }

    map
}

/// Sanitize and confine a focus file path strictly to the workspace root (Security D3).
pub fn sanitize_focus_path(root: &Path, focus: &str) -> Option<String> {
    let trimmed = focus.trim();
    if trimmed.is_empty() {
        return None;
    }
    let p = Path::new(trimmed);
    let candidate = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    };

    if let (Ok(canon_root), Ok(canon_candidate)) = (root.canonicalize(), candidate.canonicalize()) {
        if canon_candidate.starts_with(&canon_root) {
            return canon_candidate
                .strip_prefix(&canon_root)
                .ok()
                .map(|s| s.to_string_lossy().to_string());
        } else {
            return None; // Traversal attempt outside root
        }
    }

    let rel = if candidate.is_absolute() {
        match candidate.strip_prefix(root) {
            Ok(r) => r,
            Err(_) => return None,
        }
    } else {
        p
    };

    let mut normalized = std::path::PathBuf::new();
    for comp in rel.components() {
        match comp {
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    return None; // Escapes root!
                }
            }
            std::path::Component::Normal(c) => normalized.push(c),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }

    let result = normalized.to_string_lossy().to_string();
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// Build file tree for workspace
pub fn build_file_tree(root: &Path, focus_file: Option<&str>, max_depth: usize) -> FileTreeResult {
    let git_statuses = collect_git_status(root);
    let mut total_files = 0;
    let focus_norm = focus_file.and_then(|f| sanitize_focus_path(root, f));

    let nodes = walk_dir(
        root,
        root,
        focus_norm.as_deref(),
        &git_statuses,
        0,
        max_depth,
        &mut total_files,
    );

    FileTreeResult {
        root_path: root.display().to_string(),
        total_files,
        nodes,
    }
}

fn walk_dir(
    root: &Path,
    current: &Path,
    focus_file: Option<&str>,
    git_statuses: &HashMap<String, String>,
    depth: usize,
    max_depth: usize,
    total_files: &mut usize,
) -> Vec<FileNode> {
    if depth > max_depth {
        return Vec::new();
    }

    let entries = match fs::read_dir(current) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut nodes = Vec::new();
    let mut dir_entries: Vec<_> = entries.filter_map(Result::ok).collect();
    // Sort directories first, then alphabetical
    dir_entries.sort_by(|a, b| {
        let a_is_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let b_is_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if a_is_dir != b_is_dir {
            b_is_dir.cmp(&a_is_dir)
        } else {
            a.file_name().cmp(&b.file_name())
        }
    });

    for entry in dir_entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') && name != ".env" && name != ".gitignore" && name != ".chorus" {
            if name == ".git" {
                continue;
            }
        }
        if IGNORED_DIRS.contains(&name.as_str()) {
            continue;
        }

        let path = entry.path();
        let rel_path = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let size = if is_dir {
            0
        } else {
            *total_files += 1;
            entry.metadata().map(|m| m.len()).unwrap_or(0)
        };

        let git_status = git_statuses.get(&rel_path).cloned();
        let is_focus = focus_file.map(|f| f == rel_path).unwrap_or(false);

        let icon = if is_dir {
            None
        } else {
            crate::file_icons::icon_for(&name).map(str::to_string)
        };

        let children = if is_dir {
            walk_dir(
                root,
                &path,
                focus_file,
                git_statuses,
                depth + 1,
                max_depth,
                total_files,
            )
        } else {
            Vec::new()
        };

        nodes.push(FileNode {
            name,
            rel_path,
            is_dir,
            icon,
            size,
            git_status,
            is_focus,
            children,
        });
    }

    nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_sanitize_focus_path_security_d3() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        // 1. Valid relative path inside root
        let valid_rel = "src/main.rs";
        assert_eq!(
            sanitize_focus_path(root, valid_rel),
            Some("src/main.rs".to_string())
        );

        // 2. Traversal attempt out of root
        assert_eq!(sanitize_focus_path(root, "../../etc/passwd"), None);
        assert_eq!(sanitize_focus_path(root, "foo/../../.."), None);

        // 3. Absolute path outside root
        assert_eq!(sanitize_focus_path(root, "/etc/passwd"), None);

        // 4. Empty or whitespace
        assert_eq!(sanitize_focus_path(root, ""), None);
        assert_eq!(sanitize_focus_path(root, "   "), None);

        // 5. Existing file inside root
        let file_path = root.join("lib.rs");
        fs::write(&file_path, "pub fn test() {}").unwrap();
        assert_eq!(
            sanitize_focus_path(root, file_path.to_str().unwrap()),
            Some("lib.rs".to_string())
        );
    }

    #[test]
    fn test_build_file_tree_with_focus() {
        let dir = tempdir().unwrap();
        let root = dir.path();

        let sub = root.join("src");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("main.rs"), "fn main() {}").unwrap();
        fs::write(root.join("README.md"), "# Test").unwrap();

        let tree = build_file_tree(root, Some("src/main.rs"), 4);
        assert_eq!(tree.total_files, 2);

        // Find src directory and main.rs inside it
        let src_node = tree
            .nodes
            .iter()
            .find(|n| n.name == "src")
            .expect("src dir");
        assert!(src_node.is_dir);
        let main_node = src_node
            .children
            .iter()
            .find(|n| n.name == "main.rs")
            .expect("main.rs");
        assert!(main_node.is_focus, "main.rs should have is_focus = true");

        let readme_node = tree
            .nodes
            .iter()
            .find(|n| n.name == "README.md")
            .expect("README.md");
        assert!(
            !readme_node.is_focus,
            "README.md should have is_focus = false"
        );
    }
}
