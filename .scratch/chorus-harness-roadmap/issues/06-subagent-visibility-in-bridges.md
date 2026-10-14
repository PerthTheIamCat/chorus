# Subagent visibility in ACP bridges

Type: research
Status: resolved
Blocked by:

## Question

What do the ACP bridges actually expose about subagents? Read the ACP spec and the sources of `@zed-industries/claude-agent-acp` (Claude Code Task tool / subagents), `codex-acp` / Codex app-server (`subagent_spawned`, `subagent_state_update`), and Gemini CLI ACP. For each one, determine whether Chorus can get: the parent-child relationship (including nesting), a live state, the child's own message and tool stream (its transcript), per-child token usage, a way to cancel a child, and a way to send a message to a child. Note what Chorus already parses in `crates/chorus-acp/src/codex.rs` and what is missing, especially for Claude. Note any version requirements.

## Answer

- **Two Incompatible Bridge Models**: Codex (`@agentclientprotocol/codex-acp`) uses a *session-multiplexed model* emitting `subagent_spawned` / `subagent_state_update` with distinct child `sessionId`s over the shared connection. Claude (`@zed-industries/claude-agent-acp`) uses a *tool-call model* emitting subagents as `Task`/`Agent` tool calls, sending all child messages and tool events on the *parent's* session ID tagged only by `_meta.claudeCode.parentToolUseId`.
- **Gemini CLI & ACP Spec Status**: Gemini CLI (`gemini --experimental-acp`) does not expose subagents over ACP at all (flattened into main turn). The official ACP specification (draft v1/v2 schema) standardizes a unified `sessionUpdate: "subagent_update"`, which neither existing bridge currently matches verbatim.
- **Parent-Child Hierarchy & Live State**: Both Codex and Claude expose parent-child relationships and nesting (Codex via explicit `subagentSessionId` / `parentSessionId`; Claude via chained `parentToolUseId`). Live state visibility is coarse: Codex reports terminal state strings (`completed`, `cancelled`, `failed`), while Claude reports tool completion status (`pending` → `completed`/`failed`).
- **Child Transcripts vs. Tokens**: Child message and tool streams are available in both (Codex routes them under child session IDs; Claude interleaves them with metadata tags). However, per-child token usage is unsupported in both (Claude explicitly filters out child usage from `lastAssistantTotalUsage`; Codex duplicates root session usage).
- **No Individual Child Steering or Cancel**: Neither bridge allows individual child cancellation (`session/cancel` rejects child session IDs) or sending messages to a child (`session/prompt` fails). Only cancelling the entire parent turn cancels all active subagents.
- **Current Chorus Parsing Gaps (`crates/chorus-acp/src/codex.rs`)**:
  - *Claude blindness*: Because Claude sends child events with `sessionId == root_session_id`, Chorus appends child prose directly into the parent's message and logs child tool calls as parent tool calls. Zero `SubagentSpawned` events are emitted for Claude.
  - *Codex child tool dropping*: In `Accumulator`, events with `sessionId != root_session_id` only extract `agent_message_chunk`; child tool calls, diffs, and thoughts are dropped.
  - *Progress sink omission*: `report_progress` forwards neither `subagent_spawned` nor `subagent_state_update` to `ProgressSink`.
- **Version Requirements**: `@agentclientprotocol/codex-acp` >= 2.1.0 (with `clientCapabilities: { subagents: {} }`); `@zed-industries/claude-agent-acp` >= 0.23.0 (`@anthropic-ai/claude-agent-sdk` >= 0.2.80).

Full findings and architectural normalization design: [06-subagent-visibility-in-bridges.md](../research/06-subagent-visibility-in-bridges.md).
