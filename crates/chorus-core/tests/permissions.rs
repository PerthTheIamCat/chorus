//! Tests for granular permissions engine and interaction state machine.

use chorus_core::permissions::{
    classify_tool, InteractionState, PermissionLevel, PermissionsEngine, ToolCategory,
};

#[test]
fn tool_classification_distinguishes_read_write_bash_and_network() {
    // Read tools
    assert_eq!(classify_tool("read_file", None), ToolCategory::Read);
    assert_eq!(classify_tool("view_file", None), ToolCategory::Read);
    assert_eq!(classify_tool("cat", None), ToolCategory::Read);
    assert_eq!(classify_tool("grep", None), ToolCategory::Read);
    assert_eq!(classify_tool("glob", None), ToolCategory::Read);
    assert_eq!(classify_tool("list_directory", None), ToolCategory::Read);
    assert_eq!(classify_tool("custom_tool", Some("read")), ToolCategory::Read);

    // Write tools
    assert_eq!(classify_tool("write_to_file", None), ToolCategory::Write);
    assert_eq!(classify_tool("edit_file", None), ToolCategory::Write);
    assert_eq!(classify_tool("replace_file_content", None), ToolCategory::Write);
    assert_eq!(classify_tool("patch_file", None), ToolCategory::Write);
    assert_eq!(classify_tool("delete_file", None), ToolCategory::Write);
    assert_eq!(classify_tool("str_replace_editor", None), ToolCategory::Write);
    assert_eq!(classify_tool("custom_save", Some("write")), ToolCategory::Write);

    // Bash / command tools
    assert_eq!(classify_tool("bash", None), ToolCategory::Bash);
    assert_eq!(classify_tool("run_command", None), ToolCategory::Bash);
    assert_eq!(classify_tool("exec", None), ToolCategory::Bash);
    assert_eq!(classify_tool("terminal", None), ToolCategory::Bash);
    assert_eq!(classify_tool("custom_shell", Some("command")), ToolCategory::Bash);

    // Command tool with "read" in name should still be classified as Bash because arbitrary command runs!
    assert_eq!(classify_tool("run_command_read", None), ToolCategory::Bash);

    // Network tools
    assert_eq!(classify_tool("curl", None), ToolCategory::Network);
    assert_eq!(classify_tool("http", None), ToolCategory::Network);
    assert_eq!(classify_tool("search_web", None), ToolCategory::Network);

    // Other tools
    assert_eq!(classify_tool("unknown_custom_widget", None), ToolCategory::Other);
}

#[test]
fn ask_each_mode_auto_approves_reads_and_gates_writes_and_bash() {
    let engine = PermissionsEngine::new(PermissionLevel::AskEach);
    let session_id = 100;

    // 1. Read tool = Auto-approved
    let v_read = engine.evaluate(session_id, "read_file", None, "src/main.rs");
    assert!(v_read.is_auto_approved());
    assert!(!v_read.needs_interaction());

    // 2. Write tool = Gated into Interaction approval request
    let v_write = engine.evaluate(
        session_id,
        "write_to_file",
        None,
        "src/main.rs: replace line 10",
    );
    assert!(!v_write.is_auto_approved());
    assert!(v_write.needs_interaction());

    let interaction = v_write.interaction().expect("should contain interaction");
    assert_eq!(interaction.session_id, session_id);
    assert_eq!(interaction.category, ToolCategory::Write);
    assert_eq!(interaction.tool_name, "write_to_file");
    assert_eq!(interaction.detail, "src/main.rs: replace line 10");
    assert!(interaction.is_pending());

    // 3. Bash tool = Gated into Interaction approval request
    let v_bash = engine.evaluate(session_id, "run_command", None, "cargo build --release");
    assert!(!v_bash.is_auto_approved());
    assert!(v_bash.needs_interaction());

    let interaction_bash = v_bash.interaction().expect("should contain interaction");
    assert_eq!(interaction_bash.category, ToolCategory::Bash);
    assert_eq!(interaction_bash.tool_name, "run_command");
    assert!(interaction_bash.is_pending());
}

#[test]
fn interaction_state_machine_transitions() {
    let engine = PermissionsEngine::new(PermissionLevel::AskEach);
    let verdict = engine.evaluate(42, "edit_file", None, "update config");

    let mut interaction = verdict.interaction().cloned().unwrap();
    assert!(interaction.is_pending());
    assert_eq!(interaction.state, InteractionState::Pending);
    assert!(interaction.resolved_ms.is_none());

    // Test approve
    interaction.approve();
    assert!(interaction.is_approved());
    assert!(!interaction.is_pending());
    assert_eq!(interaction.state, InteractionState::Approved);
    assert!(interaction.resolved_ms.is_some());

    // Test reject
    let mut interaction2 = verdict.interaction().cloned().unwrap();
    interaction2.reject(Some("Disallowed by user".to_string()));
    assert!(interaction2.is_rejected());
    assert!(!interaction2.is_pending());
    assert_eq!(
        interaction2.state,
        InteractionState::Rejected {
            reason: Some("Disallowed by user".to_string())
        }
    );
    assert!(interaction2.resolved_ms.is_some());
}

#[test]
fn policy_levels_enforce_boundaries() {
    let session_id = 7;

    // ReadOnly policy
    let ro_engine = PermissionsEngine::new(PermissionLevel::ReadOnly);
    assert!(ro_engine
        .evaluate(session_id, "read_file", None, "README.md")
        .is_auto_approved());
    assert!(ro_engine
        .evaluate(session_id, "write_file", None, "README.md")
        .is_denied());
    assert!(ro_engine
        .evaluate(session_id, "bash", None, "ls")
        .is_denied());

    // DenyAll policy
    let deny_engine = PermissionsEngine::new(PermissionLevel::DenyAll);
    assert!(deny_engine
        .evaluate(session_id, "read_file", None, "README.md")
        .is_denied());
    assert!(deny_engine
        .evaluate(session_id, "write_file", None, "README.md")
        .is_denied());

    // AllowAll policy
    let allow_engine = PermissionsEngine::new(PermissionLevel::AllowAll);
    assert!(allow_engine
        .evaluate(session_id, "read_file", None, "README.md")
        .is_auto_approved());
    assert!(allow_engine
        .evaluate(session_id, "write_file", None, "README.md")
        .is_auto_approved());
    assert!(allow_engine
        .evaluate(session_id, "bash", None, "rm -rf /")
        .is_auto_approved());
}
