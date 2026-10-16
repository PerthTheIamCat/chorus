//! BLAKE3 Merkle Tree: Incremental change detection for workspaces.
//!
//! Provides sub-millisecond tree hashing and delta computation so agents
//! and routers know exactly which files changed without scanning unchanged subtrees.

use crate::{ChorusError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A node in the hierarchical Merkle Tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MerkleNode {
    File {
        hash: String,
        size_bytes: u64,
        modified_ms: u64,
    },
    Directory {
        hash: String,
        children: BTreeMap<String, MerkleNode>,
    },
}

impl MerkleNode {
    pub fn hash(&self) -> &str {
        match self {
            MerkleNode::File { hash, .. } => hash,
            MerkleNode::Directory { hash, .. } => hash,
        }
    }

    pub fn is_dir(&self) -> bool {
        matches!(self, MerkleNode::Directory { .. })
    }

    pub fn children(&self) -> Option<&BTreeMap<String, MerkleNode>> {
        match self {
            MerkleNode::Directory { children, .. } => Some(children),
            _ => None,
        }
    }
}

/// Changes detected between two Merkle trees.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MerkleDiff {
    pub added: Vec<PathBuf>,
    pub modified: Vec<PathBuf>,
    pub deleted: Vec<PathBuf>,
}

impl MerkleDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.modified.is_empty() && self.deleted.is_empty()
    }

    pub fn changed_paths(&self) -> Vec<PathBuf> {
        let mut all = Vec::new();
        all.extend(self.added.clone());
        all.extend(self.modified.clone());
        all.extend(self.deleted.clone());
        all.sort();
        all.dedup();
        all
    }
}

/// Builder for workspace Merkle trees.
pub struct MerkleTree;

impl MerkleTree {
    /// Compute the Merkle tree for a path recursively, skipping ignored files.
    pub fn compute<P: AsRef<Path>>(root: P, ignore_patterns: &[&str]) -> Result<MerkleNode> {
        let path = root.as_ref();
        if !path.exists() {
            return Err(ChorusError::Invalid(format!("Path does not exist: {:?}", path)));
        }

        if path.is_file() {
            Self::compute_file(path)
        } else {
            Self::compute_dir(path, ignore_patterns)
        }
    }

    fn compute_file(path: &Path) -> Result<MerkleNode> {
        let meta = fs::metadata(path).map_err(ChorusError::Io)?;
        let size_bytes = meta.len();
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let bytes = fs::read(path).map_err(ChorusError::Io)?;
        let hash = blake3::hash(&bytes).to_hex().to_string();

        Ok(MerkleNode::File {
            hash,
            size_bytes,
            modified_ms,
        })
    }

    fn compute_dir(path: &Path, ignore_patterns: &[&str]) -> Result<MerkleNode> {
        let mut children = BTreeMap::new();
        let entries = match fs::read_dir(path) {
            Ok(e) => e,
            Err(e) => return Err(ChorusError::Io(e)),
        };

        let mut child_hashes = Vec::new();

        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();

            // Basic pattern filtering
            if Self::should_ignore(&name_str, ignore_patterns) {
                continue;
            }

            let child_path = entry.path();
            if child_path.is_symlink() {
                continue;
            }

            let child_node = if child_path.is_dir() {
                Self::compute_dir(&child_path, ignore_patterns)?
            } else {
                Self::compute_file(&child_path)?
            };

            child_hashes.push(format!("{}:{}:{}", name_str, child_node.is_dir(), child_node.hash()));
            children.insert(name_str.to_string(), child_node);
        }

        // Parent hash is derived from sorted child hashes
        child_hashes.sort();
        let mut hasher = blake3::Hasher::new();
        for ch in &child_hashes {
            hasher.update(ch.as_bytes());
            hasher.update(b"\n");
        }
        let hash = hasher.finalize().to_hex().to_string();

        Ok(MerkleNode::Directory { hash, children })
    }

    fn should_ignore(name: &str, ignore_patterns: &[&str]) -> bool {
        if name.starts_with('.') && name != ".env" && name != ".env.local" {
            return true;
        }
        for pattern in ignore_patterns {
            let pat = pattern.trim_end_matches('/');
            if name == pat || name.ends_with(pat) {
                return true;
            }
        }
        false
    }

    /// Compare two Merkle trees and extract modified, added, and deleted paths.
    pub fn diff(old_tree: Option<&MerkleNode>, new_tree: Option<&MerkleNode>) -> MerkleDiff {
        let mut diff = MerkleDiff::default();
        Self::diff_recursive(old_tree, new_tree, &PathBuf::new(), &mut diff);
        diff
    }

    fn diff_recursive(
        old_node: Option<&MerkleNode>,
        new_node: Option<&MerkleNode>,
        current_path: &Path,
        diff: &mut MerkleDiff,
    ) {
        match (old_node, new_node) {
            (None, Some(new)) => {
                Self::collect_all_paths(new, current_path, &mut diff.added);
            }
            (Some(old), None) => {
                Self::collect_all_paths(old, current_path, &mut diff.deleted);
            }
            (Some(old), Some(new)) => {
                if old.hash() == new.hash() {
                    // Entire subtree is identical, prune traversal immediately!
                    return;
                }

                match (old, new) {
                    (
                        MerkleNode::File { .. },
                        MerkleNode::File { .. },
                    ) => {
                        diff.modified.push(current_path.to_path_buf());
                    }
                    (
                        MerkleNode::Directory { children: old_ch, .. },
                        MerkleNode::Directory { children: new_ch, .. },
                    ) => {
                        let mut all_keys: std::collections::BTreeSet<&String> =
                            old_ch.keys().collect();
                        all_keys.extend(new_ch.keys());

                        for key in all_keys {
                            let old_child = old_ch.get(key);
                            let new_child = new_ch.get(key);
                            let child_path = current_path.join(key);
                            Self::diff_recursive(old_child, new_child, &child_path, diff);
                        }
                    }
                    // Type changed (e.g. file became directory or vice versa)
                    _ => {
                        diff.modified.push(current_path.to_path_buf());
                    }
                }
            }
            (None, None) => {}
        }
    }

    fn collect_all_paths(node: &MerkleNode, current_path: &Path, target: &mut Vec<PathBuf>) {
        match node {
            MerkleNode::File { .. } => {
                target.push(current_path.to_path_buf());
            }
            MerkleNode::Directory { children, .. } => {
                for (name, child) in children {
                    let child_path = current_path.join(name);
                    Self::collect_all_paths(child, &child_path, target);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn merkle_tree_detects_file_add_modify_delete() {
        let dir = tempdir().unwrap();
        let path = dir.path();

        // 1. Initial state
        fs::write(path.join("a.txt"), "hello").unwrap();
        fs::create_dir(path.join("sub")).unwrap();
        fs::write(path.join("sub").join("b.txt"), "world").unwrap();

        let tree1 = MerkleTree::compute(path, &["target", "node_modules"]).unwrap();
        assert!(tree1.is_dir());

        // Identical diff is empty
        let diff0 = MerkleTree::diff(Some(&tree1), Some(&tree1));
        assert!(diff0.is_empty());

        // 2. Modify a.txt and add c.txt
        fs::write(path.join("a.txt"), "hello modified").unwrap();
        fs::write(path.join("c.txt"), "new file").unwrap();

        let tree2 = MerkleTree::compute(path, &["target"]).unwrap();
        let diff1 = MerkleTree::diff(Some(&tree1), Some(&tree2));

        assert_eq!(diff1.modified, vec![PathBuf::from("a.txt")]);
        assert_eq!(diff1.added, vec![PathBuf::from("c.txt")]);
        assert!(diff1.deleted.is_empty());

        // 3. Delete sub/b.txt
        fs::remove_file(path.join("sub").join("b.txt")).unwrap();
        let tree3 = MerkleTree::compute(path, &[]).unwrap();
        let diff2 = MerkleTree::diff(Some(&tree2), Some(&tree3));

        assert_eq!(diff2.deleted, vec![PathBuf::from("sub").join("b.txt")]);
    }
}
