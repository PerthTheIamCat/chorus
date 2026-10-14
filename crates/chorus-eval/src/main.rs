//! Binary CLI for Chorus Harness Evaluation Suite.
//!
//! Subcommands:
//!   chorus-eval tier0   - Run Tier 0 Invariants Runner (<2s)
//!   chorus-eval tier1   - Run Tier 1 Scaffolding & Router Gate Runner (<30s)
//!   chorus-eval all     - Run both Tier 0 and Tier 1

use anyhow::Result;
use std::process::exit;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");

    match cmd {
        "tier0" => {
            let report = chorus_eval::suite::run_tier0()?;
            if !report.passed {
                eprintln!("Tier 0 Invariants Runner failed.");
                exit(1);
            }
        }
        "tier1" => {
            let report = chorus_eval::suite::run_tier1()?;
            if !report.passed {
                eprintln!("Tier 1 Scaffolding & Router Gate Runner failed.");
                exit(1);
            }
        }
        "tier2" => {
            let report = chorus_eval::tier2::run_tier2()?;
            if !report.passed {
                eprintln!("Tier 2 Live Worktree & Swarm Runner failed.");
                exit(1);
            }
        }
        "all" => {
            let r0 = chorus_eval::suite::run_tier0()?;
            if !r0.passed {
                eprintln!("Tier 0 failed; aborting.");
                exit(1);
            }
            let r1 = chorus_eval::suite::run_tier1()?;
            if !r1.passed {
                eprintln!("Tier 1 failed; aborting.");
                exit(1);
            }
            let r2 = chorus_eval::tier2::run_tier2()?;
            if !r2.passed {
                eprintln!("Tier 2 failed.");
                exit(1);
            }
        }
        "help" | "--help" | "-h" => {
            println!("Chorus Harness Evaluation Suite CLI");
            println!("Version: {}\n", env!("CARGO_PKG_VERSION"));
            println!("USAGE:");
            println!("    chorus-eval <COMMAND>\n");
            println!("COMMANDS:");
            println!("    tier0    Run Tier 0 Invariants Runner (<2s) - dedup, context assembly, zero tool loops");
            println!("    tier1    Run Tier 1 Scaffolding & Router Gate (<30s) - complexity, verification, prompt assembly");
            println!("    all      Run both Tier 0 and Tier 1 suites end-to-end");
            println!("    help     Show this help message");
        }
        other => {
            eprintln!("Unknown subcommand: '{}'", other);
            eprintln!("Run `chorus-eval --help` for available commands.");
            exit(1);
        }
    }

    Ok(())
}
