//! Autonomous Swarm Merge Specialist & Conflict Resolver (Pillar 4).
//!
//! When concurrent subagents in isolated Git worktrees produce intersecting
//! file modifications, the Merge Specialist inspects conflict markers (`<<<<<<<`,
//! `=======`, `>>>>>>>`), synthesizes intent, and produces verified, conflict-free code.

use crate::{ChorusError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    /// Synthesize both changes preserving all non-duplicate additions.
    Synthesize,
    /// Prefer current HEAD branch.
    PreferOurs,
    /// Prefer incoming subagent worktree branch.
    PreferTheirs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictHunk {
    pub ours: String,
    pub theirs: String,
}

pub struct MergeSpecialist;

impl MergeSpecialist {
    /// Check whether a file's content contains unresolved Git conflict markers.
    pub fn has_conflicts(content: &str) -> bool {
        content.contains("<<<<<<<") && content.contains("=======") && content.contains(">>>>>>>")
    }

    /// Extract all conflict hunks from conflicted file content.
    pub fn extract_conflicts(content: &str) -> Vec<ConflictHunk> {
        let mut hunks = Vec::new();
        let lines: Vec<&str> = content.lines().collect();
        let mut idx = 0;

        while idx < lines.len() {
            if lines[idx].starts_with("<<<<<<<") {
                let mut ours_lines = Vec::new();
                let mut theirs_lines = Vec::new();
                let mut in_theirs = false;
                idx += 1;

                while idx < lines.len() {
                    let line = lines[idx];
                    if line.starts_with("=======") {
                        in_theirs = true;
                    } else if line.starts_with(">>>>>>>") {
                        hunks.push(ConflictHunk {
                            ours: ours_lines.join("\n"),
                            theirs: theirs_lines.join("\n"),
                        });
                        break;
                    } else if in_theirs {
                        theirs_lines.push(line);
                    } else {
                        ours_lines.push(line);
                    }
                    idx += 1;
                }
            }
            idx += 1;
        }

        hunks
    }

    /// Resolve conflict markers using the specified strategy.
    pub fn resolve_content(content: &str, strategy: MergeStrategy) -> Result<String> {
        if !Self::has_conflicts(content) {
            return Ok(content.to_string());
        }

        let mut output_lines = Vec::new();
        let lines: Vec<&str> = content.lines().collect();
        let mut idx = 0;

        while idx < lines.len() {
            let line = lines[idx];
            if line.starts_with("<<<<<<<") {
                let mut ours = Vec::new();
                let mut theirs = Vec::new();
                let mut in_theirs = false;
                idx += 1;

                while idx < lines.len() {
                    let cur = lines[idx];
                    if cur.starts_with("=======") {
                        in_theirs = true;
                    } else if cur.starts_with(">>>>>>>") {
                        break;
                    } else if in_theirs {
                        theirs.push(cur);
                    } else {
                        ours.push(cur);
                    }
                    idx += 1;
                }

                match strategy {
                    MergeStrategy::PreferOurs => {
                        output_lines.extend(ours);
                    }
                    MergeStrategy::PreferTheirs => {
                        output_lines.extend(theirs);
                    }
                    MergeStrategy::Synthesize => {
                        // Intelligent synthesis: deduplicate identical lines and retain additions
                        for ol in &ours {
                            output_lines.push(*ol);
                        }
                        for tl in &theirs {
                            if !ours.contains(tl) {
                                output_lines.push(*tl);
                            }
                        }
                    }
                }
            } else {
                output_lines.push(line);
            }
            idx += 1;
        }

        let resolved = output_lines.join("\n");
        if Self::has_conflicts(&resolved) {
            return Err(ChorusError::Invalid("failed to resolve all conflict markers".into()));
        }

        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conflict_detection_and_extraction() {
        let content = r#"fn calculate() -> i32 {
<<<<<<< HEAD
    let a = 10;
=======
    let a = 20;
    let b = 30;
>>>>>>> feature-branch
    a
}"#;

        assert!(MergeSpecialist::has_conflicts(content));
        let hunks = MergeSpecialist::extract_conflicts(content);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].ours, "    let a = 10;");
        assert_eq!(hunks[0].theirs, "    let a = 20;\n    let b = 30;");
    }

    #[test]
    fn test_resolution_strategies() {
        let content = r#"header
<<<<<<< HEAD
line_ours
=======
line_theirs
>>>>>>> incoming
footer"#;

        let ours = MergeSpecialist::resolve_content(content, MergeStrategy::PreferOurs).unwrap();
        assert!(ours.contains("line_ours"));
        assert!(!ours.contains("line_theirs"));
        assert!(!MergeSpecialist::has_conflicts(&ours));

        let theirs = MergeSpecialist::resolve_content(content, MergeStrategy::PreferTheirs).unwrap();
        assert!(!theirs.contains("line_ours"));
        assert!(theirs.contains("line_theirs"));
        assert!(!MergeSpecialist::has_conflicts(&theirs));

        let synth = MergeSpecialist::resolve_content(content, MergeStrategy::Synthesize).unwrap();
        assert!(synth.contains("line_ours"));
        assert!(synth.contains("line_theirs"));
        assert!(!MergeSpecialist::has_conflicts(&synth));
    }
}
