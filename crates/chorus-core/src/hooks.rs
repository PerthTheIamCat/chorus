//! Automated verification hooks engine.
//!
//! Ticket 03 & 21: Pre-dispatch and post-turn hooks in `.chorus/hooks/`
//! (e.g. `pre-dispatch.sh`, `post-turn.sh` running `cargo check`, linters,
//! or test suites).
//!
//! When `post-turn.sh` fails, stdout and stderr are captured and returned
//! as a structured verification failure feedback message to guide the agent
//! to self-heal without human intervention.
//!
//! Hook scripts come from the repo, so running one needs a [`Consent`]: the
//! user trusted the project, and the script runs inside the turn's sandbox
//! profile. Cloning a repo never runs its hooks.

use crate::trust::Consent;
use crate::{ChorusError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Max bytes of stdout/stderr to retain before truncating to avoid context blowout.
const MAX_HOOK_OUTPUT_BYTES: usize = 64 * 1024;

/// Lifecycle stages at which hooks can be triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookStage {
    /// Executed before dispatching a turn to an agent.
    PreDispatch,
    /// Executed after an agent completes a turn (verification loop: test, lint, typecheck).
    PostTurn,
}

impl HookStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PreDispatch => "pre-dispatch",
            Self::PostTurn => "post-turn",
        }
    }

    /// Primary script filename for this hook stage.
    pub fn script_name(&self) -> &'static str {
        match self {
            Self::PreDispatch => "pre-dispatch.sh",
            Self::PostTurn => "post-turn.sh",
        }
    }

    /// Alternative extensionless script filename.
    pub fn alt_script_name(&self) -> &'static str {
        match self {
            Self::PreDispatch => "pre-dispatch",
            Self::PostTurn => "post-turn",
        }
    }
}

/// Metadata and environment passed to the hook script.
#[derive(Debug, Clone, Default)]
pub struct HookContext {
    pub session_id: Option<i64>,
    pub agent_name: Option<String>,
    pub turn_number: Option<usize>,
    pub env_vars: Vec<(String, String)>,
}

impl HookContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_session(mut self, session_id: i64) -> Self {
        self.session_id = Some(session_id);
        self
    }

    pub fn with_agent(mut self, agent: impl Into<String>) -> Self {
        self.agent_name = Some(agent.into());
        self
    }

    pub fn with_turn(mut self, turn: usize) -> Self {
        self.turn_number = Some(turn);
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.env_vars.push((key.into(), val.into()));
        self
    }
}

/// The outcome of executing a hook script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookOutput {
    pub stage: HookStage,
    pub script_path: PathBuf,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

impl HookOutput {
    /// Did the hook succeed (exit code 0)?
    pub fn is_success(&self) -> bool {
        self.success
    }

    /// Formatted feedback message for model consumption when verification fails.
    /// Returns `None` if the hook succeeded.
    pub fn verification_feedback(&self) -> Option<String> {
        if self.success {
            return None;
        }

        let mut msg = String::new();
        msg.push_str(&format!(
            "### ⚠️ Verification Hook Failure: `{}`\n\n",
            self.stage.as_str()
        ));

        let code_str = match self.exit_code {
            Some(c) => format!("code {c}"),
            None => "terminated by signal".to_string(),
        };

        msg.push_str(&format!(
            "The verification hook `{}` exited with {}.\n\n",
            self.script_path.display(),
            code_str
        ));

        let stdout_trimmed = self.stdout.trim();
        let stderr_trimmed = self.stderr.trim();

        if !stdout_trimmed.is_empty() {
            msg.push_str("#### Hook Output (stdout):\n```\n");
            msg.push_str(stdout_trimmed);
            msg.push_str("\n```\n\n");
        }

        if !stderr_trimmed.is_empty() {
            msg.push_str("#### Hook Errors (stderr):\n```\n");
            msg.push_str(stderr_trimmed);
            msg.push_str("\n```\n\n");
        }

        if stdout_trimmed.is_empty() && stderr_trimmed.is_empty() {
            msg.push_str("(No stdout or stderr was produced by the hook script)\n\n");
        }

        msg.push_str("Please inspect the verification failure details above, address the errors or regressions, and ensure all checks pass.");
        Some(msg)
    }
}

/// Look for a hook script in `<workspace_root>/.chorus/hooks/` for the given stage.
pub fn find_hook(workspace_root: &Path, stage: HookStage) -> Option<PathBuf> {
    let hooks_dir = workspace_root.join(".chorus").join("hooks");
    if !hooks_dir.is_dir() {
        return None;
    }

    let primary = hooks_dir.join(stage.script_name());
    if primary.is_file() {
        return Some(primary);
    }

    let alt = hooks_dir.join(stage.alt_script_name());
    if alt.is_file() {
        return Some(alt);
    }

    None
}

/// Truncate long output if it exceeds `MAX_HOOK_OUTPUT_BYTES`.
fn truncate_hook_output(raw: &str) -> String {
    if raw.len() <= MAX_HOOK_OUTPUT_BYTES {
        return raw.to_string();
    }

    // Cut on char boundaries: Thai is 3 bytes per char, and a byte-offset
    // slice through one panics.
    let mut head_end = MAX_HOOK_OUTPUT_BYTES / 2;
    while !raw.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = raw.len() - MAX_HOOK_OUTPUT_BYTES / 2;
    while !raw.is_char_boundary(tail_start) {
        tail_start += 1;
    }

    let head = &raw[..head_end];
    let tail = &raw[tail_start..];

    format!("{head}\n\n... [output truncated: exceeded 64KB] ...\n\n{tail}")
}

/// Execute a hook script if present in `<workspace>/.chorus/hooks/`, where
/// the workspace is the one `consent` was given for, inside its sandbox.
/// Returns `Ok(None)` if no script exists for this stage.
pub fn run_hook(
    consent: &Consent,
    stage: HookStage,
    context: &HookContext,
) -> Result<Option<HookOutput>> {
    let workspace_root = consent.workspace();
    let Some(script_path) = find_hook(workspace_root, stage) else {
        return Ok(None);
    };

    let start = Instant::now();

    // Determine how to run the hook script.
    // If it has a .sh extension, run with `/bin/sh`. Otherwise, run directly.
    let script = script_path.to_string_lossy().into_owned();
    let argv = if script_path.extension().and_then(|e| e.to_str()) == Some("sh") {
        vec!["/bin/sh".to_string(), script]
    } else {
        vec![script]
    };
    let mut cmd = consent.command(&argv)?;

    // Standard environment variables
    cmd.env("CHORUS_HOOK_STAGE", stage.as_str());
    cmd.env(
        "CHORUS_WORKSPACE_ROOT",
        workspace_root.to_string_lossy().as_ref(),
    );

    if let Some(sess) = context.session_id {
        cmd.env("CHORUS_SESSION_ID", sess.to_string());
    }
    if let Some(agent) = &context.agent_name {
        cmd.env("CHORUS_AGENT_NAME", agent);
    }
    if let Some(turn) = context.turn_number {
        cmd.env("CHORUS_TURN_NUMBER", turn.to_string());
    }

    for (k, v) in &context.env_vars {
        cmd.env(k, v);
    }

    let output = cmd.output().map_err(|e| {
        ChorusError::Invalid(format!(
            "failed to execute hook {}: {e}",
            script_path.display()
        ))
    })?;

    let duration_ms = start.elapsed().as_millis() as u64;
    let success = output.status.success();
    let exit_code = output.status.code();

    let stdout_raw = String::from_utf8_lossy(&output.stdout);
    let stderr_raw = String::from_utf8_lossy(&output.stderr);

    let stdout = truncate_hook_output(&stdout_raw);
    let stderr = truncate_hook_output(&stderr_raw);

    Ok(Some(HookOutput {
        stage,
        script_path,
        success,
        exit_code,
        stdout,
        stderr,
        duration_ms,
    }))
}

/// Convenience runner for pre-dispatch hooks.
pub fn run_pre_dispatch(consent: &Consent, context: &HookContext) -> Result<Option<HookOutput>> {
    run_hook(consent, HookStage::PreDispatch, context)
}

/// Convenience runner for post-turn verification hooks.
pub fn run_post_turn(consent: &Consent, context: &HookContext) -> Result<Option<HookOutput>> {
    run_hook(consent, HookStage::PostTurn, context)
}

/// High-level engine managing hook execution for a workspace.
#[derive(Debug, Clone)]
pub struct HooksEngine {
    workspace_root: PathBuf,
}

impl HooksEngine {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn has_hook(&self, stage: HookStage) -> bool {
        find_hook(&self.workspace_root, stage).is_some()
    }

    /// Run a hook. `consent` must be for this engine's workspace.
    pub fn run(
        &self,
        consent: &Consent,
        stage: HookStage,
        context: &HookContext,
    ) -> Result<Option<HookOutput>> {
        if consent.workspace() != self.workspace_root {
            return Err(ChorusError::Invalid("consent is for a different workspace".into()));
        }
        run_hook(consent, stage, context)
    }

    pub fn run_pre_dispatch(&self, consent: &Consent, context: &HookContext) -> Result<Option<HookOutput>> {
        self.run(consent, HookStage::PreDispatch, context)
    }

    pub fn run_post_turn(&self, consent: &Consent, context: &HookContext) -> Result<Option<HookOutput>> {
        self.run(consent, HookStage::PostTurn, context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_cuts_on_char_boundaries() {
        // 'ก' is 3 bytes; 64 KiB / 2 is not a multiple of 3, so a byte slice
        // lands inside a char and used to panic.
        let thai = "ก".repeat(MAX_HOOK_OUTPUT_BYTES);
        let out = truncate_hook_output(&thai);
        assert!(out.contains("output truncated"));
        assert!(out.len() < thai.len());
        let mixed = format!("x{}", "ก".repeat(MAX_HOOK_OUTPUT_BYTES));
        truncate_hook_output(&mixed);
    }
}
