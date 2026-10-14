# macOS code and diff rendering components

Type: research
Status: resolved
Blocked by:

## Question

What should Chorus use to render a file tree, code with syntax highlighting, and unified plus side-by-side diffs natively in SwiftUI/AppKit? Evaluate tree-sitter (via Rust core vs. Swift bindings), Highlightr, Splash, STTextView, CodeEditSourceEditor / CodeEditTextView, Runestone, and a WKWebView approach with Shiki or Monaco's read-only diff viewer. Judge each one on performance with large files and diffs, theming and dark mode, accessibility, license, maintenance health, and fit with the "logic in Rust" rule (for example, Rust producing highlight spans and diff hunks that Swift only draws). Also check how Zed, CodeEdit, and the Codex app render diffs.

## Answer

- **Enforce Rule 2.3 ("No logic in Swift")**: Deprecate ad-hoc Swift diff parsing (`parseHunks`). All diffing, hunk extraction, line-by-line alignment, and syntax highlight spans must be computed in Rust (`chorus-core`) and passed as structured models via FFI, keeping the core 100% portable to WinUI 3 and headless daemons.
- **File Tree (`NSOutlineView`)**: Driven by a Rust workspace scanner (`ignore` + git status). Render with AppKit `NSOutlineView` wrapped in SwiftUI via `NSViewRepresentable`. Its native view recycling easily handles 50,000+ files, avoiding the severe performance drops and bugs found in SwiftUI `OutlineGroup` (as evidenced by CodeEdit).
- **Code Viewer (TextKit 2 / `STTextView`)**: Read-only AppKit `NSTextView` backed by TextKit 2 (or `STTextView`) applying highlight spans generated in Rust by `syntect` or `tree-sitter`. This provides viewport-virtualized layout, smooth 120Hz scrolling, native selection/copy, full VoiceOver accessibility, and system dark/light mode theming with zero parser overhead in Swift.
- **Unified & Side-by-Side Diff Viewer (`similar` in Rust)**: Rust computes Myers diffs and word/token-level inline diffs using the `similar` crate. For side-by-side diffs, Rust pre-aligns left and right lines by inserting explicit spacer lines (`SideBySidePairModel`), enabling Swift to render split diffs in a single scrollable view without dual-scroll synchronization drift (adapting Zed's alignment approach).
- **Evaluation of Disqualified Candidates**: `Splash` is disqualified (Swift-only); `Runestone` is disqualified (iOS/UIKit only, no native AppKit); `Highlightr` is disqualified (abandoned, slow JavaScriptCore bridge); `CodeEditSourceEditor` is rejected due to tight coupling with heavy Swift SPM packages; `WKWebView + Monaco/Shiki` is rejected due to high WebKit process memory overhead (40–80 MB/instance) and non-native feel.
- **Review Loop Integration**: Stable hunk IDs connect "Keep" / "Reject" actions directly to Rust reverse-patching (`chorus_reject_hunk`). Diff line gutter hover triggers allow users to leave inline review comments that are saved to the ledger and injected into the agent's next turn prompt (mirroring the Codex app review pattern).
- Detailed evaluation matrix, citations, and migration plan: [../research/08-macos-code-and-diff-components.md](../research/08-macos-code-and-diff-components.md).
