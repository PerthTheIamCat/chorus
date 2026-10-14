//! Metrics collector and telemetry for the Chorus harness evaluation suite.
//!
//! Tracks Pass@1, token consumption, context bloat ratio, and tool loop count
//! across engineering domains, languages, and difficulty tiers.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use crate::models::{Difficulty, Domain};

/// Result of evaluating a single task instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: String,
    pub domain: Domain,
    pub language: String,
    pub difficulty: Difficulty,
    pub passed: bool,
    pub turns_taken: usize,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
    pub raw_context_tokens: usize,
    pub assembled_context_tokens: usize,
    pub context_bloat_ratio: f64,
    pub tool_loop_count: usize,
    pub duration_ms: u64,
}

/// Aggregated metrics for an engineering domain.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DomainMetrics {
    pub total: usize,
    pub passed: usize,
    pub pass_at_1: f64,
    pub avg_tokens: f64,
    pub avg_bloat_ratio: f64,
    pub tool_loops: usize,
}

/// Aggregated metrics for a language (Thai vs English).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LanguageMetrics {
    pub total: usize,
    pub passed: usize,
    pub pass_at_1: f64,
    pub avg_tokens: f64,
    pub avg_bloat_ratio: f64,
}

/// Aggregated metrics for a difficulty level.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DifficultyMetrics {
    pub total: usize,
    pub passed: usize,
    pub pass_at_1: f64,
    pub avg_tokens: f64,
}

/// Complete aggregated evaluation suite metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalMetrics {
    pub total_tasks: usize,
    pub passed_tasks: usize,
    pub pass_at_1: f64,
    pub total_tokens: usize,
    pub avg_tokens_per_task: f64,
    pub avg_context_bloat_ratio: f64,
    pub total_tool_loops: usize,
    pub avg_duration_ms: f64,
    pub domain_breakdown: HashMap<Domain, DomainMetrics>,
    pub language_breakdown: HashMap<String, LanguageMetrics>,
    pub difficulty_breakdown: HashMap<Difficulty, DifficultyMetrics>,
}

/// In-memory accumulator for task evaluation results.
#[derive(Default)]
pub struct MetricsCollector {
    results: Vec<TaskResult>,
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            results: Vec::new(),
        }
    }

    pub fn record(&mut self, result: TaskResult) {
        self.results.push(result);
    }

    pub fn results(&self) -> &[TaskResult] {
        &self.results
    }

    pub fn compute(&self) -> EvalMetrics {
        let total_tasks = self.results.len();
        if total_tasks == 0 {
            return EvalMetrics {
                total_tasks: 0,
                passed_tasks: 0,
                pass_at_1: 0.0,
                total_tokens: 0,
                avg_tokens_per_task: 0.0,
                avg_context_bloat_ratio: 0.0,
                total_tool_loops: 0,
                avg_duration_ms: 0.0,
                domain_breakdown: HashMap::new(),
                language_breakdown: HashMap::new(),
                difficulty_breakdown: HashMap::new(),
            };
        }

        let passed_tasks = self.results.iter().filter(|r| r.passed).count();
        let pass_at_1 = (passed_tasks as f64) / (total_tasks as f64);
        let total_tokens: usize = self.results.iter().map(|r| r.total_tokens).sum();
        let avg_tokens_per_task = (total_tokens as f64) / (total_tasks as f64);
        let sum_bloat: f64 = self.results.iter().map(|r| r.context_bloat_ratio).sum();
        let avg_context_bloat_ratio = sum_bloat / (total_tasks as f64);
        let total_tool_loops: usize = self.results.iter().map(|r| r.tool_loop_count).sum();
        let sum_duration: u64 = self.results.iter().map(|r| r.duration_ms).sum();
        let avg_duration_ms = (sum_duration as f64) / (total_tasks as f64);

        // Domain breakdown
        let mut domain_map: HashMap<Domain, Vec<&TaskResult>> = HashMap::new();
        for r in &self.results {
            domain_map.entry(r.domain).or_default().push(r);
        }
        let mut domain_breakdown = HashMap::new();
        for (dom, list) in domain_map {
            let d_passed = list.iter().filter(|r| r.passed).count();
            let d_tokens: usize = list.iter().map(|r| r.total_tokens).sum();
            let d_bloat: f64 = list.iter().map(|r| r.context_bloat_ratio).sum();
            let d_loops: usize = list.iter().map(|r| r.tool_loop_count).sum();
            domain_breakdown.insert(
                dom,
                DomainMetrics {
                    total: list.len(),
                    passed: d_passed,
                    pass_at_1: (d_passed as f64) / (list.len() as f64),
                    avg_tokens: (d_tokens as f64) / (list.len() as f64),
                    avg_bloat_ratio: d_bloat / (list.len() as f64),
                    tool_loops: d_loops,
                },
            );
        }

        // Language breakdown
        let mut lang_map: HashMap<String, Vec<&TaskResult>> = HashMap::new();
        for r in &self.results {
            lang_map.entry(r.language.clone()).or_default().push(r);
        }
        let mut language_breakdown = HashMap::new();
        for (lang, list) in lang_map {
            let l_passed = list.iter().filter(|r| r.passed).count();
            let l_tokens: usize = list.iter().map(|r| r.total_tokens).sum();
            let l_bloat: f64 = list.iter().map(|r| r.context_bloat_ratio).sum();
            language_breakdown.insert(
                lang,
                LanguageMetrics {
                    total: list.len(),
                    passed: l_passed,
                    pass_at_1: (l_passed as f64) / (list.len() as f64),
                    avg_tokens: (l_tokens as f64) / (list.len() as f64),
                    avg_bloat_ratio: l_bloat / (list.len() as f64),
                },
            );
        }

        // Difficulty breakdown
        let mut diff_map: HashMap<Difficulty, Vec<&TaskResult>> = HashMap::new();
        for r in &self.results {
            diff_map.entry(r.difficulty).or_default().push(r);
        }
        let mut difficulty_breakdown = HashMap::new();
        for (diff, list) in diff_map {
            let df_passed = list.iter().filter(|r| r.passed).count();
            let df_tokens: usize = list.iter().map(|r| r.total_tokens).sum();
            difficulty_breakdown.insert(
                diff,
                DifficultyMetrics {
                    total: list.len(),
                    passed: df_passed,
                    pass_at_1: (df_passed as f64) / (list.len() as f64),
                    avg_tokens: (df_tokens as f64) / (list.len() as f64),
                },
            );
        }

        EvalMetrics {
            total_tasks,
            passed_tasks,
            pass_at_1,
            total_tokens,
            avg_tokens_per_task,
            avg_context_bloat_ratio,
            total_tool_loops,
            avg_duration_ms,
            domain_breakdown,
            language_breakdown,
            difficulty_breakdown,
        }
    }

    /// Render human-readable summary table to stdout.
    pub fn print_summary(&self) {
        let m = self.compute();
        println!("\n================================================================================");
        println!("                     CHORUS HARNESS EVALUATION METRICS                          ");
        println!("================================================================================");
        println!(
            "  Pass@1 Solve Rate:           {:.1}% ({}/{} tasks)",
            m.pass_at_1 * 100.0,
            m.passed_tasks,
            m.total_tasks
        );
        println!(
            "  Total Token Consumption:     {} tokens (avg {:.0} / task)",
            m.total_tokens,
            m.avg_tokens_per_task
        );
        println!(
            "  Average Context Bloat Ratio: {:.2} (assembled / raw context)",
            m.avg_context_bloat_ratio
        );
        println!(
            "  Total Tool Loops Detected:   {} (zero-tolerance threshold)",
            m.total_tool_loops
        );
        println!(
            "  Average Task Latency:        {:.1} ms",
            m.avg_duration_ms
        );

        println!("--------------------------------------------------------------------------------");
        println!("  DOMAIN BREAKDOWN:");
        for d in Domain::ALL {
            if let Some(dm) = m.domain_breakdown.get(&d) {
                println!(
                    "    {:<12} {:>2}/{:<2} ({:>5.1}%) | avg {:>5.0} tok | bloat: {:>4.2} | loops: {}",
                    d.as_str(),
                    dm.passed,
                    dm.total,
                    dm.pass_at_1 * 100.0,
                    dm.avg_tokens,
                    dm.avg_bloat_ratio,
                    dm.tool_loops
                );
            }
        }

        println!("--------------------------------------------------------------------------------");
        println!("  LANGUAGE BREAKDOWN:");
        for lang in ["th", "en"] {
            if let Some(lm) = m.language_breakdown.get(lang) {
                let name = if lang == "th" { "Thai (th)" } else { "English (en)" };
                println!(
                    "    {:<12} {:>2}/{:<2} ({:>5.1}%) | avg {:>5.0} tok | bloat: {:>4.2}",
                    name,
                    lm.passed,
                    lm.total,
                    lm.pass_at_1 * 100.0,
                    lm.avg_tokens,
                    lm.avg_bloat_ratio
                );
            }
        }

        println!("--------------------------------------------------------------------------------");
        println!("  DIFFICULTY BREAKDOWN:");
        for diff in Difficulty::ALL {
            if let Some(dfm) = m.difficulty_breakdown.get(&diff) {
                println!(
                    "    {:<14} {:>2}/{:<2} ({:>5.1}%) | avg {:>5.0} tok",
                    diff.as_str(),
                    dfm.passed,
                    dfm.total,
                    dfm.pass_at_1 * 100.0,
                    dfm.avg_tokens
                );
            }
        }
        println!("================================================================================\n");
    }

    /// Generate Markdown report string suitable for telemetry logging.
    pub fn to_markdown_report(&self, title: &str) -> String {
        let m = self.compute();
        let mut s = String::new();
        s.push_str(&format!("# {}\n\n", title));
        s.push_str("## Executive Summary\n\n");
        s.push_str(&format!("- **Pass@1 Solve Rate**: {:.1}% ({}/{})\n", m.pass_at_1 * 100.0, m.passed_tasks, m.total_tasks));
        s.push_str(&format!("- **Token Consumption**: {} (avg {:.0} / task)\n", m.total_tokens, m.avg_tokens_per_task));
        s.push_str(&format!("- **Context Bloat Ratio**: {:.2}\n", m.avg_context_bloat_ratio));
        s.push_str(&format!("- **Tool Loops**: {}\n\n", m.total_tool_loops));

        s.push_str("## Domain Performance\n\n");
        s.push_str("| Domain | Solved | Pass@1 | Avg Tokens | Bloat Ratio | Loops |\n");
        s.push_str("|---|---|---|---|---|---|\n");
        for d in Domain::ALL {
            if let Some(dm) = m.domain_breakdown.get(&d) {
                s.push_str(&format!(
                    "| {} | {}/{} | {:.1}% | {:.0} | {:.2} | {} |\n",
                    d.display_name(),
                    dm.passed,
                    dm.total,
                    dm.pass_at_1 * 100.0,
                    dm.avg_tokens,
                    dm.avg_bloat_ratio,
                    dm.tool_loops
                ));
            }
        }
        s.push('\n');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_computation() {
        let mut collector = MetricsCollector::new();
        collector.record(TaskResult {
            task_id: "rust-01-en".into(),
            domain: Domain::Rust,
            language: "en".into(),
            difficulty: Difficulty::Trivial,
            passed: true,
            turns_taken: 1,
            prompt_tokens: 1000,
            completion_tokens: 200,
            total_tokens: 1200,
            raw_context_tokens: 2000,
            assembled_context_tokens: 800,
            context_bloat_ratio: 0.4,
            tool_loop_count: 0,
            duration_ms: 15,
        });

        collector.record(TaskResult {
            task_id: "rust-01-th".into(),
            domain: Domain::Rust,
            language: "th".into(),
            difficulty: Difficulty::Trivial,
            passed: true,
            turns_taken: 1,
            prompt_tokens: 1100,
            completion_tokens: 220,
            total_tokens: 1320,
            raw_context_tokens: 2200,
            assembled_context_tokens: 880,
            context_bloat_ratio: 0.4,
            tool_loop_count: 0,
            duration_ms: 18,
        });

        let metrics = collector.compute();
        assert_eq!(metrics.total_tasks, 2);
        assert_eq!(metrics.passed_tasks, 2);
        assert!((metrics.pass_at_1 - 1.0).abs() < 1e-6);
        assert_eq!(metrics.total_tool_loops, 0);
        assert_eq!(metrics.total_tokens, 2520);
    }
}
