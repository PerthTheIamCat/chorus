# Prototype: File, Folder, and Code Viewer

## 1. Architectural Strategy & Rule 2.3 Compliance

Guided by [08-macos-code-and-diff-components](../research/08-macos-code-and-diff-components.md) and **`anti-ui-slop`**:
- **Zero Parser Overhead in Swift**: File hierarchy traversal (`ignore` crate + git status) and syntax highlighting (`syntect` / `tree-sitter`) run entirely in Rust (`chorus-core`). Rust passes structured highlight spans (`HighlightSpan { start, len, scope }`) and outline nodes across C FFI.
- **Virtualized High-Performance Rendering**:
  - File tree: AppKit `NSOutlineView` wrapped in `NSViewRepresentable` (flawless 120Hz scrolling across 50,000+ files; eliminates SwiftUI `OutlineGroup` lag).
  - Code viewer: AppKit `NSTextView` backed by TextKit 2 (smooth viewport virtualization, native selection, VoiceOver accessibility).
- **Premium Native Typography**: Uses Apple's SF Mono and New York system fonts; zero emojis—strictly native SF Symbols (`folder.fill`, `doc.text`, `arrow.up.forward.app`).

---

## 2. Component Wireframe & Layout

```
┌──────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│ WORKSPACE EXPLORER (⌘B)              │ CODE VIEWER (Center Stage Split)                                      │
├──────────────────────────────────────┼───────────────────────────────────────────────────────────────────────┤
│ [ Filter files... (⌘P)             ] │ 📁 chorus > crates > chorus-core > src > router.rs  [ ↗ Open in Cursor ▾ ]│
├──────────────────────────────────────┼───────────────────────────────────────────────────────────────────────┤
│ ▾ 📁 crates                          │   1  //! Pre-dispatch task router for Chorus.                         │
│   ▾ 📁 chorus-core                   │   2  use crate::types::{AgentId, ModelId};                           │
│     ▾ 📁 src                         │   3  use anyhow::Result;                                             │
│       • 📄 attachments.rs            │   4                                                                  │
│       • 📄 context.rs                │   5  /// Evaluates task complexity and recommends model.             │
│       • 📄 ledger.rs                 │   6  pub struct TaskRouter {                                         │
│       • 📄 router.rs  ● [M]          │   7      pub fn evaluate_task(prompt: &str) -> RouteProposal {       │
│       • 📄 store.rs                  │   8          let embedding = embed_query(prompt)?;                   │
│   ▸ 📁 chorus-cli                    │   9          let score = classify_probe(&embedding);                 │
│ ▾ 📁 apps                            │  10          // Apply explicit cost penalty                          │
│   ▾ 📁 macos                         │  11          let utility = score - (LAMBDA * cost);                  │
│     • 📄 ContentView.swift           │  12          RouteProposal::new(utility)                             │
│ ──────────────────────────────────── │  13      }                                                           │
│ ⚡ ACTIVE AGENT WORKTREES            │  14  }                                                               │
│ • 🌿 agent-worktree-review [3 files] │                                                                      │
└──────────────────────────────────────┴───────────────────────────────────────────────────────────────────────┘
```

---

## 3. Interaction Mechanics

### 3.1 Workspace Explorer (`NSOutlineView`)
- **Git Badges & Agent Touch Ring**:
  - `●` Glowing blue dot indicates the active agent is currently reading or editing this file.
  - `[M]` (Amber): Working tree modifications.
  - `[A]` (Green): Staged new files.
  - `[?]` (Tertiary): Untracked files.
- **Instant Filter (`⌘P` / Quick Open)**: Quick search field at top of explorer filters files instantly using Rust's fuzzy matcher.

### 3.2 Code Viewer (TextKit 2 / SF Mono)
- **Monospace Gutter & Line Anchors**: Clean line numbers in `.quaternary` color. Clicking any line copies the exact markdown link (e.g. `[router.rs:L11](file:///path/to/router.rs#L11)`).
- **Selection Context Staging (`⌘L`)**: Dragging over lines 6–13 and pressing `⌘L` stages an interactive context chip `[@router.rs:6-13]` into the composer.
- **No Heavy Minimap**: Avoids memory-heavy canvas minimaps; uses native macOS scrollbar tick marks for search hits and agent diffs.

### 3.3 External Editor Hand-Off (`⇧⌘E`)
- In the viewer header, a dedicated button shows the user's default external IDE (auto-detects VS Code, Cursor, Zed, Xcode):
  - Clicking `[ ↗ Open in Cursor ]` or pressing `⇧⌘E` immediately opens the file in the external editor and jumps to the exact line number (`cursor --goto <path>:<line>`).
  - Dropdown allows switching the preferred external editor.
