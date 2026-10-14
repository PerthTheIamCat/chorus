# Chorus Desktop Harness Master Roadmap

> **Specification & Implementation Roadmap**  
> *Transforming macOS Chorus into a First-Class Desktop Agent Harness*  
> Document Version: 1.0.0 (Roadmap Finalized)  
> Scope: Architectural Design & Phased Implementation Master Plan

---

## 1. Executive Summary & Core Invariants

Chorus is an agentic desktop harness for macOS designed to supervise, evaluate, and orchestrate frontier coding agents (Codex, Claude Code, and local open-source models).

This roadmap synthesizes findings across 9 comprehensive research surveys (Tickets 01–09) and 13 resolved architecture decisions (Tickets 10–22). It defines a structured 6-phase implementation path where each milestone is gated on empirical metrics defined in the Chorus Harness Evaluation Suite.

### Strict Architectural Invariants
1. **Rule 2.3 (Strict Rust Core Boundary)**: 100% of business logic, diff computations, Myers alignments, syntax highlighting spans, token accounting, routing math, and filesystem tree traversals live in Rust (`chorus-core` / `chorusd`). Swift and AppKit layers strictly render views and capture input via C FFI / IPC.
2. **Rule 2.2 (Single Source of Truth)**: The SQLite Ledger is the sole authority for session turns, event history, agent lineages, diff checkpoints, and interaction requests.
3. **View-First Supervisor UX**: Chorus is not an IDE. It features high-fidelity, read-only file/folder and diff viewers with instant one-click external editor hand-off (`⇧⌘E` to Cursor, VS Code, Zed) and inline review loops to steer agents.
4. **Native macOS Aesthetics**: Strictly native macOS conventions, SF Symbols typography, Floating Glass Cards, and smooth 120Hz ProMotion animations. **Zero emoji glyphs** in UI surfaces.

---

## 2. Master System Architecture

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                              macOS Client Layer (Swift / AppKit)                       │
│  ┌───────────────────────┬──────────────────────────┬───────────────────────────────┐  │
│  │  Universal Sidebar    │  Workspace Explorer      │  Center Stage (Thread / Code) │  │
│  │  (Chats, Workspaces)  │  (NSOutlineView 120Hz)   │  (TextKit 2 / Floating Cards) │  │
│  └───────────────────────┴──────────────────────────┴───────────────────────────────┘  │
│  ┌──────────────────────────────────────────────────────────────────────────────────┐  │
│  │  Inspector (Diff Viewer with Myers Alignment, Subagent Tree, Context, Memory)    │  │
│  └──────────────────────────────────────────────────────────────────────────────────┘  │
└───────────────────────────────────────────▲────────────────────────────────────────────┘
                                            │ Unix Domain Socket (`~/.chorus/engine.sock`)
                                            │ + High-throughput SSE Streaming
┌───────────────────────────────────────────▼────────────────────────────────────────────┐
│                             Headless Core Daemon (`chorusd`)                           │
│  ┌─────────────────────────┬────────────────────────────┬───────────────────────────┐  │
│  │  Agent Control Surface  │  Subagent Normalizer       │  Local Router Engine      │  │
│  │  (CLI & HTTP-MCP)       │  (Codex vs Claude Stream)  │  (In-process 3-8ms probe) │  │
│  └─────────────────────────┴────────────────────────────┴───────────────────────────┘  │
│  ┌─────────────────────────┬────────────────────────────┬───────────────────────────┐  │
│  │  Git Worktree Manager   │  Async Interaction Engine  │  Token & Cost Accounting  │  │
│  │  (.chorus/worktrees/)   │  (Approval State Machine)  │  ($ per turn / worktree)  │  │
│  └─────────────────────────┴────────────────────────────┴───────────────────────────┘  │
│  ┌──────────────────────────────────────────────────────────────────────────────────┐  │
│  │  SQLite Ledger (WAL mode, FTS5 Full-Text Search, Hierarchical Lineage Paths)     │  │
│  └──────────────────────────────────────────────────────────────────────────────────┘  │
└───────────────────────────────────────────▲────────────────────────────────────────────┘
                                            │ Process & stdio pipes
┌───────────────────────────────────────────▼────────────────────────────────────────────┐
│                               Agent Bridges & External Engines                         │
│  ┌───────────────────────┬──────────────────────────┬───────────────────────────────┐  │
│  │  Codex CLI Bridge     │  Claude Code CLI Bridge  │  Local Ollama LLM Bridge      │  │
│  └───────────────────────┴──────────────────────────┴───────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Phased Implementation Roadmap

### Phase 1: Foundation (Core Daemon, Control Surface, & Harness Eval)
> **Goal**: Establish the decoupled headless daemon, provide agents with a rich bidirectional control surface, implement post-edit verification loops, and deploy the empirical evaluation harness.

* **Key Deliverables**:
  1. **Headless Daemon (`chorusd`)** (*Ticket 20*):
     - Transition from in-process C FFI to standalone background daemon communicating over Unix Domain Socket (`~/.chorus/engine.sock`) and SSE event streaming.
     - Reconnection catch-up via `Last-Event-ID` ledger replay.
     - Async `Interaction` state machine for non-blocking tool/bash approvals.
     - Graceful `⌘Q` handling allowing background agent execution with native macOS chimes and notifications.
  2. **Agent-Facing Control Surface (CLI & Streamable MCP)** (*Ticket 11*):
     - Comprehensive agent tools: `chorus_spawn_agent`, `chorus_send_message`, `chorus_wait_agent`, `chorus_collect_results`, `chorus_cancel_agent`, `chorus_update_checklist`, `chorus_jump_to_file`, `chorus_request_review`.
     - AI Role Generator: Dynamic role synthesis and customizable agent profiles stored in `.chorus/agents/*.md`.
  3. **Harness Scaffolding & Verification Hooks** (*Ticket 03, 21*):
     - Pre-dispatch and post-turn hooks in `.chorus/hooks/` (e.g. `post-turn.sh` running `cargo check`, lint, or test suites).
     - Universal project rules projection injecting `.chorus/rules/`, `AGENTS.md`, and `CLAUDE.md` into agent system prompts.
  4. **Harness Evaluation Framework** (*Ticket 09, 10*):
     - 60 paired real-world engineering tasks (30 Thai / 30 English) across Rust, Swift, TypeScript, and Python.
     - Dual-agent matrix (Codex and Claude Code).
     - Automated gates: Tier 0 (<2s unit invariants) on pre-commit, Tier 1 (<30s router gate) on PR/CI, and Tier 2 (full live worktree suite) pre-release.
* **Exit Criteria**:
  - `chorusd` runs headlessly; CLI and MCP clients spawn and control sessions over UDS.
  - Tier 0 and Tier 1 harness eval suites pass with 100% reliability.

---

### Phase 2: Workbench UI (View-First Architecture)
> **Goal**: Ship a premier macOS supervisor interface: fluid 120Hz file tree, high-performance code viewer, and unified/split diff reviewer with inline comment steering loops.

* **Key Deliverables**:
  1. **4-Column Adaptive Layout** (*Ticket 13*):
     - Universal Sidebar (260px), Workspace Explorer (`⌘B`, 240px), Center Stage with dynamic Thread/Code split, and Inspector (`⌥⌘I`, 340px).
     - Preserves Floating Glass Cards styling with native macOS vibrancy and dividers.
  2. **File, Folder, & Code Viewer** (*Ticket 14*):
     - AppKit `NSOutlineView` wrapped in SwiftUI: virtualized rendering capable of navigating 50,000+ files at smooth 120Hz.
     - Glowing agent focus ring (`●`) indicating which file is actively being read or edited in real time.
     - Native TextKit 2 code viewer in SF Mono with Rust-computed syntax highlighting spans (`syntect`/`tree-sitter`).
     - One-click external editor handoff (`⇧⌘E`) passing `--goto <path>:<line>` to Cursor, VS Code, Zed, or Xcode in <300ms.
  3. **Diff Viewer & Review Loop** (*Ticket 15*):
     - Dual view modes: Unified and Side-by-Side Split.
     - Rust-computed Myers diffing with spacer line alignment to eliminate dual-scroll drift.
     - 4 Diff Scopes: Latest Turn, Current Agent, Full Session, and Agent Worktree.
     - Gutter hover inline review comments with floating pending comments toolbar and one-click "Send to Agent `⌘↩`" steering feedback loop.
     - Per-hunk reverse rollback to undo individual unwanted changes.
  4. **Live Plan Checklist & Git Commit Flow** (*Ticket 21*):
     - Live task checklist card in Inspector driven by `chorus_update_checklist`.
     - 1-click conventional commit generator and push/PR trigger via `gh` CLI upon diff acceptance.
* **Exit Criteria**:
  - 120Hz scrolling verified across repositories with >50,000 files.
  - Inline diff comments reliably steer agent in subsequent turns.
  - `⇧⌘E` opens targeted file and line in external editor in <300ms.

---

### Phase 3: Multi-Agent & Orchestration
> **Goal**: Enable safe parallel agent execution via Git worktrees and transparent subagent observability.

* **Key Deliverables**:
  1. **Subagent Visibility & Bridge Normalizer** (*Ticket 06, 16*):
     - Unified Subagent Normalizer in `chorus-acp`: correctly decouples Claude Code `Task` tool calls and Codex `subagent_spawned` events.
     - Native macOS hierarchy tree in Inspector showing status badges (`⟳ Running`, `✓ Completed`, execution time, tool counts).
     - Dedicated Child Transcript Drawer: isolates child reasoning traces and raw tool outputs from the main conversation thread.
  2. **Git Worktree Isolation Engine** (*Ticket 02, 12*):
     - Sub-200ms worktree provisioning under `.chorus/worktrees/<role>-<short_id>/` on dedicated branches.
     - Environment replication via `.worktreeinclude`: automatically mirrors `.env`, `.env.local`, and symlinks dependency caches (`node_modules`, `target`).
     - Dynamic `$PORT` allocation per worktree (e.g. `PORT=3001, 3002...`) with one-click external browser launch button (`[ ↗ Open localhost:3001 ]`).
     - Automated clean fast-forward merge upon task completion; conflict quarantine and guided resolution.
     - Parent context protection: child sessions return compact structured briefs (summary, diff stat, test results) to prevent token bloat.
     - User live supervision: switch between active worktrees in Workspace Explorer with mid-flight steering (`⌘↩`) or cancellation (`[ Stop ]`).
* **Exit Criteria**:
  - 3 concurrent write agents execute simultaneously without git lock collisions or file corruption.
  - Subagent logs do not leak into parent transcript; parent context remains compact.

---

### Phase 4: Router Quality & Analytics
> **Goal**: Eliminate model over-picking, accelerate pre-dispatch routing to sub-10ms, and display real-time monetary cost.

* **Key Deliverables**:
  1. **In-Process Local Router Probe** (*Ticket 07, 17*):
     - Replace external Ollama router with in-process embedding probe (`candle`/`ort`) written in Rust.
     - Routing latency drops from ~600ms to 3–8ms.
     - Removal of the 5-second countdown timer in favor of instant dispatch.
     - Cost-biased utility optimization defaulting to low-effort/cheaper model tiers (`gpt-5-codex` / low effort) for routine tasks.
     - Subtle UI pill showing selected model with 1-click model switch/regeneration.
     - SQLite vector kNN learning flywheel: stores user manual overrides as training exemplars.
  2. **Monetary Cost Accounting** (*Ticket 21*):
     - Maps token metrics from ACP bridges and Ollama to cached pricing tables.
     - Displays live cost in dollars (\$ per turn, per session, per worktree) in the Inspector.
* **Exit Criteria**:
  - Routing decision completes in <10ms without daemon dependencies.
  - Over-picking eliminated: >60% of routine coding tasks routed to cheaper tiers without regression on Tier 1 & Tier 2 evals.

---

### Phase 5: Chat Mode & Attachments UX
> **Goal**: Provide a frictionless workspace-free chat experience with rich clipboard attachments and full-text search.

* **Key Deliverables**:
  1. **Workspace-Free Chat Mode** (*Ticket 04, 18*):
     - Dedicated "Chats" section in Universal Sidebar above Projects; starts instantly with no directory selected.
     - Mid-turn model switching: swap between Codex, Claude, and local Ollama models on subsequent turns.
     - Dual Split/Compare mode to contrast answers from different models.
     - "Promote to Task" workflow: synthesizes chat discussion into a project `Brief` and launches an engineering task in a workspace.
  2. **Attachment UX Polish** (*Ticket 19*):
     - Clipboard `⌘V` thumbnail staging with Spacebar Quick Look modal preview.
     - Dropzones across composer and thread for files and screenshots.
     - `⌘L` context chip shortcut from Code and Diff viewers to stage code references into composer.
     - Vision capability indicator badges (`eye` vs `exclamationmark.triangle`) using native SF Symbols.
  3. **Full-Text Search & Export** (*Ticket 21*):
     - SQLite FTS5 instant search across all historical turns, sessions, and code snippets (`⌘F`).
     - Clean Markdown and HTML session export.
* **Exit Criteria**:
  - Instant chat launch with zero disk repository bound.
  - Clipboard image paste, thumbnail generation, and Quick Look preview respond in <1s.
  - FTS5 search queries across 1,000 sessions complete in <50ms.

---

### Phase 6: Remote-Ready Core Boundary (Daemon Verification)
> **Goal**: Solidify the headless daemon API contracts for future mobile control clients.

* **Key Deliverables**:
  1. **Headless API & Protocol Verification** (*Ticket 05, 20*):
     - Validate `chorusd` Unix Domain Socket and HTTP/SSE APIs against mobile control requirements.
     - Standardize async permission approval endpoints and event serialization.
     - Specification of mTLS / WireGuard / Tailscale tunnel encapsulation and QR-code pairing handshake for future iOS client.
* **Exit Criteria**:
  - Core API contract frozen and verified ready for independent mobile client development.

---

## 4. Ticket Dependency Matrix

```
Research Tickets (01 - 09)
       │
       ├──► 10: Harness Eval Framework ──────────────┐
       │                                             │
       ├──► 11: Agent Control Surface (MCP/CLI) ─────┼──► Phase 1: Foundation
       │                                             │
       ├──► 20: Headless Core Boundary (`chorusd`) ──┤
       │                                             │
       ├──► 21: Feature Gap Triage ──────────────────┘
       │
       ├──► 13: Workbench Layout ────────────────────┐
       │                                             │
       ├──► 14: File & Code Viewer ──────────────────┼──► Phase 2: Workbench UI
       │                                             │
       ├──► 15: Diff Viewer & Review Loop ───────────┘
       │
       ├──► 06/16: Subagent Visibility & Bridge ─────┐
       │                                             ├──► Phase 3: Multi-Agent
       ├──► 12: Git Worktrees Orchestration ─────────┘
       │
       ├──► 07/17: Router Quality Policy ────────────► Phase 4: Router
       │
       ├──► 18: Chat Mode ───────────────────────────┐
       │                                             ├──► Phase 5: Chat & Attachments
       ├──► 19: Attachment UX ───────────────────────┘
       │
       └──► 22: Roadmap Sequencing & Handoff ────────► Phase 6: Remote Core Boundary
```

---

## 5. Summary of Decision Artifacts

| Ticket | Area | Type | Document Link | Status |
|---|---|---|---|---|
| **01** | Desktop Harness Survey | Survey | [01-desktop-harness-survey.md](./research/01-desktop-harness-survey.md) | Resolved |
| **02** | Multi-Agent Orchestrators | Survey | [02-multi-agent-orchestrator-survey.md](./research/02-multi-agent-orchestrator-survey.md) | Resolved |
| **03** | OSS Agent Effectiveness | Survey | [03-oss-harness-effectiveness-survey.md](./research/03-oss-harness-effectiveness-survey.md) | Resolved |
| **04** | Chat Apps Survey | Survey | [04-chat-app-survey.md](./research/04-chat-app-survey.md) | Resolved |
| **05** | Remote & Mobile Control | Survey | [05-remote-mobile-survey.md](./research/05-remote-mobile-survey.md) | Resolved |
| **06** | Subagent Bridge Visibility | Survey | [06-subagent-visibility-in-bridges.md](./research/06-subagent-visibility-in-bridges.md) | Resolved |
| **07** | Local Router Approaches | Survey | [07-local-router-approaches.md](./research/07-local-router-approaches.md) | Resolved |
| **08** | macOS Code & Diff Components | Survey | [08-macos-code-and-diff-components.md](./research/08-macos-code-and-diff-components.md) | Resolved |
| **09** | Harness Eval Methodology | Survey | [09-harness-eval-methodology.md](./research/09-harness-eval-methodology.md) | Resolved |
| **10 (Survey)** | Super App Frontier Gap Analysis | Survey | [10-super-app-frontier-gap-analysis.md](./research/10-super-app-frontier-gap-analysis.md) | Resolved |
| **11 (Survey)** | Deep macOS Integration & Agent Research | Survey | [11-macos-deep-integration-and-agent-research.md](./research/11-macos-deep-integration-and-agent-research.md) | Resolved |
| **12 (Survey)** | Developer Tools & MCP Ecosystem Survey | Survey | [12-developer-tools-extensions-and-mcp-ecosystem.md](./research/12-developer-tools-extensions-and-mcp-ecosystem.md) | Resolved |
| **10** | Harness Eval Design | Decision | [10-harness-eval-design.md](./issues/10-harness-eval-design.md) | Resolved |
| **11** | Agent Control Surface | Decision | [11-agent-control-surface.md](./issues/11-agent-control-surface.md) | Resolved |
| **12** | Agent Orchestration & Worktrees | Decision | [12-agent-orchestration-and-worktrees.md](./issues/12-agent-orchestration-and-worktrees.md) | Resolved |
| **13** | Workbench Layout | Prototype | [13-workbench-layout.md](./issues/13-workbench-layout.md) / [Prototype](./prototypes/13-workbench-layout.md) | Resolved |
| **14** | File & Code Viewer | Prototype | [14-file-and-code-viewer.md](./issues/14-file-and-code-viewer.md) / [Prototype](./prototypes/14-file-and-code-viewer.md) | Resolved |
| **15** | Diff Viewer & Review Loop | Prototype | [15-diff-viewer-and-review-loop.md](./issues/15-diff-viewer-and-review-loop.md) / [Prototype](./prototypes/15-diff-viewer-and-review-loop.md) | Resolved |
| **16** | Subagent Visibility UX | Prototype | [16-subagent-visibility-ux.md](./issues/16-subagent-visibility-ux.md) / [Prototype](./prototypes/16-subagent-visibility-ux.md) | Resolved |
| **17** | Router Quality Policy | Decision | [17-router-quality-policy.md](./issues/17-router-quality-policy.md) | Resolved |
| **18** | Chat Mode | Decision | [18-chat-mode.md](./issues/18-chat-mode.md) | Resolved |
| **19** | Attachment UX Polish | Decision | [19-attachment-ux.md](./issues/19-attachment-ux.md) | Resolved |
| **20** | Headless Core Boundary | Decision | [20-headless-core-boundary.md](./issues/20-headless-core-boundary.md) | Resolved |
| **21** | Feature Gap Triage | Decision | [21-feature-gap-triage.md](./issues/21-feature-gap-triage.md) | Resolved |
| **22** | Roadmap Sequencing & Handoff | Decision | [22-roadmap-sequencing-and-handoff.md](./issues/22-roadmap-sequencing-and-handoff.md) | Resolved |

---
*Roadmap finalized and ready for Phase 1 implementation issue generation.*
