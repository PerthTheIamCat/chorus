# File, folder, and code viewer

Type: prototype
Status: resolved
Blocked by: 08, 13

## Question

How should Chorus show the project's files, folders, and code? Decide on the file tree (git status badges, agent-touched markers, filtering), the code viewer (syntax highlighting, line numbers, minimap or not, jump from thread mentions and diffs), the rendering stack picked from the components research, and the "Open in external editor" flow. Build a rough prototype. Consult the three UI skills named in the map's Notes.

## Answer

- **High-Performance Virtualized File Tree (`NSOutlineView`)**: Powered by AppKit `NSOutlineView` wrapped in SwiftUI to handle 50,000+ files at smooth 120Hz without SwiftUI `OutlineGroup` performance degradation. Traversal and `.gitignore` filtering run in Rust.
- **Visual Status Badges & Agent Focus Ring**:
  - Git status badges: `[M]` (modified, amber), `[A]` (added, green), `[?]` (untracked).
  - Glowing blue indicator (`●`) illuminates next to the file currently being actively read or edited by the agent in real time.
- **Native TextKit 2 Code Viewer**: Read-only viewer in SF Mono with line numbers and copyable line anchors. Syntax highlight spans calculated in Rust (`syntect`/`tree-sitter`) and applied via FFI without Swift parsing. Minimap omitted in favor of clean native scrollbar diff tick marks.
- **External Editor Hand-Off (`⇧⌘E`)**: Automatically detects installed macOS editors (VS Code, Cursor, Zed, Xcode) with a prominent toolbar jump button (`[ ↗ Open in Cursor ]`) passing `--goto <path>:<line>` to open the exact file and cursor position instantly.

Detailed ASCII wireframe, component layout, and interaction specifications: [14-file-and-code-viewer.md](../prototypes/14-file-and-code-viewer.md).
