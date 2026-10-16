# Prototype: Subagent Visibility UX

## 1. Overview & Design Principles

Designed under **`anti-ui-slop`**, **`macos-design-guidelines`**, and **`ui-ux-pro-max`**:
- **Immediate Hierarchy Comprehension**: Shows parent-child relationships and nested subagent trees at a glance, eliminating confusing interleaved log noise.
- **Zero Hallucination / Real Data Only**: Fed by the Rust `SubagentNormalizer` (`crates/chorus-acp`), unifying Codex's `subagent_spawned` and Claude's `parentToolUseId` into a single canonical tree model.
- **Dedicated Inspection without Context Pollution**: Clicking a subagent opens its isolated transcript drawer (thoughts, tool calls, tool outputs) without cluttering the main conversation thread.

---

## 2. Subagents Pane & Tree Wireframe (Inspector Tab 2)

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│  SUBAGENTS INSPECTOR (งานย่อย)                                        [Live Stream ⚡] │
│  Active in Turn: 2 Subagents · 1 Completed, 1 Running                  [Abort Turn ⌘.] │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ ▾ 🧠 Claude 3.7 Sonnet [Root Coordinator]                                              │
│   │                                                                                    │
│   ├─ 🤖 Task: Codebase Investigator (Agent-8f3a)                       [ ✓ Completed ] │
│   │  │  Goal: "Find all usages of `wanted()` in router.rs"                             │
│   │  │  Duration: 4.2s · Tools: 3 (Read, Grep, AstSearch)                             │
│   │  └─ [View Child Transcript ↗]                                                     │
│   │                                                                                    │
│   └─ 🤖 Task: Test Runner & Verifier (Agent-9b1c)                       [ ⟳ Running... ] │
│      │  Goal: "Run `cargo test --bin eval` in isolated branch"                         │
│      │  Current Action: `Tool: bash` → `Running 24 test cases...`                      │
│      └─ [View Child Transcript ↗]                                                     │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ ┌────────────────────────────────────────────────────────────────────────────────────┐ │
│ │ 📖 CHILD TRANSCRIPT DRAWER: Agent-9b1c (Test Runner)                [Close Drawer ✕]│ │
│ ├────────────────────────────────────────────────────────────────────────────────────┤ │
│ │ [Thought] "Verifying that the cost penalty term does not cause regressions..."      │ │
│ │                                                                                    │ │
│ │ [Tool Call] `bash: cargo test --bin eval`                                          │ │
│ │   [Output]                                                                         │ │
│ │   running 24 tests                                                                 │ │
│ │   test router::test_simple_typo ... ok                                             │ │
│ │   test router::test_complex_architecture ... ok                                    │ │
│ │   test result: ok. 24 passed; 0 failed; finished in 1.28s                          │ │
│ │                                                                                    │ │
│ │ [Child Message] "All 24 test cases passed successfully with 0 failures."            │ │
│ └────────────────────────────────────────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Structural & Interaction Components

### 3.1 Live Hierarchy Tree (`SubagentTreeView`)
- **Indented Tree Lines**: Clean macOS system tree lines with rounded branch elbows connecting root and children.
- **Status Pills**:
  - `⟳ Running`: Pulsing accent tint with active spinner, displaying the subagent's live action (e.g. `Tool: bash`).
  - `✓ Completed`: Green subtle badge with total run duration and tool call count.
  - `✕ Failed`: Red badge with failure reason.
  - `⊘ Cancelled`: Gray badge when parent turn is aborted.

### 3.2 Child Transcript Drawer (`ChildTranscriptView`)
- Opens either inline under the selected subagent or in an expanding inspector drawer.
- **Contains Complete Child Trace**:
  - Collapsible child reasoning / thoughts.
  - Detailed tool invocations with exact arguments.
  - Formatted command outputs with copy button.
  - Final child result message.
- Keeps the **main thread completely clean**: only high-level subagent spawn/done events appear in the main chat.

### 3.3 Visual Distinction: Subagents vs. Parallel Worktrees
- **Subagents (Child Processes)**: Displayed within the current session's Inspector panel with a purple robot icon (`🤖`), executing synchronously inside the agent's turn.
- **Chorus Parallel Agents (Orchestrated Worktrees)**: Displayed in the Workspace Explorer sidebar under `⚡ AGENT WORKTREES` with distinct branch names (`.chorus/worktrees/*`), each possessing its own independent session timeline.

### 3.4 Edge Case & Unsupported Bridge States
- **No Subagents Active**: Clean empty state with SF Symbol `point.3.connected.trianglepath.dotted` and text *"No subagents spawned in this turn"*.
- **Unsupported Bridge (e.g. Gemini CLI ACP)**: Informative subtle banner: *"Gemini CLI does not expose subagent events over ACP; subtasks execute inline."*
