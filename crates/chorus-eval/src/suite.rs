//! Eval suite definitions and runner orchestration.
//!
//! Provides Tier 0 (invariants), Tier 1 (scaffolding & router gate),
//! and full metrics telemetry.

pub use crate::classifier::{ClassificationResult, TaskClassifier, TaskKind};
pub use crate::loop_detector::{
    detect_tool_loops, has_tool_loops, LoopKind, ToolCallRecord, ToolLoopDetector,
    ToolLoopViolation,
};
pub use crate::metrics::{
    DifficultyMetrics, DomainMetrics, EvalMetrics, LanguageMetrics, MetricsCollector,
    TaskResult,
};
pub use crate::models::{
    default_tasks_dir, load_tasks_from_dir, Difficulty, Domain, HarnessTask,
};
pub use crate::tier0::{run_tier0, Tier0Report};
pub use crate::tier1::{run_tier1, Tier1Report};
pub use crate::verification::{
    TriggerDecision, VerificationLoop, VerificationOutcome, VerificationPolicy,
};

/// Combined evaluation run executing both Tier 0 and Tier 1.
pub fn run_all() -> anyhow::Result<bool> {
    let t0 = run_tier0()?;
    if !t0.passed {
        return Ok(false);
    }

    let t1 = run_tier1()?;
    Ok(t1.passed)
}
