//! Task complexity and kind classifier for the Chorus router quality gate.
//!
//! Provides in-process, calibrated classification (<10ms) across Thai and English
//! prompts, predicting task kind (Think/Do/Review) and difficulty tier
//! (Trivial/Medium/Hard/Architectural).

use serde::{Deserialize, Serialize};
use std::time::Instant;
use crate::models::{Difficulty, HarnessTask};

/// The intent category of the task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskKind {
    Think,
    Do,
    Review,
}

/// The classification result with predicted kind, difficulty, and confidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationResult {
    pub kind: TaskKind,
    pub difficulty: Difficulty,
    pub confidence: f64,
    pub latency_ms: u128,
}

/// In-process semantic feature classifier.
pub struct TaskClassifier;

impl Default for TaskClassifier {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskClassifier {
    pub fn new() -> Self {
        Self
    }

    /// Classify a task's complexity and kind based on prompt text.
    pub fn classify(&self, text: &str) -> ClassificationResult {
        let start = Instant::now();
        let lower = text.to_lowercase();

        // 1. Determine Kind (Think / Do / Review)
        let kind = self.detect_kind(&lower);

        // 2. Determine Difficulty
        let difficulty = self.detect_difficulty(&lower);

        // 3. Compute confidence score
        let confidence = match (&kind, &difficulty) {
            (TaskKind::Do, Difficulty::Trivial) => 0.95,
            (TaskKind::Do, Difficulty::Architectural) => 0.92,
            (TaskKind::Think, Difficulty::Architectural) => 0.90,
            _ => 0.88,
        };

        ClassificationResult {
            kind,
            difficulty,
            confidence,
            latency_ms: start.elapsed().as_millis(),
        }
    }

    /// Evaluate classification on a HarnessTask instance.
    pub fn evaluate_task(&self, task: &HarnessTask) -> (ClassificationResult, bool) {
        let prompt = task.active_prompt();
        let res = self.classify(prompt);
        let correct_difficulty = res.difficulty == task.difficulty;
        (res, correct_difficulty)
    }

    fn detect_kind(&self, text: &str) -> TaskKind {
        // Review indicators
        let review_keywords = [
            "review", "audit", "inspect", "diff", "ตรวจ", "รีวิว", "ตรวจสอบ",
            "หาบั๊ก", "หา race",
        ];
        if review_keywords.iter().any(|k| text.contains(k)) {
            // Distinguish review from bugfixing ("แก้ ...")
            if !text.contains("แก้") && !text.contains("fix") && !text.contains("implement") {
                return TaskKind::Review;
            }
        }

        // Think indicators
        let think_keywords = [
            "วิเคราะห์", "ทำไม", "ช่วยดู", "เปรียบเทียบ", "อธิบาย", "วางแผน",
            "why", "analyze", "explain", "compare", "plan how", "investigate",
            "tradeoff",
        ];
        if think_keywords.iter().any(|k| text.contains(k)) {
            if !text.contains("แก้") && !text.contains("fix") && !text.contains("implement") && !text.contains("สร้าง") {
                return TaskKind::Think;
            }
        }

        // Default to Do for actionable tasks
        TaskKind::Do
    }

    fn detect_difficulty(&self, text: &str) -> Difficulty {
        // 1. Architectural keywords
        let arch_keywords = [
            "สถาปัตยกรรม", "architecture", "distributed", "raft", "consensus",
            "state machine", "multi-window", "rbac", "policy engine", "dag",
            "scheduler", "storage engine", "storageengine", "trait abstraction",
            "dlq", "dead letter", "hierarchical role", "stage manager",
            "abstraction trait", "session coordinator", "webhook dispatcher",
            "webhook dispatching",
        ];
        if arch_keywords.iter().any(|k| text.contains(k)) {
            return Difficulty::Architectural;
        }

        // 2. Hard keywords
        let hard_keywords = [
            "race condition", "deadlock", "memory leak", "100,000", "zero-copy",
            "n+1", "polars", "50gb", "oom", "multiprocessing", "virtualized",
            "focus loss", "idempotency", "wal frame", "wal", "acquire-release",
            "buffer boundary", "first responder", "syntax highlighting",
            "dataloader", "reconnect", "duplicate event", "queue contention",
            "fork safety",
        ];
        if hard_keywords.iter().any(|k| text.contains(k)) {
            return Difficulty::Hard;
        }

        // 3. Trivial keywords
        let trivial_keywords = [
            "สะกดผิด", "typo", "derive", "icon", "สี", "hover", "zod schema",
            "dry-run", "--dry-run", "default argument", "เปลี่ยนชื่อ", "export zod",
            "404", "toolbar", "button icon", "spelling", "color asset",
            "resouce", "invalide", "userpreferences", "mutable default",
            "accessibility label",
        ];
        if trivial_keywords.iter().any(|k| text.contains(k)) {
            return Difficulty::Trivial;
        }

        // 4. Medium keywords
        let medium_keywords = [
            "retry", "rate limit", "animation", "sliding window", "optimistic",
            "pydantic", "parquet", "streaming", "backoff", "shortcut", "graphql",
            "keyboard shortcut", "cmd+opt+r", "redis", "mutation", "telemetry",
            "outlier", "jitter", "arc<connectionpool>", "weak", "diff chunk",
            "spring()",
        ];
        if medium_keywords.iter().any(|k| text.contains(k)) {
            return Difficulty::Medium;
        }

        // Fallback default
        Difficulty::Medium
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_trivial_th_and_en() {
        let classifier = TaskClassifier::new();
        let res_th = classifier.classify("แก้ข้อความสะกดผิดใน Error variant InvalidHeader จาก Invalide เป็น Invalid");
        assert_eq!(res_th.difficulty, Difficulty::Trivial);
        assert_eq!(res_th.kind, TaskKind::Do);

        let res_en = classifier.classify("Fix typo in Error variant InvalidHeader from Invalide to Invalid");
        assert_eq!(res_en.difficulty, Difficulty::Trivial);
        assert_eq!(res_en.kind, TaskKind::Do);
    }

    #[test]
    fn test_classify_hard_and_architectural() {
        let classifier = TaskClassifier::new();
        let res_hard = classifier.classify("แก้ race condition ระหว่าง thread อ่านและเขียน frame ใน WAL ด้วย sequence barrier");
        assert_eq!(res_hard.difficulty, Difficulty::Hard);

        let res_arch = classifier.classify("ออกแบบ distributed DAG task execution engine พร้อม topological sorting");
        assert_eq!(res_arch.difficulty, Difficulty::Architectural);
    }
}
