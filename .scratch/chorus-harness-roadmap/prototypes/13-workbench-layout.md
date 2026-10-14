# Prototype: Chorus Workbench Layout

## 1. Overview & Principles

Designed under **`anti-ui-slop`**, **`macos-design-guidelines`**, and **`ui-ux-pro-max`**:
- **Zero generic widgets**: Every pane and button has an explicit, functional purpose tied directly to the SQLite ledger and Rust engine.
- **Strictly Native macOS**: Uses standard macOS navigation paradigms (`NavigationSplitView` with resizable dividers or floating glass cards, native toolbars, persistent menu bar commands, standard keyboard shortcuts).
- **Rule 2.3 Compliance**: The Swift shell draws and captures inputs only; all file tree walking, Myers diff calculation, syntax highlight spans, and subagent state trees are computed in Rust (`chorus-core`).

---

## 2. Window Architecture & Panes

```
┌────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│ [Traffic Lights]  [Sidebar Toggle ⌃⌘S] [Project: chorus] [Session: Fix router cost]            [File ⌘B] [Diff ⌥⌘I]  │ (Toolbar)
├───────────────┬──────────────────────────┬──────────────────────────────────────────┬──────────────────────────────────┤
│ SIDEBAR       │ WORKSPACE EXPLORER       │ CENTER STAGE: THREAD / WORKBENCH         │ INSPECTOR: CHANGES & SUBAGENTS   │
│ (260px)       │ (240px, Toggle ⌘B)       │ (Flexible, min 500px)                    │ (340px, Toggle ⌥⌘I)              │
├───────────────┼──────────────────────────┼──────────────────────────────────────────┼──────────────────────────────────┤
│ 💬 CHATS      │ 📁 FILES                 │  Agent Turn: Claude 3.7 Sonnet [High]    │ ┌──────────────────────────────┐ │
│ • App ideas   │ ├─ crates/               │  "I've updated the router math in        │ │ Diff (4) │ Subagents │ Memory│ │
│ • Prompt test │ │  ├─ chorus-core/       │   `router.rs` and added test cases."     │ └──────────────────────────────┘ │
│               │ │  │  ├─ router.rs [M]   │                                          │ ▾ crates/chorus-core/router.rs   │
│ 📁 PROJECTS   │ │  │  └─ lib.rs          │  [Thinking Block: 420 chars]             │   @@ -310,6 +310,12 @@           │
│ ▾ chorus      │ │  └─ chorus-cli/        │                                          │   - let cost = 0.0;              │
│   • Router v2 │ ├─ apps/macos/           │  [Tool Call: `cargo test --bin eval`]    │   + let cost = calculate_penal.. │
│   • Subagent  │ └─ Cargo.toml            │   → Test passed (60/60)                  │   [Keep] [Reject] [💬 Comment]   │
│               │                          │                                          ├──────────────────────────────────┤
│               │ ⚡ AGENT WORKTREES       │ ──────────────────────────────────────── │ ▾ Active Subagents (1/2)         │
│               │ ├─ agent-codex-review    │ [Draft Composer: TextEditor]             │   ▸ 🤖 Code Reviewer (running)   │
│               │ └─ agent-qwen-test       │ [@file @agent /commands] [Clip Attach]   │     Task: Audit edge cases       │
│               │                          │ [Auto: gpt-5-codex (3ms)]    [Send ⌘↩]   │     Transcript: [View Stream ↗]  │
└───────────────┴──────────────────────────┴──────────────────────────────────────────┴──────────────────────────────────┘
```

---

## 3. Four-Column Adaptive Layout

### Column 1: Universal Navigation Sidebar (Width: 240–280px)
- **Top Section: 💬 Chats**: Workspace-free sessions (from Ticket 18).
- **Middle Section: 📁 Projects & Sessions**: Workspace projects and active sessions.
- **Bottom Section**: Integration status, accounts, and Settings (`⌘,`).

### Column 2: Workspace Explorer (Width: 220–280px, Toggleable via `⌘B`)
- **File & Folder Tree**: Powered by `NSOutlineView` (virtualized, 50k+ files). Git status badges (`[M]` modified, `[A]` added, `[?]` untracked).
- **Agent Focus Ring**: Subtle glow/marker indicating which file the active agent is currently reading or editing.
- **Active Worktrees**: Lists parallel agent worktrees (`.chorus/worktrees/*`) with one-click diff inspection.

### Column 3: Center Stage (Flexible, min 500px)
- **Mode A: Chat / Thread**: The conversational transcript, live tool calls, collapsible thinking blocks, and the Composer with instant auto-routing pill.
- **Mode B: Code Viewer / Diff Viewer Split**: When clicking a file or diff, center stage can split or toggle into full syntax-highlighted code viewer (`STTextView` / TextKit 2) with "Open in External Editor (VS Code / Cursor / Zed)" button in the top right.

### Column 4: Inspector: Changes, Subagents & Telemetry (Width: 320–380px, Toggleable via `⌥⌘I`)
- **Tab 1: Changes (Diffs)**:
  - Toggle between **Unified** and **Side-by-Side** diff views.
  - Myers diff alignment pre-calculated in Rust (`similar` crate).
  - Stable per-hunk "Keep" / "Reject" controls.
  - **Inline Comment Box**: Click gutter `+` on any diff line to leave a comment; injected into agent's next turn prompt!
- **Tab 2: Subagents**:
  - Live hierarchy tree of parent ↔ child agents.
  - State badges (`running`, `idle`, `failed`).
  - Child transcript drawer (inspecting child tool calls and thoughts without polluting main thread).
- **Tab 3: Context & Telemetry**:
  - Context window gauge, tokens demoted/dropped, cost tracker.
- **Tab 4: Memory**:
  - Propose / Approve queue for persistent memories.

---

## 4. Keyboard Shortcuts Model (macOS Standard)

| Shortcut | Action |
|---|---|
| `⌃⌘S` | Toggle Navigation Sidebar |
| `⌘B` | Toggle Workspace File Explorer |
| `⌥⌘I` | Toggle Right Inspector (Diffs / Subagents) |
| `⌘1` / `⌘2` / `⌘3` / `⌘4` | Switch Inspector Tabs (Changes, Subagents, Context, Memory) |
| `⌘K` | Command Palette (Slash commands, switch session, jump to file) |
| `⌘↩` | Send message / Dispatched turn |
| `⌘.` | Cancel active turn / stop agent |
| `⇧⌘E` | Open active file in External Editor (VS Code / Zed) |
