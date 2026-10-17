# Orchestra mode

Type: grilling
Status: resolved
Blocked by: 01
Parent: ../map.md
Labels: wayfinder:grilling

## Question

How does orchestra mode work end to end? Decide: where the switch lives and how it turns auto routing off; the orchestrator setting (default Claude Opus 5.5, medium); the role library format and how a new role is created and shown; what the orchestrator sends the router for each piece of work (role, how heavy the work is); the concurrency cap; what happens when the orchestrator or a child hits a limit; and what reuses the earlier control surface and worktree decisions (harness-roadmap 11, 12).

Note from the audit: `chorus_spawn_agent` only writes database rows today (no process, no worktree) and nothing calls `complete_agent`. Decide whether orchestra mode builds on it or replaces it.

Constraints from Security findings triage: follow research 03 §4 "Orchestra mode spawning agents":
- A child's permissions are never wider than its parent's.
- MCP calls are authenticated with a per-session secret.
- The control surface writes to the ledger through `ledger::append` as the agent.
- Concurrency and spend caps are enforced in Rust.
- `.worktreeinclude` is confined, and copying `.env*` is opt-in.
- Worktrees live in `~/Library/Application Support/Chorus/worktrees/`.

Deferred to here: P3-4 (pairing attempt budget and constant-time compare) and P3-6 (agents posting notifications).

## Answer

Perth accepted all ten recommendations (2026-10-17).

1. **Switch:** a per-session `Auto route | Orchestra` menu in the composer, taking effect from the next turn. Settings only holds the default for new sessions. Remove the "ขนาน" toggle and the unread `chorus.orchestrationEnabled`.
2. **Orchestrator's own work:** it coordinates and may read the code and answer simple questions itself, but it **never writes files**. All writes go to children.
3. **Orchestrator failover:** the default orchestrator is Claude Opus 5.5 at medium effort, with a configurable backup. The backup defaults to the strongest ready model from another provider. On a limit it relays with the usual handoff brief and says so in the thread.
4. **Routing work:** the orchestrator sends `role`, `task` and `weight` (`light | normal | heavy`). The router picks agent/model/effort under the session's route preset. A role file's `model:` pins a model and skips the router. The orchestrator cannot name a model directly.
5. **Build on the existing tools:** keep the MCP tool names (`chorus_spawn_agent / send_message / collect_results / cancel_agent`) and rewrite the internals. Each child is a real Chorus session with `parent_session_id`, run through the normal engine: relay, sandbox, ledger, checkpoints and task state. The row-only stub goes.
6. **Worktrees:** writing children each get a worktree in `~/Library/Application Support/Chorus/worktrees/`; read-only children share the main tree. When a child finishes, the orchestrator reviews its diff and the user approves the merge in Review. Auto-merge is a setting, off by default.
7. **Caps, enforced in Rust:** concurrency 3 (range 1–8), 10 children per orchestration, 30 minutes wall time. Hitting a cap **pauses and asks** instead of failing. All caps are configurable.
8. **Plan visibility:** the plan appears as a live `chorus_task_update` checklist in the thread and dispatch starts immediately, with Stop always visible. A setting "ask before starting the plan" is off by default. No countdown.
9. **Trivial turns:** the orchestrator answers directly without spawning when no delegation is needed. It decides this itself.
10. **New roles:** a role the orchestrator creates is usable at once and labelled "new role" on its sub session. It is written to `.chorus/agents/` only when the user clicks "Keep". Names must match `[a-z0-9-]{1,64}`.

Security rules from Security findings triage apply on top of these: children never get wider permissions than the parent, MCP calls are authenticated per session, ledger writes go through `ledger::append`, `.worktreeinclude` is confined, and copying `.env*` is opt-in.
