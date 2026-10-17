//! Closed-Loop Autonomous Verification & Self-Repair Engine.
//!
//! Executes lightning-fast post-edit compiler checks and linters (`cargo check`, `biome`,
//! `ruff`, `swiftlint`), parses diagnostics, and automatically constructs self-healing
//! error feedback to steer the agent before human review.
//!
//! Every checker runs the project's own code (build scripts, proc-macros,
//! linter plugins), so it only runs with a [`Consent`]: the user trusted the
//! project and the command is wrapped in the turn's sandbox profile.

use crate::trust::{which, Consent};
use crate::{ChorusError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Output;

/// A compiler or linter diagnostic item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticItem {
    pub file: String,
    pub line: usize,
    pub col: usize,
    pub severity: String, // "error", "warning"
    pub message: String,
}

/// Verification result for a workspace edit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub passed: bool,
    pub checker_used: String,
    pub duration_ms: u64,
    pub diagnostics: Vec<DiagnosticItem>,
    pub raw_output: String,
}

impl VerificationResult {
    /// Formats diagnostics into an actionable self-healing prompt for the agent.
    pub fn to_agent_feedback(&self) -> String {
        if self.passed {
            return "Verification passed cleanly with no compiler or linter errors.".to_string();
        }

        let mut out = format!(
            "Auto-verification failed with {} via {}:\n\n",
            if self.diagnostics.len() == 1 { "1 error" } else { "errors" },
            self.checker_used
        );

        for diag in &self.diagnostics {
            out.push_str(&format!(
                "- [{}] {}:{}:{} - {}\n",
                diag.severity.to_uppercase(),
                diag.file,
                diag.line,
                diag.col,
                diag.message
            ));
        }

        out.push_str("\nPlease repair these syntax or compile errors before concluding the turn.");
        out
    }
}

pub struct VerifyLoop;

/// Run `tool args…` under `consent`, or `None` when the tool is not installed.
fn run_tool(consent: &Consent, tool: &str, args: &[&str]) -> Result<Option<Output>> {
    let Some(bin) = which(tool) else { return Ok(None) };
    let mut argv = vec![bin.to_string_lossy().into_owned()];
    argv.extend(args.iter().map(|a| a.to_string()));
    let out = consent
        .command(&argv)?
        .output()
        .map_err(|e| ChorusError::Invalid(format!("Failed to run {tool}: {e}")))?;
    Ok(Some(out))
}

impl VerifyLoop {
    /// Detects project tooling and runs the fastest verification command,
    /// confined by `consent` (see [`crate::trust`]).
    pub fn verify_workspace(consent: &Consent) -> Result<VerificationResult> {
        let start = std::time::Instant::now();
        let workspace_root: &Path = consent.workspace();

        if workspace_root.join("Cargo.toml").exists() {
            Self::verify_rust(consent, start)
        } else if workspace_root.join("package.json").exists() {
            Self::verify_node(consent, start)
        } else if workspace_root.join("pyproject.toml").exists() || workspace_root.join("requirements.txt").exists() {
            Self::verify_python(consent, start)
        } else if workspace_root.join("Package.swift").exists() {
            Self::verify_swift(consent, start)
        } else {
            Ok(VerificationResult {
                passed: true,
                checker_used: "none".to_string(),
                duration_ms: start.elapsed().as_millis() as u64,
                diagnostics: Vec::new(),
                raw_output: String::new(),
            })
        }
    }

    fn verify_rust(consent: &Consent, start: std::time::Instant) -> Result<VerificationResult> {
        let output = run_tool(consent, "cargo", &["check", "--message-format=json", "--quiet"])?
            .ok_or_else(|| ChorusError::Invalid("Failed to run cargo check: cargo not found on PATH".into()))?;

        let raw = String::from_utf8_lossy(&output.stdout).to_string();
        let mut diagnostics = Vec::new();

        for line in raw.lines() {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                if val["reason"] == "compiler-message" {
                    let msg = &val["message"];
                    let level = msg["level"].as_str().unwrap_or("info");
                    if level == "error" {
                        let text = msg["message"].as_str().unwrap_or("").to_string();
                        let span = msg["spans"].as_array().and_then(|a| a.first());
                        let (file, line_num, col_num) = if let Some(sp) = span {
                            (
                                sp["file_name"].as_str().unwrap_or("").to_string(),
                                sp["line_start"].as_u64().unwrap_or(0) as usize,
                                sp["column_start"].as_u64().unwrap_or(0) as usize,
                            )
                        } else {
                            (String::new(), 0, 0)
                        };

                        diagnostics.push(DiagnosticItem {
                            file,
                            line: line_num,
                            col: col_num,
                            severity: "error".to_string(),
                            message: text,
                        });
                    }
                }
            }
        }

        let passed = output.status.success() && diagnostics.is_empty();
        Ok(VerificationResult {
            passed,
            checker_used: "cargo check".to_string(),
            duration_ms: start.elapsed().as_millis() as u64,
            diagnostics,
            raw_output: raw,
        })
    }

    fn verify_node(consent: &Consent, start: std::time::Instant) -> Result<VerificationResult> {
        // Try biome if available
        if let Some(out) = run_tool(consent, "biome", &["check"])? {
            let passed = out.status.success();
            let raw = String::from_utf8_lossy(&out.stdout).to_string();
            return Ok(VerificationResult {
                passed,
                checker_used: "biome".to_string(),
                duration_ms: start.elapsed().as_millis() as u64,
                diagnostics: Vec::new(),
                raw_output: raw,
            });
        }

        Ok(VerificationResult {
            passed: true,
            checker_used: "node-generic".to_string(),
            duration_ms: start.elapsed().as_millis() as u64,
            diagnostics: Vec::new(),
            raw_output: String::new(),
        })
    }

    fn verify_python(consent: &Consent, start: std::time::Instant) -> Result<VerificationResult> {
        if let Some(out) = run_tool(consent, "ruff", &["check"])? {
            let passed = out.status.success();
            let raw = String::from_utf8_lossy(&out.stdout).to_string();
            return Ok(VerificationResult {
                passed,
                checker_used: "ruff".to_string(),
                duration_ms: start.elapsed().as_millis() as u64,
                diagnostics: Vec::new(),
                raw_output: raw,
            });
        }

        Ok(VerificationResult {
            passed: true,
            checker_used: "python-generic".to_string(),
            duration_ms: start.elapsed().as_millis() as u64,
            diagnostics: Vec::new(),
            raw_output: String::new(),
        })
    }

    fn verify_swift(consent: &Consent, start: std::time::Instant) -> Result<VerificationResult> {
        if let Some(out) = run_tool(consent, "swiftlint", &["lint", "--quiet"])? {
            let passed = out.status.success();
            let raw = String::from_utf8_lossy(&out.stdout).to_string();
            return Ok(VerificationResult {
                passed,
                checker_used: "swiftlint".to_string(),
                duration_ms: start.elapsed().as_millis() as u64,
                diagnostics: Vec::new(),
                raw_output: raw,
            });
        }

        Ok(VerificationResult {
            passed: true,
            checker_used: "swift-generic".to_string(),
            duration_ms: start.elapsed().as_millis() as u64,
            diagnostics: Vec::new(),
            raw_output: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trust::TrustStore;

    fn identity(argv: &[String], _: &Path) -> std::result::Result<Vec<String>, String> {
        Ok(argv.to_vec())
    }

    /// The P1 escape: opening a Rust repo and ending a turn ran its `build.rs`.
    /// Without trust there is no `Consent`, so `verify_workspace` cannot be
    /// called at all; with trust, every command goes through the confine hook.
    #[test]
    fn verify_runs_nothing_without_trust_and_always_confines() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        std::fs::write(ws.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let store = TrustStore::at(home.path().join("t.json"));
        assert!(store.consent(ws.path(), &identity).is_err());

        store.set_trusted(ws.path(), true).unwrap();
        let seen = std::cell::RefCell::new(Vec::<Vec<String>>::new());
        let refuse = |argv: &[String], _: &Path| {
            seen.borrow_mut().push(argv.to_vec());
            Err("test: sandbox refused".to_string())
        };
        let consent = store.consent(ws.path(), &refuse).unwrap();
        if which("cargo").is_some() {
            assert!(VerifyLoop::verify_workspace(&consent).is_err(), "must fail closed");
            assert_eq!(seen.borrow().len(), 1);
            assert!(seen.borrow()[0][0].ends_with("cargo"));
        }
    }

    #[test]
    fn format_agent_feedback_renders_cleanly() {
        let res = VerificationResult {
            passed: false,
            checker_used: "cargo check".into(),
            duration_ms: 120,
            diagnostics: vec![DiagnosticItem {
                file: "src/main.rs".into(),
                line: 42,
                col: 10,
                severity: "error".into(),
                message: "cannot find value `foo` in this scope".into(),
            }],
            raw_output: "".into(),
        };

        let feedback = res.to_agent_feedback();
        assert!(feedback.contains("src/main.rs:42:10"));
        assert!(feedback.contains("cannot find value `foo`"));
    }
}
