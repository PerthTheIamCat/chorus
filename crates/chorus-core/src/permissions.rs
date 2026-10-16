//! Granular Permissions Engine and Async Interaction State Machine.
//!
//! Tickets 03, 20 & 21: Granular approval levels mapped to the async `Interaction`
//! state machine.
//!
//! Classification logic:
//! - Read tools (read_file, search, glob, etc.) = Auto-approved in Ask-Each mode.
//! - Write / Bash tools (edit, patch, delete, bash, run_command, etc.) = Gated
//!   into an async `Interaction` approval request.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Categories of tool actions for permission classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolCategory {
    /// Read-only inspection (view_file, read, grep, glob, search, list).
    Read,
    /// Modification of files or state (write, edit, patch, delete, replace).
    Write,
    /// Arbitrary command execution (bash, sh, run_command, exec, terminal).
    Bash,
    /// Outbound network or web requests (curl, http, search_web).
    Network,
    /// Unclassified or custom tools.
    Other,
}

impl ToolCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Bash => "bash",
            Self::Network => "network",
            Self::Other => "other",
        }
    }
}

/// Classify a tool invocation based on tool name and optional tool kind.
pub fn classify_tool(tool_name: &str, kind: Option<&str>) -> ToolCategory {
    let name = tool_name.to_lowercase();
    let kind_lower = kind.map(str::to_lowercase);
    let k = kind_lower.as_deref().unwrap_or("");

    // Command execution is always Bash category even if command name hints at read
    if is_bash_tool(&name, k) {
        return ToolCategory::Bash;
    }

    if is_write_tool(&name, k) {
        return ToolCategory::Write;
    }

    if is_network_tool(&name, k) {
        return ToolCategory::Network;
    }

    if is_read_tool(&name, k) {
        return ToolCategory::Read;
    }

    ToolCategory::Other
}

fn is_bash_tool(name: &str, kind: &str) -> bool {
    matches!(
        name,
        "bash"
            | "sh"
            | "zsh"
            | "shell"
            | "terminal"
            | "cmd"
            | "exec"
            | "run_command"
            | "execute_command"
            | "command"
            | "pty"
    ) || name.contains("run_command")
        || name.contains("execute_command")
        || matches!(kind, "bash" | "command" | "terminal" | "execute" | "shell" | "exec")
}

fn is_write_tool(name: &str, kind: &str) -> bool {
    matches!(
        name,
        "write_file"
            | "write_to_file"
            | "edit_file"
            | "replace_file_content"
            | "patch_file"
            | "create_file"
            | "delete_file"
            | "remove_file"
            | "modify_file"
            | "save_file"
            | "rm"
            | "touch"
            | "mkdir"
            | "truncate"
            | "str_replace_editor"
    ) || name.starts_with("write_")
        || name.starts_with("edit_")
        || name.contains("replace")
        || name.contains("patch")
        || matches!(
            kind,
            "write" | "edit" | "delete" | "create" | "patch" | "modify" | "save"
        )
}

fn is_read_tool(name: &str, kind: &str) -> bool {
    matches!(
        name,
        "read_file"
            | "view_file"
            | "cat"
            | "grep"
            | "search"
            | "find_files"
            | "glob"
            | "list_directory"
            | "list_dir"
            | "dir"
            | "inspect"
            | "probe"
            | "read_url_content"
            | "fetch"
    ) || name.starts_with("read_")
        || name.starts_with("view_")
        || name.contains("grep")
        || name.contains("glob")
        || name.contains("search")
        || matches!(kind, "read" | "fetch" | "search" | "inspect" | "view" | "list")
}

fn is_network_tool(name: &str, kind: &str) -> bool {
    matches!(
        name,
        "http" | "curl" | "fetch_web" | "search_web" | "download" | "web"
    ) || name.contains("network")
        || name.contains("search_web")
        || matches!(kind, "network" | "web")
}

/// The state of an interaction approval request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InteractionState {
    /// Pending user or client review.
    Pending,
    /// Explicitly approved.
    Approved,
    /// Explicitly rejected.
    Rejected { reason: Option<String> },
}

/// An async interaction approval request for gated actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interaction {
    /// Unique identifier for this interaction.
    pub id: String,
    /// The session this interaction belongs to.
    pub session_id: i64,
    /// Classified category of the requested tool action.
    pub category: ToolCategory,
    /// Tool name requested.
    pub tool_name: String,
    /// Contextual detail (e.g. file path, command string, rationale).
    pub detail: String,
    /// Current state in the approval lifecycle.
    pub state: InteractionState,
    /// Creation timestamp (ms since UNIX epoch).
    pub created_ms: u64,
    /// Resolution timestamp (ms since UNIX epoch), if resolved.
    pub resolved_ms: Option<u64>,
}

impl Interaction {
    pub fn new(
        session_id: i64,
        tool_name: impl Into<String>,
        category: ToolCategory,
        detail: impl Into<String>,
    ) -> Self {
        let tool_name = tool_name.into();
        let detail = detail.into();
        let now_ms = now_ms();
        let hash_input = format!("{session_id}:{tool_name}:{detail}:{now_ms}");
        let hash = blake3::hash(hash_input.as_bytes()).to_hex();
        let id = format!("int_{session_id}_{}", &hash[..12]);

        Self {
            id,
            session_id,
            category,
            tool_name,
            detail,
            state: InteractionState::Pending,
            created_ms: now_ms,
            resolved_ms: None,
        }
    }

    pub fn approve(&mut self) {
        self.state = InteractionState::Approved;
        self.resolved_ms = Some(now_ms());
    }

    pub fn reject(&mut self, reason: Option<String>) {
        self.state = InteractionState::Rejected { reason };
        self.resolved_ms = Some(now_ms());
    }

    pub fn is_pending(&self) -> bool {
        matches!(self.state, InteractionState::Pending)
    }

    pub fn is_approved(&self) -> bool {
        matches!(self.state, InteractionState::Approved)
    }

    pub fn is_rejected(&self) -> bool {
        matches!(self.state, InteractionState::Rejected { .. })
    }
}

/// Evaluation verdict for a tool invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionVerdict {
    /// Automatically approved by policy without user interruption.
    AutoApproved,
    /// Disallowed by current permission policy.
    Denied { reason: String },
    /// Requires asynchronous interaction approval.
    NeedsInteraction(Interaction),
}

impl PermissionVerdict {
    pub fn is_auto_approved(&self) -> bool {
        matches!(self, Self::AutoApproved)
    }

    pub fn is_denied(&self) -> bool {
        matches!(self, Self::Denied { .. })
    }

    pub fn needs_interaction(&self) -> bool {
        matches!(self, Self::NeedsInteraction(_))
    }

    pub fn interaction(&self) -> Option<&Interaction> {
        match self {
            Self::NeedsInteraction(i) => Some(i),
            _ => None,
        }
    }
}

/// Permission policy governing agent actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionLevel {
    /// Strict lockdown: all tool actions are denied.
    DenyAll,
    /// Read-only: reads auto-approved; writes, bash, and network denied.
    ReadOnly,
    /// Granular Ask-Each: reads auto-approved; writes and bash gated into interaction.
    AskEach,
    /// Full access: all actions auto-approved.
    AllowAll,
}

/// Permissions classification and policy evaluation engine.
#[derive(Debug, Clone)]
pub struct PermissionsEngine {
    level: PermissionLevel,
}

impl PermissionsEngine {
    pub fn new(level: PermissionLevel) -> Self {
        Self { level }
    }

    pub fn level(&self) -> PermissionLevel {
        self.level
    }

    pub fn set_level(&mut self, level: PermissionLevel) {
        self.level = level;
    }

    /// Evaluate a requested tool call against the configured permission policy.
    ///
    /// In `AskEach` mode:
    /// - Read tools are auto-approved.
    /// - Write and Bash tools are gated into an `Interaction` request.
    pub fn evaluate(
        &self,
        session_id: i64,
        tool_name: &str,
        kind: Option<&str>,
        detail: &str,
    ) -> PermissionVerdict {
        let category = classify_tool(tool_name, kind);

        match self.level {
            PermissionLevel::AllowAll => PermissionVerdict::AutoApproved,
            PermissionLevel::DenyAll => PermissionVerdict::Denied {
                reason: format!("Action '{tool_name}' denied by DenyAll policy"),
            },
            PermissionLevel::ReadOnly => {
                if category == ToolCategory::Read {
                    PermissionVerdict::AutoApproved
                } else {
                    PermissionVerdict::Denied {
                        reason: format!(
                            "Tool '{tool_name}' ({:?}) denied by ReadOnly policy",
                            category
                        ),
                    }
                }
            }
            PermissionLevel::AskEach => match category {
                // Read tools = Auto-approved
                ToolCategory::Read => PermissionVerdict::AutoApproved,
                // Write / Bash / Network / Other tools = Gated into Interaction
                ToolCategory::Write
                | ToolCategory::Bash
                | ToolCategory::Network
                | ToolCategory::Other => {
                    let interaction = Interaction::new(session_id, tool_name, category, detail);
                    PermissionVerdict::NeedsInteraction(interaction)
                }
            },
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
