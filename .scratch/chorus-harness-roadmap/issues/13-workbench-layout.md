# Workbench layout

Type: prototype
Status: resolved
Blocked by: 01

## Question

What should the main Chorus window look like as a harness workbench? Decide how the project/session sidebar, thread, inspector, file tree, code viewer, diff viewer, subagent view, and parallel-agents view fit together: panes vs. tabs vs. split views, what is primary per mode, menu bar and keyboard model, and how multi-session parallel work is shown. Build a rough UI prototype to react to. Consult `anti-ui-slop`, `macos-design-guidelines`, and `ui-ux-pro-max`, and the existing `docs/ref/Chorus Desktop Blueprint`.

## Answer

- **Four-Column Adaptive Layout Architecture**:
  1. *Universal Sidebar* (260px): Divided into top 💬 Chats and bottom 📁 Projects & Sessions.
  2. *Workspace Explorer* (240px, Toggle `⌘B`): Virtualized file tree (`NSOutlineView`), Git badges (`[M]`, `[A]`), active agent focus ring, and live list of parallel Agent Worktrees.
  3. *Center Stage* (Flexible): Dynamically splits into side-by-side **Thread (left)** and **Code Viewer (right)** upon clicking file references or code, allowing simultaneous code inspection and conversation steering.
  4. *Inspector* (340px, Toggle `⌥⌘I`): 4 tabs—Changes (Unified/Side-by-side diff with inline comments), Subagents (hierarchy tree & live transcript stream), Context budget gauge, and Memory approval queue.
- **Visual Aesthetic & Styling**: Retains Chorus's distinctive **Floating Glass Cards** with subtle depth shadows and rounded continuous corners over a soft backdrop, enhanced with interactive split dividers allowing custom width resizing.
- **Keyboard & Navigation Model**: Fully supports standard macOS shortcuts (`⌃⌘S` sidebar, `⌘B` file tree, `⌥⌘I` inspector, `⌘1–4` inspector tabs, `⇧⌘E` external editor hand-off, `⌘K` command palette, `⌘↩` send, `⌘.` abort).
- **Compliance with Rule 2.3**: File hierarchy scanning, highlight spans, diff hunks, and subagent state graphs are computed 100% in Rust (`chorus-core`) and consumed by Swift purely for rendering.

Detailed ASCII wireframes, component hierarchy, and interaction specifications: [13-workbench-layout.md](../prototypes/13-workbench-layout.md).
