//! Zero tolerance tool loop detector for agent evaluation runs.
//!
//! Asserts invariant: calling identical tool with identical args > 2 times
//! immediately fails the run, preventing infinite no-action loops.

use serde::{Deserialize, Serialize};

/// A recorded tool call invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub arguments: String,
}

impl ToolCallRecord {
    pub fn new(tool_name: impl Into<String>, arguments: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            arguments: arguments.into(),
        }
    }
}

/// A detected loop violation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolLoopViolation {
    pub tool_name: String,
    pub arguments: String,
    pub repetitions: usize,
    pub index: usize,
    pub kind: LoopKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoopKind {
    /// Consecutive identical tool calls with identical arguments.
    Consecutive,
    /// Alternating 2-step cycle (A -> B -> A -> B -> A -> B).
    Cycle2,
}

/// Tool loop detector enforcing harness safety limits.
#[derive(Debug, Clone)]
pub struct ToolLoopDetector {
    max_identical_consecutive: usize,
}

impl Default for ToolLoopDetector {
    fn default() -> Self {
        Self {
            max_identical_consecutive: 2,
        }
    }
}

impl ToolLoopDetector {
    pub fn new(max_identical_consecutive: usize) -> Self {
        Self {
            max_identical_consecutive,
        }
    }

    /// Check a sequence of tool calls for loop violations.
    pub fn check(&self, calls: &[ToolCallRecord]) -> Vec<ToolLoopViolation> {
        let mut violations = Vec::new();
        if calls.is_empty() {
            return violations;
        }

        // 1. Check consecutive identical tool calls
        let mut current_tool = &calls[0].tool_name;
        let mut current_args = &calls[0].arguments;
        let mut streak = 1;
        let mut start_idx = 0;

        for (i, call) in calls.iter().enumerate().skip(1) {
            if &call.tool_name == current_tool && &call.arguments == current_args {
                streak += 1;
            } else {
                if streak > self.max_identical_consecutive {
                    violations.push(ToolLoopViolation {
                        tool_name: current_tool.clone(),
                        arguments: current_args.clone(),
                        repetitions: streak,
                        index: start_idx,
                        kind: LoopKind::Consecutive,
                    });
                }
                current_tool = &call.tool_name;
                current_args = &call.arguments;
                streak = 1;
                start_idx = i;
            }
        }
        if streak > self.max_identical_consecutive {
            violations.push(ToolLoopViolation {
                tool_name: current_tool.clone(),
                arguments: current_args.clone(),
                repetitions: streak,
                index: start_idx,
                kind: LoopKind::Consecutive,
            });
        }

        // 2. Check 2-step alternating cycles (A -> B -> A -> B -> A -> B)
        if calls.len() >= 6 {
            for i in 0..calls.len().saturating_sub(5) {
                if calls[i] == calls[i + 2]
                    && calls[i + 2] == calls[i + 4]
                    && calls[i + 1] == calls[i + 3]
                    && calls[i + 3] == calls[i + 5]
                    && calls[i] != calls[i + 1]
                {
                    violations.push(ToolLoopViolation {
                        tool_name: format!("{}/{}", calls[i].tool_name, calls[i + 1].tool_name),
                        arguments: "cyclic alternation".into(),
                        repetitions: 3,
                        index: i,
                        kind: LoopKind::Cycle2,
                    });
                    break;
                }
            }
        }

        violations
    }
}

/// Helper function to detect tool loops with default policy (> 2 repetitions).
pub fn detect_tool_loops(calls: &[ToolCallRecord]) -> Vec<ToolLoopViolation> {
    ToolLoopDetector::default().check(calls)
}

/// Returns true if any tool loop was detected.
pub fn has_tool_loops(calls: &[ToolCallRecord]) -> bool {
    !detect_tool_loops(calls).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_calls_no_loop() {
        let calls = vec![
            ToolCallRecord::new("read_file", "src/lib.rs"),
            ToolCallRecord::new("read_file", "src/main.rs"),
            ToolCallRecord::new("bash", "cargo check"),
            ToolCallRecord::new("edit_file", "src/lib.rs"),
        ];
        assert!(!has_tool_loops(&calls));
    }

    #[test]
    fn test_two_repetitions_allowed() {
        let calls = vec![
            ToolCallRecord::new("read_file", "src/lib.rs"),
            ToolCallRecord::new("read_file", "src/lib.rs"),
            ToolCallRecord::new("edit_file", "src/lib.rs"),
        ];
        assert!(!has_tool_loops(&calls));
    }

    #[test]
    fn test_three_repetitions_flags_violation() {
        let calls = vec![
            ToolCallRecord::new("read_file", "src/lib.rs"),
            ToolCallRecord::new("read_file", "src/lib.rs"),
            ToolCallRecord::new("read_file", "src/lib.rs"),
        ];
        let violations = detect_tool_loops(&calls);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].repetitions, 3);
        assert_eq!(violations[0].kind, LoopKind::Consecutive);
    }

    #[test]
    fn test_cycle_alternation_flags_violation() {
        let calls = vec![
            ToolCallRecord::new("read_file", "a.rs"),
            ToolCallRecord::new("read_file", "b.rs"),
            ToolCallRecord::new("read_file", "a.rs"),
            ToolCallRecord::new("read_file", "b.rs"),
            ToolCallRecord::new("read_file", "a.rs"),
            ToolCallRecord::new("read_file", "b.rs"),
        ];
        let violations = detect_tool_loops(&calls);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].kind, LoopKind::Cycle2);
    }
}
