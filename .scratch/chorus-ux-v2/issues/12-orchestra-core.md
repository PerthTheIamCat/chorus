# Orchestra mode core

Status: ready-for-agent
Blocked by: 02, 03, 09
Parent: ../spec.md
Decided in: [Orchestra mode](../../chorus-ux-overhaul/issues/10-orchestra-mode.md) · [Security findings triage](../../chorus-ux-overhaul/issues/16-security-triage.md) · research 03 §4

## What

- Keep the MCP tool names and rewrite the internals: `chorus_spawn_agent(role, task, weight)` starts a real Chorus session with `parent_session_id` through the normal engine (relay, sandbox, checkpoints, task state).
- The router picks each child's model under the session preset. A role file's `model:` pins the model instead.
- The orchestrator never writes files. Children's permissions are never wider than the parent's.
- Writing children get a worktree under `~/Library/Application Support/Chorus/worktrees/` (fixes D4).
- `.worktreeinclude` is confined to repo-relative paths. Copying `.env*` is opt-in.
- The orchestrator reviews a child's diff, and merge goes through Review with user approval. Auto-merge is a setting, off by default.
- Caps enforced in Rust: concurrency 3 (1–8), 10 children, 30 min. Hitting a cap pauses and asks.
- Orchestrator failover goes to its backup with a handoff brief.
- Cancelling the orchestrator cancels and reaps its children.
- New roles are usable at once and saved only on "Keep". Names must match `[a-z0-9-]{1,64}`.
- Picks up the deferred P3-4 (pairing attempt budget, constant-time compare) and P3-6 (agent notifications) from triage.

## Done when

- Integration test with the mock agent: spawn two children, one writer and one reader, collect their briefs, hit the cap and pause, cancel, and check no orphan processes remain.
- The `complete_agent` and `wait_agent` timeouts work for real.
