# Agent-facing control surface

Type: grilling
Status: resolved
Blocked by: 02, 03

## Question

What can an agent ask Chorus to do, and through which interface? Decide the following:
- Whether the control surface is a `chorus` CLI, MCP tools on the existing Chorus MCP server, or both over one core API.
- The capability list. Candidates: spawn/ask/wait/collect another agent, read and write memory, list and read attachments, show a file or diff to the user, request a review, notify the user, open a checkpoint, and query session state.
- How agent-initiated actions are permissioned and shown to the user.
- How the CLI reaches a running app's engine.

## Answer

- **Dual Control Surface (MCP & CLI)**: The core API in Rust exposes capabilities through both the built-in Streamable-HTTP MCP server (primary for agents, zero subprocess overhead) and a companion CLI (`chorus agent ...`) for bash tool execution. Both share identical engine primitives.
- **Full Capability Primitive Set**:
  1. *Orchestration*:
     - `chorus_spawn_agent(role, task, write_mode, env)`: Spawns an agent; write mode allocates an isolated Git worktree.
     - `chorus_send_message(agent_id, message)`: Direct inter-agent steering or review comments.
     - `chorus_collect_results(agent_ids, join_mode)`: Collects concise handoff briefs and diff summaries (protecting parent context).
     - `chorus_cancel_agent(agent_id)`: Terminate child session and clean up its worktree.
  2. *Steering & Task Visibility*:
     - `chorus_task_update(tasks)`: Updates active to-do / plan checklist, rendering live in UI's Task State pane.
     - `chorus_show_file(path, line_range)`: Directs the Chorus Workbench file/code viewer to open and highlight code for the human.
     - `chorus_request_review(summary, hunks)`: Presents an interactive approval banner in UI with per-hunk review.
     - `chorus_notify_user(title, message, urgency)`: Fires a native macOS notification with audio chime.
     - `chorus_verify(test_command)`: Runs project verification inside the child worktree before reporting back.
  3. *Memory & Storage*:
     - `memory_propose`, `attachment_list`, `attachment_read`.
- **Custom Agent Roles & AI Generator**: Users can define, customize, and edit custom agent roles via the macOS Settings UI ("Agent Roles") or repo-level markdown files (`.chorus/agents/<name>.md`). Includes an **"AI Role Generator"** (one-click in UI or prompt-driven) where Chorus drafts optimized system instructions, model preferences, and tool boundaries for any specialized task. Orchestrating agents can also synthesize ad-hoc roles dynamically when delegating novel subtasks.
- **Permission & Safety Model**: Inherits session permission mode (`Ask` prompts user via dialog; `AllowAll` executes). Includes an adjustable concurrency slider in settings (default 3, range 1–8 agents) to prevent runaway resource usage.
- **IPC Transport**: Headless communication between CLI and the running macOS app engine uses local Unix Domain Sockets (`.chorus/engine.sock`), guaranteeing sub-millisecond latency, zero port conflicts, and user-isolated file permissions.
