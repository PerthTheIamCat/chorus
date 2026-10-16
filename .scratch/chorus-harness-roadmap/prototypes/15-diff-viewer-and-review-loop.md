# Prototype: Diff Viewer and Review Loop

## 1. Overview & Architectural Foundation

Under **Rule 2.3** ("No logic in Swift") and research from [08-macos-code-and-diff-components](../research/08-macos-code-and-diff-components.md):
- **Diff Computation in Rust**: Myers diff, intra-line word/token diffs, and side-by-side line pairing with spacers are computed in `chorus-core` via the `similar` crate.
- **Rendering in AppKit / SwiftUI**: Rendered natively using virtualized text views (TextKit 2) ensuring 120Hz scrolling across 10,000+ line diffs.
- **Ledger-Backed Review**: Review comments are first-class structured events recorded in the SQLite ledger before being dispatched into the agent's turn prompt.

---

## 2. Diff Viewer UI Layout & Wireframe

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│  CHANGES / DIFF INSPECTOR                                       [Unified | Split ◫]   │
│  Scope: [ Latest Turn ▾ ]   Target: [ All Files (3) ▾ ]        [Revert All] [Apply All]│
├────────────────────────────────────────────────────────────────────────────────────────┤
│ ▾ crates/chorus-core/src/router.rs                    +14 -3   [Keep Hunk] [Reject Hunk]│
│   @@ -310,7 +310,18 @@                                                                │
│   310   310     let score = evaluate_task_score(&prompt);                              │
│   311       -   let cost = 0.0;                                                        │
│         311 +   let cost = calculate_penalty(task_difficulty);                         │
│ ┌────────────────────────────────────────────────────────────────────────────────────┐ │
│ │ 💬 Review Comment on L311:                                                         │ │
│ │ "Don't hardcode calculate_penalty; pass the configured cost penalty lambda from    │ │
│ │  the router config instead."                                                       │ │
│ │  [Remove] [Edit]                                                                   │ │
│ └────────────────────────────────────────────────────────────────────────────────────┘ │
│   312   312     let utility = score - cost;                                            │
│   313   313     if utility > threshold {                                               │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ ▾ crates/chorus-cli/src/session.rs                     +5 -1   [Keep Hunk] [Reject Hunk]│
│   @@ -124,3 +124,7 @@                                                                 │
│   ...                                                                                  │
├────────────────────────────────────────────────────────────────────────────────────────┤
│ ⚡ PENDING REVIEW BAR (Floating Bottom Banner)                                         │
│ 2 comments in 2 files                                  [Clear All]  [ Send to Agent ⌘↩ ]│
└────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Key Interaction Workflows

### 3.1 Dual View Modes (Unified & Split)
- **Unified Diff**: Classic single-column view with green (`+`) and red (`-`) line tints and intra-line highlighted word changes.
- **Split (Side-by-Side) Diff**: Dual-column view (Old on left, New on right). Rust's `SideBySidePairModel` inserts virtual spacer lines to ensure both columns remain vertically aligned without dual-scroll drift.

### 3.2 Granular Scope Switching
A dropdown in the diff toolbar allows scoping changes to:
1. **Latest Turn**: Only modifications produced in the most recent turn.
2. **Current Agent**: Cumulative modifications made by the current agent.
3. **Session**: Entire working diff since the session began.
4. **Agent Worktree**: Isolated diff generated in a background worktree before merging into main.

### 3.3 Gutter Hover & Inline Review Comments
1. Hovering over any line number in the diff reveals a blue `+` icon in the gutter.
2. Clicking opens an inline auto-growing comment box with Markdown support.
3. The user can navigate to other hunks or files and add multiple comments.
4. Comments remain persistently pinned below their target line.

### 3.4 The "Send to Agent" Steering Loop
1. When comments are staged, a floating **Pending Review Bar** appears at the bottom.
2. Clicking **"Send to Agent" (⌘↩)**:
   - Formats comments into an authoritative review block referencing exact file paths, line numbers, and diff contexts.
   - Records an event in the SQLite ledger.
   - Automatically dispatches the prompt to the responsible agent (or switches to a dedicated `Code Reviewer` / fix agent).
   - Dismisses the comment boxes upon turn dispatch.

### 3.5 Per-Hunk Rollback Integration
- Each hunk carries a **[Keep] / [Reject]** toggle.
- Rejecting a hunk triggers `chorus_reject_hunk` via Rust shadow git (`git apply -R`), instantly restoring that specific hunk on disk while leaving other hunks intact.
