//! Tier 0 Invariants Runner (<2s).
//!
//! Validates core system invariants:
//! 1. Content-addressed blob deduplication (multiple reads -> single blob row).
//! 2. Context assembly constraints (task state never trimmed, demotion precedes dropping).
//! 3. Zero tolerance for no-action tool loops.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Instant;

use chorus_core::actor;
use chorus_core::context::{AssembleInput, Assembler, Budget, TurnItem};
use chorus_core::ledger;
use chorus_core::memory;
use chorus_core::render::{self, ContextItem, FileChange};
use chorus_core::tokens::{CachedCounter, HeuristicCounter};
use chorus_core::{EventKind, NewEvent, Store};

use crate::loop_detector::{detect_tool_loops, has_tool_loops, ToolCallRecord};

/// Outcome report for the Tier 0 Invariants run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tier0Report {
    pub passed: bool,
    pub dedup_passed: bool,
    pub dedup_savings_pct: f64,
    pub blob_rows: usize,
    pub context_assembly_passed: bool,
    pub assemble_duration_ms: u128,
    pub task_state_preserved: bool,
    pub zero_tool_loops_passed: bool,
    pub loop_detection_verified: bool,
    pub total_duration_ms: u128,
    pub failure_reasons: Vec<String>,
}

/// Run Tier 0 invariants evaluation (<2s budget).
pub fn run_tier0() -> Result<Tier0Report> {
    let started = Instant::now();
    let mut failures = Vec::new();

    println!("================================================================================");
    println!("              CHORUS HARNESS EVAL: TIER 0 INVARIANTS RUNNER (<2s)               ");
    println!("================================================================================");

    let temp_dir = tempfile::tempdir().context("create tempdir for tier0")?;
    let db_path = temp_dir.path().join("tier0.db");
    let workspace_path = temp_dir.path().join("workspace");
    std::fs::create_dir_all(&workspace_path)?;

    // -------------------------------------------------------------------------
    // 1. BLOB DEDUPLICATION INVARIANT
    // -------------------------------------------------------------------------
    print!("  [1/3] Checking blob deduplication invariant... ");
    let store = Store::open(&db_path).context("open sqlite store")?;
    let c = store.conn();

    let claude = actor::intern(c, "claude-opus", "anthropic")?;
    let user = actor::intern(c, "user", "human")?;
    let sess = ledger::new_session(c, "แก้ N+1 query ในหน้า order list", "/tmp/project")?;

    let file_content = generate_dummy_file("src/order/repo.ts", 600);
    let reads = 8;
    ledger::append(
        c,
        &NewEvent::new(
            sess,
            EventKind::Msg,
            user,
            "หน้า order list ช้ามาก ช่วยดูให้หน่อย",
        ),
    )?;

    for _ in 0..reads {
        ledger::append(
            c,
            &NewEvent::new(sess, EventKind::ToolCall, claude, "read src/order/repo.ts"),
        )?;
        ledger::append(
            c,
            &NewEvent::new(sess, EventKind::ToolResult, claude, file_content.clone()),
        )?;
    }

    let blob_rows: i64 = c.query_row("SELECT count(*) FROM blob", [], |r| r.get(0))?;
    let blob_bytes: i64 =
        c.query_row("SELECT coalesce(sum(length(bytes)), 0) FROM blob", [], |r| {
            r.get(0)
        })?;
    let naive_bytes = file_content.len() as i64 * reads as i64;
    let savings_pct = 100.0 - (blob_bytes as f64 / naive_bytes as f64) * 100.0;

    let dedup_passed = blob_rows == 1 && savings_pct > 70.0;
    if dedup_passed {
        println!("PASS (1 row stored, {:.1}% storage saved)", savings_pct);
    } else {
        println!("FAIL (blob_rows={}, savings={:.1}%)", blob_rows, savings_pct);
        failures.push(format!(
            "Blob dedup invariant failed: expected 1 blob row, got {}; savings {:.1}%",
            blob_rows, savings_pct
        ));
    }

    // -------------------------------------------------------------------------
    // 2. CONTEXT ASSEMBLY CONSTRAINTS INVARIANT
    // -------------------------------------------------------------------------
    print!("  [2/3] Checking context assembly constraints... ");
    let assemble_start = Instant::now();

    for f in [
        "ใช้ pnpm ไม่ใช่ npm; รัน test ด้วย `pnpm test:order`",
        "Postgres 16, migration อยู่ใน migrations/ เรียงเลขสี่หลัก",
        "ห้ามแก้ schema โดยไม่ถามก่อน",
    ] {
        memory::add(c, 1, f)?;
    }
    let facts = memory::top(c, 1, 8)?;
    let facts_rendered = memory::render(&facts);

    let events = ledger::tail(c, sess, 200)?;
    let turns: Vec<TurnItem> = events
        .iter()
        .rev()
        .enumerate()
        .map(|(age, e)| {
            let body = ledger::body(c, e).unwrap_or_default();
            TurnItem {
                text_l1: Some(format!(
                    "[สรุป] {}",
                    body.chars().take(80).collect::<String>()
                )),
                text_l2: Some("[สรุปช่วงงาน] สืบสาเหตุ N+1".into()),
                text_l0: body,
                age,
                key: e.blob.as_ref().map(|b| b.0.clone()),
            }
        })
        .collect();

    let code = vec![
        ContextItem {
            text: file_content.clone(),
            key: Some("b3:repo".into()),
            label: "src/order/repo.ts".into(),
        },
        ContextItem {
            text: render::file_manifest(&[FileChange {
                path: "src/order/repo.ts".into(),
                status: "modified".into(),
                added: 38,
                removed: 11,
            }]),
            key: None,
            label: "diff".into(),
        },
    ];

    let task_state_text = "goal: ทำหน้า order list ให้ต่ำกว่า 200ms\ndone: เจอ N+1 ใน findAll\npending: eager-load, รัน pnpm test:order";
    let input = AssembleInput {
        system: "You are a coding agent working in a Postgres/TypeScript repo.".into(),
        task_state: task_state_text.into(),
        memory: facts_rendered
            .lines()
            .map(|l| ContextItem {
                text: l.to_string(),
                key: None,
                label: "fact".into(),
            })
            .collect(),
        turns,
        code,
    };

    let mut task_state_preserved = true;
    let mut demotion_precedes_dropping = true;

    // Test across tight, medium, and large budgets
    for budget in [6_000_i64, 24_000, 200_000] {
        let assembler = Assembler::new(Budget::new(budget), CachedCounter::new(HeuristicCounter));
        let out = assembler.assemble(&input);

        if !out.text.contains("goal: ทำหน้า order list") {
            task_state_preserved = false;
        }
        // Under tight budget (6000), turns must be demoted before being dropped
        if budget == 6_000 && out.demoted_turns == 0 && out.dropped_turns > 0 {
            demotion_precedes_dropping = false;
        }
    }

    let assemble_duration = assemble_start.elapsed().as_millis();
    let assemble_fast = assemble_duration < 2000; // must be < 2s

    let context_assembly_passed = task_state_preserved && demotion_precedes_dropping && assemble_fast;
    if context_assembly_passed {
        println!("PASS (assembled in {}ms, task state preserved)", assemble_duration);
    } else {
        println!("FAIL (assemble duration {}ms)", assemble_duration);
        if !task_state_preserved {
            failures.push("Context assembly failed: task state lost under budget".into());
        }
        if !demotion_precedes_dropping {
            failures.push("Context assembly failed: dropping turns without prior demotion".into());
        }
        if !assemble_fast {
            failures.push(format!("Context assembly took too long: {}ms >= 2000ms", assemble_duration));
        }
    }

    // -------------------------------------------------------------------------
    // 3. ZERO TOOL LOOPS INVARIANT
    // -------------------------------------------------------------------------
    print!("  [3/3] Checking zero tool loop invariants... ");
    let mut zero_tool_loops_passed = true;
    let mut loop_detection_verified = true;

    // 3a. Legitimate multi-turn sequence has zero loops
    let legitimate_calls = vec![
        ToolCallRecord::new("read_file", "src/auth/errors.rs"),
        ToolCallRecord::new("edit_file", "src/auth/errors.rs"),
        ToolCallRecord::new("cargo_test", "--test auth"),
    ];
    if has_tool_loops(&legitimate_calls) {
        zero_tool_loops_passed = false;
        failures.push("False positive tool loop reported on legitimate call trace".into());
    }

    // 3b. Synthetic loop (> 2 identical calls) must be immediately caught
    let loop_calls = vec![
        ToolCallRecord::new("read_file", "src/auth/errors.rs"),
        ToolCallRecord::new("read_file", "src/auth/errors.rs"),
        ToolCallRecord::new("read_file", "src/auth/errors.rs"),
    ];
    let violations = detect_tool_loops(&loop_calls);
    if violations.is_empty() || violations[0].repetitions < 3 {
        loop_detection_verified = false;
        failures.push("Loop detector failed to catch 3 identical consecutive calls".into());
    }

    let loop_test_passed = zero_tool_loops_passed && loop_detection_verified;
    if loop_test_passed {
        println!("PASS (zero loops on normal trace; detector verified)");
    } else {
        println!("FAIL");
    }

    // -------------------------------------------------------------------------
    // SUMMARY
    // -------------------------------------------------------------------------
    let total_duration = started.elapsed().as_millis();
    let passed = dedup_passed && context_assembly_passed && loop_test_passed && total_duration < 2000;

    println!("--------------------------------------------------------------------------------");
    if passed {
        println!("✓ ALL TIER 0 INVARIANTS PASSED in {}ms (<2s SLA met)", total_duration);
    } else {
        println!("✗ TIER 0 INVARIANTS FAILED in {}ms", total_duration);
        for f in &failures {
            println!("  - Error: {}", f);
        }
    }
    println!("================================================================================\n");

    Ok(Tier0Report {
        passed,
        dedup_passed,
        dedup_savings_pct: savings_pct,
        blob_rows: blob_rows as usize,
        context_assembly_passed,
        assemble_duration_ms: assemble_duration,
        task_state_preserved,
        zero_tool_loops_passed,
        loop_detection_verified,
        total_duration_ms: total_duration,
        failure_reasons: failures,
    })
}

fn generate_dummy_file(path: &str, lines: usize) -> String {
    let mut out = String::new();
    out.push_str(&format!("// Source file: {}\n", path));
    out.push_str("use std::collections::HashMap;\n\n");
    for i in 0..lines {
        out.push_str(&format!(
            "pub fn process_entry_{i}(id: u64, payload: &str) -> Result<(), &'static str> {{\n    Ok(())\n}}\n"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tier0_invariants_pass() {
        let report = run_tier0().expect("tier0 execution should succeed");
        assert!(report.passed, "tier 0 invariants must pass: {:?}", report.failure_reasons);
        assert!(report.total_duration_ms < 2000, "tier 0 must complete in < 2 seconds");
        assert_eq!(report.blob_rows, 1);
        assert!(report.dedup_savings_pct > 70.0);
        assert!(report.task_state_preserved);
    }
}
