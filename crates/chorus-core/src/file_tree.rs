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

/// Build file tree for workspace
pub fn build_file_tree(
    root: &Path,
    focus_file: Option<&str>,
    max_depth: usize,
) -> FileTreeResult {
    let git_statuses = collect_git_status(root);
    let mut total_files = 0;
    let focus_norm = focus_file.map(|f| f.trim_start_matches('/'));

    let nodes = walk_dir(
        root,
        root,
        focus_norm,
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
        let is_focus = focus_file.map(|f| f == rel_path || rel_path.ends_with(f)).unwrap_or(false);

        let icon = if is_dir { None } else { crate::file_icons::icon_for(&name).map(str::to_string) };

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
