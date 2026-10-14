//! Structural AST Diffing and Formatting-Invariant Change Analysis.
//!
//! Inspired by `difftastic`, this module analyzes whether code edits are purely
//! cosmetic (indentation, line wrapping, whitespace) or contain functional semantic
//! modifications, reducing review noise and token payloads.

use serde::{Deserialize, Serialize};

/// Classification of a change hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSignificance {
    /// Purely cosmetic (spaces, indentation, blank lines).
    Cosmetic,
    /// Comment or docstring modification.
    Documentation,
    /// Functional code modification (variables, statements, control flow).
    Semantic,
}

/// Analysis result for a file diff.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuralDiffAnalysis {
    pub total_lines_changed: usize,
    pub cosmetic_lines: usize,
    pub semantic_lines: usize,
    pub significance: ChangeSignificance,
    pub token_reduction_percentage: f32,
}

pub struct StructuralDiff;

impl StructuralDiff {
    /// Analyzes differences between old_code and new_code.
    pub fn analyze(old_code: &str, new_code: &str) -> StructuralDiffAnalysis {
        let diff = similar::TextDiff::from_lines(old_code, new_code);
        let mut total_lines_changed = 0;
        let mut cosmetic_lines = 0;
        let mut semantic_lines = 0;

        for change in diff.iter_all_changes() {
            match change.tag() {
                similar::ChangeTag::Delete | similar::ChangeTag::Insert => {
                    total_lines_changed += 1;
                    let text = change.value().trim();
                    if text.is_empty() {
                        cosmetic_lines += 1;
                    } else if text.starts_with("//") || text.starts_with("/*") || text.starts_with('#') {
                        // Comment line
                        cosmetic_lines += 1;
                    } else {
                        semantic_lines += 1;
                    }
                }
                similar::ChangeTag::Equal => {}
            }
        }

        let mut cosmetic_lines = cosmetic_lines;
        let mut semantic_lines = semantic_lines;

        let old_norm = Self::normalize_tokens(old_code);
        let new_norm = Self::normalize_tokens(new_code);

        let significance = if total_lines_changed == 0 {
            ChangeSignificance::Cosmetic
        } else if old_norm == new_norm {
            ChangeSignificance::Cosmetic
        } else if semantic_lines == 0 {
            ChangeSignificance::Documentation
        } else {
            ChangeSignificance::Semantic
        };

        if significance == ChangeSignificance::Cosmetic {
            cosmetic_lines = total_lines_changed;
            semantic_lines = 0;
        }

        let token_reduction = if total_lines_changed > 0 {
            (cosmetic_lines as f32 / total_lines_changed as f32) * 100.0
        } else {
            0.0
        };

        StructuralDiffAnalysis {
            total_lines_changed,
            cosmetic_lines,
            semantic_lines,
            significance,
            token_reduction_percentage: token_reduction,
        }
    }

    /// Strip all non-essential whitespace for token comparison.
    fn normalize_tokens(source: &str) -> String {
        source
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<&str>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<&str>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_change_classified_as_cosmetic() {
        let code1 = "fn foo() {\n    let x = 1;\n}";
        let code2 = "fn foo() {\n        let x = 1;\n}";

        let analysis = StructuralDiff::analyze(code1, code2);
        assert_eq!(analysis.significance, ChangeSignificance::Cosmetic);
        assert!(analysis.token_reduction_percentage > 50.0);
    }

    #[test]
    fn functional_change_classified_as_semantic() {
        let code1 = "fn foo() -> i32 { 1 }";
        let code2 = "fn foo() -> i32 { 2 }";

        let analysis = StructuralDiff::analyze(code1, code2);
        assert_eq!(analysis.significance, ChangeSignificance::Semantic);
        assert_eq!(analysis.semantic_lines, 2);
    }
}
