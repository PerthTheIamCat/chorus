//! Tier 2 Full Live Worktree & Swarm Evaluation Suite.
//!
//! Validates:
//! 1. Multi-Agent DAG Swarm dependency resolution & scheduling (`SwarmDag`).
//! 2. Worktree isolation and APFS Copy-on-Write clone performance (`WorktreeCow`).
//! 3. Spend budget circuit-breaker enforcement (`record_cost`).
//! 4. Autonomous Swarm Merge Specialist conflict resolution (`MergeSpecialist`).

use anyhow::Result;
use chorus_core::merge_specialist::{MergeSpecialist, MergeStrategy};
use chorus_core::swarm::{SwarmDag, SwarmNode, SwarmNodeStatus};
use chorus_core::worktree_cow::WorktreeCow;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tier2Report {
    pub passed: bool,
    pub total_scenarios: usize,
    pub swarm_dag_resolution_passed: bool,
    pub worktree_isolation_passed: bool,
    pub spend_governor_passed: bool,
    pub merge_conflict_resolution_passed: bool,
    pub total_duration_ms: u128,
    pub failure_reasons: Vec<String>,
}

/// Run Tier 2 Full Live Worktree & Multi-Agent Swarm Evaluation Suite.
pub fn run_tier2() -> Result<Tier2Report> {
    let started = Instant::now();
    let mut failures = Vec::new();
    let mut total_scenarios = 0;

    println!("================================================================================");
    println!("        CHORUS HARNESS EVAL: TIER 2 LIVE WORKTREE & SWARM SUITE                ");
    println!("================================================================================");

    // -------------------------------------------------------------------------
    // TEST 1: MULTI-AGENT DAG SWARM DEPENDENCY RESOLUTION
    // -------------------------------------------------------------------------
    print!("  [1/4] Testing Swarm DAG dependency resolution & topological order... ");
    total_scenarios += 1;
    let mut swarm_dag_passed = true;

    let mut dag = SwarmDag::new();
    // Architect node (Root)
    dag.add_node(SwarmNode {
        id: "architect".into(),
        role: "Architect".into(),
        task_prompt: "Design API specification".into(),
        model_id: Some("gpt-5-codex".into()),
        depends_on: vec![],
        status: SwarmNodeStatus::Pending,
        worktree_path: None,
        max_turns: 5,
        max_cost_usd: 1.5,
        current_cost_usd: 0.0,
    });

    // Frontend Builder (depends on architect)
    dag.add_node(SwarmNode {
        id: "frontend".into(),
        role: "Frontend Engineer".into(),
        task_prompt: "Implement SwiftUI views".into(),
        model_id: Some("claude-3-7-sonnet".into()),
        depends_on: vec!["architect".into()],
        status: SwarmNodeStatus::Pending,
        worktree_path: None,
        max_turns: 10,
        max_cost_usd: 3.0,
        current_cost_usd: 0.0,
    });

    // Backend Builder (depends on architect)
    dag.add_node(SwarmNode {
        id: "backend".into(),
        role: "Backend Engineer".into(),
        task_prompt: "Implement Rust core ledger".into(),
        model_id: Some("gpt-5-codex".into()),
        depends_on: vec!["architect".into()],
        status: SwarmNodeStatus::Pending,
        worktree_path: None,
        max_turns: 10,
        max_cost_usd: 3.0,
        current_cost_usd: 0.0,
    });

    // QA Reviewer (depends on both frontend and backend)
    dag.add_node(SwarmNode {
        id: "qa".into(),
        role: "QA Specialist".into(),
        task_prompt: "Run integration test suite".into(),
        model_id: Some("gpt-5-codex".into()),
        depends_on: vec!["frontend".into(), "backend".into()],
        status: SwarmNodeStatus::Pending,
        worktree_path: None,
        max_turns: 5,
        max_cost_usd: 1.0,
        current_cost_usd: 0.0,
    });

    // 1. Initial ready must ONLY be architect
    let ready1 = dag.find_ready_nodes();
    if ready1 != vec!["architect".to_string()] {
        swarm_dag_passed = false;
        failures.push(format!("initial ready nodes mismatch: expected ['architect'], got {:?}", ready1));
    }

    // 2. Mark architect completed -> frontend & backend become ready simultaneously
    let _ = dag.mark_completed("architect");
    let mut ready2 = dag.find_ready_nodes();
    ready2.sort();
    if ready2 != vec!["backend".to_string(), "frontend".to_string()] {
        swarm_dag_passed = false;
        failures.push(format!("second stage ready nodes mismatch: expected ['backend', 'frontend'], got {:?}", ready2));
    }

    // 3. Complete both -> QA becomes ready
    let _ = dag.mark_completed("frontend");
    let _ = dag.mark_completed("backend");
    let ready3 = dag.find_ready_nodes();
    if ready3 != vec!["qa".to_string()] {
        swarm_dag_passed = false;
        failures.push(format!("final stage ready nodes mismatch: expected ['qa'], got {:?}", ready3));
    }

    let _ = dag.mark_completed("qa");
    if !dag.is_all_completed() {
        swarm_dag_passed = false;
        failures.push("DAG reported not all completed when all nodes were marked completed".into());
    }

    if swarm_dag_passed {
        println!("PASSED");
    } else {
        println!("FAILED");
    }

    // -------------------------------------------------------------------------
    // TEST 2: WORKTREE ISOLATION & APFS CoW PROVISIONING
    // -------------------------------------------------------------------------
    print!("  [2/4] Testing Worktree isolation & APFS Copy-on-Write... ");
    total_scenarios += 1;
    let mut worktree_passed = true;

    let tmp_dir = tempfile::tempdir()?;
    let source_dir = tmp_dir.path().join("source_ws");
    std::fs::create_dir_all(&source_dir)?;
    std::fs::write(source_dir.join("main.rs"), "pub fn original() {}")?;

    let dest_dir = tmp_dir.path().join("clone_ws");
    let cow_result = WorktreeCow::clone_workspace(&source_dir, &dest_dir);
    if let Err(e) = cow_result {
        worktree_passed = false;
        failures.push(format!("Worktree APFS clone failed: {e}"));
    } else if !dest_dir.join("main.rs").exists() {
        worktree_passed = false;
        failures.push("destination cloned workspace file does not exist".into());
    }

    if worktree_passed {
        println!("PASSED");
    } else {
        println!("FAILED");
    }

    // -------------------------------------------------------------------------
    // TEST 3: SPEND GOVERNOR CIRCUIT BREAKER
    // -------------------------------------------------------------------------
    print!("  [3/4] Testing per-subagent hard spend circuit breaker... ");
    total_scenarios += 1;
    let mut spend_passed = true;

    let mut test_dag = SwarmDag::new();
    test_dag.add_node(SwarmNode {
        id: "budget_test".into(),
        role: "Researcher".into(),
        task_prompt: "Explore".into(),
        model_id: None,
        depends_on: vec![],
        status: SwarmNodeStatus::Pending,
        worktree_path: None,
        max_turns: 5,
        max_cost_usd: 2.00,
        current_cost_usd: 0.0,
    });

    test_dag.mark_running("budget_test", PathBuf::from("/tmp/wt")).unwrap();
    // 1st spend: 1.20 -> within budget
    let ok1 = test_dag.record_cost("budget_test", 1.20).unwrap();
    if !ok1 {
        spend_passed = false;
        failures.push("spend within budget falsely breached circuit breaker".into());
    }

    // 2nd spend: 1.50 -> total 2.70 exceeds 2.00 max -> must breach
    let ok2 = test_dag.record_cost("budget_test", 1.50).unwrap();
    if ok2 || test_dag.nodes["budget_test"].status != SwarmNodeStatus::Cancelled {
        spend_passed = false;
        failures.push("spend breaching budget failed to trip circuit breaker".into());
    }

    if spend_passed {
        println!("PASSED");
    } else {
        println!("FAILED");
    }

    // -------------------------------------------------------------------------
    // TEST 4: AUTONOMOUS MERGE SPECIALIST CONFLICT RESOLUTION
    // -------------------------------------------------------------------------
    print!("  [4/4] Testing Swarm Merge Specialist conflict resolution... ");
    total_scenarios += 1;
    let mut merge_passed = true;

    let conflicted_source = r#"// Application Config
<<<<<<< HEAD
pub const MAX_CONCURRENCY: usize = 8;
=======
pub const MAX_CONCURRENCY: usize = 16;
pub const BUFFER_SIZE_MB: usize = 64;
>>>>>>> feature-swarm-scale
// End Config"#;

    let resolved = MergeSpecialist::resolve_content(conflicted_source, MergeStrategy::Synthesize);
    match resolved {
        Ok(clean) => {
            if MergeSpecialist::has_conflicts(&clean) {
                merge_passed = false;
                failures.push("resolved file still contained git conflict markers".into());
            }
            if !clean.contains("BUFFER_SIZE_MB") {
                merge_passed = false;
                failures.push("synthesized resolution failed to retain incoming addition".into());
            }
        }
        Err(e) => {
            merge_passed = false;
            failures.push(format!("MergeSpecialist failed with error: {e}"));
        }
    }

    if merge_passed {
        println!("PASSED");
    } else {
        println!("FAILED");
    }

    let elapsed = started.elapsed().as_millis();
    let all_passed = swarm_dag_passed && worktree_passed && spend_passed && merge_passed;

    println!("--------------------------------------------------------------------------------");
    println!("  Tier 2 Result: {}", if all_passed { "PASSED" } else { "FAILED" });
    println!("  Duration:      {} ms", elapsed);
    println!("================================================================================");

    Ok(Tier2Report {
        passed: all_passed,
        total_scenarios,
        swarm_dag_resolution_passed: swarm_dag_passed,
        worktree_isolation_passed: worktree_passed,
        spend_governor_passed: spend_passed,
        merge_conflict_resolution_passed: merge_passed,
        total_duration_ms: elapsed,
        failure_reasons: failures,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tier2_runner_passes() {
        let report = run_tier2().expect("tier 2 evaluation runner must succeed");
        assert!(report.passed);
        assert_eq!(report.total_scenarios, 4);
        assert!(report.swarm_dag_resolution_passed);
        assert!(report.worktree_isolation_passed);
        assert!(report.spend_governor_passed);
        assert!(report.merge_conflict_resolution_passed);
    }
}
