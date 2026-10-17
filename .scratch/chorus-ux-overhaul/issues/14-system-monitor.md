# System monitor

Type: grilling
Status: resolved
Blocked by: 02, 03
Parent: ../map.md
Labels: wayfinder:grilling

## Question

What does the system monitor show and allow? Decide: where it lives (pane, window, menu bar), the columns (port, process, service name, CPU, RAM, started by agent), how often it refreshes, the stop flow and its safety rules, Chorus's own usage (RAM, disk by category), and which optimise actions are offered (blob GC, worktree prune, VACUUM, unload the local model, clear caches).

Constraints from Security findings triage: follow research 03 §4 "Killing processes" (re-check pid, start time and path before `kill`; no MCP kill tool; mask token-like command-line arguments; never show environment variables).

## Answer

Perth accepted all seven recommendations (2026-10-17). The technical approach is in research 02, and the stop rules come from Security findings triage (research 03 §4).

1. **Placement:** a separate window, opened from the Window menu or `⌥⌘M`. A small Chorus RAM label in the sidebar footer also opens it.
2. **Scope:** two groups.
   - **Chorus:** the app, the daemon, each session's agent bridge, and the local model.
   - **Services:** processes that own a listening port.
   - Not a full process list, since Activity Monitor already covers that.
3. **Services columns:** `Port · Service · Process · PID · CPU · RAM (footprint) · started by (agent/session, a hint) · uptime`, sorted by port.
   - TCP by default, with a UDP toggle.
   - Other users' listeners show greyed out with no actions.
4. **Refresh:** every 1 s while the window is visible. Stopped when it is hidden or minimised. No background polling.
5. **Stopping:**
   - A confirm sheet shows name, pid, path and the agent-spawned hint, then sends SIGTERM.
   - If the process is still alive after 3 s, the row offers **"บังคับปิด"** (SIGKILL), clicked explicitly and never automatic.
   - "Stop everything this session's agents started" uses `killpg` only on groups Chorus created.
   - All triage rules apply: re-check uid, start time and path; the system-path denylist; no MCP kill tool; masked command lines; environment variables never shown.
6. **Disk usage:** `ledger (store.db + wal) · checkpoints · worktrees · models · memory · cache`, with a per-project breakdown.
7. **Optimise actions:** all manual. Each shows the estimated space it will free, then asks to confirm.
   - **Non-destructive:** blob GC · VACUUM (only when no turn is running) · unload the local model · clear caches.
   - **Destructive, with a list to pick from:** prune merged, discarded or orphaned worktrees · prune checkpoints older than N days, keeping any `review_base` points to · delete unused models.
   - Scheduled clean-up stays in the fog.
