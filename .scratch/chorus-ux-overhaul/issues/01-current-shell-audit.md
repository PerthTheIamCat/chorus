# Current macOS shell audit

Type: research
Status: resolved
Blocked by: —
Parent: ../map.md
Labels: wayfinder:research

## Question

What does the macOS shell actually do today, area by area? For each area, give file:line, what is implemented, and what is missing or broken, compared with the harness-roadmap decisions (tickets 13–19). Areas: the workbench layout and file tree/viewer, code highlighting, sub agent display, the orchestrator UI (`OrchestratorView.swift`) and the agent control surface, the router UI and policy, the composer, the Settings structure, the Context/token panel and how it refreshes, the diff viewer and its baseline, and chat scrolling/list rendering. Also list other UX gaps worth fixing that you notice.

Research: ./research/01-current-shell-audit.md

## Answer

Found by reading the code, not by running the app.

- **Live updates:** Swift never calls `chorus_events_since`, and panels refresh only when `refresh()` runs at turn end. All calls share one serial queue that the running turn holds.
- **Context gauge:** relay results carry no `context_used` (`RelayJson`, lib.rs:1079). Reopening a session resets it, and `usage_update` is ignored.
- **Diff:** `parseHunks` runs 4–5 times per render, and views observe the whole engine.
  - Keep/reject per hunk can't be reached, and the scope picker does nothing.
  - "Split" isn't side by side, and line numbers count within the hunk.
  - The baseline is the shadow HEAD at session open, and it ignores git commits.
- **Highlighting:** a regex colourer in Swift with only 5 languages, instead of the Rust spans ticket 14 decided. Chat code blocks get no colour.
- **Orchestration and subagents:** the Subagents pane is never shown. `chorus_spawn_agent` only writes database rows: no process, no worktree. `complete_agent` is called only from tests. The orchestration settings are saved but never read, and their defaults disagree.
- **Router:** ticket 17 isn't implemented: the Review tier floor is still there and there's no cost term. The router logs its own picks as learned routes.
- **Composer:** no auto-focus. Return sends during IME composition.
- **Smaller gaps:** collapsed folders re-expand every 15 s, shortcuts collide, there's no menu bar, the copy mixes Thai and English, and the permission dialog names the wrong agent.

Detail: [research](../research/01-current-shell-audit.md).
