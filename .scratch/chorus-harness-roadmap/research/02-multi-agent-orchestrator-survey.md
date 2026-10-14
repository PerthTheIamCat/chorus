# Multi-Agent Orchestrator Survey & Worktree Lifecycle Architecture

This survey examines modern multi-agent harnesses, developer environments, and orchestration systems to determine how they isolate parallel agent execution, handle worktree and container lifecycles, enable agent-to-agent delegation, collect results, and display multi-agent progress. It concludes with a recommended primitive set for Chorus's agent-driven orchestration architecture.

---

## 1. Executive Summary & Paradigm Overview

Across the developer tooling ecosystem, multi-agent execution has coalesced around two primary architectural axes:

1. **Isolation Model**:
   - **Local Git Worktrees**: Ephemeral linked checkouts sharing the main `.git` object database. Worktrees provide filesystem and terminal isolation without container virtualization overhead. This is the dominant pattern for local developer desktop tools (Conductor, Nimbalyst, Vibe Kanban, Claude Squad, Uzi, Stably Orca).
   - **Sandboxed MicroVMs / Containers**: Full Linux containerization (Docker, gVisor, Kata Containers, Cloud VMs). Necessary when untrusted code execution or isolated system dependencies (ports, system packages, OS daemons) are required (Sculptor, Google Jules, Codex Cloud, OpenHands).

2. **Orchestration Driver**:
   - **Human-in-the-Loop Supervision**: The human breaks tasks down via a UI (Kanban, grid, terminal multiplexer) and assigns them to independent agent instances running in parallel worktrees (Vibe Kanban, Claude Squad, Nimbalyst).
   - **Agent-Driven / Hierarchical Swarms**: A root or manager agent autonomously spawns, dispatches tasks to, and awaits results from specialized subagents via MCP tools, CLI bridges, or SDK APIs (Claude Code subagents, OpenAI Agents SDK, Conductor MCP, Amp, Claude-Flow).

For Chorus—which follows an **agent-driven** philosophy where Chorus acts as the engine rather than the planner—the ideal architecture combines lightweight Git worktree isolation for writing agents with a lean set of MCP and CLI primitives for spawning, messaging, awaiting, and collecting child agents.

---

## 2. Detailed Survey of Orchestration Harnesses

### 2.1 Conductor (Melty Labs)
*Primary Sources:* [Conductor Website](https://www.conductor.build), [Conductor Docs](https://www.conductor.build/docs), [Git Worktrees Concept](https://www.conductor.build/docs/concepts/git-worktrees), [Conductor MCP Server](https://www.conductor.build/docs/api/mcp), [Conductor API](https://www.conductor.build/docs/api).

* **Isolation Model & Lifecycle:**
  * **Mechanism:** Git worktrees located by default under `~/conductor/workspaces/<repo>/<workspace>`. Workspaces are named mnemonically after world cities.
  * **Create:** Creates a new Git worktree linked to the parent repository and checks out a task-specific branch. Runs configured `setup` scripts (dependency installs, builds).
  * **File Copying (`.worktreeinclude`):** Since Git worktrees only check out tracked files, Conductor copies untracked, gitignored configuration files (such as `.env.local`, local certificates) matching patterns defined in `.worktreeinclude`.
  * **Run:** Agents (Claude Code, OpenAI Codex, Cursor Agent, OpenCode) execute within the workspace directory. Each workspace has independent terminal states, processes, and dev servers.
  * **Merge Back:** Diffs are tracked against the base branch. Merging occurs either by pushing the branch and opening a pull request on GitHub or merging directly in Git.
  * **Clean Up:** Workspaces can be archived or deleted via UI/API (`archive_workspace`), which removes the worktree and prunes Git references.
  * **Cloud Mode:** Offers "Conductor Cloud" where workspaces run inside isolated cloud microVMs with SSH and port forwarding.

* **Agent-to-Agent Spawning Primitives:**
  * Conductor exposes a hosted Streamable HTTP **Model Context Protocol (MCP)** server at `https://api.conductor.build/mcp` and a REST API (`/v0`).
  * Any agent running inside or outside Conductor (e.g., Claude Code or Codex) can invoke Conductor MCP tools to programmatically manage workspaces and sessions:
    * `create_workspace(project_id, repository_url)`: Spawns an isolated workspace and initial session.
    * `create_session(workspace_id, model, agent)`: Spawns a secondary agent session inside an existing workspace.
    * `send_message(session_id, prompt)`: Sends or queues a prompt for a specific session.
    * `cancel_session(session_id)`: Cancels execution and discards pending messages.

* **Result Collection & Notification:**
  * Result collection is pull-based or query-based:
    * `get_session_status(session_id)`: Polls whether the agent is active or idle.
    * `list_messages(session_id, after)`: Retrieves transcript items paginated by cursor.
    * `run_sql(query)`: Executes read-only SQL queries directly against `session_transcripts_view`.
  * In the UI, state updates are streamed live via WebSocket/event streams.

* **Concurrency & Quotas:**
  * Local mode is bounded by host memory, disk space, and Git index contention.
  * Cloud mode enforces concurrency quotas per plan tier and organization.

* **Multi-Agent UI:**
  * Native macOS desktop application.
  * Features a multi-workspace sidebar, side-by-side split panels, Big Terminal Mode, turn-by-turn checkpoint scrubber, per-hunk diff review (keep/reject), checks, and to-do trackers.

---

### 2.2 Crystal / Nimbalyst
*Primary Sources:* [Nimbalyst GitHub](https://github.com/nimbalyst/nimbalyst), [Nimbalyst Website](https://nimbalyst.com), [Release Notes & Worktree Fixes](https://github.com/nimbalyst/nimbalyst/releases).

* **Background:** Originally developed as "Crystal" (a desktop manager for parallel Claude Code sessions using Git worktrees), the project was rebranded and upgraded to "Nimbalyst" (MIT licensed) in early 2026.
* **Isolation Model & Lifecycle:**
  * **Mechanism:** Native Git worktrees.
  * **Create:** Spawns a dedicated worktree and Git branch for each task or agent session. Also supports attaching to externally created Git worktrees.
  * **Run:** Agents (Claude Code, Codex, OpenCode, Gemini, Copilot) run isolated in their own checkout.
  * **Merge Back:** Built-in Git tooling allows reviewing diffs and merging feature branches back into main.
  * **Clean Up:** Includes automated background resource cleanup (e.g., PR #1579 garbage collector) for stale or inactive project worktrees.
* **Agent-to-Agent Spawning Primitives:**
  * Primarily human-driven task dispatching through an Agent Manager.
  * Acts as an MCP client and exposes tool integration, rendering MCP tool outputs as rich visual widgets rather than raw JSON strings.
* **Result Collection & Notification:**
  * Reactive event streaming to desktop and mobile companion apps (iOS/Android).
  * Completion notifications and visual status badges indicate when agents complete, fail, or wait for input.
* **Concurrency & Quotas:**
  * Multi-agent parallelism limited only by local machine resources and LLM API rate limits.
* **Multi-Agent UI:**
  * Desktop GUI (macOS, Windows, Linux) with integrated Kanban board, side-by-side red/green diff reviewer, embedded Monaco code/markdown editor, and mobile push notifications.

---

### 2.3 Vibe Kanban (bloopAI)
*Primary Sources:* [Vibe Kanban GitHub](https://github.com/bloopai/vibe-kanban), [Vibe Kanban Website](https://vibekanban.com), [Documentation](https://vibekanban.com).

* **Isolation Model & Lifecycle:**
  * **Mechanism:** Automated Git worktrees per task card.
  * **Create:** When a card moves to "In Progress" or is launched (`npx vibe-kanban`), a worktree is created in a configurable directory (`Settings → General → Workspace Directory`) on a dedicated branch. Automatic project setup scripts install dependencies and start dev servers.
  * **Run:** CLI agents (Claude Code, Codex, Copilot, Gemini CLI) run inside the worktree directory. Dev servers run on dynamically allocated, conflict-free ports.
  * **Merge Back:** Developer inspects changes; once approved, Vibe Kanban pushes the branch and creates/merges a GitHub PR.
  * **Clean Up:** Automated garbage collection: a background collector daemon removes the worktree directory and prunes Git references once the task card is moved to "Done" or "Archived" (default threshold ~1 hour). Startup sweeps also detect and clean orphaned worktrees.
* **Agent-to-Agent Spawning Primitives:**
  * Vibe Kanban is a board-driven orchestrator where humans define the cards. Subagents cannot natively call a Vibe Kanban API to dynamically spawn sibling Kanban cards, though agents can execute CLI sub-commands inside their own shell.
* **Result Collection & Notification:**
  * Streaming logs over WebSockets; web UI renders live terminal output, diff changesets, and test runner outputs.
* **Concurrency & Quotas:**
  * Multiple parallel tasks can be run concurrently. Limiting factors are local CPU/RAM, port allocation ranges, and provider token quotas.
* **Multi-Agent UI:**
  * Web-based Kanban board (Columns: Backlog, Planning, In Progress, Review, Done).
  * Task drawers feature embedded terminal viewers, unified diff view with inline review comments, and built-in browser previews.

---

### 2.4 Claude Squad (smtg-ai)
*Primary Sources:* [Claude Squad GitHub](https://github.com/smtg-ai/claude-squad), [Installation & CLI Documentation](https://github.com/smtg-ai/claude-squad).

* **Isolation Model & Lifecycle:**
  * **Mechanism:** Git worktrees paired with `tmux` session multiplexing.
  * **Create:** On executing `cs` or `cs -p "<agent-command>"`, Claude Squad generates a Git worktree and new branch for the task.
  * **Run:** Launches CLI agents (`claude`, `aider`, `codex`, `opencode`, `amp`) inside dedicated `tmux` panes. Supports "Auto-Yes" mode to auto-approve agent confirmation prompts.
  * **Merge Back:** Manual developer-mediated merge via standard Git commands or `gh pr create`.
  * **Clean Up:** Exiting a session tears down the `tmux` pane and deletes the associated worktree directory, executing `git worktree prune`.
* **Agent-to-Agent Spawning Primitives:**
  * Purely human-driven terminal supervisor. No programmatic API or MCP server is exposed for an agent to spawn another agent.
* **Result Collection & Notification:**
  * Direct terminal multiplexing: developers switch between tmux panes or inspect the TUI dashboard for agent exit codes.
* **Concurrency & Quotas:**
  * Bounded by tmux window management and terminal readability, typically 3–8 parallel agents.
* **Multi-Agent UI:**
  * Terminal User Interface (TUI) written in Go. Displays active workspaces, current git branch, running process status, and active terminal panes.

---

### 2.5 Sculptor (Imbue)
*Primary Sources:* [Imbue Website](https://imbue.com), [Sculptor Launch Details](https://www.ycombinator.com), [Imbue GitHub](https://github.com/imbue-ai).

* **Isolation Model & Lifecycle:**
  * **Mechanism:** Docker container sandboxes (with optional fallback to Git worktree / in-place execution).
  * **Create:** Spins up an isolated Linux Docker container for each task. The repository is mounted or cloned inside.
  * **Run:** Coding agents execute entirely inside the container. Sculptor features "Pairing Mode"—a bidirectional filesystem and editor sync engine that mirrors edits between the containerized agent and the developer's local IDE in real time.
  * **Merge Back:** When an agent task finishes, Sculptor computes a unified diff patch between the container checkout and the base repository, presenting it for user review.
  * **Clean Up:** The Docker container is stopped and destroyed, leaving no lingering artifacts on the host OS.
* **Agent-to-Agent Spawning Primitives:**
  * Leverages Imbue’s reasoning systems and agent frameworks. Subagents can be dispatched internally within the container sandbox to perform specialized analysis.
* **Result Collection & Notification:**
  * Real-time file sync notifications, structured task checklist updates (to-do items), and diff summaries.
* **Concurrency & Quotas:**
  * Highly constrained by Docker container memory and CPU overhead on macOS (where Docker runs inside a Linux VM).
* **Multi-Agent UI:**
  * Native macOS desktop GUI (optimized for Apple Silicon). Features a dedicated diff reviewer, task checklist pane, and real-time pairing mode status indicators.

---

### 2.6 Amp (Sourcegraph)
*Primary Sources:* [Sourcegraph Amp Architecture & Announcements](https://sourcegraph.com), [Sourcegraph Docs](https://sourcegraph.com).

* **Isolation Model & Lifecycle:**
  * **Mechanism:** Cloud-indexed repository context + ephemeral execution workspaces.
  * **Create & Run:** Unlike tools that clone the entire repository locally for every subtask, Amp leverages Sourcegraph's remote code search index. For write tasks, it uses local ephemeral worktrees or cloud branches.
  * **Merge Back:** Standard Git commit and pull request flow.
  * **Clean Up:** Ephemeral execution directories are discarded at thread termination.
* **Agent-to-Agent Spawning Primitives:**
  * **Hierarchical Specialist Subagent Architecture:** Amp does not treat agents as generic clones. Instead, the main coordinator agent dispatches subtasks to specialized subagents using internal agent-as-tool invocations:
    * **The Oracle:** Specialized in deep architectural reasoning, root-cause debugging, and complex plan synthesis.
    * **The Librarian:** Dedicated to wide-scale code search and retrieval across public packages and internal enterprise repositories without local checkouts.
    * **The Painter:** Specialized in multimodal asset generation and UI mockups.
    * **Code Reviewer:** Performs automated diff inspection and test validation.
* **Result Collection & Notification:**
  * Synchronous/asynchronous tool results return directly to the caller agent's context window.
  * Threads are stored on the server, allowing team members to pick up, share, and branch conversations.
* **Concurrency & Quotas:**
  * Managed by Sourcegraph enterprise infrastructure; features execution modes (*Smart*, *Rush*, *Deep*) that balance reasoning depth against token quotas.
* **Multi-Agent UI:**
  * Command-line interface with interactive thread switching, paired with a web UI for shared team transcripts.

---

### 2.7 ORCA (Stably AI)
*Primary Sources:* [Stably AI Orca GitHub](https://github.com/stablyai/orca), [Orca ADE Documentation](https://github.com/stablyai/orca).

* **Identity:** In the multi-agent coding landscape, "ORCA" refers to **Stably AI's open-source Agent Development Environment (ADE)** (`stablyai/orca`), licensed under MIT.
* **Isolation Model & Lifecycle:**
  * **Mechanism:** Git worktrees for local tasks, combined with SSH worktrees for remote server execution.
  * **Create:** `orca` CLI or ADE creates a dedicated worktree and Git branch for each task.
  * **Run:** Executes fleets of parallel coding agents (Claude Code, OpenAI Codex, Cursor CLI, Gemini, Grok) side-by-side.
  * **Merge Back:** Visual diff annotation and snapshot comparison enable merging clean patches.
  * **Clean Up:** Managed via `orca` CLI cleanup commands and `git worktree prune`.
* **Agent-to-Agent Spawning Primitives:**
  * **Orchestration Skills:** Orca provides built-in orchestration primitives designed for coordinator loops:
    * Task dispatching to child agents across worktrees.
    * Decision gates and blocking ask/reply flows.
    * Scriptable coordinator loops using the `orca` CLI.
* **Result Collection & Notification:**
  * Coordinator blocking wait, structured git snapshots, diff metrics, and ADE desktop alerts.
* **Concurrency & Quotas:**
  * Supports BYO-subscription model (users plug in their existing CLI keys/credentials). Concurrency is throttled by user API limits and machine disk space.
* **Multi-Agent UI:**
  * Full desktop ADE featuring a Monaco editor, terminal multiplexer, diff annotator, and an integrated browser with **Design Mode** (where clicking UI elements extracts DOM state and sends it as context to the agent).

---

### 2.8 Asynchronous Cloud Platforms: Google Jules & OpenAI Codex Cloud
*Primary Sources:* [Google Jules Announcement](https://blog.google/technology/developers/google-jules/), [Google Jules Platform](https://jules.google), [OpenAI Agents SDK Documentation](https://openai.github.io/openai-agents-python/), [OpenAI Platform Guides](https://platform.openai.com/docs/guides/agents).

#### Google Jules
* **Isolation Model:** **Transient Cloud VM per Task**. Each task creates an isolated Google Cloud Virtual Machine with 20GB disk space, full network egress for package installation (`npm`, `pip`, `cargo`), and no shared filesystem state between tasks.
* **Lifecycle:** Task submitted (via Web or `jules` CLI) → Fresh VM spun up → Gemini agent plans, edits, and runs tests → Branch pushed to GitHub → GitHub PR opened → VM is completely destroyed.
* **Agent Spawning:** Primarily user-to-Jules dispatch.
* **Result Collection:** Asynchronous GitHub Pull Requests with detailed reasoning logs, test outputs, and commit history.

#### OpenAI Codex Cloud & OpenAI Agents SDK
* **Isolation Model:** **Tiered Sandboxing**.
  * *Harness (Control Plane):* Hosted in OpenAI trusted infrastructure running the model reasoning loop, compaction, and approval policies.
  * *Execution Plane:* Isolated Linux sandboxes (microVMs / gVisor / Kata containers) or self-hosted machines running `codex exec-server` connected over WebSockets. Recently evolved to support persistent, reusable cloud environments alongside throwaway containers.
* **Agent-to-Agent Spawning Primitives:**
  * The **OpenAI Agents SDK** provides two canonical delegation patterns:
    1. **Handoffs (`handoff()` / `transfer_to_<agent>`):** Transfers full conversation ownership to a specialist agent. The original agent steps aside; the specialist now owns the turn.
    2. **Agents as Tools (`agent.asTool()`):** The orchestrator retains full ownership. It invokes a specialist subagent as a function call, waits for its result, and incorporates the answer into its own context.
* **Result Collection:** Web/desktop notifications, structured JSON function responses, and automated GitHub PR creation.

---

### 2.9 Notable Open-Source Tooling

#### Uzi (`devflowinc/uzi`)
*Source:* [Uzi GitHub](https://github.com/devflowinc/uzi).
* Go-based CLI tool combining **Git worktrees** and **tmux**.
* Designed specifically for running multiple coding agents in parallel to solve the "attention bottleneck."
* Automatically manages dev server port allocation: uses a root `uzi.yaml` defining `devCommand` with `$PORT` and `portRange` so parallel agents running tests or web servers do not collide.
* Provides live monitoring (`uzi ls -w`) and worktree checkpoints.

#### Claude-Flow / Ruflo (`ruvnet/claude-flow`)
*Source:* [Claude-Flow GitHub](https://github.com/ruvnet/claude-flow).
* Enterprise swarm orchestration framework for Claude Code utilizing the SPARC methodology.
* Coordinates specialized swarms (researchers, implementers, testers) with shared hierarchical memory and vector search (AgentDB).
* Exposes MCP servers to allow Claude instances to trigger cross-agent handoffs and task delegations.

#### Anthropic Claude Code Subagents
*Source:* [Claude Code Documentation](https://docs.anthropic.com/en/docs/agents-and-tools/claude-code).
* Built-in `Agent` (or `Task`) tool that spawns a subagent in an isolated context window.
* Custom subagents are defined declaratively in `.claude/agents/<name>.md` with YAML frontmatter specifying role, allowed tools (e.g. `Read`, `Grep`, `Bash`), and model tier.
* Prevents context explosion: a noisy 50-file search executes inside the subagent, and only a concise final summary is returned to the parent agent.
* Enforces concurrency limits (`CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS`) and recursion depth caps (`CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH`, default 5).

#### OpenHands (formerly OpenDevin)
*Source:* [OpenHands GitHub](https://github.com/All-Hands-AI/OpenHands), [OpenHands Architecture](https://openhands.dev).
* Client-server architecture with Docker container execution.
* Uses Git worktrees inside containers to allow concurrent task attempts without write collisions.
* Orchestrator-worker pattern over an `EventStream` abstraction. All agent actions and environment observations are published as immutable events.
* Controlled via the "Agent Canvas" web UI.

---

## 3. Cross-Cutting Comparison Matrices

### 3.1 Isolation & Workspace Lifecycle Matrix

| Harness / Orchestrator | Primary Isolation | Worktree / Container Location | Config / File Copying (`.env`) | Cleanup Mechanism |
|---|---|---|---|---|
| **Conductor** | Git worktree (local) / microVM (cloud) | `~/conductor/workspaces/<repo>/<city>` | `.worktreeinclude` rule-based copy | `archive_workspace` deletes directory + `git worktree prune` |
| **Nimbalyst (Crystal)** | Git worktree | `.nimbalyst/worktrees/` or repo worktree | Manual or template copy | Background GC daemon cleans inactive worktrees |
| **Vibe Kanban** | Git worktree | Configurable (`Settings → Workspace Directory`) | Automatic setup scripts | Background daemon cleans "Done" tasks after ~1h; orphan cleanup on boot |
| **Claude Squad** | Git worktree + tmux | Sibling directories / `.claude-squad/` | Inherited from branch | Deletes worktree directory on pane exit + `git worktree prune` |
| **Sculptor** | Docker container | Container filesystem with host volume mount | Container environment variables | Container destruction on task completion |
| **Amp** | Ephemeral worktree / Remote search | Local temp dir or remote virtual repo | Remote repo secrets / env | Discarded on thread completion |
| **Stably Orca** | Git worktree + SSH worktree | Local worktree / remote SSH worktree | Snapshot & project templates | CLI prune commands + git worktree prune |
| **Google Jules** | Google Cloud VM | Transient VM `/workspace` | GitHub Actions secrets / repo config | Full VM teardown after PR push |
| **Codex Cloud** | MicroVM / Container sandbox | Sandboxed cloud filesystem | Secure environment secrets | Ephemeral container destruction or persistent workspace freeze |
| **Uzi** | Git worktree + tmux | Project sibling worktrees | Automatic port remapping via `uzi.yaml` | CLI worktree cleanup |

---

### 3.2 Agent-to-Agent Spawning & Coordination Primitives

| Harness | Can Agent Spawn Agent? | Protocol / Primitives Used | Return / Collect Mechanism | Concurrency Limits |
|---|---|---|---|---|
| **Conductor** | **Yes** | Streamable HTTP MCP (`create_workspace`, `create_session`, `send_message`) | Polling (`get_session_status`, `list_messages`) or SQL view | Host resources (local); Plan tier (cloud) |
| **Nimbalyst** | Partial (UI/MCP) | MCP client/server integration | Event stream / UI widgets | Host CPU/RAM + API limits |
| **Vibe Kanban** | No (Human-driven cards) | Board Kanban cards (runs CLI in background) | WebSocket logs, diff inspection | User-configured task limits |
| **Claude Squad** | No (Human supervisor) | CLI invocation (`cs -p`) | Tmux pane observation | Tmux screen space (3–8 agents) |
| **Sculptor** | Internal subagents | Internal reasoning harnesses | Diffs & checklist status | Docker VM RAM/CPU limits |
| **Amp** | **Yes** | Internal specialist tools (`Oracle`, `Librarian`, `Reviewer`) | Synchronous tool result return | Cloud tier concurrency |
| **Stably Orca** | **Yes** | Orchestration Skills / `orca` CLI scripts | Blocking wait / git snapshots | Local host resources + API rate limits |
| **Google Jules** | No (External dispatch) | Web / CLI task submission | Asynchronous GitHub PR | Cloud VM quota |
| **Codex Cloud** | **Yes** | OpenAI Agents SDK (`handoff()`, `agent.asTool()`) | Tool return or PR notification | Organization concurrency limits |
| **Claude Code** | **Yes** | Built-in `Agent` / `Task` tool (`.claude/agents/*.md`) | Summary returned to parent context | `MAX_CONCURRENT_SUBAGENTS`, depth limit 5 |
| **Chorus (Target)** | **Yes (Agent-driven)** | Chorus MCP Server & CLI primitives (`spawn_agent`, `send_message`, `collect`) | Async ledger events + blocking tool return | Configurable per session (default 4) |

---

## 4. Key Lessons for Chorus

From this survey, several key patterns emerge that directly inform Chorus's architecture:

1. **Worktrees are the gold standard for local parallel execution**:
   Containers introduce massive startup latency and resource overhead on macOS (requiring a Linux VM). Git worktrees create zero-copy checkouts in under 200ms and share Git's object store.
2. **Untracked files are the #1 source of worktree failures**:
   Every developer survey noted that raw `git worktree add` breaks builds because `.env`, `.env.local`, and build caches are missing. Conductor's `.worktreeinclude` pattern and Vibe Kanban's automated setup scripts are necessary to ensure child worktrees can build and test.
3. **Port collision must be handled proactively**:
   As demonstrated by Uzi and Vibe Kanban, if two parallel agents run `npm test` or a dev server, they collide on default ports (e.g., 3000, 8080). A harness must inject unique port allocations or environment variables into child worktrees.
4. **Context explosion requires subagent summarization**:
   Claude Code's greatest win with subagents is context hygiene. If an agent inspects 40 files, those 40 file contents must **never** be injected into the parent ledger. The subagent runs in an isolated context and returns only a compact summary and diff reference.
5. **Orchestrator agents require both "Fork & Forget" and "Call & Collect"**:
   - *Call & Collect (Agent as Tool)*: Root agent asks a subagent to research something, waits for the brief, and continues.
   - *Fork & Run (Background Worker)*: Root agent spawns 3 parallel coding agents across 3 worktrees, proceeds with other work, and collects or merges their branches later.

---

## 5. Recommended Primitive Set for Chorus

In accordance with Chorus's core design rules:
- **Rule 2.1:** Storage $\neq$ Context. Child transcripts stay in child ledgers/blobs; only compact summaries enter parent context.
- **Rule 2.2:** The Ledger is the single source of truth.
- **Rule 2.3:** All logic in Rust (`chorus-core`), Swift UI only renders and accepts input.
- **Standing Preference:** Orchestration is **agent-driven** (agents call Chorus CLI/MCP; Chorus does not run its own planner). Parallel writing agents get their own Git worktree; read-only agents share the main tree.

We recommend dividing Chorus's orchestration primitives into two layers:
1. **Worktree Lifecycle Engine (Internal Rust Core)**
2. **Agent Control Surface (MCP & CLI Tools)**

```
┌────────────────────────────────────────────────────────────────────────┐
│                        Root Agent (Codex / Claude)                     │
└───────────────────────────────────┬────────────────────────────────────┘
                                    │ Calls MCP / CLI
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│                         Chorus MCP / CLI Bridge                        │
│   chorus_spawn_agent · chorus_send_message · chorus_collect_results    │
└───────────────────────────────────┬────────────────────────────────────┘
                                    │
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│                    Chorus Core Engine (Rust crate)                     │
│  ┌──────────────────────────────┐    ┌──────────────────────────────┐  │
│  │   Worktree Lifecycle Mgr     │    │     Ledger & Event Log       │  │
│  │  - create (.chorus/worktrees)│    │  - SubagentSpawned event     │  │
│  │  - copy .worktreeinclude     │    │  - SubagentCompleted event   │  │
│  │  - allocate PORT range       │    │  - Diff snapshotting         │  │
│  │  - merge / cleanup           │    │  - Context compaction brief  │  │
│  └──────────────┬───────────────┘    └──────────────┬───────────────┘  │
└─────────────────┼───────────────────────────────────┼──────────────────┘
                  │                                   │
                  ▼                                   ▼
       ┌──────────────────────┐             ┌───────────────────┐
       │ Child Worktree (Git) │             │ Swift Desktop UI  │
       │  Dedicated Branch    │             │ Subagent Tree Pane│
       │  Isolated Process    │             │ Live Transcripts  │
       └──────────────────────┘             └───────────────────┘
```

### 5.1 Worktree Lifecycle Primitives (Rust Core Engine)

All worktree operations must live in `chorus-core` (or a dedicated module `chorus-core::worktree`) with C-ABI FFI exposure to Swift:

1. **`worktree_create(repo_path, branch_name, base_ref, mode)`**
   - **Path:** Creates the worktree under `.chorus/worktrees/<subagent-id>` (automatically gitignored).
   - **Branch:** Creates and checks out a new branch (`chorus/<subagent-id>/<task-slug>`).
   - **File Copying:** Reads `.worktreeinclude` from the repository root (falling back to default patterns like `.env*`, `local.properties`). Copies matching gitignored files to the new worktree.
   - **Environment Isolation:** Injects unique environment variables:
     - `CHORUS_WORKTREE_ID=<id>`
     - `PORT=<allocated_port>` (from a managed pool, e.g. 4100–4199)
   - **Read-Only Optimization:** If `mode == ReadOnly`, bypasses worktree creation entirely; child shares the main repository directory with write permissions denied.

2. **`worktree_diff(worktree_id)`**
   - Computes a structured unified diff between the worktree branch and `base_ref`.
   - Returns affected files, hunk stats, and additions/deletions.

3. **`worktree_merge(worktree_id, strategy)`**
   - Validates that the worktree branch merges cleanly into the target branch.
   - Supports:
     - `FastForwardOrSquash`: Squashes child commits into a clean parent commit.
     - `CreateBranchOnly`: Leaves the branch ready for a GitHub Pull Request.

4. **`worktree_cleanup(worktree_id, force)`**
   - Checks for uncommitted changes (errors unless `force: true`).
   - Removes the worktree directory via `git worktree remove --force`.
   - Runs `git worktree prune`.
   - Deletes ephemeral worktree files.

---

### 5.2 Agent-Facing Orchestration Primitives (MCP Tools & CLI)

Exposed to root agents via Chorus's embedded MCP server and the `chorus` CLI:

#### 1. `chorus_spawn_agent`
Spawns a specialized subagent in a background session.
* **Input Parameters:**
  * `prompt` (*string*, required): Clear instructions for the subagent.
  * `role` (*string*, required): Human-readable role (e.g., `"Auth Bug Investigator"`, `"CSS Styler"`).
  * `agent` (*string*, optional): Agent binary or bridge to use (e.g., `"codex"`, `"claude"`, `"gemini"`, or local Ollama model). Defaults to parent agent or router recommendation.
  * `mode` (*string*, optional): `"read_only"` (shares tree, cannot write) or `"read_write"` (allocates dedicated Git worktree). Default: `"read_write"`.
  * `tools` (*array of strings*, optional): Scoped tools allowed for this subagent (e.g., `["read_file", "search_web"]` for pure research).
* **Return Value:**
  * `subagent_id` (*string*): Unique UUID for tracking.
  * `worktree_path` (*string* or null): Absolute filesystem path of the allocated worktree.
  * `branch_name` (*string* or null): Branch checked out in the worktree.
  * `status`: `"running"`.

#### 2. `chorus_send_message`
Sends guidance, follow-up requests, or inline diff review comments to an active or idle subagent.
* **Input Parameters:**
  * `subagent_id` (*string*, required): Target subagent ID.
  * `message` (*string*, required): Message content (e.g., review rejection comments).
* **Return Value:**
  * `status`: `"delivered"` or `"queued"`.

#### 3. `chorus_collect_results`
Awaits or polls completion of one or more subagents.
* **Input Parameters:**
  * `subagent_ids` (*array of strings*, required): List of subagent IDs to collect.
  * `wait_mode` (*string*, optional):
    * `"all"`: Blocks until all specified subagents complete or fail (join barrier).
    * `"any"`: Returns as soon as the first subagent completes.
    * `"poll"`: Non-blocking check; returns current state immediately.
  * `timeout_seconds` (*integer*, optional): Maximum wait time before returning partial results.
* **Return Value:**
  * Array of subagent result records, each containing:
    * `subagent_id`: Identifier.
    * `state`: `"completed"`, `"failed"`, `"running"`, or `"cancelled"`.
    * `summary`: Compact summary text generated by the subagent upon completion.
    * `diff_stats`: Number of files changed, additions, deletions.
    * `token_usage`: Total tokens consumed by child session.

#### 4. `chorus_list_subagents`
Inspects all subagents spawned within the current parent session.
* **Return Value:**
  * Tree of subagents with: `subagent_id`, `role`, `agent_type`, `state`, `worktree_path`, `uptime_seconds`.

#### 5. `chorus_cancel_agent`
Terminates a runaway or obsolete subagent.
* **Input Parameters:**
  * `subagent_id` (*string*, required).
  * `cleanup_worktree` (*boolean*, optional, default: true).
* **Return Value:**
  * `status`: `"cancelled"`.

---

### 5.3 Ledger Integration & Context Compaction Contract

To preserve Rule 2.1 (*storage $\neq$ context*) and Rule 2.2 (*ledger is single source of truth*):

1. **Ledger Events:**
   Every subagent lifecycle action generates immutable ledger events in `store.db`:
   * `SubagentSpawned { subagent_id, parent_session_id, role, agent_type, worktree_path, branch }`
   * `SubagentMessage { subagent_id, direction: Inbound/Outbound, text }`
   * `SubagentTurnCompleted { subagent_id, turn_index, token_usage }`
   * `SubagentFinished { subagent_id, exit_code, summary, diff_id }`
2. **Context Compaction on Collection:**
   When the root agent invokes `chorus_collect_results`, Chorus does **not** dump the child's turn-by-turn event log into the root agent's prompt. Instead, it extracts the child's **Handoff Brief / Summary** and the **`worktree_diff` summary**, inserting a clean, compact block into the parent prompt:
   ```markdown
   [Subagent "Auth Bug Investigator" (id: sub-01) finished]
   Branch: chorus/sub-01/fix-oauth-refresh
   Status: Completed successfully
   Diff: 2 files changed, +18 lines, -4 lines
   Summary: Fixed token expiration check in `auth/refresh.rs` by comparing against UTC epoch. Added unit test.
   ```
3. **Swift UI Observability:**
   Because all subagent events stream into the ledger, the Swift UI can render:
   * A live **Subagent Tree Pane** in the sidebar showing child agents, active turns, and tokens consumed.
   * A **Transcript Inspector** to view the full child log without affecting the parent conversation.
   * A **Unified Multi-Worktree Diff Viewer** allowing the human to inspect and comment on child changesets before the root agent merges them.
