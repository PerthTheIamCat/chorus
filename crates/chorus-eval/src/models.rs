//! Task data models and loader for the Chorus evaluation harness.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The engineering domain of an evaluation task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Domain {
    Rust,
    Swift,
    #[serde(rename = "typescript")]
    TypeScript,
    Python,
}

impl Domain {
    pub const ALL: [Self; 4] = [
        Domain::Rust,
        Domain::Swift,
        Domain::TypeScript,
        Domain::Python,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Domain::Rust => "rust",
            Domain::Swift => "swift",
            Domain::TypeScript => "typescript",
            Domain::Python => "python",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Domain::Rust => "Rust (backend/systems)",
            Domain::Swift => "Swift (macOS UI)",
            Domain::TypeScript => "TypeScript (web/APIs)",
            Domain::Python => "Python (data/scripts)",
        }
    }
}

/// The complexity / difficulty level of an evaluation task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Difficulty {
    Trivial,
    Medium,
    Hard,
    Architectural,
}

impl Difficulty {
    pub const ALL: [Self; 4] = [
        Difficulty::Trivial,
        Difficulty::Medium,
        Difficulty::Hard,
        Difficulty::Architectural,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Difficulty::Trivial => "trivial",
            Difficulty::Medium => "medium",
            Difficulty::Hard => "hard",
            Difficulty::Architectural => "architectural",
        }
    }
}

/// A paired evaluation task from `evals/harness/tasks/`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarnessTask {
    pub task_id: String,
    pub language: String,
    pub domain: Domain,
    pub difficulty: Difficulty,
    pub prompt_th: String,
    pub prompt_en: String,
    pub test_command: String,
    pub expected_files: Vec<String>,
}

impl HarnessTask {
    /// Return the active prompt for this task instance's configured language.
    pub fn active_prompt(&self) -> &str {
        if self.language == "th" {
            &self.prompt_th
        } else {
            &self.prompt_en
        }
    }
}

/// Resolve the default task directory `evals/harness/tasks`.
pub fn default_tasks_dir() -> PathBuf {
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let p = Path::new(&manifest).join("../../evals/harness/tasks");
        if p.exists() {
            return p;
        }
    }
    let p = Path::new("evals/harness/tasks");
    if p.exists() {
        return p.to_path_buf();
    }
    PathBuf::from("evals/harness/tasks")
}

/// Load and parse all task JSON files from a directory.
pub fn load_tasks_from_dir(dir: &Path) -> Result<Vec<HarnessTask>> {
    let mut tasks = Vec::new();
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read task directory {}", dir.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
            let data = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let task: HarnessTask = serde_json::from_str(&data)
                .with_context(|| format!("failed to parse {}", path.display()))?;
            tasks.push(task);
        }
    }
    tasks.sort_by(|a, b| a.task_id.cmp(&b.task_id));
    Ok(tasks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_evaluation_tasks() {
        let dir = default_tasks_dir();
        assert!(
            dir.exists(),
            "tasks directory must exist at {}",
            dir.display()
        );
        let tasks = load_tasks_from_dir(&dir).expect("should load tasks");
        assert_eq!(tasks.len(), 60, "expected exactly 60 evaluation tasks");

        let th_count = tasks.iter().filter(|t| t.language == "th").count();
        let en_count = tasks.iter().filter(|t| t.language == "en").count();
        assert_eq!(th_count, 30, "expected 30 Thai tasks");
        assert_eq!(en_count, 30, "expected 30 English tasks");

        for d in Domain::ALL {
            let d_count = tasks.iter().filter(|t| t.domain == d).count();
            assert!(d_count >= 14, "expected at least 14 tasks in domain {:?}", d);
        }
    }
}
