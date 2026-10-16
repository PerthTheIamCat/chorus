//! Verification loop triggering and execution engine.
//!
//! Controls automated test verification post-edit. Tasks that produce code
//! modifications or specify test commands trigger the verification loop;
//! pure read-only queries skip it to save time and compute.

use serde::{Deserialize, Serialize};
use crate::models::HarnessTask;

/// Decision on whether to trigger the post-edit verification loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerDecision {
    Trigger,
    Skip,
}

/// Policy configuration for verification loops.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationPolicy {
    pub auto_verify_code_edits: bool,
    pub max_turns: usize,
    pub require_expected_files: bool,
}

impl Default for VerificationPolicy {
    fn default() -> Self {
        Self {
            auto_verify_code_edits: true,
            max_turns: 3,
            require_expected_files: true,
        }
    }
}

/// Simulated or recorded result of a verification loop pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationOutcome {
    pub triggered: bool,
    pub attempts: usize,
    pub passed: bool,
    pub touched_expected_files: bool,
    pub feedback_message: Option<String>,
}

/// Verification loop engine.
pub struct VerificationLoop {
    policy: VerificationPolicy,
}

impl Default for VerificationLoop {
    fn default() -> Self {
        Self::new(VerificationPolicy::default())
    }
}

impl VerificationLoop {
    pub fn new(policy: VerificationPolicy) -> Self {
        Self { policy }
    }

    /// Determine whether a task should trigger the verification loop.
    pub fn should_trigger(&self, task: &HarnessTask) -> TriggerDecision {
        if !self.policy.auto_verify_code_edits {
            return TriggerDecision::Skip;
        }

        // Tasks with an active test command and expected modified files must trigger
        if !task.test_command.trim().is_empty() && !task.expected_files.is_empty() {
            TriggerDecision::Trigger
        } else {
            TriggerDecision::Skip
        }
    }

    /// Determine whether arbitrary text / action should trigger verification.
    pub fn should_trigger_prompt(&self, text: &str, has_test_command: bool) -> TriggerDecision {
        let lower = text.to_lowercase();
        let is_action = lower.contains("แก้")
            || lower.contains("สร้าง")
            || lower.contains("เพิ่ม")
            || lower.contains("ปรับปรุง")
            || lower.contains("fix")
            || lower.contains("implement")
            || lower.contains("refactor")
            || lower.contains("add");

        if is_action && has_test_command {
            TriggerDecision::Trigger
        } else {
            TriggerDecision::Skip
        }
    }

    /// Evaluate verification on a task with simulated or real edited files and outcome.
    pub fn evaluate_turn(
        &self,
        task: &HarnessTask,
        edited_files: &[String],
        test_success: bool,
        current_attempt: usize,
    ) -> VerificationOutcome {
        let trigger = self.should_trigger(task);
        if trigger == TriggerDecision::Skip {
            return VerificationOutcome {
                triggered: false,
                attempts: 0,
                passed: true,
                touched_expected_files: true,
                feedback_message: None,
            };
        }

        // Verify that expected files were touched
        let touched_all = task.expected_files.iter().all(|expected| {
            edited_files.iter().any(|edited| edited == expected || edited.ends_with(expected))
        });

        if !touched_all {
            let missing: Vec<_> = task
                .expected_files
                .iter()
                .filter(|expected| {
                    !edited_files.iter().any(|edited| edited == *expected || edited.ends_with(expected.as_str()))
                })
                .cloned()
                .collect();
            return VerificationOutcome {
                triggered: true,
                attempts: current_attempt,
                passed: false,
                touched_expected_files: false,
                feedback_message: Some(format!(
                    "Verification failed: expected modified files not touched: {:?}",
                    missing
                )),
            };
        }

        if test_success {
            VerificationOutcome {
                triggered: true,
                attempts: current_attempt,
                passed: true,
                touched_expected_files: true,
                feedback_message: None,
            }
        } else {
            VerificationOutcome {
                triggered: true,
                attempts: current_attempt,
                passed: false,
                touched_expected_files: true,
                feedback_message: Some(format!(
                    "Verification test command failed: `{}` exited with non-zero status",
                    task.test_command
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Difficulty, Domain};

    #[test]
    fn test_verification_triggers_on_code_tasks() {
        let vloop = VerificationLoop::default();
        let task = HarnessTask {
            task_id: "test-01".into(),
            language: "en".into(),
            domain: Domain::Rust,
            difficulty: Difficulty::Trivial,
            prompt_th: "".into(),
            prompt_en: "Fix typo".into(),
            test_command: "cargo test".into(),
            expected_files: vec!["src/lib.rs".into()],
        };

        assert_eq!(vloop.should_trigger(&task), TriggerDecision::Trigger);
    }

    #[test]
    fn test_verification_skips_without_command() {
        let vloop = VerificationLoop::default();
        let task = HarnessTask {
            task_id: "test-02".into(),
            language: "en".into(),
            domain: Domain::Rust,
            difficulty: Difficulty::Trivial,
            prompt_th: "".into(),
            prompt_en: "Explain architecture".into(),
            test_command: "".into(),
            expected_files: vec![],
        };

        assert_eq!(vloop.should_trigger(&task), TriggerDecision::Skip);
    }

    #[test]
    fn test_verification_detects_missing_files() {
        let vloop = VerificationLoop::default();
        let task = HarnessTask {
            task_id: "test-03".into(),
            language: "en".into(),
            domain: Domain::Rust,
            difficulty: Difficulty::Trivial,
            prompt_th: "".into(),
            prompt_en: "Fix typo".into(),
            test_command: "cargo test".into(),
            expected_files: vec!["src/lib.rs".into()],
        };

        let res = vloop.evaluate_turn(&task, &["src/other.rs".to_string()], true, 1);
        assert!(!res.passed);
        assert!(!res.touched_expected_files);
        assert!(res.feedback_message.unwrap().contains("src/lib.rs"));
    }
}
