# Subagent Visibility in ACP Bridges

## Executive Summary

Subagent visibility is one of the most critical requirements for an agent harness. Users and orchestrators need to know when a subagent is spawned, what it is doing, what tools it runs, what tokens it consumes, and whether it can be steered or cancelled.

An investigation into the primary sources—the **Agent Client Protocol (ACP) specification**, **`@agentclientprotocol/codex-acp`**, **`@zed-industries/claude-agent-acp`**, and the **Gemini CLI ACP bridge**—reveals a stark divergence in how subagents are exposed:

1. **Two incompatible paradigms exist across bridges**:
   - **Codex (`@agentclientprotocol/codex-acp`)** implements a **session-multiplexed model**. It emits explicit notifications (`subagent_spawned`, `subagent_state_update`) and routes child events over the same stdio stream with distinct child `sessionId`s.
   - **Claude (`@zed-industries/claude-agent-acp`)** implements a **tool-call-scoped model**. It does not emit `subagent_spawned` at all; subagents are invoked as standard `Task` or `Agent` tool calls. Child messages and child tool calls are sent with the *parent's* `sessionId`, marked only by proprietary metadata (`_meta.claudeCode.parentToolUseId`).
   - **Gemini CLI (`gemini --experimental-acp`)** does not expose subagents over ACP at all. Any internal delegation is collapsed into the main turn stream.
   - **The ACP Specification (Draft v1/v2)** proposes a third, standardized syntax: `sessionUpdate: "subagent_update"` with `SubagentSessionCapabilities`.

2. **Severe parsing gaps in Chorus today**:
   - **Claude subagents are completely misattributed**: Because Claude does not change `sessionId`, Chorus's `Accumulator` treats all Claude subagent prose as parent assistant prose, concatenating child messages directly into the parent output. Furthermore, Claude child tool calls (`Read`, `Bash`, `Edit`) are logged as direct parent tool calls.
   - **Codex child tool calls and thoughts are discarded**: Chorus only extracts `agent_message_chunk` for non-root sessions; child tool calls and diffs are dropped.
   - **Live progress is blind to subagents**: The `report_progress` callback does not notify the UI of subagent spawn or state events.
   - **Tokens and Control**: Neither bridge currently supports per-child token accounting, individual child cancellation via ACP, or direct messaging to child sessions.

This document provides primary-source analysis, a comprehensive capability matrix, detailed gap analysis for `crates/chorus-acp`, and concrete architectural recommendations for Chorus.

---

## 1. Primary Sources & Architectural Paradigms

### 1.1 The ACP Specification (`@agentclientprotocol/sdk`)

Primary sources:
- `@agentclientprotocol/sdk` v1.5.0 (`schema/schema.json`)
- `@agentclientprotocol/sdk` v2 draft (`schema/v2/schema.unstable.json`)

#### Capability Negotiation
In the official ACP specification, subagent support is governed by an unstable client capability declared during `initialize`:
```json
{
  "protocolVersion": 1,
  "clientCapabilities": {
    "subagents": {}
  }
}
```
The specification describes `SubagentCapabilities`:
> "Capability marker for exposing reusable child sessions as restricted ACP sessions. Supplying `{}` advertises support for child association and state updates, session-directed messages, and restricted-session semantics."

#### Notification Schema: `subagent_update`
Rather than splitting spawn and state updates into separate notification kinds, the official ACP specification defines a single unified notification: `sessionUpdate: "subagent_update"` (`SubagentUpdate`):
- `sessionId`: (required) The opaque session ID identifying the child.
- `title`: (string, nullable) Display title for the child.
- `description`: (string, nullable) Role or purpose of the child.
- `capabilities`: (`SubagentSessionCapabilities`, nullable) Client mutations permitted on the child (e.g. `{ "cancel": {} }`).
- `state`: (`StateUpdate`, nullable) Mirrored child foreground state (`running`, `idle`, `completed`, `cancelled`).
- `_meta`: Extension metadata.

The spec also specifies `SessionMessage` for inter-session communication (`messageId`, `senderSessionId`, `recipientSessionId`).

---

### 1.2 OpenAI Codex Bridge (`@agentclientprotocol/codex-acp` & Codex App-Server)

Primary sources:
- `@agentclientprotocol/codex-acp` v2.1.1 (and predecessor `@zed-industries/codex-acp` v0.16.0)
- `@openai/codex` v0.159.1 (Codex app-server)
- Inspected source files: `dist/index.js`, `src/subagents/CodexSubagentEventRouter.ts`, `src/CodexAcpServer.ts`

#### Activation & Capability Negotiation
In `src/subagents/AcpSubagents.ts`:
```typescript
function clientSupportsSubagents(capabilities) {
  const subagents = capabilities?.subagents;
  if (typeof subagents === "object" && subagents !== null && !Array.isArray(subagents)) {
    return true;
  }
  return clientSupportsAirCapability(capabilities, AIR_NATIVE_SUBAGENT_SESSIONS_KEY);
}
```
If the client passes `"subagents": {}` in `clientCapabilities`, `codex-acp` activates `CodexSubagentEventRouter`. If omitted, it falls back to a legacy tool call representation (`collabAgentToolCall` / `SubagentActivityReporter`).

#### Lifecycle Events
Codex app-server emits notifications on the main connection:
1. **Spawn**: Emits `sessionUpdate: "subagent_spawned"`:
   ```json
   {
     "sessionId": "<parentSessionId>",
     "update": {
       "sessionUpdate": "subagent_spawned",
       "subagentSessionId": "<childThreadId>",
       "name": "Agent 3a8f1b2c",
       "task": "Delegated task for Agent 3a8f1b2c",
       "capabilities": {}
     }
   }
   ```
   *Notice*: The outer `sessionId` is the **parent's** session ID. The child's ID is carried in `update.subagentSessionId`.
2. **Terminal State**: Emits `sessionUpdate: "subagent_state_update"`:
   ```json
   {
     "sessionId": "<parentSessionId>",
     "update": {
       "sessionUpdate": "subagent_state_update",
       "subagentSessionId": "<childThreadId>",
       "state": "completed"
     }
   }
   ```
   Possible states: `"completed"`, `"cancelled"`, `"failed"`, `"disconnected"`.
3. **Child Stream Multiplexing**:
   While the child executes, child-generated events (`item/agentMessage/delta`, `item/started`, `item/completed`) are routed through `CodexEventHandler`. In line 30317:
   ```typescript
   await this.session.update(updateEvent, this.subagents.notificationSessionId(notification));
   ```
   Because `notificationSessionId` returns `child.sessionId`, the `session/update` notification has:
   ```json
   {
     "sessionId": "<childSessionId>",
     "update": {
       "sessionUpdate": "agent_message_chunk",
       "content": { "type": "text", "text": "..." }
     }
   }
   ```
   Child tool calls (`tool_call`, `tool_call_update`) and diffs are also sent with `sessionId: <childSessionId>`.

#### Cancellation & Prompting Limitations
- **Cancellation**: `CodexAcpServer.cancel(params)` looks up `this.sessions.get(params.sessionId)`. Child sessions are managed inside `sessionState.subagents` and are **not** in `this.sessions`. Sending `session/cancel` with a child ID fails with:
  `"Cancel request rejected: session not found"`.
  Cancelling the parent session *does* cancel all outstanding child subagents.
- **Messaging**: `CodexAcpServer.prompt(params)` calls `this.getSessionState(params.sessionId)`, which throws `Error: Session <childSessionId> not found`. Direct prompts to children are unsupported.

---

### 1.3 Anthropic Claude Bridge (`@zed-industries/claude-agent-acp`)

Primary sources:
- `@zed-industries/claude-agent-acp` v0.23.1
- `@anthropic-ai/claude-agent-sdk` v0.2.83
- Inspected source files: `dist/acp-agent.js`, `dist/tools.js`

#### Activation & Capability Negotiation
`claude-agent-acp` **completely ignores** `clientCapabilities.subagents`. It contains no checks for `subagents` and emits neither `subagent_spawned` nor `subagent_state_update`.

#### Tool-Call-Scoped Representation
Claude Code runs subagents via its internal `Task` (or `Agent`) tool. In `dist/tools.js` lines 18–34:
```typescript
case "Agent":
case "Task": {
    const input = toolUse.input;
    return {
        title: input?.description ? input.description : "Task",
        kind: "think",
        content: input && "prompt" in input
            ? [{ type: "content", content: { type: "text", text: input.prompt } }]
            : [],
    };
}
```
A subagent is launched as a standard ACP `tool_call`:
- `sessionUpdate: "tool_call"`
- `toolCallId: "<task_tool_use_id>"`
- `title: "<description or Task>"`
- `kind: "think"`
- `_meta: { "claudeCode": { "toolName": "Task" } }`

#### Metadata-Linked Child Events
In `@anthropic-ai/claude-agent-sdk`, subagent operations produce stream messages with `message.parent_tool_use_id` set to the ID of the parent `Task` tool call.

In `dist/acp-agent.js` lines 1386–1394 & 1588–1596:
```typescript
if (options?.parentToolUseId) {
    update._meta = {
        ...update._meta,
        claudeCode: {
            ...(update._meta?.claudeCode || {}),
            parentToolUseId: options.parentToolUseId,
        },
    };
}
output.push({ sessionId, update });
```
**Key architectural fact**:
- `sessionId` is **always the parent's session ID** (`params.sessionId`).
- Child events are distinguished **only** by `update._meta.claudeCode.parentToolUseId`.
- Child assistant messages, child tool calls (`Read`, `Bash`, `Edit`), and tool completions are delivered as normal updates on the parent session with this metadata tag.
- In `acp-agent.js` lines 325–331, SDK events `task_started`, `task_notification`, and `task_progress` are explicitly unhandled (`// Todo: process via status api`).

#### Token Accounting
In `dist/acp-agent.js` lines 455–463:
```typescript
// Store latest assistant usage (excluding subagents)
if (message.message.usage && message.parent_tool_use_id === null) {
    const messageWithUsage = message.message;
    lastAssistantTotalUsage =
        messageWithUsage.usage.input_tokens +
        messageWithUsage.usage.output_tokens + ...;
}
```
Child usage is **explicitly excluded** from the assistant usage tracking. Cumulative usage across the whole turn is reported at the end in `usage_update`, but per-child token usage is not exposed.

---

### 1.4 Gemini CLI ACP (`gemini --experimental-acp`)

Primary sources:
- Gemini CLI ACP mode documentation and protocol traces.
- `initialize` handshake behavior with `clientCapabilities`.

#### Subagent Support in ACP Mode
While the interactive Gemini CLI includes subagent definitions (e.g. `@codebase_investigator` under `.gemini/subagents/`), when launched with `--experimental-acp` (or `--acp`):
- It negotiates standard ACP v1 (`protocolVersion: 1`, `fs`, `terminal`).
- It does **not** advertise or process the `subagents` capability.
- Subagent tasks executed internally are collapsed into the primary turn. No child session IDs, no `subagent_spawned` events, and no `parentToolUseId` metadata are emitted.
- All tool calls and output appear as direct actions of the primary agent.

---

## 2. Capability Matrix Across Bridges

| Capability Dimension | ACP Spec (Draft v1/v2) | Codex (`@agentclientprotocol/codex-acp`) | Claude (`@zed-industries/claude-agent-acp`) | Gemini CLI (`gemini --experimental-acp`) |
| :--- | :--- | :--- | :--- | :--- |
| **Capability Handshake** | `clientCapabilities: { subagents: {} }` (UNSTABLE) | **Required**: checks `capabilities.subagents` | **Ignored**: no check, runs unconditionally | **Unsupported**: not recognized |
| **Notification Syntax** | `subagent_update` | `subagent_spawned` & `subagent_state_update` | `tool_call` (`name: Task`) + `tool_call_update` | None (plain `tool_call` / message) |
| **Session Identification** | Distinct `sessionId` for child | Distinct `subagentSessionId` for child | **None**: uses parent `sessionId` | None: uses parent `sessionId` |
| **Parent-Child Link** | Enclosing session ID is parent; `sessionId` is child | `params.sessionId` is parent; `subagentSessionId` is child | `update._meta.claudeCode.parentToolUseId` = tool ID | None: completely flattened |
| **Hierarchy / Nesting** | Yes: recursive `subagent_update` | Yes: intermediate parent session tracked | Yes: chained `parentToolUseId` | No |
| **Live State Visibility** | Full (`StateUpdate` snapshot) | Partial: `state` string on completion (`completed`, `cancelled`, `failed`) | Minimal: tool call status (`pending` → `completed`) | None |
| **Child Prose Stream** | `SessionMessage` or child session updates | Yes: `agent_message_chunk` with child `sessionId` | Yes, but interleaved on parent `sessionId` with `_meta` | Flattened into main stream |
| **Child Tool Stream** | Sent on child session | Yes: sent with child `sessionId` (dropped by Chorus) | Interleaved on parent stream with `parentToolUseId` | Flattened into main stream |
| **Per-Child Token Usage** | Unspecified | Leaky / Unreliable (copies root context to child) | None (subagents explicitly filtered out) | None |
| **Individual Child Cancel** | Proposed (`SubagentSessionCapabilities.cancel`) | **Unsupported**: rejects child `sessionId` | **Unsupported**: no per-tool cancel | **Unsupported**: turn-level only |
| **Send Message to Child** | Proposed (`SessionMessage` / `session/prompt`) | **Unsupported**: throws session not found | **Unsupported**: tool execution is internal loop | **Unsupported** |

---

## 3. Analysis of Current Chorus Implementation & Gaps

### 3.1 What Chorus Currently Implements

In [`crates/chorus-acp/src/codex.rs`](file:///Users/perth/private_project/chorus/crates/chorus-acp/src/codex.rs):
1. **Capability advertisement** (line 445):
   ```rust
   "clientCapabilities": {
       "fs": { "readTextFile": true, "writeTextFile": true },
       "terminal": false,
       "subagents": {}
   }
   ```
   Chorus correctly advertises `"subagents": {}` to satisfy `codex-acp`.
2. **Parsing Codex events** (lines 731–756):
   - Handles `subagent_spawned`: maps to `AgentEvent::SubagentSpawned { session_id, parent_session_id, name, task }`.
   - Handles `subagent_state_update`: maps to `AgentEvent::SubagentState { session_id, state, message }`.
3. **Isolating child prose** (lines 759–766):
   ```rust
   if !session_id.is_empty() && session_id != self.root_session_id {
       if kind == "agent_message_chunk" {
           if let Some(text) = chunk_text(update) {
               self.child_messages.entry(session_id.to_string()).or_default().push_str(&text);
           }
       }
       return;
   }
   ```
   Prevents child messages with distinct `session_id` from polluting the parent `self.message`.
4. **Ledger mapping** in [`crates/chorus-cli/src/session.rs`](file:///Users/perth/private_project/chorus/crates/chorus-cli/src/session.rs#L1646-L1664):
   - `AgentEvent::SubagentSpawned` maps to `EventKind::ToolCall` with a JSON payload.
   - `AgentEvent::SubagentState` maps to `EventKind::ToolResult` with state and accumulated message.

---

### 3.2 Current Chorus Parsing Gaps

#### Gap 1: Claude Subagent Blindness and Stream Pollution (Critical)
Because `@zed-industries/claude-agent-acp` does not use distinct `sessionId`s and does not emit `subagent_spawned`:
- Every notification from a Claude subagent has `sessionId == self.root_session_id`.
- The condition `session_id != self.root_session_id` in `codex.rs` evaluates to **false**.
- **Consequences**:
  1. All child assistant prose is appended directly into `self.message` (the parent's response).
  2. All child tool calls (`Read`, `Bash`, `Write`, `Edit`) are appended directly to `self.events` as parent tool calls.
  3. The `Task` tool call itself is recorded as a generic `ToolCall { title: "Task", kind: "think" }`.
  4. **Zero** `AgentEvent::SubagentSpawned` or `SubagentState` events are emitted for Claude.

#### Gap 2: Dropped Codex Child Tool Calls and Diffs (High)
In `codex.rs` line 760:
```rust
if !session_id.is_empty() && session_id != self.root_session_id {
    if kind == "agent_message_chunk" {
        ...
    }
    return;
}
```
When Codex spawns a subagent that runs commands, reads files, or generates diffs:
- The events arrive with `session_id == child_session_id`.
- Chorus checks *only* `kind == "agent_message_chunk"` and then immediately calls `return;`.
- **Consequences**:
  1. Child `tool_call` and `tool_call_update` events are discarded.
  2. Child diffs (`AgentEvent::Diff`) are lost.
  3. Child thoughts (`agent_thought_chunk`) are lost.
  Chorus has no transcript of the subagent's actual actions.

#### Gap 3: Missing Subagent Live Progress (Medium)
In `codex.rs` lines 671–700:
`report_progress` forwards `agent_thought_chunk`, `agent_message_chunk`, `tool_call`, and `tool_call_update` to `crate::emit_progress`.
- For Codex: Child events trigger `emit_progress` without indicating their subagent origin, mixing child progress with parent progress.
- Neither `subagent_spawned` nor `subagent_state_update` is forwarded to the progress sink.
- For Claude: Child progress is indistinguishable from parent progress.
- The UI layer cannot display an active subagent spinner, child task description, or status badge.

#### Gap 4: ACP Spec `subagent_update` Incompatibility (Future-Proofing)
Chorus only recognizes `subagent_spawned` and `subagent_state_update`. If an upstream bridge adheres to the official ACP specification (`subagent_update`), Chorus will ignore the events.

#### Gap 5: Absence of Per-Child Token and Cost Tracking
- `AgentEvent::SubagentState` contains only `(session_id, state, message)`.
- `TurnOutcome` maintains only a single global `context_used` / `context_window`.
- Neither bridge currently provides reliable per-child tokens over ACP (Claude filters them out; Codex duplicates root usage).

#### Gap 6: No Independent Cancellation or Steering
- In [`crates/chorus-acp/src/lib.rs`](file:///Users/perth/private_project/chorus/crates/chorus-acp/src/lib.rs#L83-L94), cancellation is implemented via `cancel_active()`, which kills the process group (`killpg(pid, 15)`).
- Chorus has no protocol-level `session/cancel` dispatch for specific child IDs.

---

## 4. Version Requirements

To guarantee subagent compatibility across bridges, the following versions are required:

| Component | Minimum Version | Recommended / Pinned Version | Notes |
| :--- | :--- | :--- | :--- |
| **`@agentclientprotocol/codex-acp`** | `2.1.0` | `^2.1.1` | Native subagents require `clientCapabilities: { subagents: {} }`. Legacy `@zed-industries/codex-acp` 0.16.0 also supported `subagent_spawned`. |
| **`@openai/codex` (app-server)** | `0.150.0` | `^0.159.1` | Embedded inside `codex-acp`; provides `collabAgentToolCall` / `subAgentActivity`. |
| **`@zed-industries/claude-agent-acp`** | `0.23.0` | `^0.23.1` | Renamed from deprecated `@zed-industries/claude-code-acp`. Uses Claude Agent SDK. |
| **`@anthropic-ai/claude-agent-sdk`** | `0.2.80` | `^0.2.83` | Provides `parent_tool_use_id` on stream messages and tool uses. |
| **Gemini CLI** | `0.5.0` | Latest (`--experimental-acp` / `--acp`) | Subagents not exposed over ACP in current releases. |
| **ACP Protocol Spec** | `v1` (with drafts) | `v1` + `v2 draft` | Draft extensions define `SubagentUpdate` (`subagent_update`). |

---

## 5. Architectural Recommendations for Chorus

To achieve first-class subagent visibility without depending on upstream bridges unifying their protocol dialects, Chorus should adopt a **Unified Subagent Normalizer** inside `crates/chorus-acp`.

```
                  ┌────────────────────────────────────────────────────────┐
                  │                 Incoming ACP Frames                    │
                  └────────────────────────────────────────────────────────┘
                                              │
                      ┌───────────────────────┴───────────────────────┐
                      ▼                                               ▼
             [Codex / ACP v2 Model]                         [Claude Model]
          sessionId != root_session_id             sessionId == root_session_id AND
          OR sessionUpdate: subagent_*             _meta.claudeCode.parentToolUseId != null
                      │                                               │
                      └───────────────────────┬───────────────────────┘
                                              ▼
                  ┌────────────────────────────────────────────────────────┐
                  │            Chorus Subagent Normalizer Layer            │
                  │  - Tracks active subagent hierarchy (parent ↔ child)   │
                  │  - Routes child messages, thoughts, tools, diffs       │
                  │  - Maintains per-child transcript buffers              │
                  │  - Emits normalized SubagentSpawned & SubagentState    │
                  └────────────────────────────────────────────────────────┘
                                              │
                      ┌───────────────────────┴───────────────────────┐
                      ▼                                               ▼
          [Live Progress Sink]                               [Session Ledger]
     emit_progress("subagent_spawn", ...)              EventKind::ToolCall (spawn)
     emit_progress("subagent_tool", ...)               EventKind::ToolResult (state)
     emit_progress("subagent_done", ...)               Child events mapped or scoped
```

### 5.1 Dual-Dialect Event Ingestion in `Accumulator`

In `crates/chorus-acp/src/codex.rs`, modify `Accumulator::absorb` to detect subagent context from **either**:
1. **Explicit Session ID**: `session_id != self.root_session_id` (Codex / ACP v2).
2. **Metadata Link**: `update._meta.claudeCode.parentToolUseId` (Claude).

When Claude emits a `tool_call` with `name == "Task"` or `name == "Agent"`:
- Treat this tool call as a subagent spawn.
- Generate an internal child session key corresponding to `toolCallId`.
- Any subsequent frame carrying `parentToolUseId == toolCallId` is diverted into that child session's transcript.
- When the `Task` tool call resolves with `tool_call_update` (`status: completed` or `failed`), emit `AgentEvent::SubagentState`.

### 5.2 Capture Child Tool Calls and Diffs
Expand the non-root handling in `codex.rs`:
```rust
if is_child {
    let child_entry = self.child_transcripts.entry(child_id.clone()).or_default();
    match kind {
        "agent_message_chunk" => {
            if let Some(text) = chunk_text(update) {
                child_entry.message.push_str(&text);
            }
        }
        "agent_thought_chunk" => {
            if let Some(text) = chunk_text(update) {
                child_entry.thoughts.push_str(&text);
            }
        }
        "tool_call" => {
            let title = update.get("title").and_then(Value::as_str).unwrap_or("tool");
            child_entry.events.push(ChildEvent::ToolCall { title: title.to_string() });
        }
        "tool_call_update" => {
            // Capture diffs and completions inside the child
            ...
        }
        _ => {}
    }
    return;
}
```
This preserves the full subagent execution record (what commands were run, what files were inspected or edited).

### 5.3 Enhance Live Progress Notifications
Extend `crates/chorus-acp/src/lib.rs` and `report_progress`:
- Add progress channels: `"subagent_spawn"`, `"subagent_tool"`, `"subagent_done"`.
- When `subagent_spawned` (or Claude `Task`) arrives, emit:
  `crate::emit_progress("subagent_spawn", &format!("{child_id}\t{name}\t{task}"))`.
- The Swift UI receives structured live updates and can render the subagent hierarchy tree with active status spinners.

### 5.4 Support the Official ACP `subagent_update`
Add a pattern match for `sessionUpdate == "subagent_update"` in `Accumulator::absorb`. If `update.sessionId` is new, emit `SubagentSpawned`. If `update.state` is present, emit `SubagentState`. This ensures Chorus automatically supports future ACP bridges conforming to the official specification.

### 5.5 Per-Child Cancellation and Messaging Strategy
Since neither bridge currently allows individual subagent cancellation or messaging:
- **UI Policy**: Clearly indicate in the macOS UI that subagents are executed synchronously within the parent turn.
- The user can cancel the **turn** (which reliably cancels all active subagents across both Codex and Claude).
- If fine-grained control is required (e.g. independently spawning, messaging, and stopping subagents), Chorus should drive them as **top-level agent sessions** using Chorus's own agent-as-orchestrator architecture (with dedicated git worktrees), rather than relying on provider-internal subagent loops.
