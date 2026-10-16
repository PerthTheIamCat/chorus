# Diff viewer and review loop

Type: prototype
Status: resolved
Blocked by: 08, 13

## Question

How should the user see, review, and steer changes? Decide on unified and side-by-side views, syntax-highlighted diffs, scope switching (per turn, per agent, per session, per worktree), how per-hunk keep/reject carries over, and the **inline comment → send back to agent** loop (batching comments, which agent receives them, how they enter the ledger). Build a rough prototype. Consult the three UI skills named in the map's Notes.

## Answer

- **Dual View Modes (Unified & Split Side-by-Side)**: Myers diff and intra-line word/token diffs computed in Rust (`similar`). For side-by-side diffs, Rust inserts spacer lines in `SideBySidePairModel` to ensure synchronized vertical alignment in a single native scroll view without scroll drift.
- **Four Granular Diff Scopes**: Toolbar dropdown switches between `Latest Turn` (turn delta), `Current Agent` (cumulative agent changes), `Session` (entire session working diff), and `Agent Worktree` (isolated branch diff before merge).
- **Inline Review Comment Steering Loop**:
  - Gutter hover triggers on any diff line open an inline comment box.
  - Multiple comments can be staged across files; a floating bottom bar tracks pending comments.
  - One-click **"Send to Agent (⌘↩)"** bundles comments with file paths, line numbers, and diff context, appends an event to the SQLite ledger, and dispatches the prompt directly to the agent.
- **Per-Hunk Rollback & Selective Keep**: Each hunk retains stable `[Keep]` / `[Reject]` buttons backed by Rust shadow git reverse patching (`chorus_reject_hunk` via `git apply -R`), preserving undamaged changes on disk.

Detailed ASCII wireframe, component layout, and workflow specification: [15-diff-viewer-and-review-loop.md](../prototypes/15-diff-viewer-and-review-loop.md).
