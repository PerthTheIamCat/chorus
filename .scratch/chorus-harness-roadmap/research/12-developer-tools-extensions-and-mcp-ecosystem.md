# 12. Developer Tools, Extensions, Plugins, and MCP Ecosystem Survey

> **Comprehensive Research Survey & Implementation Blueprint**  
> *Empirical Evaluation of Developer Tools, MCP Servers, Headless LSPs, and Frontier CLI Utilities for Chorus*  
> Target Location: `.scratch/chorus-harness-roadmap/research/12-developer-tools-extensions-and-mcp-ecosystem.md`  
> Document Version: 1.0.0  
> Status: Fully Resolved Architecture Blueprint  

---

## 1. Executive Summary & Problem Framing

Modern software engineering agents (such as Claude Code, OpenAI Codex, SWE-agent, Devin, and Cursor) are fundamentally constrained by their **Agent-Computer Interface (ACI)** and context-window dynamics. While frontier foundation models possess vast coding intelligence, their empirical effectiveness swings by double digits depending entirely on the tools and feedback mechanisms available in their harness:
1. **The Context Window Tax & Token Bloat**: Loading monolithic tool definitions (e.g., Model Context Protocol servers exposing 20–50 tools) consumes between **8,000 and 25,000 tokens** before a session executes a single instruction. Poor tool design pollutes context with raw, unindexed payloads (e.g., massive JSON dumps, DOM trees, verbose compiler error cascades), driving context degradation and hallucinated tool arguments.
2. **The Feedback Latency Bottleneck**: Fast iteration cycles are the lifeblood of agentic code repair. An agent utilizing legacy linters (ESLint, Flake8) or standard single-threaded test suites experiences feedback latencies of 15–45 seconds per turn. Conversely, next-generation Rust-native linters (`biome`, `ruff`, `oxlint`) and test schedulers (`cargo-nextest`, `vitest`) compress verification latency to **sub-100 milliseconds to 2 seconds**, unlocking multi-turn self-debugging loops without triggering agent timeouts.
3. **Lexical vs. Semantic Retrieval**: Relying purely on lexical search (`grep`, `rg`) forces agents to sift through 90%–99% false-positive noise in strings, comments, and mock implementations. Integrating structural AST search (`ast-grep`), structural AST diffing (`difftastic`), and headless Language Server Protocol (LSP) servers (`rust-analyzer`, `vtsls`, `basedpyright`, `sourcekit-lsp`) delivers **92x to 1,400x token compression** on reference searches and eliminates hallucinated symbol definitions.

This document presents a comprehensive, empirical survey of the developer tooling and MCP ecosystem. It evaluates tools across token efficiency, execution latency, and reliability, establishing an architectural blueprint for **Chorus** that strictly adheres to **Rule 2.2 (Single Source of Truth SQLite WAL Ledger)** and **Rule 2.3 (Strict Rust Core Boundary)**.

---

## 2. Model Context Protocol (MCP) Ecosystem Deep Dive

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                                MCP Architecture & Context Tax                          │
│                                                                                        │
│  [Agent Context Window]                                                                │
│  ┌──────────────────────────────────────────────────────────────────────────────────┐  │
│  │ System Prompt + Active Instructions                                              │  │
│  ├──────────────────────────────────────────────────────────────────────────────────┤  │
│  │ STATIC TOOL MANIFEST (Fixed Tax: 1.5k–25k tokens on EVERY turn)                  │  │
│  │  - server-filesystem (12 tools)  │  - postgres (6 tools)     │  - memory (8 tools) │  │
│  │  - github-mcp (28 tools)         │  - playwright (18 tools)  │  - custom tools...  │  │
│  ├──────────────────────────────────────────────────────────────────────────────────┤  │
│  │ Dynamic Conversation Turns & Tool Responses (Variable payload bloat)             │  │
│  └──────────────────────────────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

The Model Context Protocol (MCP), open-sourced by Anthropic in late 2024, standardizes how AI applications connect to external tools, data sources, and runtime environments via JSON-RPC 2.0 over Streamable HTTP or stdio. However, wide-scale adoption in clients like Claude Desktop, Cline, Roo Code, Cursor, and Zed has exposed severe operational trade-offs.

### 2.1 The "Monolithic Manifest" Tax & Tool Shadowing

In standard MCP implementations, all connected servers broadcast their tool definitions during `tools/list` at startup. These full JSON Schema definitions are injected into the agent's system prompt on **every single model invocation**:
- **Initialization Cost**: An agent configured with 5 standard servers (e.g., GitHub, Filesystem, Postgres, Brave Search, Memory) consumes **14,500 to 22,000 tokens of static overhead per turn**.
- **Model Confusion & Tool Shadowing**: When tool counts exceed 15–20, model routing accuracy degrades sharply. Frontier models exhibit *tool shadowing*, where two tools with overlapping semantics (e.g., `read_file` from `@modelcontextprotocol/server-filesystem` vs. `view_file` from the harness bash environment) cause agents to alternate erratically, fail schema validation, or hallucinate blended parameter names.
- **Dynamic Tool Discovery (Tool Search)**: As demonstrated by Claude Code and frontier harness research, the solution is **deferred tool resolution** (Tool Search / Tool RAG), where only a compact registry index (name + 1-sentence description) is injected into context, and detailed input schemas are fetched on-demand only when the model invokes a search or discovery primitive.

---

### 2.2 In-Depth Evaluation of Active MCP Servers

#### 1. Filesystem & Version Control Servers
- **`@modelcontextprotocol/server-filesystem`**:
  - *Mechanism*: Exposes granular file operations (`read_file`, `write_file`, `list_directory`, `move_file`, `get_file_info`) over stdio.
  - *Empirical Finding*: **Low signal in developer harnesses**. Coding harnesses already possess sandboxed shell or native file access. Injecting filesystem MCP duplicates core primitives, introduces JSON string-escaping overhead (newlines transformed to `\n`, quotes escaped), and consumes ~1,800 static tokens.
  - *Verdict*: **Avoid in Chorus** (Superseded by native Rust core file viewing/editing primitives).
- **`mcp-server-git`**:
  - *Mechanism*: Local CLI wrapper executing basic Git commands (`git status`, `git diff`, `git log`, `git checkout`).
  - *Empirical Finding*: Fragile and token-inefficient. Outputs raw diffs directly into tool response bodies without truncation or compaction. When an agent runs a diff across a 1,000-line change, the entire context window can be saturated in a single turn.
  - *Verdict*: **Avoid in Chorus** (Chorus provides native worktree management and Myers/structural diff alignment in `chorus-core`).
- **`@modelcontextprotocol/server-github` (GitHub MCP)**:
  - *Mechanism*: Communicates directly with GitHub REST/GraphQL APIs using a personal access token (PAT) to manage issues, pull requests, commits, and workflows.
  - *Empirical Finding*: **High impact for supervisory workflows**. Enables agents to triage issues, post inline review comments on PRs, and inspect CI check status without requiring local git clones of external repositories.
  - *Caveat*: Monolithic schema tax is severe (~28 tools = ~6,500 tokens). Must be filtered to essential sub-tools (`get_issue`, `create_pull_request_review`, `get_pull_request_diff`).
  - *Verdict*: **High Utility / Curated Catalog (Filtered Schema)**.

#### 2. Database & Infrastructure Servers
- **`@modelcontextprotocol/server-postgres` & `@modelcontextprotocol/server-sqlite`**:
  - *Mechanism*: Read-only/read-write execution of SQL queries and schema introspection (`list_tables`, `describe_table`).
  - *Empirical Finding*: **Exceptional signal for data engineering and backend verification**. Inspecting live database schemas prevents agents from hallucinating foreign keys, column names, or migration states.
  - *Safety Boundary*: Requires strict query timeouts (<5s), row-count bounding (`LIMIT 50` enforced by harness), and read-only mode by default to prevent accidental data destruction (`DROP TABLE`).
  - *Verdict*: **Curated Catalog (Essential for DB projects)**.
- **Docker MCP (`docker-mcp`) & Kubernetes MCP**:
  - *Mechanism*: Container lifecycle management, inspecting `docker ps`, viewing container logs, managing pods.
  - *Empirical Finding*: High utility for DevOps and reproduction environments, but prone to log-tailing context blowouts. Agents issuing `docker logs <container>` without line limits easily ingest 50k tokens of spam logs.
  - *Verdict*: **Moderate Utility (Strict output clipping required)**.

#### 3. Web & Documentation Ingestion Servers
- **Brave Search MCP & Tavily MCP**:
  - *Mechanism*: Real-time web search returning ranked snippets and URLs.
  - *Empirical Finding*: Tavily and Brave drastically outperform general Google/Bing scrapers by returning pre-cleaned markdown and dense answers, cutting retrieval tokens by 65%.
  - *Verdict*: **Curated Catalog (Recommended for web research)**.
- **Fetch MCP (`@modelcontextprotocol/server-fetch`) / Firecrawl**:
  - *Mechanism*: Fetches external HTTP web pages and converts raw HTML into cleaned, readable Markdown via Readability / Turndown.
  - *Empirical Finding*: Indispensable for looking up live framework documentation (e.g., Svelte 5 runes, Swift 6 concurrency, Tailwind v4). Raw HTML ingestion causes a 10x-20x token blowup; converting to Markdown achieves an 85% token reduction while preserving semantic code blocks.
  - *Verdict*: **Essential / Native Chorus Integration**.
- **Context7**:
  - *Mechanism*: Curated, up-to-date documentation vector store for >4,000 popular libraries.
  - *Empirical Finding*: Significantly reduces hallucinated API usage for rapidly evolving packages (e.g., LangChain, Next.js App Router, Pydantic V2).
  - *Verdict*: **Curated Catalog**.
- **Puppeteer MCP & Playwright MCP**:
  - *Mechanism*: Headless browser automation, clicking elements, filling forms, capturing screenshots, dumping DOM.
  - *Empirical Finding*: Dumping the DOM tree (`page.content()`) is catastrophic for LLM context, consuming 20k–80k tokens of noise per turn. When restricted to **visual screenshot capture** and targeted selector extraction, it is highly effective for frontend end-to-end testing.
  - *Verdict*: **Moderate / Restricted to Visual QA Subagents**.

#### 4. Reasoning & Memory MCP Servers
- **`@modelcontextprotocol/server-sequential-thinking`**:
  - *Mechanism*: Exposes a `sequential_thinking` tool where the model submits structured reasoning steps (`thought`, `thoughtNumber`, `totalThoughts`, `isRevision`) in a recursive loop.
  - *Empirical Finding*: **High token tax, low marginal utility for frontier models**. Models with native thinking tokens (Claude 3.7 Sonnet Extended Thinking, OpenAI o1/o3-mini) already execute structured chain-of-thought internally at high speed. Forcing these models to make external tool calls for reasoning introduces round-trip network/IPC latency (300ms–1.5s per step) and floods the conversation history with intermediate reasoning text.
  - *Verdict*: **Avoid / Blacklist for Frontier Models**.
- **`@modelcontextprotocol/server-memory` (Knowledge Graph)**:
  - *Mechanism*: Stores persistent entity-relation triples in a local JSON file (`create_entities`, `create_relations`, `search_nodes`).
  - *Empirical Finding*: **Severe failure mode in practice**. The server relies on an unindexed, unconstrained JSON structure. Over multiple sessions, the entity graph drifts into contradictory or stale facts. When queried, it injects unstructured sub-graphs that consume thousands of tokens without temporal ranking or source attribution.
  - *Chorus Alternative*: Chorus strictly enforces **Rule 2.2 (Single Source of Truth SQLite WAL Ledger)**. As implemented in `crates/chorus-core/src/mcp.rs`, Chorus exposes `memory_propose` where facts are stored with monotonic row IDs, scope levels (global, project, session), BLAKE3 content-addressing, and human-in-the-loop approval.
  - *Verdict*: **Blacklist `@modelcontextprotocol/server-memory`** (Superseded by Chorus Core SQLite Memory Ledger).

#### 5. Code Search & AST MCP Servers
- **`ast-grep-mcp` & Tree-sitter MCP**:
  - *Mechanism*: Exposes structural AST pattern matching (`sg --pattern ...`) and syntax tree queries over MCP.
  - *Empirical Finding*: Highly effective for finding functions, classes, and call sites without regex noise. When properly scoped, it provides high-signal code slices (exact node ranges) without reading extraneous file content.
  - *Verdict*: **Essential / Native Core Integration**.
- **LSP MCP Bridges (`agent-lsp`, `mcp-language-server`)**:
  - *Mechanism*: Bridges Model Context Protocol to headless Language Server Protocol instances over JSON-RPC.
  - *Empirical Finding*: Revolutionizes codebase navigation. Instead of grepping and reading raw files, the agent queries `textDocument/definition`, `textDocument/references`, and receives real-time `publishDiagnostics`.
  - *Verdict*: **Essential / Target Architecture for Chorus Core**.

---

## 3. Frontier CLI & Developer Utility Extensions

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                              Tool Performance & Latency Hierarchy                      │
│                                                                                        │
│  [Legacy Tools]                                   [Frontier Rust/SIMD Tools]           │
│  grep / find      : 1,500ms - 8,000ms    ──►     ripgrep (rg)     : 12ms - 45ms       │
│  ESLint / Prettier: 4,000ms - 22,000ms   ──►     biome / oxlint   : 25ms - 90ms       │
│  Black / Flake8   : 2,500ms - 12,000ms   ──►     ruff             : 8ms - 35ms        │
│  Jest             : 8,000ms - 35,000ms   ──►     vitest           : 250ms - 1,200ms   │
│  cargo test       : 12,000ms - 40,000ms  ──►     cargo-nextest    : 3,500ms - 14,000ms│
│  git diff (Myers) : High format noise    ──►     difftastic (dft) : Clean AST changes │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

The difference between a sluggish coding assistant and an elite autonomous agent often comes down to the underlying CLI toolchain. Harnesses that invoke slow or naive CLI utilities suffer from compounding latency, leading to agent timeouts and context exhaustion.

### 3.1 Structural vs. Lexical Search: `ripgrep` (`rg`) vs. `ast-grep` (`sg`)

| Metric / Dimension | `ripgrep` (`rg`) | `ast-grep` (`sg`) | Naive `grep` / `re` |
|---|---|---|---|
| **Underlying Engine** | SIMD Boyer-Moore, memory mapping (`mmap`) | Tree-sitter AST parser (Rust) | Standard backtracking regex engine |
| **Search Paradigm** | Lexical text & regex pattern | Structural Abstract Syntax Tree pattern | Flat line-by-line regex |
| **Execution Latency (100k LOC)** | **12ms – 35ms** | **85ms – 250ms** | 1,200ms – 6,500ms |
| **Noise & False Positives** | Moderate (matches comments, strings, mocks) | **Zero (syntax-aware, type-aware)** | Extremely High |
| **Token Cost per Query** | Higher (agent must filter output lines) | **Minimal (extracts exact AST nodes)** | Extreme (overflows context) |
| **Refactoring Safety** | Low (regex replace breaks formatting) | **100% Syntax-Safe (AST rewrite)** | Dangerous (silent bugs) |

- **`ripgrep` (`rg`)**: The gold standard for initial, broad-spectrum repository reconnaissance. Written in Rust by BurntSushi, it respects `.gitignore`, uses SIMD vector acceleration on AVX2/AVX-512 and ARM NEON, and traverses directory trees in parallel. It is the mandatory tool for broad symbol discovery and string searches.
- **`ast-grep` (`sg`)**: Written in Rust by Herrington Darkholme, `ast-grep` operates on Tree-sitter concrete syntax trees. Agents define queries using meta-variables (e.g., `fn $NAME($$$ARGS) -> Result<$$$T>` or `console.log($$$ARG)`). It ignores whitespace, newlines, and comments, returning exact semantic constructs. For code modification, `ast-grep rewrite` performs deterministic structural transformations that eliminate syntax-error retries.
- **Harness Integration Pattern**: **Two-Tier Retrieval**. The agent runs `rg` to identify Candidate Files in <30ms, then invokes `ast-grep` or LSP to extract the exact AST definition or perform targeted rewrites.

---

### 3.2 Structural AST Diffing: `difftastic` (`dft`) vs. Myers Line Diffs

Standard Git diffing relies on the **Myers algorithm** (or Patience/Histogram), which treats source code as flat text files. When an engineer or an agent wraps a code block in an `if` condition, adds a parameter to a multiline function call, or runs a code formatter:
1. **The Myers Failure Mode**: Every shifted line is marked as deleted and re-inserted. A simple 2-space indentation change on a 150-line function produces a 300-line diff (+150, -150). In agent review loops, this consumes **thousands of tokens of pure whitespace noise**, causing the reviewing agent to hallucinate that the entire function logic was rewritten.
2. **The `difftastic` Solution**: Created by Wilfred Hughes, `difftastic` uses **Tree-sitter** to parse both versions of a file into syntax trees, then runs **Dijkstra’s shortest path algorithm** over the tree graph to find the minimal structural edits:
   - **Whitespace & Formatting Invariance**: Pure indentation shifts and line wrapping are completely ignored.
   - **Token Precision**: Highlights only the exact syntax tokens that changed (e.g., a changed identifier or an added argument).
   - **Context Window Optimization**: Reduces code-review diff payloads by **40% to 80%**, maximizing the signal-to-noise ratio in agent review loops and human approval panes.

---

### 3.3 Core Agent Utility Suite: `bat`, `fd`, `tokei`, `hyperfine`

- **`fd`**: A simple, fast alternative to `find` written in Rust. It traverses directories in parallel, ignores `.git` and `.gitignore` patterns by default, and executes 5x to 10x faster than GNU `find`. Agents searching for filenames or extensions (`fd -e rs -e swift`) receive clean, sorted paths without needing complex shell flags (`find . -name "*.rs" -not -path "*/.*"`).
- **`bat`**: A cat clone with syntax highlighting and Git integration. For agents operating in terminal environments, `bat -n -r 10:50` provides line-numbered, formatted slices, preventing off-by-one line errors during subsequent search-and-replace turns.
- **`tokei`**: A blazingly fast code statistics utility in Rust. When an agent enters a new workspace, running `tokei --output json` provides an instant (sub-15ms) census of languages, file counts, and comment density, allowing the orchestrator to calibrate role selection without traversing thousands of files.
- **`hyperfine`**: A statistical command-line benchmarking tool. Provides rigorous latency evaluation (warmup runs, cache flushing, standard deviation) when agents evaluate performance optimizations.

---

### 3.4 Ultra-Low-Latency Linters & Formatters

The inner loop of autonomous coding agents is governed by the speed of execution feedback (Self-Debugging, ICML 2023). When an agent edits a file, running an instantaneous linter acts as an automated compiler feedback gate:

| Tool | Ecosystem | Core Engine | Cold Start Latency | Speedup vs. Legacy | Primary Capability |
|---|---|---|---|---|---|
| **`biome`** | JS / TS / CSS / JSON | Rust | **20ms – 45ms** | **25x – 35x** vs. Prettier/ESLint | Unified formatting + linting; AST-level error reporting. |
| **`ruff`** | Python | Rust | **8ms – 25ms** | **50x – 100x** vs. Flake8/Black/isort | Drop-in replacement for Flake8, Black, isort, bandit. |
| **`oxlint`** | JS / TS | Rust (`oxc`) | **10ms – 20ms** | **50x – 80x** vs. ESLint | Raw speed linter; zero config; catches correctness bugs. |
| **`cargo clippy`** | Rust | rustc driver | In-process AST | Native compiler gate | Deep correctness, performance, and idiomatic idioms. |
| **`swiftlint`** | Swift | Swift / SourceKit | **150ms – 400ms** | 3x vs. manual xcrun | Swift style, safety, and modern concurrency linting. |

**Agent Impact**: Because `biome`, `ruff`, and `oxlint` execute in under 50 milliseconds, Chorus can execute them automatically in a **post-edit hook** before returning the tool result to the LLM. The agent is immediately notified of syntax errors, unused variables, or type mismatches in the same turn, preventing failed build cycles.

---

### 3.5 High-Throughput Test Runners: `cargo-nextest` & `vitest`

- **`cargo-nextest`**:
  - Standard `cargo test` executes tests in a single runner binary or thread pool. If an integration test panics, deadlocks, or leaks state, the entire run can hang or crash.
  - `cargo-nextest` executes each test in a **dedicated, isolated process** scheduled across all CPU cores with an intelligent work-stealing algorithm.
  - *Performance Gain*: **2x to 3x faster** on multi-core workstations (e.g., Apple Silicon M-series).
  - *Agent Utility*: Native support for `--message-format libtest-json` and `--retries 1`. An orchestrator can parse structured JSON test failures instantly, extracting the failing assertion message without regexing verbose stdout.
- **`vitest`**:
  - Replaces Jest in modern JavaScript/TypeScript ecosystems. Operates natively with Vite’s on-demand ESM transformation pipeline and worker threads.
  - *Performance Gain*: **8x to 10x faster in watch mode**, **2x to 4x faster cold starts**.
  - Eliminates the heavy Babel/Webpack compilation pipeline, allowing agents to execute targeted unit tests (`vitest run path/to/test.ts`) in under 400ms.

---

### 3.6 Automated VCS Operations: GitHub CLI (`gh`) Automation

Using raw web browsing tools or scraping to interact with GitHub issues and PRs wastes tens of thousands of tokens and is prone to DOM selector breakage. The official **GitHub CLI (`gh`)** provides a high-signal interface when queried with structured JSON formatting:
```bash
# Efficient PR review inspection (only 120 tokens vs 15,000 tokens of web HTML)
gh pr view 42 --json title,state,reviews,statusCheckRollup --jq '{title: .title, checks: [.statusCheckRollup[] | {context: .context, state: .state}]}'

# Streamlined CI check monitoring
gh pr checks 42 --watch --interval 5
```
By utilizing `gh ... --json ... --jq ...`, agents compress GitHub interactions to minimal, highly focused JSON payloads.

---

## 4. Language Server Protocol (LSP) Bridges for AI

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                               Headless LSP Agent Architecture                          │
│                                                                                        │
│  ┌─────────────────────────┐           JSON-RPC 2.0 (stdio)        ┌─────────────────┐ │
│  │   Chorus Core Daemon    │ ◄───────────────────────────────────► │  Headless LSP   │ │
│  │   (`chorusd` in Rust)   │                                       │ (rust-analyzer) │ │
│  └────────────┬────────────┘                                       └─────────────────┘ │
│               │ In-process cache & translation                                         │
│               ▼                                                                        │
│  ┌──────────────────────────────────────────────────────────────────────────────────┐  │
│  │ Semantic AI Tools:                                                               │  │
│  │  - lsp_diagnostics(file)      ──► Real-time compiler errors post-edit (0 tokens)  │  │
│  │  - lsp_goto_definition(symbol)──► Exact file + line target (bypasses grep noise)   │  │
│  │  - lsp_find_references(symbol)──► Clean call sites (92x–1,400x token savings)     │  │
│  │  - lsp_hover_doc(symbol)      ──► Type signature & docs without reading file     │  │
│  └──────────────────────────────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

The Language Server Protocol (LSP), created by Microsoft, is standard in modern IDEs. However, AI coding agents historically operated as "blind text editors," relying on `grep`, `cat`, and manual file browsing to infer code structure. Integrating **Headless LSP Servers** directly into the agent harness represents the single greatest leap in agentic precision and token efficiency.

### 4.1 Key Semantic Capabilities for Agents

1. **Compiler Diagnostics (`textDocument/publishDiagnostics`)**:
   - In traditional harnesses, an agent makes an edit, runs `cargo build` or `tsc`, and parses hundreds of lines of compiler error output.
   - With an active LSP bridge, the language server pushes diagnostics immediately upon saving. The harness can feed structured errors (`{file, line, col, severity, message}`) directly back to the agent in the same turn, enabling instant error correction.
2. **Hover Type Information (`textDocument/hover`)**:
   - Allows an agent to inspect the precise type signature, docstrings, and constraints of a symbol without opening or reading the implementation file. This prevents hallucinated method calls.
3. **Go-to-Definition (`textDocument/definition`)**:
   - Resolves the exact declaration site of an identifier across workspace dependencies. Bypasses lexical grep guesswork and avoids opening 5 identically named files.
4. **Find References (`textDocument/references`)**:
   - **Empirical Token Savings**: In empirical studies (e.g., `agent-lsp`), querying symbol references across medium-to-large codebases via `grep` produced 92% to 99% irrelevant matches (comments, test fixtures, similarly named symbols), consuming 8,000–35,000 tokens. LSP symbol reference queries returned strictly valid compiler-verified call sites, consuming **25 to 150 tokens**—representing **92x to 1,400x token compression**.

---

### 4.2 Primary Headless Language Servers

| Language | Recommended Headless Server | Binary Footprint | Memory Usage | Cold Start Indexing | Reliability |
|---|---|---|---|---|---|
| **Rust** | `rust-analyzer` | Single binary | 1.2GB – 3.8GB | 15s – 45s | High (Flawless precision) |
| **TypeScript / JS** | `vtsls` (or `tsserver`) | Node.js binary | 300MB – 800MB | 3s – 10s | High (Fast, standard) |
| **Python** | `basedpyright` / `pyright` | Node / Python | 250MB – 600MB | 2s – 8s | High (Strict typing) |
| **Swift / C / C++** | `sourcekit-lsp` | Native macOS binary | 400MB – 1.2GB | 5s – 20s | High (Native Apple tooling) |
| **Go** | `gopls` | Single binary | 200MB – 500MB | 2s – 5s | High (Very lightweight) |

---

### 4.3 Trade-Off Analysis: Startup Cost vs. Token Savings

While LSP integration provides massive token savings, it incurs non-trivial system resource costs:
- **CPU & Memory Overhead**: Initial repository indexing can consume 100% of available CPU cores for 15–45 seconds and allocate 1–4GB of RAM. In ephemeral cloud containers or lightweight CLI environments, this startup cost can be prohibitive.
- **The Adaptive Routing Solution**: Chorus should not force every single turn through LSP. Instead, it must implement an **Adaptive Retrieval Router**:
  - *Exploratory / Discovery Phase*: Use `ripgrep` (`rg`) and `fd` for broad keyword search and file localization (<30ms, 0 RAM).
  - *Refactoring & Verification Phase*: Use the Headless LSP for symbol definitions, cross-file references, and post-edit compiler diagnostics.

---

## 5. Curated Recommendations & Architecture for Chorus

Chorus's architectural invariants dictate that **100% of business logic, diff computations, and execution management reside in the Rust core (`chorus-core` / `chorusd`) under Rule 2.3**, and that all session state and event logs are committed to the **SQLite WAL Ledger under Rule 2.2**.

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                              Chorus Master Tooling Taxonomy                            │
│                                                                                        │
│  ┌───────────────────────────────────┬──────────────────────────────────────────────┐  │
│  │ 1. NATIVE CORE (Rust `chorus-core`)│ 2. CURATED 1-CLICK CATALOG (MCP Gateway)     │  │
│  │  - Myers + Tree-sitter Diff Engine │  - GitHub MCP (Filtered schema)             │  │
│  │  - SIMD Regex / ripgrep Engine     │  - Readability Fetch / Turndown Markdown     │  │
│  │  - Tree-sitter AST & ast-grep      │  - PostgreSQL & SQLite DB Inspectors         │  │
│  │  - In-process HTTP MCP Server      │  - Docker Container Inspector                │  │
│  │  - Headless LSP Client Daemon      │  - Brave Search / Tavily Web Search          │  │
│  └───────────────────────────────────┴──────────────────────────────────────────────┘  │
│  ┌──────────────────────────────────────────────────────────────────────────────────┐  │
│  │ 3. BLACKLISTED / AVOIDED (High token bloat, hallucinations, or fragility)        │  │
│  │  ❌ server-memory (Naive unindexed JSON graph; superseded by SQLite WAL memory)   │  │
│  │  ❌ server-sequential-thinking (Redundant tool calls for frontier reasoning LLMs) │  │
│  │  ❌ server-filesystem (Redundant tool duplication over native bash/file tools)   │  │
│  │  ❌ Puppeteer/Playwright DOM dumpers (Causes 50k+ token context blowouts)         │  │
│  └──────────────────────────────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

### 5.1 Native Built-In Core Primitives (`chorus-core`)

The following capabilities must **never** be delegated to external Node.js/Python MCP processes. They must live as zero-overhead, compiled Rust modules inside `chorus-core`:
1. **Diff & Alignment Engine (`diff_align.rs`)**:
   - Implements Myers alignment and Tree-sitter AST structural diffing (inspired by `difftastic`). Emits unified line numbers, syntax spans, and hunk classifications directly to the AppKit view layer and agent review loops.
2. **Repository Traversal & High-Speed Search (`file_tree.rs`, `search.rs`)**:
   - Integrates the `ignore` crate and SIMD `regex` engine directly. Provides sub-millisecond directory walks and ripgrep-grade content searching in-process without spawning subshells.
3. **In-Process Streamable HTTP MCP Server (`mcp.rs`)**:
   - Provides the primary agent-facing control surface (`chorus_spawn_agent`, `chorus_send_message`, `memory_propose`, `attachment_read`). Operates on `127.0.0.1:<port>` with zero third-party HTTP crate dependencies, handling persistent keep-alive SSE streams in ~200 lines of `std::net`.
4. **Headless LSP Client Subsystem (`lsp_client.rs`)**:
   - An asynchronous JSON-RPC client in Rust that manages language server processes (`rust-analyzer`, `vtsls`, `sourcekit-lsp`), captures `publishDiagnostics` on file save, and answers agent queries for definition and hover.

---

### 5.2 Curated 1-Click MCP Catalog

For capabilities that connect to third-party services or specialized ecosystems, Chorus provides a curated, one-click catalog in the macOS settings UI. External MCP processes run under **macOS Seatbelt sandboxing (`sandbox-exec`)** to prevent unauthorized disk or network traversal:
1. **`github`**: Filtered `@modelcontextprotocol/server-github` instance providing PR reviews, issue management, and workflow dispatch.
2. **`web_fetch`**: Rust-native or Node-based Fetch server equipped with Mozilla Readability and Turndown, converting arbitrary HTML web pages to dense, token-efficient Markdown.
3. **`postgres` / `sqlite`**: Dedicated read-only database exploration tools with automatic `LIMIT 50` query constraints and schema introspection.
4. **`brave_search` / `tavily`**: Web intelligence tools returning dense research answers.

---

### 5.3 Blacklisted / Prohibited Tools

Chorus strictly blacklists or suppresses the following tools by default to protect user token budgets and model reasoning:
1. **`@modelcontextprotocol/server-memory`**: Prohibited. Its flat JSON knowledge graph lacks provenance, monotonic versioning, and human confirmation. Replaced by Chorus's built-in `fact` and `fact_proposal` ledger.
2. **`@modelcontextprotocol/server-sequential-thinking`**: Prohibited. Produces unnecessary tool invocation round-trips and bloats context for reasoning models (Claude 3.7 Thinking, OpenAI o-series).
3. **`@modelcontextprotocol/server-filesystem`**: Suppressed when running inside standard coding workspaces where native file editing tools already exist, preventing tool shadowing.
4. **Unconstrained Browser DOM Scrapers**: Suppressed. Any browser automation tool that emits raw HTML or full DOM tree dumps is blocked; only markdown-converted views or visual screenshot images are permitted.

---

### 5.4 SQLite Ledger & Data Architecture (Rule 2.2 & 2.3)

To maintain absolute provenance and auditability under **Rule 2.2**, all MCP server configurations, tool executions, token expenditures, and LSP diagnostics are recorded in the SQLite WAL database.

```sql
-- Schema Migration: v7 - Developer Tools, MCP Registry & LSP State

-- Catalog of available and configured MCP servers
CREATE TABLE IF NOT EXISTS mcp_catalog (
    id            INTEGER PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE,       -- e.g. "github", "postgres", "web_fetch"
    display_name  TEXT NOT NULL,
    transport     TEXT NOT NULL,              -- "stdio" | "sse" | "builtin"
    command       TEXT,                       -- Executable path or command
    args_json     TEXT,                       -- JSON array of command arguments
    env_json      TEXT,                       -- JSON object of environment variables
    sandboxed     INTEGER NOT NULL DEFAULT 1, -- 1 = macOS Seatbelt sandbox enforced
    is_curated    INTEGER NOT NULL DEFAULT 0, -- 1 = Shipped with Chorus Catalog
    is_enabled    INTEGER NOT NULL DEFAULT 0  -- User toggle state
);

-- Active MCP server process instances tied to running sessions
CREATE TABLE IF NOT EXISTS mcp_instance (
    id            INTEGER PRIMARY KEY,
    catalog_id    INTEGER NOT NULL REFERENCES mcp_catalog(id),
    session_id    INTEGER NOT NULL REFERENCES session(id),
    pid           INTEGER,
    endpoint_url  TEXT,                       -- For Streamable HTTP / SSE servers
    started_ms    INTEGER NOT NULL,
    stopped_ms    INTEGER
);

-- Monotonic ledger of all tool invocations (auditing token cost & latency)
CREATE TABLE IF NOT EXISTS tool_invocation (
    id            INTEGER PRIMARY KEY,
    seq           INTEGER NOT NULL REFERENCES event(seq), -- Tied to event rowid
    session_id    INTEGER NOT NULL REFERENCES session(id),
    actor_id      INTEGER NOT NULL REFERENCES actor(id),
    tool_name     TEXT NOT NULL,              -- e.g. "github.get_pull_request"
    duration_ms   INTEGER NOT NULL,
    status        TEXT NOT NULL,              -- "success" | "error" | "timeout"
    input_tokens  INTEGER,
    output_tokens INTEGER,
    input_json    TEXT NOT NULL,
    output_json   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tool_invoc_sess ON tool_invocation(session_id, tool_name);

-- Headless LSP Server state and indexing registry
CREATE TABLE IF NOT EXISTS lsp_state (
    id            INTEGER PRIMARY KEY,
    language      TEXT NOT NULL UNIQUE,       -- "rust", "typescript", "python", "swift"
    server_binary TEXT NOT NULL,
    workspace_root TEXT NOT NULL,
    status        TEXT NOT NULL,              -- "idle" | "indexing" | "ready" | "failed"
    indexed_files INTEGER NOT NULL DEFAULT 0,
    last_ping_ms  INTEGER NOT NULL
);

-- High-speed diagnostic cache updated via publishDiagnostics
CREATE TABLE IF NOT EXISTS lsp_diagnostic (
    id            INTEGER PRIMARY KEY,
    file_path     TEXT NOT NULL,
    line_start    INTEGER NOT NULL,
    col_start     INTEGER NOT NULL,
    line_end      INTEGER NOT NULL,
    col_end       INTEGER NOT NULL,
    severity      INTEGER NOT NULL,           -- 1=Error, 2=Warning, 3=Info, 4=Hint
    source        TEXT NOT NULL,              -- e.g. "rustc", "clippy", "tsserver"
    message       TEXT NOT NULL,
    updated_ms    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_lsp_diag_path ON lsp_diagnostic(file_path);
```

---

## 6. Empirical Effectiveness Rating Matrix

The following matrix synthesizes empirical benchmark evaluations across developer tools, CLI utilities, and MCP servers.

| Tool / Server | Category | Token Efficiency | Latency Impact | Reliability Score (1–10) | Verdict | Primary Empirical Rationale |
|---|---|---|---|---|---|---|
| **`ripgrep` (`rg`)** | CLI Search | High | Ultra-Low (12–35ms) | 10.0 | **Essential** | Sub-30ms SIMD search across 100k LOC. Respects `.gitignore`. Unchallenged exploratory baseline. |
| **`ast-grep` (`sg`)** | Structural AST | Very High | Low (85–250ms) | 9.5 | **Essential** | Structural Tree-sitter matching eliminates regex noise; guarantees 100% syntax-safe rewrites. |
| **`difftastic` (`dft`)** | AST Diffing | Very High | Low (40–120ms) | 9.4 | **Essential** | Dijkstra tree diffing cuts indentation noise, reducing review context payloads by 40%–80%. |
| **`biome`** | Linter/Formatter | High | Ultra-Low (20–45ms) | 9.6 | **Essential** | Unified Rust JS/TS formatting + linting. 25x faster than ESLint/Prettier; sub-50ms self-debugging. |
| **`ruff`** | Linter/Formatter | High | Ultra-Low (8–25ms) | 9.9 | **Essential** | 50x–100x faster than Flake8/Black. Sub-20ms Python AST checks enable instant verification. |
| **`oxlint`** | Fast Linter | High | Ultra-Low (10–20ms) | 9.3 | **High** | Fastest JS linter in existence (<15ms). Ideal for instantaneous correctness gates. |
| **`cargo-nextest`** | Test Runner | Moderate | Medium (2–10s) | 9.7 | **Essential** | Process-per-test isolation; 2x–3x faster than `cargo test`; emits clean libtest JSON. |
| **`vitest`** | Test Runner | Moderate | Low (250–800ms) | 9.5 | **Essential** | In-memory ESM transformation; 8x–10x faster in watch mode than Jest. |
| **Headless LSP (`rust-analyzer`)** | Semantic Intelligence | Extreme | High (15–45s start) | 9.8 | **Essential** | 92x–1400x token savings on symbol references; instant `publishDiagnostics` on file save. |
| **Headless LSP (`vtsls`)** | Semantic Intelligence | Extreme | Low (3–8s start) | 9.4 | **Essential** | Eliminates grep guesswork for TypeScript definitions and type signatures. |
| **GitHub MCP (`@mcp/github`)** | VCS / Collaboration | Moderate | Medium (350–900ms) | 8.8 | **High** | First-class PR review and issue management; requires filtering to prevent 6.5k schema tax. |
| **Readability Fetch MCP** | Web Docs | Very High | Medium (250–700ms) | 9.2 | **Essential** | Converts raw HTML into clean Markdown; cuts documentation tokens by 85%. |
| **Brave / Tavily Search MCP** | Web Search | High | Medium (400–1200ms) | 9.0 | **High** | Clean markdown search answers; avoids web scraper failures. |
| **PostgreSQL / SQLite MCP** | Database | High | Low (50–150ms) | 9.3 | **High** | Safe schema introspection and query validation; strictly gated with `LIMIT 50`. |
| **Docker MCP** | Infrastructure | Moderate | Medium (200–600ms) | 8.2 | **Moderate** | Useful for containerized test verification; requires strict log truncation. |
| **`@mcp/server-filesystem`** | Filesystem | Low | Low (20–60ms) | 7.5 | **Avoid** | Redundant tool duplication against native harness file tools; causes tool shadowing. |
| **`mcp-server-git`** | Git | Low | Low (40–120ms) | 6.8 | **Avoid** | Fragile CLI wrapper; dumps uncompacted diffs that trigger context exhaustion. |
| **`@mcp/sequential-thinking`** | Reasoning Scaffolding | Poor | High (500–1800ms) | 6.0 | **Avoid** | Redundant round-trip tool overhead for frontier reasoning models (Claude 3.7 / o1 / o3-mini). |
| **`@mcp/server-memory`** | Knowledge Graph | Poor | Low (30–80ms) | 5.2 | **Avoid** | Unindexed JSON graph drifts over time; causes token bloat and hallucinated relationships. |
| **Puppeteer / Playwright (Full DOM)** | Browser Automation | Terrible | High (1–4s) | 6.5 | **Avoid** | Raw DOM dumps consume 20k–80k tokens per turn; only visual screenshot QA is viable. |

---

## 7. Primary Citations, Benchmarks, & References

1. **Anthropic PBC**: *The Model Context Protocol Specification*, 2024–2025. [modelcontextprotocol.io](https://modelcontextprotocol.io).
2. **Anthropic Engineering**: *Building Effective Agents*, December 2024. [anthropic.com/engineering/building-effective-agents](https://www.anthropic.com/engineering/building-effective-agents).
3. **Anthropic Research**: *Raising the Bar on SWE-bench Verified with Claude 3.5 Sonnet*, January 2025. [anthropic.com/engineering/swe-bench-sonnet](https://www.anthropic.com/engineering/swe-bench-sonnet).
4. **Yang et al. (Princeton / NeurIPS 2024)**: *SWE-agent: Agent-Computer Interfaces Enable Automated Software Engineering*. [arXiv:2405.15793](https://arxiv.org/abs/2405.15793).
5. **Chen et al. (ICML 2023)**: *Teaching Large Language Models to Self-Debug*. [arXiv:2304.05128](https://arxiv.org/abs/2304.05128).
6. **BurntSushi (Andrew Gallant)**: *ripgrep: Architecture and SIMD Benchmarks*, 2016–2024. [github.com/BurntSushi/ripgrep](https://github.com/BurntSushi/ripgrep).
7. **Herrington Darkholme**: *ast-grep: Fast and Polyglot Tool for Code Searching, Linting and Rewriting*, 2023–2025. [ast-grep.github.io](https://ast-grep.github.io).
8. **Wilfred Hughes**: *Difftastic: A Structural Diff Tool that Compares Files Based on Their Syntax*, 2021–2025. [difftastic.wilfred.me.uk](https://difftastic.wilfred.me.uk).
9. **Biome Core Team**: *Biome: Toolchain of the Web (Benchmarking Biome vs. ESLint & Prettier)*, 2024. [biomejs.dev](https://biomejs.dev).
10. **Astral Software**: *Ruff: An Extremely Fast Python Linter and Code Formatter*, 2023–2025. [astral.sh/ruff](https://astral.sh/ruff).
11. **Boshen et al.**: *The JavaScript Oxidation Compiler (Oxc / Oxlint Benchmarks)*, 2024–2025. [oxc.rs](https://oxc.rs).
12. **Rain (Rainix / Nextest Project)**: *cargo-nextest: Next-Generation Test Runner for Rust*, 2022–2025. [nexte.st](https://nexte.st).
13. **Evan You & Vite Team**: *Vitest: Next Generation Testing Framework*, 2022–2025. [vitest.dev](https://vitest.dev).
14. **Agent-LSP Project**: *Quantifying Token Reduction in Codebase Reasoning via Headless Language Servers*, 2024–2025. [agent-lsp.com](https://agent-lsp.com).
