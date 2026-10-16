# Multi-agent orchestrator survey

Type: research
Status: resolved
Blocked by:

## Question

How do multi-agent harnesses run, isolate, and coordinate parallel agents? Cover Conductor, Crystal/Nimbalyst, Vibe Kanban, Claude Squad, Sculptor, Amp, ORCA (identify which well-regarded "Orca" agent orchestrator the user means), Jules / Codex cloud, plus any other well-reviewed open-source orchestrators found along the way (for example, claude-flow, uzi, and similar projects). For each one, record: the worktree/container isolation model and lifecycle (create, run, merge back, clean up), whether **agents themselves** can spawn or call other agents (and through which CLI, MCP, or API primitives), how results come back (wait, collect, notify), concurrency and quota limits, and the UI for watching many agents at once. Pull out the primitive set that agent-driven orchestration needs.

## Answer

- **Landscape & Isolation Model:** Local desktop orchestrators (Conductor, Nimbalyst, Vibe Kanban, Claude Squad, Uzi, Stably Orca) uniformly converge on **Git worktrees** over container virtualization for sub-200ms setup, zero-copy Git storage, and branch/terminal isolation; cloud engines (Google Jules, OpenAI Codex Cloud, Sculptor) isolate via transient microVMs or Docker containers.
- **ORCA Identified:** In the coding agent landscape, "ORCA" refers to **Stably AI's open-source Agent Development Environment (ADE)** (`stablyai/orca`), which manages fleets of parallel agents across Git and SSH worktrees with orchestration skills for coordinator loops.
- **Worktree Lifecycle Essentials:** Raw `git worktree add` reliably breaks builds unless the harness copies untracked environment configs (Conductor's `.worktreeinclude` pattern for `.env.local`) and dynamically assigns conflict-free ports (Uzi's `$PORT` allocation pool). Cleanup requires background garbage collection after task completion plus startup orphan pruning.
- **Agent-to-Agent Spawning:** Systems split between human-only boards (Vibe Kanban, Claude Squad) and agent-callable APIs. Leading agent-driven systems use MCP or tool calls: Conductor exposes Streamable HTTP MCP tools (`create_workspace`, `create_session`, `send_message`), Anthropic Claude Code provides the `Agent` tool with `.claude/agents/*.md`, and OpenAI Agents SDK provides `handoff()` (transferred ownership) and `agent.asTool()` (orchestrator retention).
- **Result Collection & Context Protection:** To prevent fatal context window bloat, parent agents collect compact final briefs and diff summaries rather than raw child transcripts, while full turn events stream to the ledger for human observability.
- **Recommended Primitive Set for Chorus:**
  1. *Internal Core Worktree Engine (Rust):* `worktree_create` (with `.worktreeinclude` copy and port injection), `worktree_diff`, `worktree_merge` (clean squash/fast-forward), and `worktree_cleanup` (safe remove + prune). Read-only agents share the main tree.
  2. *Agent Control Surface (MCP/CLI):* `chorus_spawn_agent` (spawns background subagent, allocates worktree if write mode), `chorus_send_message` (inter-agent steering / review comments), `chorus_collect_results` (supports join modes `"all"`, `"any"`, and non-blocking `"poll"`), `chorus_list_subagents`, and `chorus_cancel_agent`.

Detailed findings, comparative tables, and architectural specifications are documented in [../research/02-multi-agent-orchestrator-survey.md](../research/02-multi-agent-orchestrator-survey.md).
