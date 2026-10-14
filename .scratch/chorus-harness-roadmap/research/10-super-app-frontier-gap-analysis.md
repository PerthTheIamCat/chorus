# 10. Super App Frontier Gap Analysis: Architectural Blueprint & Master Roadmap

> **Specification & Architectural Gap Analysis**  
> *Transforming Chorus into the Ultimate Desktop Super App for Frontier AI Coding & Multi-Agent Swarms*  
> Document Version: 1.0.0  
> Status: Fully Resolved Architectural Blueprint

---

## 1. Executive Summary & The Super App Vision

Modern software engineering with AI is fragmented across siloed paradigms:
- **Cursor** provides high-speed speculative autocomplete and Merkle-tree context indexing, but locks developers into an Electron IDE fork with a single-agent paradigm.
- **Claude Code & Codex** offer deep reasoning and terminal execution, but lack a unified desktop visualization surface, cross-session ledger persistence, and multi-model collaboration.
- **Devin & OpenHands** pioneer autonomous sandbox execution and browser debugging, but suffer from high cloud latency, heavy container overhead, and lack local-first privacy.
- **Cline & Roo Code** innovate on shadow git checkpoints and MCP extensibility, but run inside VS Code extensions bound by JavaScript runtime limits and UI sluggishness.
- **Aider** achieves benchmark-topping code editing via its Tree-sitter Repository Map and Architect/Editor split, but operates purely in a terminal CLI without visual multi-agent orchestration.

**Chorus's Super App Thesis**:  
Chorus is uniquely positioned to unify these capabilities into a single, high-performance, native macOS desktop application. By anchoring all state into a high-throughput **SQLite WAL Ledger (Rule 2.2)**, executing 100% of core logic, AST parsing, diff computation, and agent sandboxing in **Rust (Rule 2.3)**, and rendering at native 120Hz via **AppKit/SwiftUI**, Chorus can become the command center where **heterogeneous frontier agents (Claude Opus, OpenAI Codex, GPT-5, and Apple Silicon MLX/local models) collaborate inside isolated Git worktrees within the same durable session**.

---

## 2. Comparative Feature Matrix: Chorus vs. Frontier Benchmarks

| Capability Area | Feature / Mechanism | Current Chorus | Frontier Benchmark State | Target Chorus Super App |
|---|---|---|---|---|
| **1. Context Engine** | **High-Density Repo Map** | ❌ None (file tree only) | **Aider**: Tree-sitter AST + PageRank (~1k tokens) | **Rust `tree-sitter` PageRank Repo Map** with SQLite tag cache |
| | **Incremental Indexing** | ❌ Full fs scan | **Cursor**: Merkle Tree hashing (<1ms delta check) | **BLAKE3 Merkle Tree** change detector in `chorus-core` |
| | **@-Mentions Context** | ❌ None | **Cursor/Windsurf**: `@file`, `@symbol`, `@git`, `@web` | **Token-budgeted `@` mention parser** injecting AST snippets |
| | **Prompt Cache Alignment** | ⚠️ Partial (rung ladder) | **Claude / OpenAI**: Strict static prefix stability | **Zero-invalidation prompt cache prefix pipeline** |
| **2. Multi-Agent Swarm** | **Worktree Isolation** | ⚠️ Basic CLI git worktree | **Devin/Claude**: Independent parallel branches/subagents | **Sub-200ms Git Worktree Swarm** with `.worktreeinclude` |
| | **Execution Topology** | ⚠️ Linear child agents | **CrewAI/OpenHands**: Dynamic DAG fan-out/fan-in | **Declarative Agent DAG Swarm** (Plan → Code → Review → Test) |
| | **Merge Conflict Synthesis** | ⚠️ Fast-forward only | **Devin**: Autonomous merge & conflict resolution | **LLM-assisted 3-way semantic conflict synthesis** |
| | **Attribution & Blame** | ⚠️ Actor ID in events | Basic commit authorship | **Per-line Subagent Blame & Token Accounting** |
| **3. Tool Ecosystem** | **MCP Host Architecture** | ⚠️ Server only (exposes tools) | **Claude Desktop / Cline**: Full client & tool runner | **Native Rust MCP Host & Gateway Multiplexer** |
| | **MCP Resources & Prompts** | ❌ Tools only | **MCP Spec**: `resources/read`, `prompts/get` | **Full MCP 1.0 Host** supporting Tools, Resources, Prompts |
| | **Tool Audit & Sandbox** | ⚠️ Seatbelt on bridge | **Cline**: Granular per-tool approval & token logging | **Ledger-backed MCP proxy** with Seatbelt sandboxing |
| **4. Checkpoints & Rewind** | **Shadow Git Snapshot** | ⚠️ Flat `.chorus/ckpt` | **Cline/Roo**: Per-turn commit DAG & rewind | **Turn-indexed Git Branching DAG** tied to SQLite `seq` |
| | **3-Way Selective Rewind** | ❌ Full restore only | **Claude `/rewind`**: Code only, chat only, or both | **3-Way Selective Rewind**: Code+Ledger, Code-Only, Ledger-Only |
| | **Shadow Speculative Run** | ❌ Runs in workspace | **Cursor**: Background shadow workspace validation | **Speculative Worktree**: Test before touching workspace |
| **5. Visual Canvas** | **Interactive Artifacts** | ❌ None | **Claude Artifacts**: Live React, HTML, SVG, Mermaid | **Native macOS Canvas**: Live HTML, SVG, Mermaid & TextKit 2 |
| | **Dev Server Browser Bridge** | ❌ External browser only | **Devin / Manus**: Headless CDP / Playwright debugging | **In-Harness CDP Bridge**: Autonomous screenshot & console log |
| | **Visual Regression Loop** | ❌ None | **OpenCode / Playwright**: Visual snapshot diffing | **Agent Screenshot Visual Feedback Loop** |
| **6. Cost & Orchestration** | **Architect/Editor Pipeline** | ❌ Single agent turn | **Aider**: Opus plans → Haiku/Qwen edits (-70% cost) | **Native Architect-Editor Turn Pipeline** in `chorus-core` |
| | **In-Process Fast Routing** | ⚠️ Rule heuristics | **Cursor**: Deep intent routing (<10ms) | **3-8ms Rust SIMD Embedding Probe** + kNN override memory |
| | **Hard Spend Guardrails** | ⚠️ Passive tracking | **OpenRouter/Portkey**: Hard spend limits & circuit breaker | **Enforced Dollar Budgets** per turn, session, and day |
| **7. Local Privacy** | **Apple Silicon Inference** | ⚠️ External Ollama | **MLX / llama.cpp**: Metal unified memory acceleration | **Native MLX / Metal in-process engine** (Qwen2.5/DeepSeek) |
| | **Local Vector Indexing** | ❌ None | **sqlite-vec**: Embedded vector search in SQLite | **`sqlite-vec` + BLAKE3 deduplication** for zero-latency search |
| | **Air-Gapped Shield** | ⚠️ Seatbelt file-deny | Enterprise offline mode | **Seatbelt Network Kill-Switch**: Zero WAN egress mode |
| **8. Auto-Verification** | **Closed-Loop Self-Correction**| ⚠️ Post-turn hooks only | **Cursor / Devin**: Autonomous lint/test retry loop | **Autonomous Verification Loop**: Edit → Lint/Test → Fix (max N) |
| | **LSP Diagnostics Engine** | ❌ None (external only) | **Cursor / Windsurf**: Real-time language server errors | **Embedded Lightweight LSP Client** (Rust, TS, Python, Swift) |
| | **AST-Aware Guardrails** | ⚠️ Path denylist | **Claude hooks**: Command safety AST validator | **Rust Command AST Inspector**: Blocks destructive operations |

---

## 3. Deep-Dive Gap Analysis Across the 8 Strategic Pillars

---

### Pillar 1: Context & Knowledge Engine (Repo Map, AST, @-mentions, Memory)

#### Primary Sources & Prior Art:
- **Aider Repository Map**: Uses `tree-sitter` to parse source code, extracts function/type signatures, constructs a directed graph of symbol dependencies, runs Personalized PageRank, and formats a dense structural map within a strict token budget (~1,024 tokens) ([Aider Repo Map Architecture](https://aider.chat/docs/repomap.html)).
- **Cursor Codebase Indexing**: Utilizes a hierarchical Merkle Tree of BLAKE3/SHA-256 hashes to detect file diffs in <1ms without traversing the entire filesystem. Chunks ASTs via Tree-sitter and stores embeddings in a vector index for `@Codebase` queries ([Cursor Architecture Specs](https://cursor.com)).
- **Anthropic & OpenAI KV Prompt Caching**: Both Anthropic and OpenAI cache prompt prefixes. Prefixes must remain 100% byte-identical across requests to hit the cache, saving up to 90% on input costs and reducing Time-To-First-Token (TTFT) by 80% ([Anthropic Prompt Caching RFC](https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching)).

#### Current Chorus Implementation:
- `crates/chorus-core/src/memory.rs`: Compact facts (≤140 chars) in SQLite, `fact_proposal` queue for human approval.
- `crates/chorus-core/src/context.rs`: Rung ladder (L0=recent 10 turns, L1=11–40, L2=41+), table manifests, and summarization.
- `crates/chorus-core/src/projector.rs`: Projects memories into `CLAUDE.md`, `AGENTS.md`, and `GEMINI.md`.
- `crates/chorus-core/src/search.rs`: FTS5 full-text search over `event.body`.

#### The Exact Gaps:
1. **Lack of a High-Density AST Repo Map**: Agents have no global structural awareness of classes, structs, traits, and function signatures across the repository without manually issuing dozens of `fs/read_text_file` calls.
2. **No Merkle Tree Incremental Tracking**: File changes are detected by full directory scanning or `git status`, which incurs unacceptable I/O latency in repositories with >20,000 files.
3. **Missing `@-Mention` Ingestion Subsystem**: Users cannot explicitly pin symbols (`@UserAuthService`), files (`@crates/chorus-core/src/ledger.rs`), diffs (`@git-staged`), or docs into the composer context.
4. **Prompt Cache Invalidation**: Assembling context dynamically without strict prefix segregation invalidates the LLM's KV prompt cache on every turn.

#### Concrete Architectural Design (Rule 2.2 & Rule 2.3):
- **`crates/chorus-core/src/repo_map.rs`**:
  - Implement a Rust-native Tree-sitter symbol extractor supporting Rust, Swift, TypeScript, Python, and Go.
  - Extract `definitions` (functions, structs, classes, traits) and `references`.
  - Maintain a symbol dependency graph and run **Personalized PageRank** focused on recently touched files.
  - Budget-aware renderer: Generates a 500–1,500 token structural synopsis injected into system context.
- **SQLite Schema Migration (v7)**:
  ```sql
  CREATE TABLE IF NOT EXISTS ast_symbol (
      id          INTEGER PRIMARY KEY,
      file_path   TEXT NOT NULL,
      symbol_name TEXT NOT NULL,
      kind        TEXT NOT NULL, -- "fn", "struct", "trait", "class", "interface"
      signature   TEXT NOT NULL,
      line_start  INTEGER NOT NULL,
      line_end    INTEGER NOT NULL,
      file_hash   TEXT NOT NULL
  );
  CREATE INDEX IF NOT EXISTS idx_ast_symbol_lookup ON ast_symbol(symbol_name, file_path);
  
  CREATE TABLE IF NOT EXISTS merkle_node (
      path       TEXT PRIMARY KEY,
      blake3_hex TEXT NOT NULL,
      is_dir     INTEGER NOT NULL,
      updated_ms INTEGER NOT NULL
  );
  ```
- **Strict Prompt Cache Layout Invariant**:
  ```
  [1. Static System Prompt]  ──► Byte-stable across session (Cached)
  [2. Static Project Rules]   ──► AGENTS.md / CLAUDE.md (Cached)
  [3. Repo Map Structure]    ──► Updates only on git commit/file write (Cached)
  [4. Durable Facts (Mem)]   ──► Approved facts list (Cached)
  [5. Conversation History]  ──► Rung ladder (L2/L1 summaries + L0 turns)
  [6. Active Turn Context]   ──► Current user prompt + staged @-mentions
  ```

---

### Pillar 2: Autonomous Multi-Agent Swarm & Git Worktree Isolation

#### Primary Sources & Prior Art:
- **Devin & OpenHands Architecture**: Autonomous task planning using decoupled agents running in parallel isolated sandbox branches. Subagents report back structured progress briefs while child transcripts are segregated from parent context ([OpenHands Architecture](https://github.com/All-Hands-AI/OpenHands)).
- **Git Worktrees (`git worktree add`)**: Enables checkout of multiple branches simultaneously from a single `.git` repository, avoiding file lock contention and allowing concurrent agent file writing ([Git Worktree Documentation](https://git-scm.com/docs/git-worktree)).
- **ChatDev / MetaGPT**: Multi-agent software development topologies (Architect → Coder → Code Reviewer → Test Engineer).

#### Current Chorus Implementation:
- `crates/chorus-core/src/worktrees.rs`: Provisions worktrees under `.chorus/worktrees/<role>-<short_id>`, syncs `.worktreeinclude`, and assigns dynamic ports.
- `crates/chorus-core/src/agent_control.rs`: Models `subagent` and `subagent_message` in SQLite. Exposes `chorus_spawn_agent`, `chorus_send_message`, `chorus_wait_agent`, `chorus_collect_results`.

#### The Exact Gaps:
1. **Linear / Sequential Orchestration Only**: Chorus currently treats subagents as ad-hoc child turns rather than a coordinated DAG (Directed Acyclic Graph) with concurrent execution.
2. **Rudimentary Merge Strategy**: Worktree completion relies on naive `git merge --ff-only`. If concurrent agents touch intersecting files, merge aborts without intelligent conflict resolution.
3. **No Swarm Token/Cost Governance**: A subagent can trigger infinite loops or burn hundreds of thousands of tokens without a hard spend circuit-breaker.
4. **Lack of Agent Attribution**: The codebase lacks fine-grained per-line attribution indicating which agent/model authored which line of code across a multi-agent swarm run.

#### Concrete Architectural Design (Rule 2.2 & Rule 2.3):
- **`crates/chorus-core/src/swarm.rs`**:
  - Implement a **Declarative Swarm Coordinator** supporting DAG topologies:
    ```
    User Task ──► [Lead Architect] 
                        │
         ┌──────────────┴──────────────┐
         ▼                             ▼
    [Backend Builder]           [Frontend Builder]  (Parallel Worktrees)
         │                             │
         └──────────────┬──────────────┘
                        ▼
                 [QA & Reviewer]
    ```
  - Subagents run concurrently on background OS threads in isolated Git worktrees.
  - **Autonomous Merge Resolver**: If fast-forward fails, invoke a dedicated `MergeSpecialist` agent turn that inspects `git diff` conflict markers, synthesizes both intents, runs verification tests, and commits the resolution.
  - **Hard Spend Governor**: Every subagent is initialized with strict bounds: `max_turns: u32`, `max_tokens: usize`, `max_cost_usd: f64`. Once breached, the daemon issues a graceful `SubagentStatus::Cancelled` with partial diff capture.

---

### Pillar 3: Extensibility & Tool Ecosystem (Model Context Protocol - MCP Host/Client)

#### Primary Sources & Prior Art:
- **Model Context Protocol (MCP) 1.0 Specification**: Open standard by Anthropic for connecting AI models to data sources and tools. Defines JSON-RPC 2.0 protocol over stdio and Streamable-HTTP/SSE for `tools`, `resources`, and `prompts` ([Model Context Protocol Spec](https://modelcontextprotocol.io/)).
- **Cline & Roo Code MCP Marketplace**: Extensions allow users to install community MCP servers (GitHub, PostgreSQL, Brave Search, Slack, Puppeteer) with one click, automatically parsing tool definitions and exposing them to models.

#### Current Chorus Implementation:
- `crates/chorus-core/src/mcp.rs`: Implements an MCP *server* (`McpServer`) exposing internal Chorus primitives (`memory_propose`, `attachment_read`, `chorus_spawn_agent`) to the connected agent.
- `crates/chorus-core/src/mcp_config.rs`: Reads `.chorus/mcp.json` and forwards entries to ACP `session/new`.
- `crates/chorus-core/src/integration_catalog.rs`: Curated integration recipes (Homebrew, npm, uv, cargo).

#### The Exact Gaps:
1. **Chorus is Not an MCP Client/Host**: Chorus delegates tool execution to external agent CLIs (Codex/Claude). If a user configures a local model (via MLX or Ollama) or an agent runtime without native MCP client capabilities, external tools cannot be utilized.
2. **No MCP Gateway / Virtual Multiplexer**: Tools from 10 different MCP servers pollute the model's tool schema namespace. There is no dynamic filtering, rate limiting, or permission proxying in Rust.
3. **No Support for MCP Resources & Prompts**: Chorus completely ignores the `resources` (`mcp://...`) and `prompts` primitives of the MCP protocol, which are essential for reading documentation and database schemas.

#### Concrete Architectural Design (Rule 2.2 & Rule 2.3):
- **`crates/chorus-core/src/mcp_host.rs`**:
  - Implement a robust, async **MCP Client Manager** in Rust.
  - Spawn stdio child processes or establish HTTP-SSE connections to configured MCP servers.
  - Perform `tools/list`, `resources/list`, and `prompts/list` handshakes on startup.
- **Unified MCP Gateway**:
  - Prefix external tools into a clean, collision-free namespace: `mcp::<server_id>::<tool_name>`.
  - When an agent invokes a tool, `chorusd` intercepts the call, evaluates permission policy (Auto, Ask, Deny), executes the call via the Rust MCP Client, records the input and output directly into the SQLite ledger (`event` table), and returns the result.
  - Supports streaming resource reading (`resources/read`) to stage documentation into the context assembler without model overhead.

---

### Pillar 4: Time-Travel Checkpoint & Safe Execution (Rewind, Myers Diff Review Loops)

#### Primary Sources & Prior Art:
- **Cline Checkpoint System**: Maintains a shadow Git repository committing snapshots before and after every tool execution. Provides interactive rewind to any historical turn ([Cline Checkpoint Documentation](https://cline.bot)).
- **Claude Code `/rewind` Modal**: Interactive recovery dialog with selective rollbacks: Restore Code & Conversation, Restore Code Only, Restore Conversation Only, or Summarize ([Claude Code Checkpointing](https://code.claude.com/docs/en/checkpointing)).
- **Cursor Shadow Workspace**: Hidden background workspace where proposed edits are applied and checked against linters before presenting the diff to the user.

#### Current Chorus Implementation:
- `crates/chorus-core/src/checkpoint.rs`: Flat shadow git repo under `.chorus/ckpt` with `snapshot()`, `diff_since()`, `restore()`, and `revert_if_touched()`.
- `crates/chorus-core/src/diff_align.rs`: Myers diff algorithm with spacer line alignment in Rust to prevent dual-scroll drift.
- `crates/chorus-core/src/review.rs`: Gutter hover review comments stored in SQLite table `inline_review_comment`.
- `crates/chorus-acp/src/sandbox.rs`: macOS Seatbelt (`sandbox-exec -p`) denying file writes outside workspace.

#### The Exact Gaps:
1. **No Granular Turn-to-Commit Mapping**: The shadow repo currently stores linear snapshots. If a user forks a session or navigates the ledger DAG, checkpointing does not track branches.
2. **Missing 3-Way Selective Rewind**: If an agent wrote good code but hallucinated in the conversation, the user cannot keep the code changes while wiping the conversational transcript (or vice versa).
3. **No Speculative Shadow Workspace**: Edits are applied directly to the user's working copy; if an agent breaks syntax or generates corrupt files, the user must rely on manual rollback.

#### Concrete Architectural Design (Rule 2.2 & Rule 2.3):
- **`crates/chorus-core/src/checkpoint_dag.rs`**:
  - Map each SQLite `event.seq` to a cryptographic Git commit hash in `.chorus/ckpt`.
  - Maintain a branch hierarchy mirroring the session fork tree.
- **3-Way Selective Rewind Implementation**:
  ```rust
  pub enum RewindMode {
      CodeAndLedger, // Roll back working tree + mark subsequent events soft-deleted
      CodeOnly,      // Roll back working tree files; preserve chat context
      LedgerOnly,    // Keep working tree edits; roll back conversation to target seq
  }
  
  pub fn rewind_to_seq(
      store: &Store,
      ckpt: &Checkpoint,
      session_id: i64,
      target_seq: i64,
      mode: RewindMode
  ) -> Result<RewindSummary>;
  ```
- **Speculative Shadow Execution**:
  - Agent file edits occur in a background worktree (`.chorus/shadow/<session_id>`).
  - Auto-verification hooks (lint, build, unit test) run inside the shadow worktree.
  - Only when checks pass (or user explicitly clicks "Approve Diff"), changes are synchronized to the main workspace in a single atomic transaction.

---

### Pillar 5: Multi-Modal Visual Canvas & Interactive Artifacts (HTML/Mermaid, Embedded Browser)

#### Primary Sources & Prior Art:
- **Claude Artifacts**: Real-time rendering of side-by-side interactive content (React components, raw HTML/CSS/JS, SVG vector graphics, Mermaid state diagrams, and Markdown documents) ([Anthropic Artifacts Specification](https://claude.ai)).
- **Devin / OpenHands Headless Browser Automation**: Agents control a headless browser via Chrome DevTools Protocol (CDP) or Playwright. The agent can navigate to `localhost:<port>`, interact with elements, inspect console errors, and capture full-page screenshots to visually debug UI bugs ([OpenHands BrowserToolSet](https://github.com/All-Hands-AI/OpenHands)).

#### Current Chorus Implementation:
- `apps/macos`: View-First Supervisor architecture with `WorkspaceExplorerView` (file tree), `CodeViewerView` (TextKit 2), and `DiffReviewerView` (Myers diff).
- `crates/chorus-core/src/attachments.rs`: Session-scoped image and file attachment storage with deduplication.
- Embedded browser preview was historically classified as "in the fog" (Ticket 21) due to resource concerns.

#### The Exact Gaps:
1. **No Artifact Rendering Engine**: When agents output architectural diagrams, UI mockups, or SVG flowcharts, users see raw code blocks instead of rendered visual artifacts.
2. **Zero Agent Visual Perception of Dev Servers**: Agents building web applications are completely blind. They cannot verify whether a CSS layout broke, whether a button is clickable, or whether a client-side JavaScript error was logged to the console.
3. **No Visual Diffing**: When styling changes occur, there is no before/after visual snapshot comparison.

#### Concrete Architectural Design (Rule 2.2 & Rule 2.3):
- **`crates/chorus-core/src/artifacts.rs`**:
  - Streaming parser in Rust detecting tagged artifact blocks (````html:artifact`, ````mermaid:artifact`, ````svg:artifact`).
  - Store artifact revisions in SQLite table `artifact` with monotonic versioning.
- **macOS Native Interactive Canvas**:
  - Implement `CanvasView.swift` leveraging native AppKit `WKWebView` with local security sandboxing (`about:blank`, CSP blocking external network egress).
  - Native Mermaid.js rendering engine embedded in the application bundle.
- **Headless Chrome DevTools Protocol (CDP) Bridge (`cdp_bridge.rs`)**:
  - Expose an MCP tool `chorus_browser_inspect(port: u16, path: String)`:
    - Launches headless Chromium (via system Chrome or Playwright driver).
    - Captures viewport screenshot, DOM accessibility tree, and browser console error logs.
    - Encodes the screenshot into `attachment` table and returns visual feedback to the agent.

---

### Pillar 6: Smart Cost/Model Orchestration (Architect-Editor Pipeline, Local Routing, Cost Tracking)

#### Primary Sources & Prior Art:
- **Aider Architect/Editor Workflow**: Decouples software engineering into two distinct model stages:
  1. **Architect Model** (Claude 3.7 Sonnet / Claude Opus / OpenAI o1): Focuses purely on high-level reasoning, system design, and file selection.
  2. **Editor Model** (Claude 3.5 Haiku / Qwen2.5-Coder / DeepSeek-V3): Applies the diffs, adheres to strict syntax formatting, and verifies file modifications.
  *Empirical Result*: Cuts total token expenditure by 65–80% while matching or exceeding single-model benchmark accuracy ([Aider Architect Mode](https://aider.chat/docs/architect.html)).
- **Windsurf Cascade Flow**: Continuous dual-model intent routing predicting when to autocomplete, when to plan, and when to execute.

#### Current Chorus Implementation:
- `crates/chorus-core/src/local_router.rs`: In-process heuristic classifier (3–8ms) scoring task complexity and estimating cost.
- `crates/chorus-core/src/handoff.rs`: Inter-agent handoff pipeline (Freeze → Repack → Brief → Verify → Resume) for quota failover.

#### The Exact Gaps:
1. **Single-Agent Turn Constraint**: Chorus currently routes a turn to either Model A or Model B. It lacks an orchestrated pipeline where an Architect plans and an Editor writes code within a single user request.
2. **Lack of Hard Budget Enforcement**: Chorus calculates cost in dollars, but cannot enforce hard ceilings (e.g. "Stop if this turn exceeds $1.50" or "Daily workspace cap of $20.00").
3. **Static Routing Heuristics**: The local router relies on keyword heuristics rather than semantic vector similarity against real task exemplars.

#### Concrete Architectural Design (Rule 2.2 & Rule 2.3):
- **`crates/chorus-core/src/pipeline.rs`**:
  - Implement the **Architect-Editor Execution Pipeline**:
    ```
    User Prompt ──► [Stage 1: Architect (e.g. Claude Opus)]
                           │  Produces structured Plan & Pseudo-Diff
                           ▼
                    [Stage 2: Editor (e.g. Claude Haiku / Qwen-Coder)]
                           │  Applies exact file hunks & formats syntax
                           ▼
                    [Verification Stage]
    ```
  - Both stages share the same session turn, logging intermediate reasoning as `EventKind::Thought` and final code edits as `EventKind::Diff`.
- **Budget & Guardrail Policy**:
  - Add `budget_policy` table to SQLite schema with `turn_max_cost_usd`, `session_max_cost_usd`, and `alert_threshold_usd`.
  - When the accumulator crosses thresholds, `chorusd` halts agent execution and requests user approval via the `InteractionManager`.

---

### Pillar 7: Local-First Privacy & Apple Silicon Acceleration (MLX, llama.cpp, Air-Gapped Mode)

#### Primary Sources & Prior Art:
- **Apple MLX Framework**: Native machine learning framework for Apple Silicon utilizing the unified memory architecture (UMA) to achieve blazing fast token generation and zero-copy data exchange between CPU and GPU ([Apple MLX GitHub](https://github.com/ml-explore/mlx)).
- **`sqlite-vec`**: Lightweight SQLite extension for vector search written in C, enabling embedded vector similarity search directly inside standard `.db` files ([sqlite-vec Documentation](https://github.com/asg017/sqlite-vec)).
- **Air-Gapped Developer Environments**: Enterprise security standard requiring 100% offline execution with zero network telemetry or code egress.

#### Current Chorus Implementation:
- Relies on an external Ollama daemon (`compaction.rs`) for local summarization.
- macOS Seatbelt (`sandbox.rs`) blocks filesystem writes outside workspace, but allows outbound WAN network traffic.

#### The Exact Gaps:
1. **External Daemon Dependency Debt**: Requiring users to install and manage an external Ollama background process introduces configuration failure points and high latency.
2. **No Local Code Embedding Engine**: Chorus has no in-process vector search capabilities for symbol lookup or semantic code exploration.
3. **Absence of a Strict Air-Gapped Privacy Mode**: Sensitive proprietary codebases cannot be fully isolated from external network leaks.

#### Concrete Architectural Design (Rule 2.2 & Rule 2.3):
- **Embedded Apple Silicon Inference Engine (`crates/chorus-core/src/local_engine.rs`)**:
  - Integrate native Apple Silicon acceleration via `llama-cpp-rs` (Metal backend) or bundled MLX binaries.
  - Automatically detect M-series hardware capabilities (M1/M2/M3/M4 Max/Ultra) and allocate unified RAM budgets.
  - Bundle lightweight local models for summarization (Qwen-2.5-Coder-1.5B) and embedding generation (`bge-small-en-v1.5`).
- **`sqlite-vec` Integration in SQLite WAL Ledger**:
  - Store 384-dimensional code embeddings in SQLite virtual table `code_vector USING vec0`.
  - Perform nearest-neighbor vector queries in <5ms without external database processes.
- **Air-Gapped Shield Profile (`sandbox.rs`)**:
  - Add `Profile::AirGapped`:
    ```scheme
    (version 1)
    (deny default)
    (allow file-read*)
    (allow file-write* (subpath "/path/to/workspace"))
    (deny network-outbound*)
    (allow network-outbound (to ip "127.0.0.1")) ; Allow only local IPC/dev servers
    ```

---

### Pillar 8: Auto-Verification & Guardrail Engine (Post-Edit Test/Lint Loops, Safety Hooks)

#### Primary Sources & Prior Art:
- **Cursor Background Linter Loop**: Continuously queries language server diagnostics (LSP) on modified files. If an agent introduces a syntax error, type mismatch, or lint violation, the diagnostic message is automatically fed back into the agent context before presenting changes to the user ([Cursor Background Linter](https://cursor.com)).
- **Windsurf Proactive Test Execution**: Automatically identifies the relevant test runner (`cargo test`, `pytest`, `npm test`) and executes affected test suites, allowing the agent to self-heal up to a user-defined retry threshold.

#### Current Chorus Implementation:
- `crates/chorus-core/src/hooks.rs`: Runs `.chorus/hooks/post-turn.sh` after an agent completes its turn.
- `crates/chorus-eval`: Implements loop detection and tier verification.
- `crates/chorus-core/src/checkpoint.rs`: `revert_if_touched()` rolls back changes if an unpermitted write occurs.

#### The Exact Gaps:
1. **No Autonomous Self-Correction Loop**: If `post-turn.sh` fails, the error is displayed to the user rather than being automatically fed back to the agent for self-repair.
2. **Missing LSP Language Server Integration**: Chorus has no visibility into compiler diagnostics (Rust `rust-analyzer`, TypeScript `tsserver`, Python `pyright`) without running full build commands.
3. **No AST-Aware Command Safety Interceptor**: Dangerous shell commands (`git reset --hard`, `rm -rf`, destructive SQL migrations) are not inspected prior to execution.

#### Concrete Architectural Design (Rule 2.2 & Rule 2.3):
- **`crates/chorus-core/src/verify_loop.rs`**:
  - Closed-loop autonomous verification orchestrator:
    ```
    Agent Edit ──► Run Linters / Tests ──► Success? ──► Present to User
                        │
                        ▼ (Failure)
    Parse Stderr / Diagnostics ──► Feed Error to Agent Context ──► Retry (Max N)
    ```
  - Configurable `max_auto_retries` (default: 3). If verification fails after N attempts, pause and escalate to user with full failure logs.
- **Lightweight LSP Diagnostics Client (`crates/chorus-core/src/lsp.rs`)**:
  - Spawns background language server instances (`rust-analyzer`, `typescript-language-server`).
  - Subscribes to `publishDiagnostics` notifications.
  - Automatically computes diagnostic badges (errors, warnings) and displays them in the Diff Reviewer gutter.
- **AST Command Guardrail Inspector**:
  - Parses shell commands via Rust AST parser (`tree-sitter-bash`).
  - Intercepts destructive patterns (`rm -rf /`, `git push --force`, `DROP DATABASE`) and triggers an explicit, high-priority user permission prompt via the `InteractionManager`.

---

## 4. Master Architectural Specifications (Rule 2.2 & Rule 2.3)

All new capabilities strictly adhere to the project's non-negotiable invariants:
1. **Rule 2.1 (*Storage ≠ Context*)**: Cold on-disk storage is compressed, indexed, and byte-optimized in SQLite and BLAKE3 blobs. Hot LLM context is dynamically assembled and never written back.
2. **Rule 2.2 (*SQLite WAL Ledger is SoT*)**: All turns, tool calls, worktrees, checkpoints, and artifacts are durable rows in SQLite.
3. **Rule 2.3 (*Strict Rust Core Boundary*)**: Zero business logic in Swift. AppKit/SwiftUI is strictly a high-performance presentation shell.

### Unified System Architecture Diagram

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                              macOS Client Layer (Swift / AppKit)                       │
│  ┌───────────────────────┬──────────────────────────┬───────────────────────────────┐  │
│  │  Universal Sidebar    │  Workspace Explorer      │  Center Stage (Thread / Code) │  │
│  │  (Chats, Workspaces)  │  (NSOutlineView 120Hz)   │  (TextKit 2 / Canvas Stage)   │  │
│  └───────────────────────┴──────────────────────────┴───────────────────────────────┘  │
│  ┌──────────────────────────────────────────────────────────────────────────────────┐  │
│  │  Inspector (Aligned Diff Reviewer, Subagent Swarm Tree, Interactive Artifacts)  │  │
│  └──────────────────────────────────────────────────────────────────────────────────┘  │
└───────────────────────────────────────────▲────────────────────────────────────────────┘
                                            │ Unix Domain Socket (`~/.chorus/engine.sock`)
                                            │ + High-throughput SSE Event Stream
┌───────────────────────────────────────────▼────────────────────────────────────────────┐
│                             Headless Core Daemon (`chorusd`)                           │
│  ┌─────────────────────────┬────────────────────────────┬───────────────────────────┐  │
│  │  Swarm DAG Coordinator  │  Rust MCP Host / Client    │  Architect-Editor Pipeline│  │
│  │  (Multi-Worktree Engine)│  (Virtual Tool Gateway)    │  (Cost-Optimized Dual Run)│  │
│  └─────────────────────────┴────────────────────────────┴───────────────────────────┘  │
│  ┌─────────────────────────┬────────────────────────────┬───────────────────────────┐  │
│  │  Tree-sitter Repo Map   │  Auto-Verification Loop    │  Turn Checkpoint DAG      │  │
│  │  (PageRank + Merkle AST)│  (LSP + Self-Healing Test) │  (3-Way Selective Rewind) │  │
│  └─────────────────────────┴────────────────────────────┴───────────────────────────┘  │
│  ┌─────────────────────────┬────────────────────────────┬───────────────────────────┐  │
│  │  Local Engine (Metal/UMA│  Air-Gapped Seatbelt Shield│  CDP Dev Server Bridge    │  │
│  │  + sqlite-vec Indexing) │  (Zero-Egress Sandbox)     │  (Headless Chrome Inspect)│  │
│  └─────────────────────────┴────────────────────────────┴───────────────────────────┘  │
│  ┌──────────────────────────────────────────────────────────────────────────────────┐  │
│  │  SQLite WAL Ledger (Schema v7: AST, Vectors, Swarms, Artifacts, Checkpoints)     │  │
│  └──────────────────────────────────────────────────────────────────────────────────┘  │
└───────────────────────────────────────────▲────────────────────────────────────────────┘
                                            │ stdio / IPC pipes
┌───────────────────────────────────────────▼────────────────────────────────────────────┐
│                         Frontier Agent Bridges & Local Runtimes                        │
│  ┌───────────────────────┬──────────────────────────┬───────────────────────────────┐  │
│  │  Claude Code ACP      │  Codex CLI ACP           │  Apple Silicon Metal Engine   │  │
│  │  (Opus / Sonnet 3.7)  │  (GPT-5 Sol / Luna)      │  (MLX / Qwen2.5-Coder / Llama)│  │
│  └───────────────────────┴──────────────────────────┴───────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 5. Actionable Phased Implementation Plan

```
Phase 1: Knowledge Engine & Checkpoint DAG
  ├── Milestone 1.1: Tree-sitter Repo Map & PageRank in Rust
  ├── Milestone 1.2: Merkle Tree Incremental Change Tracker
  ├── Milestone 1.3: Turn-Indexed Git Branching Checkpoint DAG
  └── Milestone 1.4: 3-Way Selective Rewind Engine & UI Modal

Phase 2: MCP Host Gateway & Autonomous Verification
  ├── Milestone 2.1: Native Rust MCP Client (stdio & HTTP-SSE)
  ├── Milestone 2.2: Unified Virtual Tool Namespace & Gateway
  ├── Milestone 2.3: Closed-Loop Auto-Verification Test/Lint Engine
  └── Milestone 2.4: Lightweight LSP Diagnostics Client

Phase 3: Multi-Agent Swarms & Architect-Editor Pipeline
  ├── Milestone 3.1: Declarative Multi-Agent DAG Swarm Coordinator
  ├── Milestone 3.2: Concurrent Worktree Isolation & Merge Specialist
  ├── Milestone 3.3: In-Turn Architect-Editor Orchestration Pipeline
  └── Milestone 3.4: Per-Subagent Hard Spend Circuit Breakers

Phase 4: Visual Canvas & Apple Silicon Acceleration
  ├── Milestone 4.1: Native macOS Artifact Canvas (HTML, SVG, Mermaid)
  ├── Milestone 4.2: Headless CDP Dev Server Inspection Bridge
  ├── Milestone 4.3: In-Process Metal/MLX Engine & `sqlite-vec` Search
  └── Milestone 4.4: Air-Gapped Zero-Egress Seatbelt Shield
```

### Milestone Exit Criteria Tied to Harness Evals:
1. **Phase 1 Exit Gate**:
   - Repo Map generation on a 100,000 LOC codebase completes in `<80ms`.
   - Merkle tree change detection resolves in `<2ms`.
   - 3-Way Rewind successfully restores files and ledger state without corrupting SQLite history.
2. **Phase 2 Exit Gate**:
   - Rust MCP Client connects to standard MCP servers (GitHub, PostgreSQL) with zero error rate.
   - Autonomous verification self-repairs induced syntax errors across 10 consecutive test cases.
3. **Phase 3 Exit Gate**:
   - 3 concurrent agents execute tasks in parallel Git worktrees without file collisions or git lock errors.
   - Architect-Editor pipeline achieves `>60%` token cost reduction compared to single-Opus turns with zero loss in task success rate on the Chorus Eval Suite.
4. **Phase 4 Exit Gate**:
   - Artifact canvas renders Mermaid diagrams and HTML mockups at 120Hz.
   - CDP bridge captures headless localhost screenshot in `<300ms`.
   - Air-Gapped mode completely blocks external DNS/TCP egress while maintaining full local execution.

---

## 6. Primary Sources, Specifications & RFC Citations

1. **Aider Repository Map**: [Aider Architecture & PageRank](https://aider.chat/docs/repomap.html)
2. **Anthropic Prompt Caching**: [Claude API Prompt Caching Guide](https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching)
3. **Model Context Protocol (MCP)**: [Anthropic MCP 1.0 Specification](https://modelcontextprotocol.io/)
4. **Agent Client Protocol (ACP)**: [Zed Industries & Agent Client Protocol](https://agentclientprotocol.com/)
5. **Cursor IDE Architecture**: [Cursor Technical Blog & Codebase Indexing](https://cursor.com)
6. **OpenHands Architecture**: [OpenHands Autonomous Agent Platform](https://github.com/All-Hands-AI/OpenHands)
7. **Apple MLX Framework**: [Apple Machine Learning Research MLX](https://github.com/ml-explore/mlx)
8. **`sqlite-vec`**: [Vector Search SQLite Extension](https://github.com/asg017/sqlite-vec)
9. **macOS Seatbelt (`sandbox-exec`)**: [Apple Sandbox Reference Guide](https://developer.apple.com/)
10. **Tree-sitter**: [Tree-sitter Incremental Parsing Library](https://tree-sitter.github.io/tree-sitter/)
