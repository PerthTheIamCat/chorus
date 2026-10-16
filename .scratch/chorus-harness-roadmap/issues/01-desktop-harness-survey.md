# Desktop harness feature survey

Type: research
Status: resolved
Blocked by:

## Question

How do the leading desktop/IDE agent harnesses lay out their UI and support the agent loop? Cover Claude Code Desktop / claude.ai Code, the Codex app, Cursor (Agents window and background agents), Zed (ACP agent panel), Google Antigravity, Windsurf, and Kiro. For each one, record: window layout (sidebar, thread, inspector, tabs, multi-session view), how files, folders, and code are shown, how diffs are shown and reviewed (unified/side-by-side, per-turn scope, inline comments back to the agent, accept/reject), plan or todo display, checkpoints and rewind, permissions UX, notifications, terminal, browser preview, git commit/PR flow, and keyboard/command-palette design. End with a list of features Chorus lacks today, judged against the current inventory in the map's Notes and in CLAUDE.md, and rank them by how much each would help agents work effectively.

## Answer

- **Survey Scope**: Investigated 7 leading desktop/IDE agent harnesses (Claude Code Desktop, Codex App / ChatGPT Work, Cursor, Zed ACP Agent Panel, Google Antigravity, Windsurf, Kiro) across all 11 required dimensions with primary-source citations and structured comparisons.
- **Diff Review & Steering Loop**: Frontier harnesses converge on unified and side-by-side diff viewers where **inline diff comments feed directly back to the agent's prompt context** (Codex app, Claude Code, Cursor), enabling tight human-in-the-loop steering without manual copying.
- **Multi-Agent Isolation via Worktrees**: Parallel write agents require isolated Git worktrees (standard in Codex app, Claude Code, and Antigravity branch workspaces) to prevent workspace and index corruption during concurrent multi-agent pairing.
- **View-First Supervisor Advantage**: Dedicated harnesses (Antigravity 2.0, Claude Desktop, Chorus) maximize velocity by serving as high-fidelity observers and orchestrators (live file tree, diff inspector, external editor hand-off) rather than duplicating complex IDE editing or LSP infrastructure.
- **Checkpoints & Granular Rewind**: Automated turn-by-turn checkpointing with granular rewind controls (reverting code vs conversation, e.g. Claude Code `/rewind`, Cursor checkpoints, Kiro `/checkpoint`) is standard practice to recover safely from agent mistakes.
- **Spec-Driven Planning**: Formal living checklists and spec hierarchies (Kiro's `requirements`/`design`/`tasks.md`, Antigravity's `/plan` artifacts, Claude Code's Plan mode) anchor model focus and prevent drift over long-horizon tasks.
- **Ranked Chorus Gaps**:
  1. *Inline diff comments sent back to agent* (highest leverage steering loop)
  2. *Git worktree isolation for parallel write agents* (safe multi-agent concurrency)
  3. *View-first file tree & syntax-highlighted code viewer* (context visibility & external editor hand-off)
  4. *Side-by-side split diffs with per-turn / per-agent scoping*
  5. *Interactive checkpoint & rewind UI timeline*
  6. *Subagent tree with live status, child transcripts, and process controls*
  7. *Interactive plan / spec / task-state checklist*
  8. *Embedded terminal / streaming PTY output*
  9. *Command palette (`Cmd+K`) and slash command surface*
  10. *Native macOS completion notifications & audio chimes*
  11. *Headless daemon core boundary (remote-ready API)*
  12. *Integrated browser preview (webview)*
- Full detailed findings and comparison matrix: [01-desktop-harness-survey.md](../research/01-desktop-harness-survey.md).
