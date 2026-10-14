# Subagent visibility UX

Type: prototype
Status: resolved
Blocked by: 06, 13

## Question

Given what the bridges expose, how should subagents appear so the user understands them at a glance? Decide on the tree with nesting and live state, the per-child transcript view, per-child tokens, stop/message controls (where supported), how subagents differ visually from Chorus-spawned parallel agents, and the empty, running, failed, and unsupported-bridge states. Build a rough prototype. Consult the three UI skills named in the map's Notes.

## Answer

- **Subagent Hierarchy Tree (`SubagentTreeView`)**: Rendered in the Inspector’s Subagents tab with native macOS hierarchy lines. Status badges display live states (`⟳ Running` with active tool name, `✓ Completed` with duration/tool count, `✕ Failed`, `⊘ Cancelled`). Resolves Claude stream mixing by separating child events in the Rust normalizer.
- **Dedicated Child Transcript Drawer**: Keeps the main conversation clean while allowing 100% inspection of child thoughts, individual tool calls (`Read`, `Bash`, `Edit`), and stdout outputs in an expanding drawer (or quick inline toggle) below the node.
- **Visual Separation (Subagents vs Worktrees)**:
  - Provider subagents (spawned synchronously inside an agent turn) are displayed within the Inspector Subagents pane.
  - Chorus parallel agents (spawned across git worktrees) are managed in the Workspace Explorer sidebar under `⚡ AGENT WORKTREES`.
- **Turn Abort & Empty States**: Supports instant turn cancellation (`⌘.`), which cleanly terminates all running subagents across both Codex and Claude. Clear empty state (`point.3.connected.trianglepath.dotted`) and informative badge for bridges that do not expose subagents (Gemini CLI).

Detailed ASCII wireframe, component layout, and drawer interaction specification: [16-subagent-visibility-ux.md](../prototypes/16-subagent-visibility-ux.md).
