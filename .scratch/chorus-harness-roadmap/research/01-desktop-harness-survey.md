# Desktop Agent Harness Feature Survey

This research investigates the leading desktop and IDE AI agent harnesses to inform the design of the **Chorus desktop agent harness** (macOS). It analyzes window layout, file/code presentation, diff review workflows, plan/task tracking, checkpoints/rewind, permissions and sandboxing, notifications, terminals, browser previews, Git/PR flows, and command surfaces across seven industry-leading harnesses.

The survey evaluates:
1. **Claude Code Desktop / claude.ai Code** (Anthropic)
2. **Codex App / ChatGPT Work** (OpenAI)
3. **Cursor** (Anysphere — Agents window, Composer, and Background Agents)
4. **Zed** (Zed Industries — Agent Panel & Agent Client Protocol / ACP)
5. **Google Antigravity** (Google DeepMind — Antigravity IDE & Antigravity 2.0 Desktop)
6. **Windsurf** (Codeium — Cascade & AI Flow)
7. **Kiro** (AWS — Kiro IDE, CLI, and Spec-Driven Architecture)

It concludes with a comprehensive comparison matrix and a ranked gap analysis for Chorus, evaluated against Chorus's current architecture (macOS SwiftUI shell, Rust core ledger, ACP client, Seatbelt sandboxing, shadow Git checkpoints) and roadmap principles.

---

## 1. Deep-Dive: Leading Desktop / IDE Agent Harnesses

---

### 1.1 Claude Code Desktop & claude.ai Code (Anthropic)

Anthropic delivers its agentic coding capabilities through three unified surfaces: the terminal CLI (`claude`), the **Claude Desktop Application** (macOS/Windows/Linux), and web-based persistent cloud sessions on [claude.ai](https://claude.ai) ([Claude Code Overview](https://code.claude.com/docs/en/overview), [Claude Desktop Documentation](https://claude.com/download)).

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ Claude Desktop — Code Workspace                                            │
├──────────────┬────────────────────────────────┬─────────────────────────────┤
│ Left Sidebar │ Central Agent Thread           │ Right Auxiliary Inspector   │
│              │                                │                             │
│ • Projects   │ • Prompt Composer              │ • Visual Diff Viewer        │
│ • Sessions   │ • Collapsible Thought Blocks   │   (Unified & Side-by-Side)  │
│ • Live File  │ • Structured Tool Calls        │ • Interactive Terminal Dock │
│   Tree       │ • Inline Feedback Loops        │ • Dev Server Preview /      │
│              │                                │   Computer Use Surface      │
└──────────────┴────────────────────────────────┴─────────────────────────────┘
```

#### Window Layout & Multi-Session View
* **Workspace Organization**: The desktop application provides three dedicated top-level modes: **Chat** (general reasoning), **Cowork** (multi-app agentic automation with computer use), and **Code** (dedicated repository development) ([Anthropic Desktop Workspaces](https://claude.com/download)).
* **Pane Structure**: Resizable multi-pane layout featuring a left project sidebar, a central conversational agent thread, and an auxiliary inspector pane housing diffs, terminal sessions, and preview webviews.
* **Multi-Session Management**: Multiple parallel sessions can run simultaneously. Each session maintains its own thread history, local context window, and independent Git isolation via Git worktrees ([Claude Agent Sessions](https://code.claude.com/docs/en/agent-sdk/sessions)).

#### File, Folder, and Code Presentation
* **Live File Tree**: The left sidebar renders an interactive filesystem tree with real-time badges indicating files actively read, modified, or staged by the agent.
* **Code in Conversation**: Code snippets appear in fenced syntax-highlighted blocks with file path badges, line numbers, and one-click copy/insert buttons. Full files open in the auxiliary file viewer pane.

#### Diff Viewer & Review Workflow
* **Visual Diff Viewer**: Dedicated diff pane supporting both **unified** and **side-by-side** split rendering ([Claude Code Diff Inspection](https://code.claude.com/docs/en/overview)).
* **Review Scope**: Diffs can be inspected at the per-turn level (changes introduced by the latest prompt) or as a cumulative working tree diff against the branch base.
* **Inline Commenting Back to Agent**: Users can highlight lines within the diff viewer and submit inline review comments. These comments are injected directly into Claude's prompt context for the next turn.
* **Accept / Reject**: Granular controls allow accepting or rejecting individual hunks, entire files, or all staged changes, alongside direct manual editing within the diff editor.

#### Plan & Todo Display
* **Plan Mode**: Invoked via `--permission-mode plan` or pressing `Shift+Tab` ([Claude Code Permissions](https://code.claude.com/docs/en/permissions)). In this mode, Claude explores the codebase in read-only mode, formulates an implementation plan, and outputs structured task lists.
* **Progress Tracking**: Step-by-step checklists are displayed in the conversation thread, updating dynamically as sub-tasks complete.

#### Checkpoints & Rewind
* **Turn Checkpointing**: An automated checkpoint snapshot is captured at the start of every prompt turn, retaining a rolling buffer of the 100 most recent checkpoints ([Claude Code Checkpointing](https://code.claude.com/docs/en/checkpointing)).
* **Rewind Menu (`/rewind`)**: Pressing `Esc` twice or running `/rewind` opens an interactive recovery modal with four options:
  1. *Restore code and conversation*: Complete rollback to the target turn.
  2. *Restore conversation only*: Revert chat history while preserving working tree edits.
  3. *Restore code only*: Revert filesystem changes while preserving chat transcript.
  4. *Summarize*: Compress prior turns to reclaim context window capacity.

#### Permissions & Sandboxing UX
* **Autonomy Modes**:
  * `manual`: Default prompt-on-every-tool mode.
  * `plan`: Read-only exploration; code writes are denied.
  * `auto`: Background classifier verifies actions against intent without manual prompts.
  * `acceptEdits`: Automatically permits file writes and common shell commands (`mkdir`, `touch`, `cp`, `mv`).
  * `dontAsk`: Rejects all tool calls requiring confirmation.
  * `bypassPermissions`: Disables safety gates (container/CI only).
* **Granular Rules**: Defined in `.claude/settings.json` using syntax like `Tool(specifier)` (e.g., `Bash(npm test)` allow, `FileEdit(.env)` deny).

#### Terminal, Browser, Notifications, and Commands
* **Terminal**: Integrated terminal dock with PTY support and real-time log streaming.
* **Browser Preview**: Integrated dev server preview and support for "computer use" OS automation.
* **Notifications**: macOS desktop alerts and audio chimes on task completion or permission blockage.
* **Command Surface**: Keyboard shortcuts (`Shift+Tab` for modes, double `Esc` for rewind), slash commands (`/plan`, `/rewind`, `/compact`, `/cost`, `/status`), and `@` context mentions.

---

### 1.2 OpenAI Codex App / ChatGPT Work (OpenAI)

OpenAI consolidated its standalone developer tools and desktop application into the unified **ChatGPT Desktop Application** (macOS, Windows, Linux preview), which provides three operational modes: **Chat**, **ChatGPT Work** (business process automation), and **Codex** (software engineering agent) ([OpenAI Desktop Announcement](https://openai.com/index/introducing-chatgpt-desktop/), [Agentic Coding Operator](https://openai.com/index/operator-and-agentic-coding/)). The engine is powered by `codex-cli` and the `@agentclientprotocol/codex-acp` bridge ([Codex ACP Bridge](https://github.com/agentclientprotocol/codex-acp)).

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ ChatGPT Desktop — Codex Command Center                                      │
├──────────────┬────────────────────────────────┬─────────────────────────────┤
│ Projects &   │ Conversational Agent Thread    │ Inspector & Action Panes    │
│ Sessions     │                                │                             │
│              │ • Hidden/Expanded Reasoning    │ • Diff Inspector            │
│ • Thread 1   │   Traces (Low / Med / High)    │   (Inline Comments, Hunks)  │
│   (Worktree) │ • Tool Invocation Feeds        │ • Worktree File Changes     │
│ • Thread 2   │ • Structured Code Projections  │ • Embedded Sandbox Terminal │
│   (Worktree) │ • User Steering Composer       │ • Mobile Relay Approval     │
└──────────────┴────────────────────────────────┴─────────────────────────────┘
```

#### Window Layout & Multi-Session View
* **Multi-Threaded Command Center**: Left navigation bar lists active projects and parallel agent threads. Each thread can be assigned its own isolated Git worktree to prevent workspace contamination.
* **Central Thread**: Displays prompt input, expandable model reasoning traces (reflecting o1/o3/gpt-5 reasoning effort settings), and structured tool execution streams.
* **Right Inspector**: Tabbed inspector containing the Diff Inspector, File Changes list, and Terminal emulator.
* **Header Bar**: Mode switcher (Chat / Work / Codex), Model selector with reasoning depth sliders, and active Git branch/worktree indicator.

#### File, Folder, and Code Presentation
* **Worktree Tree**: Changed and accessed files are indexed in the inspector with clear status chips (`[M]` modified, `[A]` added, `[D]` deleted).
* **Code Viewing**: Code modifications can be inspected directly in the diff inspector or launched into external editors.

#### Diff Viewer & Review Workflow
* **Dedicated Diff Pane**: Supports unified and side-by-side diff views.
* **Inline Diff Commenting**: Users can select code lines directly inside the diff view, open a comment popover, and type feedback. When submitted, these comments become targeted steering instructions for the next agent turn.
* **Hunk & Manual Edits**: Developers can accept or reject individual hunks or edit lines directly in the diff buffer before approval.

#### Plan & Todo Display
* **Plan Mode Execution**: Codex analyzes requirements, inspects directory structures, and generates an implementation plan detailing planned file changes and test suites before modifying code.
* **Progress Verification**: Plan items update interactively as tasks are verified by test runs.

#### Checkpoints & Rewind
* **Worktree Snapshots**: Automatically records shadow Git commits or snapshot trees before every turn. Users can revert the worktree to any earlier turn in the thread history.

#### Permissions & Sandboxing UX
* **Sandbox Policies**: Operates inside an OS sandbox where network access is restricted by default and filesystem writes outside the designated workspace are blocked.
* **Interactive Approval**: When commands fall outside pre-approved policy (`on-request`), a visual banner prompts the user with *Allow Once*, *Allow Always*, or *Reject*.
* **Mobile / Remote Relay**: If the developer steps away, approval requests and diff summaries are relayed to the ChatGPT iOS/Android app for remote review and authorization ([OpenAI Operator & Desktop](https://openai.com/index/operator-and-agentic-coding/)).

#### Terminal, Browser, Notifications, and Commands
* **Terminal**: Built-in sandbox terminal session bound to the session's worktree.
* **Browser Preview**: Artifact canvas for HTML/JS rendering and local dev server webviews.
* **Notifications**: Native system notifications, audio cues, and mobile push notifications.
* **Command Surface**: `Cmd+K` command palette, slash commands, `@` workspace context mentions.

---

### 1.3 Cursor (Anysphere)

Cursor is an AI-first IDE built on a fork of VS Code. It combines local interactive editing with **Composer** (agentic multi-file editing) and cloud-hosted **Background Agents** running on sandboxed virtual machines ([Cursor Composer](https://docs.cursor.com/composer), [Cursor Agent Documentation](https://docs.cursor.com/agent), [Cursor Checkpoints](https://docs.cursor.com/advanced/checkpoints)).

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ Cursor IDE — Composer & Background Agents                                  │
├──────────────┬────────────────────────────────┬─────────────────────────────┤
│ File Tree &  │ Main Editor / Buffer           │ Docked Composer / Agent     │
│ Background   │                                │                             │
│ Agents List  │ • Inline Red/Green Diffs       │ • Context Chips (@Files)    │
│              │ • Per-Hunk Accept/Reject Bar   │ • Tool Calls (Max 25/loop)  │
│ • Cloud VMs  │ • Code Lenses                  │ • Checkpoint Timeline       │
│ • Local Tree │ • Multi-Tab Code Editing       │ • Yolo Mode Toggle          │
└──────────────┴────────────────────────────────┴─────────────────────────────┘
```

#### Window Layout & Multi-Session View
* **Composer Modalities**: Composer can run as:
  1. A floating overlay window (`Cmd+I`).
  2. A docked sidebar panel.
  3. A full-screen editor tab (`Cmd+Shift+I`).
* **Multi-Session History**: Drawer icon in Composer reveals past sessions with timestamps and message previews.
* **Background Agents View**: Cloud dashboard and dedicated sidebar tab showing asynchronous agent runs executing in remote cloud VMs ([Cursor Background Agents](https://cursor.com/blog)).

#### File, Folder, and Code Presentation
* **Context Badges**: Files referenced in the prompt appear as chips above the composer input box.
* **File Changes**: Edits generate interactive cards in Composer showing filename, diff stats (+/-), and jump-to-code buttons.

#### Diff Viewer & Review Workflow
* **Inline Buffer Diffs**: Proposes changes directly within the active editor buffer using green/red inline diff decorations.
* **Hunk-Level Controls**: Floating action bar above each diff hunk offers *Accept* and *Reject* buttons.
* **Composer Global Controls**: The Composer panel displays *Accept All* (`Cmd+Enter`) and *Reject All* (`Cmd+Backspace`).
* **Inline Comments**: Users can highlight editor lines and invoke `Cmd+K` to instruct the agent to revise specific code sections.

#### Plan & Todo Display
* **Plan Prompts**: Agents break tasks down into numbered checklists in the Composer thread.
* **Collapsible Thought Blocks**: Detailed tool calls and reasoning traces collapse into compact accordions.

#### Checkpoints & Rewind
* **Turn Checkpoints**: Composer automatically captures a checkpoint before modifying files ([Cursor Checkpoints Guide](https://docs.cursor.com/advanced/checkpoints)).
* **Checkout Button**: Each checkpoint card in Composer features a *Checkout* button that restores the project's files to that exact moment without clearing the conversation transcript.

#### Permissions & Sandboxing UX
* **Yolo Mode**: Toggle switch that permits unrestricted terminal command execution and file edits without user prompts.
* **Safety Guards**: In standard mode, each terminal command requires manual confirmation. A hard limit of 25 tool executions per turn prevents infinite agent loops.

#### Terminal, Browser, Notifications, and Commands
* **Terminal**: Integrated VS Code terminal; background agent commands stream stdout/stderr into a dedicated output pane.
* **Browser Preview**: Supported via VS Code Simple Browser and MCP browser tools.
* **Notifications**: Desktop notifications; cloud background agents send Slack messages, emails, and mobile alerts with video/PR links upon task completion.
* **Command Surface**: `Cmd+I` (Composer), `Cmd+K` (inline edit), `Cmd+.` (Agent mode), `Cmd+Shift+I` (fullscreen), `@` mentions menu (Files, Folders, Docs, Web, Git, Notepads).

---

### 1.4 Zed Agent Panel & ACP Integration (Zed Industries)

Zed is a high-performance native macOS/Linux code editor written in Rust (GPUI). It supports both built-in LLM assistants and external agents via the open-standard **Agent Client Protocol (ACP)** ([Zed Agent Panel](https://zed.dev/docs/agent-panel), [Zed AI Agents](https://zed.dev/docs/ai-agents), [Agent Client Protocol Specification](https://agentclientprotocol.com/protocol/overview)).

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ Zed — Native Agent Panel & Terminal Threads                                │
├──────────────┬────────────────────────────────┬─────────────────────────────┤
│ Project Dock │ Main Buffer / Editor Canvas    │ Right Agent Panel           │
│              │                                │                             │
│ • File Tree  │ • Active Code Buffer           │ • Threads Sidebar           │
│ • Git Status │ • Agent Diff View              │ • Native Terminal Threads   │
│              │   (`agent: open agent diff`)   │ • ACP Agent / Model Picker  │
│              │ • Editable Unified Diff Buffer │ • Granular Tool Approvals   │
└──────────────┴────────────────────────────────┴─────────────────────────────┘
```

#### Window Layout & Multi-Session View
* **Dockable Agent Panel**: Located in the right dock by default, but can be moved to the left dock, bottom dock, or opened as a central editor buffer (`Cmd+?` or `agent: new thread`) ([Zed Agent Panel](https://zed.dev/docs/agent-panel)).
* **Threads Sidebar**: Collapsible drawer inside the Agent Panel listing all concurrent agent threads.
* **Terminal Threads**: Specialized threads that run native terminal sessions and TUIs side-by-side with agent conversations ([Zed Terminal Threads](https://zed.dev/docs/ai-agents)).

#### File, Folder, and Code Presentation
* **Context Badges**: File mentions (`@file`), symbols (`@symbol`), git branches (`@git`), and slash commands appear as compact context pills in the composer.
* **Code Navigation**: Clicking file badges opens files in standard GPUI editor tabs.

#### Diff Viewer & Review Workflow
* **Agent Diff (`agent: open agent diff`)**: Renders proposed changes in an **editable unified diff buffer** ([Zed Review UI](https://zed.dev/docs/agent-panel)).
* **Direct Diff Editing**: Users can edit diff buffers directly using standard editor keybindings before accepting.
* **Keyboard-Driven Review**: Fast keybindings allow stepping through hunks, accepting hunks, or discarding all proposed edits.

#### Plan & Todo Display
* **Conversational Planning**: Agents output structured Markdown task lists in the thread. Inline `/context` workflows allow injecting files dynamically.

#### Checkpoints & Rewind
* **Undo Buffer**: Reverts file changes through Zed's high-speed transaction undo history.
* **Thread Forking**: Users can branch or duplicate agent threads to explore alternate solutions.

#### Permissions & Sandboxing UX
* **ACP Tool Approvals**: When an external ACP agent requests tool execution, an interactive confirmation dialog offers:
  * *Allow Once*: Authorizes the single tool call.
  * *Allow for Thread*: Authorizes the tool for the active conversation.
  * *Always Allow*: Persists the authorization into Zed settings.
  * *Deny*: Rejects the action.
* **Process Sandboxing**: Native tools run within OS sandboxes restricting unauthorized disk writes.

#### Terminal, Browser, Notifications, and Commands
* **Terminal**: Integrated terminal dock plus dedicated Terminal Threads in the Agent Panel.
* **Browser Preview**: No built-in browser engine; relies on external browser launching.
* **Notifications**: Minimal desktop notifications when background agent threads finish.
* **Command Surface**: `Cmd+Shift+P` (Command Palette), `Cmd+?` (toggle panel), `agent: open agent diff`, `zed: acp registry`, `@` context menu.

---

### 1.5 Google Antigravity (Google DeepMind)

Google Antigravity is an AI-first development platform featuring two complementary desktop surfaces: **Antigravity 2.0** (standalone Electron desktop orchestrator) and **Antigravity IDE** (VS Code fork) ([Antigravity Documentation](https://antigravity.google/docs), [Antigravity Permissions Guide](https://antigravity.google/docs/permissions), [Antigravity Sandbox](https://antigravity.google/docs/sandbox)).

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ Antigravity 2.0 — Orchestrator Architecture                                │
├──────────────┬────────────────────────────────┬─────────────────────────────┤
│ Navigation   │ Chat Canvas                    │ Auxiliary Pane (Collapsible)│
│ Sidebar      │                                │                             │
│              │ • Rich Markdown Stream         │ • Subagents Hierarchy Tree  │
│ • Projects   │ • Generative UI Widgets        │ • Background Task Manager   │
│ • Sessions   │ • Interactive Artifact Cards   │ • Artifacts Gallery         │
│ • Cron/Tasks │ • Slash Commands & @-Mentions  │ • Files Changed (Diff View) │
│ • Settings   │ • Media & Screenshot Uploads   │ • Sandboxed Terminals       │
└──────────────┴────────────────────────────────┴─────────────────────────────┘
```

#### Window Layout & Multi-Session View
* **Antigravity 2.0 (Desktop Orchestrator)**:
  * *Left Sidebar*: Projects switcher, conversation session list, scheduled tasks (cron and one-shot timers), and global/project settings.
  * *Center Chat Canvas*: Interactive chat stream supporting generative UI widgets, markdown rendering, and artifact cards.
  * *Right Auxiliary Pane*: Tabbed side pane housing the **Subagents Hierarchy Tree**, **Background Tasks Monitor**, **Artifacts Gallery**, **Files Changed Diff Viewer**, and **Terminal Sessions**.
* **Antigravity IDE**: In-editor experience featuring sidebar agent chat, inline code lenses (`⌘I`), visual diff overlays, and Node.js Sidecar UI extensions.

#### File, Folder, and Code Presentation
* **Workspace Explorer**: Left project explorer tree.
* **Files Changed Pane**: Auxiliary pane displays all touched files with addition/deletion metrics.
* **Interactive File Chips**: References in chat include line-range anchors that navigate directly to code positions.

#### Diff Viewer & Review Workflow
* **Dual Diff Views**: Unified and side-by-side diff views inside the "Files Changed" pane and in-editor diff overlays.
* **Per-Hunk Review**: Users can review changes file-by-file or hunk-by-hunk, with dedicated *Accept* and *Reject* buttons.
* **Artifact Review Mode**: Configurable policies (`always-proceed`, `agent-decides`, `asks-for-review`) controlling whether artifact changes require explicit approval before application.

#### Plan & Todo Display
* **Planning Mode (`/plan`)**: Agent generates structured implementation plans rendered as rich Markdown artifacts with alerts, checklists, KaTeX formulas, and action buttons.
* **Live Task States**: Background task statuses and cron countdowns display in real time.

#### Checkpoints & Rewind
* **Turn Checkpoints**: Automatically snapshots working state per turn.
* **Branch Workspaces**: Subagents can be spawned with `Workspace: branch`, creating isolated Git worktrees/clones that prevent interference with the parent workspace.
* **Conversation Forking**: Sessions can be branched from any prior turn.

#### Permissions & Sandboxing UX
* **Four-Tier Tool Policy**: `always-proceed`, `request-review`, `strict`, `proceed-in-sandbox`.
* **OS-Level Sandboxing**: Terminal commands execute inside a restricted sandbox environment.
* **Fine-Grained Policies**: Dedicated access policies for non-workspace files (`allow`, `ask`, `deny`) and internet access (`allow`, `ask`, `deny`), backed by command and domain allowlists/denylists.

#### Terminal, Browser, Notifications, and Commands
* **Terminal**: Sandboxed terminal emulator supporting persistent terminals (`RunPersistent: true`) and background task process control (`manage_task`: list, status, send_input, kill).
* **Browser Preview**: Integrated browser automation (`read_browser_page`, `interact_browser_page`, Playwright CLI) and generative UI widgets.
* **Notifications**: Native macOS notifications on task completion, agent idle, or input prompts; scheduled notifications via `schedule` tool.
* **Command Surface**: Slash commands (`/goal`, `/plan`, `/schedule`, `/browser`, `/grill-me`, `/boost`), `@` mentions menu (Files, Folders, Conversations, Terminals, Rules, MCP tools).

---

### 1.6 Windsurf (Codeium)

Windsurf is an AI-native IDE built on a VS Code fork, powered by Codeium's **Cascade** agent and "AI Flow" engine ([Windsurf Official](https://codeium.com/windsurf), [Windsurf Documentation](https://docs.codeium.com/windsurf), [Cascade Architecture](https://codeium.com/blog/windsurf-cascade)).

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ Windsurf — Cascade & AI Flow                                                │
├──────────────┬────────────────────────────────┬─────────────────────────────┤
│ File Tree &  │ Main Editor Canvas             │ Cascade Panel               │
│ Project Nav  │                                │                             │
│              │ • Inline Multi-File Diffs      │ • Conversational Thread     │
│ • Explorer   │ • Buffer Action Banner         │ • Modified File Chips       │
│ • Source     │   (Accept / Reject File)       │ • Turbo Mode Toggle         │
│   Control    │ • Built-in Chromium Preview    │ • Global Accept/Reject All  │
│              │ • Terminal Output Dock         │ • MCP & Memory Drawer       │
└──────────────┴────────────────────────────────┴─────────────────────────────┘
```

#### Window Layout & Multi-Session View
* **IDE Integration**: Standard VS Code layout with Cascade housed in the right dock panel (`Cmd+L`).
* **Session Drawer**: Left header drawer in Cascade lists historical chat sessions.
* **Integrated Preview Split**: Editor tabs can split to display the built-in Chromium browser preview alongside code.

#### File, Folder, and Code Presentation
* **Modified File Pills**: Files edited by Cascade appear as interactive chips at the top of the turn. Clicking a pill jumps directly to that file's staged changes in the editor.
* **Context Awareness**: Automatically tracks currently open editor tabs, cursor line positions, and terminal selections.

#### Diff Viewer & Review Workflow
* **Buffer-Level Inline Diffs**: Edits are staged directly into open editor buffers with colored line additions and deletions.
* **Buffer Action Banner**: A sticky banner at the top of each edited buffer provides *Accept* and *Reject* buttons.
* **Cascade Global Actions**: The Cascade thread panel offers global *Accept All* and *Reject All* buttons for multi-file operations.

#### Plan & Todo Display
* **Actionable Plans**: Cascade writes structured plans into the chat thread before multi-file refactors.
* **Status Trackers**: Real-time progress pills indicate whether Cascade is searching files, running shell commands, or editing code.

#### Checkpoints & Rewind
* **File Edit Undo**: Changes staged in editor buffers can be undone using standard undo stacks or the *Reject* action before saving.
* **Git Checkpointing**: Encourages staging or committing to Git before running major autonomous refactors.

#### Permissions & Sandboxing UX
* **Turbo Mode**: When enabled, terminal commands execute automatically without prompts. When disabled, every command requires clicking *Run* or *Cancel*.
* **Buffer Isolation**: File edits remain in an uncommitted buffer state until accepted by the developer.

#### Terminal, Browser, Notifications, and Commands
* **Terminal**: Integrated VS Code terminal; Cascade runs commands inside a dedicated "Cascade Terminal" with live streaming output.
* **Browser Preview**: Built-in Chromium webview ("Preview" panel) allows Cascade to inspect rendered web apps, extract DOM elements, and capture browser console errors.
* **Notifications**: Editor status bar notifications and OS desktop alerts on completion.
* **Command Surface**: `Cmd+L` (open Cascade), `Cmd+I` (inline edit), `Cmd+Shift+P` (Command Palette), `@` mentions for files, folders, web, and docs.

---

### 1.7 Kiro (AWS)

Kiro is AWS's agentic software development platform designed for full-lifecycle software engineering. It spans three coordinated surfaces: **Kiro IDE** (built on Code OSS), **Kiro CLI**, and **Kiro Web & Mobile** ([Kiro Platform](https://kiro.dev), [Kiro Specs Guide](https://kiro.dev/docs/specs), [Kiro Checkpoints](https://kiro.dev/docs/checkpoints), [Kiro Steering](https://kiro.dev/docs/steering)).

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ Kiro IDE — Spec-Driven Architecture                                         │
├──────────────┬────────────────────────────────┬─────────────────────────────┤
│ Spec & File  │ Editor Buffer                  │ Kiro Agent Panel            │
│ Navigation   │                                │                             │
│              │ • Code & Design Buffers        │ • Spec Generation Workflow  │
│ • Specs Tree │ • Checkpoint Diff Viewer       │ • Task Checklist Execution  │
│ • Steering   │ • Pull Request Review View     │ • MCP Tool Stream           │
│ • Workspace  │ • Integrated Terminal Dock     │ • Cloud Session Sync        │
└──────────────┴────────────────────────────────┴─────────────────────────────┘
```

#### Window Layout & Multi-Session View
* **Three Operational Surfaces**:
  * *Kiro IDE*: Desktop editor based on Code OSS with specialized sidebars for Specs, Steering, and Agent Chat.
  * *Kiro CLI*: Headless terminal interface supporting slash commands (`/spec`, `/checkpoint`) for terminal-native or CI/CD pipelines.
  * *Kiro Web & Mobile*: Cloud management interface for monitoring autonomous background agents and reviewing PRs remotely.
* **Multi-Session Handling**: Cloud-synced sessions enable initiating an agent task locally and monitoring or approving it from mobile.

#### File, Folder, and Code Presentation
* **Specs Hierarchy**: Left sidebar features a dedicated "Specs" explorer alongside the traditional file tree, organizing tasks by feature and milestone.
* **Code in Chat**: Files modified are listed with status badges; code snippets feature one-click insert and diff inspection.

#### Diff Viewer & Review Workflow
* **Checkpoint Diff View**: Accessible via `/checkpoint diff` or the IDE diff pane, comparing any two checkpoints or comparing current state against a baseline.
* **Review Controls**: Side-by-side and unified diff views with hunk-level accept/reject capabilities.

#### Plan & Todo Display (Spec-Driven Development)
* **Three-Tier Spec Hierarchy**: Kiro formalizes planning into three mandatory specification files:
  1. `requirements.md`: User stories, acceptance criteria, and operational scope.
  2. `design.md`: Technical architecture, data models, and component contracts.
  3. `tasks.md`: Discrete, sequenced implementation tasks.
* **Traceable Execution**: The agent iterates through `tasks.md`, checking off items as tests pass, ensuring full traceability from prompt to Pull Request.

#### Checkpoints & Rewind
* **Workspace Checkpoints (`/checkpoint`)**: First-class snapshot system that captures full workspace state.
* **Checkpoint CLI & UI**: Commands include `/checkpoint create`, `/checkpoint list`, `/checkpoint restore <id>`, and `/checkpoint diff <id1> <id2>`.
* **Rollback Reliability**: Provides instant, deterministic rollback to any prior checkpoint snapshot.

#### Permissions & Sandboxing UX
* **Agent Steering**: Persistent rule repository (`.kiro/steering/`) enforcing architectural conventions, security boundaries, and coding standards.
* **Agent Hooks**: Event-driven automation triggers agent actions on file creation, modification, or save.
* **Approval Gates**: Sensitive operations and external network calls trigger approval requests in the IDE or mobile app.

#### Terminal, Browser, Notifications, and Commands
* **Terminal**: Built-in IDE terminal dock plus standalone Kiro CLI.
* **Browser Preview**: Supported via webview integrations and cloud preview environments.
* **Notifications**: IDE alert banners, terminal bells, and mobile push notifications for PR approvals or human steering requests.
* **Command Surface**: Slash commands (`/spec`, `/checkpoint`, `/steer`), VS Code command palette (`Cmd+Shift+P`), `@` mentions menu.

---

## 2. Comprehensive Comparison Matrix

The table below summarizes all seven harnesses across the eleven surveyed dimensions:

| Dimension | Claude Code Desktop | Codex App / Work | Cursor | Zed (ACP Agent Panel) | Google Antigravity | Windsurf | Kiro |
|---|---|---|---|---|---|---|---|
| **Base Surface** | Standalone Desktop (Electron) + CLI | Desktop App + CLI (`codex-cli`) | VS Code Fork + Cloud VMs | Native Rust Editor (GPUI) | Standalone (2.0) + IDE (VS Code) | VS Code Fork | Code OSS Fork + CLI + Web/Mobile |
| **Window Layout** | 3 tabs (Chat/Cowork/Code), live tree, thread, inspector | Multi-threaded command center, thread, diff/terminal pane | Editor buffer, floating/docked Composer (`Cmd+I`), BG agent view | Right dock Agent Panel, Threads sidebar, Terminal threads | Left sidebar, central Chat Canvas, Auxiliary Pane | Editor canvas, right Cascade dock, bottom terminal, preview | Left Specs/Steering explorer, center editor, right Agent dock |
| **Multi-Session** | Parallel sessions with Git worktrees | Thread list backed by isolated Git worktrees | Composer history drawer + Cloud agent dashboard | Threads sidebar with concurrent agent & terminal threads | Left session list + Subagents tree in Auxiliary Pane | Session history drawer in Cascade | Cloud-synced persistent sessions (IDE + Mobile) |
| **Files & Code** | Live sidebar file tree with read/write badges | Inspector changed file list, external editor handoff | Context chips (`@file`), modified file cards | Project panel, `@file`/`@symbol` context pills | Workspace tree, Files Changed auxiliary pane, line anchors | File pills above turn, open tab tracking, jump-to-code | Dedicated Specs tree + project file explorer |
| **Diff Review** | Unified & side-by-side, per-turn & cumulative | Dedicated Diff pane (unified & side-by-side) | Inline buffer diffs (green/red) + side-by-side editor | Editable unified diff buffer (`agent: open agent diff`) | Auxiliary Files Changed pane + in-editor overlays | Inline buffer diffs with buffer action banner | Checkpoint diff viewer (unified & side-by-side) |
| **Inline Diff Comments** | Supported (comments feed into next prompt) | Supported (highlight lines + comment popover) | Supported (via inline `Cmd+K` feedback) | Manual editing in diff buffer | Artifact feedback and in-editor feedback | Review banner feedback | PR comment feedback updates steering |
| **Accept / Reject** | Per-hunk, per-file, Accept/Reject all | Per-hunk, per-file, manual diff edits | Per-hunk floating bar, Accept All (`Cmd+Enter`) | Per-hunk, discard all, direct buffer edits | Per-hunk, per-file in Files Changed pane | Per-buffer banner + Cascade Accept/Reject All | Per-hunk and per-file checkpoint controls |
| **Plan / Todo UX** | Plan mode (`Shift+Tab`), read-only exploration | Plan mode: architecture & test plan checklist | Checklist in chat, Notepads, thought accordions | Markdown checklist in thread, `/context` workflows | `/plan` mode, interactive Artifact cards & checklists | Structured plan in chat, real-time status pills | **Spec-Driven**: `requirements`, `design`, `tasks.md` |
| **Checkpoints & Rewind** | Automatic turn checkpoints (100 buffer), `/rewind` | Turn snapshots in worktrees, rollback to prior turn | Checkpoints per turn, "Checkout" button in timeline | Editor transaction undo, thread branching | Turn snapshots, branch workspaces (`Workspace: branch`) | Action undo stack, recommended Git checkpoints | First-class `/checkpoint` CLI & UI (create/restore/diff) |
| **Permissions UX** | Modes: `manual`, `plan`, `auto`, `acceptEdits` | Policies: `on-request`, `workspace-write`, danger | "Yolo Mode" toggle vs manual approval (25 limit) | Tool prompt: Once, Thread, Always, Deny | 4 policies, terminal sandbox, non-workspace file access | "Turbo Mode" toggle vs per-command Run/Cancel | Agent Steering rules, Hooks, cloud approval gates |
| **Sandboxing** | Process-level rules in `.claude/settings.json` | OS sandbox, network blocked by default | Isolated cloud VMs (background agents) | OS sandbox for tools, ACP protocol permissions | OS terminal sandbox, non-workspace write blocks | Buffer isolation until accepted | Workspace boundary checks, steering constraints |
| **Notifications** | macOS desktop alerts, audio chimes | Desktop alerts, mobile push relay | Desktop alerts, Slack/email/mobile for cloud agents | Minimal native macOS alerts | Native macOS alerts, timer/cron schedule alerts | Editor status bar alerts, OS notifications | IDE banners, terminal bells, mobile push alerts |
| **Terminal** | Integrated PTY terminal dock | Integrated worktree sandbox terminal | Integrated VS Code terminal + output streaming | Native Terminal Threads in Agent Panel | Sandboxed terminal, persistent sessions, process control | Dedicated Cascade Terminal with live output | Integrated terminal dock + headless Kiro CLI |
| **Browser Preview** | Dev server preview tab + computer use | Canvas / HTML artifact rendering | Simple Browser extension, MCP browser tools | None built-in (external browser) | Integrated browser automation & generative UI | Built-in Chromium "Preview" panel with DOM tools | Cloud web surface preview |
| **Git / PR Flow** | Worktrees, commit generation, GitHub PR creation | Worktree lifecycle, commit generation, GitHub PRs | Git view, background agents auto-push & open PRs | Status bar git integration, `/git` slash command | Branch workspaces, commit generation, git safety hooks | Standard VS Code Git integration | End-to-end PR drafting, review response, auto-steering |
| **Command Surface** | `Shift+Tab`, double `Esc`, slash commands, `@` menu | `Cmd+K`, slash commands, `@` context mentions | `Cmd+I`, `Cmd+K`, `Cmd+.`, `Cmd+Enter`, `@` menu | `Cmd+Shift+P`, `Cmd+?`, `agent: open agent diff`, `@` | Slash commands (`/plan`, `/goal`), `@` mentions menu | `Cmd+L`, `Cmd+I`, `Cmd+Shift+P`, `@` mentions menu | Slash commands (`/spec`, `/checkpoint`), `Cmd+Shift+P` |

---

## 3. Chorus Current State & Architecture

Chorus is an AI agent desktop harness for macOS designed around the core principle: ***"Many agents. One score."*** It allows multiple frontier and local agents (Codex, Claude Code, Gemini, local Ollama) to operate within the **same session**, share the **same memory store**, access the **same skills**, and **handoff work mid-turn** when rate limits, token ceilings, or capabilities are reached ([CLAUDE.md](../../../CLAUDE.md#L8-L18)).

### Core Architectural Invariants
1. **Storage ≠ Context**: Cold on-disk storage optimizes for byte density and fast indexing; hot context assembled for models optimizes for minimal token consumption and is never written back to disk ([CLAUDE.md rule 2.1](../../../CLAUDE.md#L26-L34)).
2. **The Ledger is the Single Source of Truth**: The session belongs to Chorus, not the agent provider. Provider sessions are ephemeral single-turn execution targets ([CLAUDE.md rule 2.2](../../../CLAUDE.md#L35-L38)).
3. **Zero Logic in Swift**: Every piece of domain logic resides in Rust (`chorus-core`, `chorus-acp`, `chorus-cli`, `chorus-ffi`). Swift/SwiftUI is strictly a rendering and input shell communicating via a unidirectional C-ABI event stream ([CLAUDE.md rule 2.3](../../../CLAUDE.md#L39-L43)).
4. **Agent-Driven Orchestration**: Chorus does not run an internal LLM planner; an agent invokes Chorus (via CLI or MCP) to spawn, message, wait on, and collect other agents ([map.md Notes](../../../.scratch/chorus-harness-roadmap/map.md#L15-L16)).
5. **View-First File Experience**: Chorus is not an IDE. It features a polished file tree, code viewer, and diff viewer with "Open in External Editor" shortcuts, avoiding in-app text editing or LSP complexity ([map.md Notes](../../../.scratch/chorus-harness-roadmap/map.md#L18)).

### Inventory of Current Features (CLAUDE.md Section 3 & Roadmap)
* **macOS SwiftUI Shell (`apps/macos`)**:
  * Multi-agent execution modes: Single, Relay (auto-failover on quota/rate-limits), Compare (forked tree per agent), and Ask-Each.
  * Conversational thread with visual agent blame markers.
  * Inspector panes: Context ladder, Changes, Memory approve queue, and Task State.
  * Diff viewer: Displays unified diffs with **per-hunk keep/reject buttons** (`reject_hunks` via FFI).
  * Interactive permission dialog for ACP tool authorizations.
  * Background compaction trigger (`compactIfIdle`) driving warm-tier summarization.
* **Rust Core Engine (`chorus-core`)**:
  * Append-only SQLite ledger with BLAKE3 hashing, zstd compression, and DAG fork-chain history.
  * Blob store for oversized tool results and content-addressed payload deduplication.
  * Fact memory store (≤140 chars) with approve queue and automated projection to `CLAUDE.md`, `AGENTS.md`, and `GEMINI.md`.
  * Context budget allocator with 3-tier rung ladder ($L_0$ conversation, $L_1$ extractive summary, $L_2$ Ollama background summary).
  * Checkpoint manager utilizing a shadow Git repository (`<cwd>/.chorus/ckpt`) for filesystem snapshots, diff calculation, and full rollback (`revert_if_touched`).
  * macOS Seatbelt sandboxing (`sandbox-exec`) enforcing file-write restrictions to the workspace and preventing tampering with `.chorus/` or `.git/hooks`.
  * Streamable-HTTP MCP server exposing `memory_propose`.
* **Agent Integration (`chorus-acp`)**:
  * Client implementation of Agent Client Protocol (ACP) over stdio.
  * Production bridges for Codex (`@agentclientprotocol/codex-acp`) and Claude Code (`@zed-industries/claude-agent-acp`).

---

## 4. Ranked Chorus Gaps Analysis

Judged against the survey of the seven desktop harnesses and Chorus's design roadmap, the following features are missing from Chorus today. They are ranked in order of **how significantly each capability enhances an agent's operational effectiveness and pairing reliability**:

---

### Rank 1: Inline Diff Comments Sent Back to the Agent (Review Loop)
* **Gap Description**: Chorus currently displays unified diffs in the `Changes` pane and allows accepting or rejecting hunks. However, it provides no way for the user to select lines in a diff and submit targeted feedback comments that get piped directly into the agent's prompt context for the next turn.
* **Why It Matters for Agent Effectiveness**: In all leading harnesses (Codex app, Claude Code, Cursor), the primary human steering loop during code generation is diff review. Forcing users to copy-paste code snippets and line numbers into the general chat composer is slow, error-prone, and consumes excessive context. Inline diff comments provide pinpoint grounding that allows models to correct hallucinations or missed edge cases on the first try.
* **Roadmap Alignment**: Explicitly designated as a core Phase 2 requirement in `map.md` ("diff: unified and side-by-side, scoped per turn / per agent / per session, with inline comments sent back to the agent to fix").

---

### Rank 2: Git Worktree Isolation for Parallel Write Agents
* **Gap Description**: When running compare modes or orchestrating parallel subagents, all agents currently share the project's working directory. Chorus uses shadow Git snapshots, but concurrent writes by two agents into the same tree create race conditions and corrupt the index.
* **Why It Matters for Agent Effectiveness**: Independent Git worktrees (used by the Codex app and Claude Code) provide complete filesystem isolation. Parallel agents can execute builds, write files, and run tests independently without stepping on one another, allowing safe concurrent multi-agent pairing.
* **Roadmap Alignment**: Stated preference in `map.md` ("Parallel agents that write get their own git worktree. Read-only agents may share the main tree").

---

### Rank 3: View-First File Tree & Code Viewer (with External Editor Hand-Off)
* **Gap Description**: Chorus lacks a workspace file explorer and a full-file code viewer. Users can only reference files by typing `@` to insert paths into the text composer. There is no visual file tree showing files touched or read by the agent, nor a viewer with syntax highlighting and line numbers.
* **Why It Matters for Agent Effectiveness**: Developers need to see what files the agent is inspecting and modifying in real time. A fast, view-first file tree with read/write badges helps the developer supervise the agent's context scope and spot when the agent is exploring the wrong directories.
* **Roadmap Alignment**: Phase 2 priority in `map.md` ("File handling is view-first: beautiful file tree, folder, and code display with syntax highlighting, plus 'Open in external editor'. No in-app editing, no IDE").

---

### Rank 4: Side-by-Side Split Diff View & Per-Turn Scoping
* **Gap Description**: Chorus's `Changes` pane only renders unified diffs, and diffs are computed across the entire working tree since the session baseline rather than scoped to the specific turn or specific agent that produced them.
* **Why It Matters for Agent Effectiveness**: Unified diffs become unreadable when reviewing large refactors across multiple files. Side-by-side split views allow rapid visual verification of structural edits. Furthermore, in a multi-agent relay or compare session, attributing diffs to the exact agent and turn that authored them is essential for evaluating model performance.
* **Roadmap Alignment**: Directly requested in `map.md` ("Diff: unified and side-by-side, scoped per turn / per agent / per session").

---

### Rank 5: Interactive Checkpoint & Rewind Timeline UI
* **Gap Description**: Chorus has an underlying shadow Git checkpoint engine (`Checkpoint::snapshot` and `restore`), but lacks an interactive UI timeline. Users cannot browse past turn checkpoints, preview what the project looked like at turn 3, or perform granular rollbacks (revert code vs revert conversation).
* **Why It Matters for Agent Effectiveness**: When an agent takes a wrong architectural turn, developers need an immediate, friction-free way to rewind the codebase without losing their entire chat transcript. Claude Code's `/rewind` and Cursor's checkpoint timeline demonstrate that recovery speed directly determines developer trust in agent autonomy.
* **Roadmap Alignment**: Leverages existing `checkpoint.rs` primitives; requires exposing checkpoint indices over FFI to SwiftUI.

---

### Rank 6: Subagent Tree with Live Status, Transcripts, and Process Controls
* **Gap Description**: Chorus has no subagent monitoring dashboard. When an orchestrator agent spawns subagents, the human supervisor has no visual hierarchy showing child agent states, active tools, per-child token expenditure, or kill/message controls.
* **Why It Matters for Agent Effectiveness**: In agent-driven orchestration, runaway subagents can quickly burn tokens or enter infinite tool loops. The Auxiliary Pane pattern from Antigravity and Cursor's background agent monitors prove that hierarchical visibility and instant cancel controls are vital for multi-agent safety.
* **Roadmap Alignment**: Phase 3 Multi-Agent milestone in `map.md` ("Subagents: tree with live status, per-child transcript, per-child tokens, and stop/message where bridges allow").

---

### Rank 7: Interactive Plan / Spec / Task-State Checklist
* **Gap Description**: Chorus computes a derived task state from the ledger (`task_state().done` and brief constraints), but lacks an interactive, editable checklist or spec interface. The agent cannot check off milestones interactively, and the user cannot edit constraints directly in the UI.
* **Why It Matters for Agent Effectiveness**: Kiro's Spec-Driven Development (`requirements.md`, `design.md`, `tasks.md`) and Claude Code's Plan Mode demonstrate that maintaining an explicit, evolving contract prevents model drift over multi-step workflows. A living checklist keeps the model anchored to the user's acceptance criteria.
* **Roadmap Alignment**: Enhances the existing `Task state` pane in Chorus with editable constraints and interactive task checkboxes.

---

### Rank 8: Embedded Terminal / PTY Output Stream
* **Gap Description**: Chorus captures tool execution output into blobs, but does not render a live interactive terminal or PTY output stream. Users cannot watch long-running test suites execute in real time or interact with terminal programs that expect TTY input.
* **Why It Matters for Agent Effectiveness**: Long builds or test suites feel like silent freezes when output is buffered until completion. Live streaming terminal logs (as in Claude Code, Zed, and Windsurf) provide immediate feedback and allow the user to interrupt stuck processes early.
* **Roadmap Alignment**: Can be implemented as an auxiliary inspector dock in SwiftUI powered by standard macOS PTY bindings.

---

### Rank 9: Keyboard-First Command Palette & Slash Commands
* **Gap Description**: Chorus lacks a global command palette (`Cmd+K`) and in-composer slash commands (`/plan`, `/rewind`, `/compact`, `/handoff`). All operations currently require mouse clicks on sidebar buttons or sheets.
* **Why It Matters for Agent Effectiveness**: High-velocity developer pairing relies heavily on keyboard shortcuts. A unified command palette allows switching models, toggling modes, referencing files, and triggering handoffs without lifting hands from the keyboard.
* **Roadmap Alignment**: Standard macOS shortcut and menu integration following `macos-design-guidelines`.

---

### Rank 10: Native macOS Completion Notifications & Audio Chimes
* **Gap Description**: Chorus currently polls `events_since` and updates UI views, but emits no macOS user notifications (`UNUserNotificationCenter`) or system audio cues when a long-running turn completes, a subagent finishes, or a permission confirmation is needed.
* **Why It Matters for Agent Effectiveness**: When pairing on tasks that take several minutes (e.g. running extensive test suites or refactoring multiple files), developers switch windows. Without notifications, the agent sits idle waiting for approval, wasting valuable developer time.
* **Roadmap Alignment**: Lightweight Swift-side addition respecting macOS notification permissions.

---

### Rank 11: Headless Daemon Core Boundary (Remote-Ready Architecture)
* **Gap Description**: `chorus-cli` and `chorus-ffi` currently run in-process with the desktop shell or CLI runner. There is no decoupled headless daemon exposing a local JSON-RPC/Unix domain socket API that remote web or mobile clients can connect to.
* **Why It Matters for Agent Effectiveness**: As demonstrated by Codex's mobile relay and Kiro's web/mobile surfaces, developers want long-running agent tasks to execute on their desktop workstation while retaining the ability to review diffs and grant approvals remotely from a phone or browser.
* **Roadmap Alignment**: Phase 6 architectural requirement in `map.md` ("Remote/mobile: this map only decides that the core can run as a headless daemon with an API").

---

### Rank 12: Integrated Browser Preview (Webview / Dev Server)
* **Gap Description**: Chorus provides no internal browser preview pane for inspecting frontend web applications or rendered HTML artifacts.
* **Why It Matters for Agent Effectiveness**: For frontend engineering workflows, an integrated webview (like Windsurf Previews, Devin Desktop, or Antigravity's generative UI) allows capturing console logs, DOM state, and visual screenshots to feed back into the agent loop.
* **Roadmap Alignment**: Lower priority for Perth's immediate core workflow, but valuable for general-purpose web development.

---

## 5. Summary and Architectural Recommendations for Chorus

1. **Prioritize the Review Loop First (Phases 1 & 2)**:
   Chorus already has the hardest architectural pieces in place: an immutable, content-addressed ledger, multi-model ACP bridging, and Seatbelt sandboxing. To transform into an elite agent harness, Chorus must close the **human feedback loop** by pairing **side-by-side diffs** with **inline diff comments** that automatically assemble into the next turn's prompt.
2. **Protect Multi-Agent Concurrency with Git Worktrees (Phase 3)**:
   Chorus's headline capability is multi-agent coordination (Relay, Compare, Handoff). To scale this to autonomous agent-driven orchestration, Chorus must implement Git worktree management in Rust, giving every parallel writing agent an isolated workspace.
3. **Double Down on the View-First Philosophy**:
   Chorus must resist the temptation to become a full-blown IDE with text editing and LSP support. By focusing exclusively on being the best **inspector, reviewer, and orchestrator** (beautiful file tree, high-performance diff viewer, external editor launching, and deep multi-model memory), Chorus can offer a cleaner, faster experience than bloated Electron or VS Code forks.
