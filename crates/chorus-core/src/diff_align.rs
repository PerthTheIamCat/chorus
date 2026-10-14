//! Myers diff alignment and side-by-side spacer generator.
//! Eliminates dual-scroll drift by pre-aligning hunks in Rust.

use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlignedDiffReport {
    pub files: Vec<AlignedFileDiff>,
    pub total_additions: usize,
    pub total_deletions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlignedFileDiff {
    pub path: String,
    pub status: String, // "modified", "added", "deleted"
    pub additions: usize,
    pub deletions: usize,
    pub hunks: Vec<AlignedHunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlignedHunk {
    pub header: String,
    pub old_start: usize,
    pub new_start: usize,
    pub pairs: Vec<AlignedLinePair>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlignedLinePair {
    pub left: Option<DiffLine>,
    pub right: Option<DiffLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffLine {
    pub line_num: usize,
    pub text: String,
    pub kind: String, // "context", "add", "delete"
}

/// Compute aligned diff between two text contents
pub fn align_file_diff(path: &str, old_text: &str, new_text: &str) -> AlignedFileDiff {
    let diff = TextDiff::from_lines(old_text, new_text);
    let mut hunks = Vec::new();
    let mut additions = 0;
    let mut deletions = 0;

    for group in diff.grouped_ops(3) {
        let mut pairs = Vec::new();
        let mut old_start = 0;
        let mut new_start = 0;

        for op in group {
            let mut left_lines = Vec::new();
            let mut right_lines = Vec::new();

            for change in diff.iter_changes(&op) {
                let text = change.value().trim_end_matches('\n').to_string();
                match change.tag() {
                    ChangeTag::Equal => {
                        let old_idx = change.old_index().map(|i| i + 1).unwrap_or(0);
                        let new_idx = change.new_index().map(|i| i + 1).unwrap_or(0);
                        if old_start == 0 { old_start = old_idx; }
                        if new_start == 0 { new_start = new_idx; }
                        left_lines.push(Some(DiffLine {
                            line_num: old_idx,
                            text: text.clone(),
                            kind: "context".into(),
                        }));
                        right_lines.push(Some(DiffLine {
                            line_num: new_idx,
                            text,
                            kind: "context".into(),
                        }));
                    }
                    ChangeTag::Delete => {
                        deletions += 1;
                        let old_idx = change.old_index().map(|i| i + 1).unwrap_or(0);
                        if old_start == 0 { old_start = old_idx; }
                        left_lines.push(Some(DiffLine {
                            line_num: old_idx,
                            text,
                            kind: "delete".into(),
                        }));
                    }
                    ChangeTag::Insert => {
                        additions += 1;
                        let new_idx = change.new_index().map(|i| i + 1).unwrap_or(0);
                        if new_start == 0 { new_start = new_idx; }
                        right_lines.push(Some(DiffLine {
                            line_num: new_idx,
                            text,
                            kind: "add".into(),
                        }));
                    }
                }
            }

            // Align left and right with spacers
            let max_len = left_lines.len().max(right_lines.len());
            for i in 0..max_len {
                let left = left_lines.get(i).cloned().flatten();
                let right = right_lines.get(i).cloned().flatten();
                pairs.push(AlignedLinePair { left, right });
            }
        }

        hunks.push(AlignedHunk {
            header: format!("@@ -{},{} +{},{} @@", old_start, pairs.len(), new_start, pairs.len()),
            old_start,
            new_start,
            pairs,
        });
    }

    let status = if old_text.is_empty() {
        "added"
    } else if new_text.is_empty() {
        "deleted"
    } else {
        "modified"
    };

    AlignedFileDiff {
        path: path.to_string(),
        status: status.into(),
        additions,
        deletions,
        hunks,
    }
}
