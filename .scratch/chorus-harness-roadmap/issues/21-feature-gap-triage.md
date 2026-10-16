# Feature gap triage

Type: grilling
Status: resolved
Blocked by: 01, 02, 03, 04

## Question

Go through every feature the surveys found that Chorus lacks and is not covered by another ticket, and rule each one in this roadmap (with a phase), in the fog, or out of scope. Known candidates from the inventory: notifications, integrated terminal, git commit/PR flow, cross-session full-text search, browser/app preview, monetary cost display, plan/todo display, hooks, project rules management, ask-each mode, and session export/share. New tickets are spawned for any that need their own decision.

## Answer

- **In Roadmap (Phased Allocation)**:
  - **Phase 1 (Foundation & Core Scaffolding)**:
    - *macOS Native Notifications with Audio Chime*: Dispatches system notifications via `NSUserNotificationCenter` with an elegant audio chime on task/agent completion.
    - *Automated Post-Edit Verification Hooks*: Pre-dispatch and post-turn hooks in `.chorus/hooks/` (e.g. `post-turn.sh` running `cargo check`, linters, or test suites).
    - *Project Rules Projection*: Universal injection of `.chorus/rules/`, `AGENTS.md`, and `CLAUDE.md` into agent system instructions across all bridges.
    - *Granular Permissions / Ask-Each*: Granular approval levels (Read=Auto, Write/Bash=Ask) mapped to the async `Interaction` state machine.
  - **Phase 2 (Workbench UI)**:
    - *Plan / Task Checklist Card*: Live progress checklist rendered in the Inspector or floating glass card, integrated with `chorus_update_checklist`.
    - *1-Click Git Commit & PR Flow*: Diff review acceptance toolbar offers 1-click conventional commit generator and push/PR creation via `gh` CLI.
  - **Phase 3 (Multi-Agent & Orchestration)**:
    - *Dynamic Port Browser Trigger*: Detects allocated dev server ports in worktrees and surfaces a 1-click external browser launch button (`[ ↗ Open localhost:3001 ]`).
  - **Phase 4 (Router & Analytics)**:
    - *Monetary Cost Accounting*: Calculates and displays real-time dollar cost per turn, session, and worktree based on token usage.
  - **Phase 5 (Chat Mode & Sessions)**:
    - *SQLite FTS5 Cross-Session Search*: Instant sub-millisecond search across historical messages, sessions, and code snippets (`⌘F`).
    - *Session Export & Share*: Formats and exports conversation threads, diffs, and briefs to Markdown and HTML.
- **In Fog (Deferred)**:
  - *Embedded WKWebView Browser Preview*: Deferred; opening native macOS default browser provides higher fidelity and lower resource footprint for desktop.
  - *Cloud Sync & Cross-Device Telemetry*: Deferred until public multi-device release.
- **Out of Scope (Explicitly Excluded)**:
  - *Full Embedded Interactive PTY Terminal*: Chorus is a view-first agent supervisor; command outputs are streamed into the Thread/Transcript without embedding a terminal emulator.
  - *In-App Code Editing / LSP Refactoring*: Chorus remains view-first with fast external editor handoff (`⇧⌘E`).
