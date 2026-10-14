# Agent-driven orchestration and worktrees

Type: grilling
Status: resolved
Blocked by: 02, 11, 20

## Question

How does agent-driven orchestration work end to end? Decide the following:
- How a spawned session links to its parent in the ledger.
- The worktree lifecycle: create, branch naming, run, show diff, merge back or discard, conflicts, clean up.
- Concurrency and quota limits across accounts.
- How the parent waits on and collects results.
- Failure and cancel semantics.
- How handoff and relay interact with spawned sessions.
- How the user watches and steers many parallel agents.

## Answer

- **Ledger Lineage & Linking**:
  - Spawned sessions are first-class records in the SQLite Ledger with `parent_session_id` and a hierarchically indexed `lineage_path` (e.g. `root_id/child_1`).
  - The spawning event links the caller's specific `tool_call_id` to the child's genesis turn, allowing bidirectional navigation in both UI and headless APIs.
- **Git Worktree Lifecycle**:
  - **Location & Branch**: Isolated worktrees are provisioned under `.chorus/worktrees/<role>-<short_id>/` on dedicated branches `chorus/<role>-<short_id>` branched from current HEAD (<200ms setup).
  - **Environment Replication (`.worktreeinclude`)**: Automatically synchronizes uncommitted/gitignored environment files (e.g., `.env`, `.env.local`) and symlinks package dependency caches (`node_modules`, `target`) to allow immediate test verification without redundant installs.
  - **Dynamic `$PORT` Allocation**: Automatically injects dynamic environment ports (e.g. `PORT=3001, 3002...`) per worktree to prevent dev server collisions.
  - **Merge vs. Discard**:
    - **Clean Auto-Merge**: If the child's changes can be cleanly fast-forwarded or merged without conflicts, the branch merges into the active working tree, and the worktree directory is cleanly removed.
    - **Conflict Handling**: If conflicts occur, a warning banner appears with 3 resolution paths: agent self-heal prompt, manual resolution in external editor (`⇧⌘E`), or one-click discard.
    - **Discard**: `git worktree remove --force` purges files and branch without touching parent tree.
- **Context Protection (Compact Brief Collection)**:
  - When child agents complete, `chorus_collect_results` returns a compact structured brief: 3–5 line prose summary, execution status, diff stat (`+45, -12`), touched files, and test verification output.
  - Full internal child thoughts and tool traces remain queryable via `chorus_read_session_log` but are NEVER dumped en masse into the parent session context, preventing context exhaustion.
- **Concurrency & Resource Limits**:
  - Default parallel worktree concurrency cap: **3 agents** (configurable in Settings between 1–8).
  - Rate-limit handling: 429/quota errors trigger exponential backoff queues rather than failing active branches.
- **User Supervision & Live Steering**:
  - The Workspace Explorer displays a dedicated `⚡ ACTIVE AGENT WORKTREES` list showing real-time child statuses.
  - Users can click any worktree to inspect its live Center Stage thread, code viewer, and diff viewer.
  - Users can intervene at any moment: inject steering prompts (`⌘↩`), pause execution, or cancel (`[ Stop ]`) individually without interrupting other concurrent agents.
