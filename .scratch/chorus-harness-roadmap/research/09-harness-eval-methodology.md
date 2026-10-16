# Harness Evaluation Methodology

## Executive Summary

Evaluating an AI coding agent harness is fundamentally different from evaluating a foundation model. As established by recent benchmark literature (Yang et al., 2024; Vats & Golev, 2026; Higgins et al., 2026), an autonomous agent is not an isolated large language model (LLM), but a composite **`(model, harness)` tuple**. The harness—comprising context assembly, state tracking, tool presentation, observation pruning, compaction, retry handling, and agent routing—acts as a decisive multiplier on agent capability and efficiency.

In controlled studies where model weights and seeds are held constant, changing the harness alone produces:
- Up to a **40× variation in token consumption and operational cost** per resolved task (Vats & Golev, arXiv:2607.22585).
- Up to a **10–20 percentage point swing in task completion rate** simply by restructuring tool feedback, file viewing, and edit guardrails (Yang et al., NeurIPS 2024).
- Persistent, harness-specific **"failure fingerprints"** (e.g., context overflow, repetitive tool retries, hallucinated path loops) that reproduce across completely different model providers.

For macOS Chorus, which operates with an immutable SQLite ledger as its single source of truth, multi-agent ACP bridges, and multi-tier context assembly, evaluating the harness requires a methodology that adheres to the **"real-path rule"** (`CLAUDE.md` §3): evaluating real `Session` engine execution rather than hand-assembled mocks.

This paper synthesizes primary-source methodologies from SWE-bench (Verified/Lite), Terminal-Bench, Aider, OpenHands, METR, and first-party agent evaluation literature. It then specifies a small, cheap, repeatable Chorus harness evaluation system capable of reliably judging feature changes and router quality gates.

---

## 1. Primary Benchmark & Harness Evaluation Survey

### 1.1 SWE-bench, SWE-bench Lite, and SWE-bench Verified
* **Primary Sources:**
  - Jimenez et al., *"SWE-bench: Can Language Models Resolve Real-World GitHub Issues?"*, ICLR 2024.
  - OpenAI & Princeton NLP, *"SWE-bench Verified"*, August 2024.
* **Task Selection:**
  - **Full (2,294 tasks):** Real GitHub issues paired with merged pull requests across 12 popular Python repositories (e.g., `django`, `sympy`, `pytest`, `matplotlib`).
  - **Lite (300 tasks):** A smaller subset selected for self-contained execution and faster runtime. However, audits revealed ~30% of tasks suffered from ambiguous issue descriptions, under-specified requirements, or leaky tests.
  - **Verified (500 tasks):** Human-annotated and filtered by professional software engineers to ensure problem descriptions are unambiguous, unit tests reliably reproduce the issue, and solutions do not require out-of-band information.
* **Environment & Sandboxing:**
  - One Docker container per repository commit/environment, bundling specific conda environments and system dependencies.
* **Scoring & Verification:**
  - Strict code-based programmatic grading: The agent's generated `git diff` patch is applied to a clean repository baseline. The test runner executes two test sets:
    1. `FAIL_TO_PASS`: Tests that failed before the fix and must pass after.
    2. `PASS_TO_PASS`: Existing unit tests that must remain green (regression prevention).
* **Limitations for Fast Harness Iteration:**
  - Prohibitively expensive and slow: A single 500-instance run costs \$100–\$400+ in API credits and requires hours to days of compute.
  - High Docker container setup overhead (10–60s per container spin-up).
  - Python-only, obscuring polyglot desktop harness performance.

### 1.2 Terminal-Bench, Harbor, and Terminus 2
* **Primary Sources:**
  - Terminal-Bench Consortium & Artificial Analysis, *"Terminal-Bench: A Benchmark for AI Agents in Computer Terminal Environments"*, 2025/2026.
  - Harbor Framework & Terminus 2 Agent Scaffold, ICLR / arXiv 2025.
* **Task Selection:**
  - Command-line oriented tasks spanning software engineering, system operations, security, machine learning, and data pipelines. Tasks require navigation, script compilation, environment debugging, and CLI tool chaining.
  - Semantic versioning (e.g., Terminal-Bench 4.0 with 66 tasks; Terminal-Bench Pro with 50-task stratified subsets).
* **Environment & Sandboxing:**
  - Sandboxed Linux containers managed by the **Harbor** framework.
* **Scoring & Metrics:**
  - Programmatic verification scripts executing inside the container upon task completion.
  - Metrics tracked: Resolution rate, wall-clock latency, token count, bash command executions, and time-to-first-command.
* **Role in Harness Research:**
  - **Terminus 2** was explicitly introduced as a "neutral agent scaffold" (minimal bash execution loop). By evaluating models on Terminus 2 versus advanced harnesses (like Goose or OpenHands), researchers could cleanly separate base model reasoning from scaffold-provided advantages.

### 1.3 Aider Benchmarks (Polyglot & Refactoring)
* **Primary Sources:**
  - Paul Gauthier, *"Aider LLM Leaderboards"*, *"Refactoring Benchmark"*, and *"Code Editing Benchmark"*, `aider.chat/docs/leaderboards/` (2023–2026).
* **Task Selection:**
  - **Polyglot Benchmark (225 tasks):** Sourced from Exercism across 6 languages (Python, JavaScript, TypeScript, Go, Rust, C++). Evaluates multi-language code editing against existing unit test suites.
  - **Refactoring Benchmark (89 tasks):** Large Python refactoring problems specifically designed to catch "lazy coding" (models omitting code, replacing functions with comments like `# ...rest of code...`, or botching large AST modifications).
* **Isolating Harness vs. Model:**
  - Aider is the prime example of using controlled benchmarking to optimize harness components holding the model constant:
    1. **Edit Formats:** Benchmarking `diff`, `whole`, `udiff`, and search/replace formats against the same LLM. Found that diff formats dramatically reduced token cost and latency on large files, but smaller models struggled with diff syntax compliance.
    2. **Context Sizing & Repo Map:** Evaluated tree-sitter tag graph repo maps against full-file stuffing. Showed that a compact ~1k-token repo map yielded equivalent solve rates to 30k-token full dumps.
    3. **Architect / Editor Split:** Decoupled reasoning from editing. An expensive reasoning model produces an implementation plan, and a cheap fast model generates the syntactic edit block. This configuration achieved frontier performance at a fraction of the cost.
* **Methodology & Sandboxing:**
  - **Zero Docker Dependency:** Operates inside native local Git repositories. Every task begins with a clean baseline commit.
  - **Two-Pass Reflection Loop:** Pass 1: prompt execution. If tests fail, Pass 2 provides compiler/test output back to the agent ("try to fix"). Tracks both Pass@1 and Pass@2.
  - Instant cleanup via `git reset --hard` or throwaway clones.
* **Metrics:**
  - Pass rate (first attempt and second attempt), edit format adherence (% clean diff apply without syntax errors), token usage, and dollar cost per completed exercise.

### 1.4 OpenHands Evaluation Suite (OpenHands Index & CodeAct)
* **Primary Sources:**
  - Wang et al., *"Executable Code Actions Elicit Better LLM Agents"* (CodeAct), ICML 2024 / arXiv:2402.01030.
  - OpenHands Team, *"OpenHands: An Open Platform for AI Software Developers as Generalist Agents"*, arXiv:2407.16741 (2024/2025).
  - OpenHands Index, `openhands.dev` (2025/2026).
* **Task Selection:**
  - Multi-dimensional evaluation across 5 pillars: Issue Resolution (SWE-bench Verified), Greenfield Development (Commit0), Frontend/UI (SWE-bench Multimodal), Test Writing (SWT-bench), and General Knowledge/Info (GAIA).
* **Isolating Harness Components:**
  - The CodeAct paper demonstrated that consolidating an agent's action space into executable Python/Bash code rather than fragmented JSON tool-calls reduced action steps by ~20% and improved task completion by over 12% on identical models.
  - Evaluated alternative agent loops: `CodeActAgent` vs. `PlannerAgent` vs. `BrowsingAgent`.
* **Metrics & Telemetry:**
  - Success rate, iteration count (turn depth), token efficiency, error recovery rate (how often an agent recovers after a tool command fails), and tool failure frequency.
* **Sandboxing:**
  - Docker-based runtime containers (AllHands micro-runtime) with bidirectional streaming APIs.

### 1.5 METR Task Standard & Autonomous Capabilities Suite
* **Primary Sources:**
  - Model Evaluation and Threat Research (METR / formerly ARC Evals), *"METR Task Standard Specification"* and *"Measuring Autonomous Capabilities of Language Models"*, arXiv:2412.21199 (2024–2026).
  - Platforms: Vivaria, Hawk, UK AISI Inspect AI.
* **Task Selection:**
  - Real-world, long-horizon tasks calibrated against human baseline times (tasks taking human experts 30 minutes to 8+ hours).
  - Domains: System administration, vulnerability exploitation, software refactoring, data analysis, open-source setup.
* **Standardization & Architecture:**
  - **METR Task Standard:** Standardizes agent test suites into 4 modular components:
    1. `environment`: Dockerfile / VM definition.
    2. `instructions`: Prompt provided to the agent.
    3. `score`: Python grading script with intermediate score milestones.
    4. `checkpoints`: Deterministic state snapshots.
* **Scaffolding Findings:**
  - METR's research explicitly treats the harness ("scaffold") as an independent capability variable. Comparing scaffolds (Modular agent, Triframe, Flock) demonstrated that context compression, scratchpad prompting, and subagent delegation significantly push out the horizon of solvable tasks on unchanged base weights.
* **Sandboxing:**
  - Full isolation via containers or microVMs, egress network allowlists (preventing data leakage or unauthorized web browsing), cgroup CPU/memory constraints, and strict execution timeouts.

### 1.6 First-Party Harness Literature

#### SWE-agent & The Agent-Computer Interface (ACI)
* **Primary Source:** Yang et al., *"SWE-agent: Agent-Computer Interfaces Enable Automated Software Engineering"*, NeurIPS 2024.
* **Core Contribution:**
  - Established the concept of the **Agent-Computer Interface (ACI)**. Just as human programmers require tailored IDEs, LLMs require interfaces tailored to their context and attention limitations.
* **Empirical Harness Ablation (SWE-bench Lite, GPT-4):**
  - **Full ACI:** 18.0% resolved.
  - **w/o specialized edit command (fallback to naive shell echo/sed):** Dropped to **10.3%** (-7.7%).
  - **w/o specialized file search (fallback to raw `find`/`grep`):** Dropped to **12.0%** (-6.0%).
  - **w/o syntax linter guardrails on edit:** Dropped to **15.0%** (-3.0%).
* **Takeaway:** Over 40% of the agent's net problem-solving power came directly from harness interface ergonomics, not from model capabilities.

#### The Scaffold Effect in Coding Agents
* **Primary Source:** Naman Vats and Oleg Golev, *"The Scaffold Effect in Coding Agents: Harness Choice as a Hidden Variable in Coding-Agent Evaluation"*, arXiv:2607.22585 (June 2026).
* **Key Findings:**
  - Directly compared 3 open-source harnesses (**Goose**, **OpenCode**, **OpenHands-SDK**) across 300 controlled trials on a 50-task subset of Terminal-Bench Pro using two frontier coding models (**Qwen 3.6 Plus** and **MiniMax M2.5**).
  - **The 40× Token Disparity:** While paired pass rates differed by only 0–8 percentage points across harnesses, token consumption per solved task varied by up to **40×**. Harnesses with undisciplined context stuffing and uncompressed tool dumps burned orders of magnitude more tokens to reach the same result.
  - **Failure Fingerprints:** Specific failure modes (e.g., getting trapped in tool loops, losing track of modified files, hallucinating tool signatures) were properties of the *harness*, remaining constant even when swapping the underlying model.
  - **Primary Recommendation:** Treat the **`(model, harness)` pair** as the foundational unit of evaluation, and always evaluate efficiency (tokens, turns, cost) alongside pass rate.

#### Anthropic: Agent Evaluation Principles
* **Primary Source:** Colin Higgins et al., *"Demystifying evals for AI agents"*, Anthropic Engineering, January 2026; and *"Building Effective AI Agents"*, 2024.
* **Standard Vocabulary:**
  - **Task:** Unit test case with defined initial state and success criteria.
  - **Trial:** A single stochastic execution of an agent on a task.
  - **Transcript / Trace:** The complete, structured event record (prompts, tool calls, tool responses, thoughts).
  - **Outcome:** The concrete final state of the environment.
* **Grading Hierarchy:**
  - *Code-based graders:* Fast, deterministic, objective, zero marginal cost (unit tests, exit codes, file diff checks).
  - *Model-based graders:* Nuanced, semantic, but non-deterministic and requiring calibration.
  - *Human graders:* High-fidelity calibration baseline, but slow and expensive.
* **Capability vs. Regression Evals:**
  - Capability evals measure frontier headroom (low pass rates, difficult tasks).
  - Regression evals verify that core features, context rules, and tool contracts remain unbroken (expecting near 100% pass rates).

---

## 2. Comparative Analysis of Evaluation Dimensions

| Dimension | SWE-bench (Verified) | Terminal-Bench / Harbor | Aider Benchmarks | OpenHands Index | METR Task Standard | Recommended Chorus Eval |
|---|---|---|---|---|---|---|
| **Target Evaluated** | Model + Agent script | Model + CLI scaffold | Model vs. Edit Formats | Full Agent System | Autonomous Scaffolding | **Chorus Desktop Harness & Router** |
| **Suite Size** | 500 tasks | 66 tasks (v4.0) | 225 Polyglot / 89 Refactor | 500+ across 5 suites | 50–100 long-horizon | **24 tasks (12 Thai, 12 English)** |
| **Execution Horizon** | 10–30 turns | 5–25 turns | 1–2 passes | 10–50 turns | 50–200+ turns | **2–8 turns** |
| **Environment / Isolation** | Heavy Docker containers | Linux Docker / Harbor | Native Git repo (`git reset`) | Docker micro-containers | MicroVMs / Docker + Netfilter | **macOS Git Worktrees / Throwaway dirs** |
| **Runtime per Full Run** | 4–12 hours | 1–2 hours | 15–30 minutes | 4–8 hours | 12–48 hours | **3–5 minutes** |
| **Cost per Full Run** | \$100 – \$400+ | \$20 – \$80 | \$2 – \$10 | \$50 – \$200 | \$200 – \$1,000+ | **\$0.30 – \$1.20 (or \$0 with local models)** |
| **Grader Type** | Unit tests (`FAIL_TO_PASS`) | Programmatic test scripts | Unit tests (pytest, cargo, etc.) | Unit tests & web assertions | Checkpoint test scripts | **Unit tests + Ledger invariant checks** |
| **Harness Telemetry** | Final patch only | Token count, CLI actions | Diff syntax errors, tokens, cost | Turn count, tool errors, cost | Action trace, score milestones | **Ledger events, Hot/Warm tokens, Blobs, Router tier** |

---

## 3. Core Methodological Requirements for Harness Evals

### 3.1 Task Selection
Harness tasks must test the specific capabilities that a harness provides:
1. **Multi-Turn Context Management:** Tasks that cannot be solved in a single shot. They require inspecting files, reasoning, executing an edit, seeing a compiler or test failure, and adjusting. This directly exercises Chorus's `assemble_context`, task state tier, warm tier compaction, and demote/drop rungs.
2. **Precision Diff & File Editing:** Tasks that require targeted file edits without corrupting unrelated code or introducing syntax noise.
3. **Router & Triage Calibration:** Tasks with clear difficulty tiers (trivial formatting vs. multi-module architectural refactoring) and language variance (Thai and English) to verify that the router selects cost-effective models rather than over-escalating to expensive frontier models.
4. **Bilingual Robustness:** Natural mix of Thai and English prompts reflecting Chorus's real-world workspace.

### 3.2 Metrics & Telemetry
Evaluating a harness requires tracking both **outcome metrics** and **harness operational telemetry**:
* **Outcome Effectiveness:**
  - `Resolved Rate (Pass@1)`: Fraction of tasks solved on the first continuous turn sequence.
  - `Resolved Rate (Pass@2)`: Fraction of tasks solved after receiving automated test failure feedback.
* **Efficiency & Cost (The "Scaffold Effect" Metrics):**
  - `Hot Input Tokens`: Size of the prompt payload assembled by `Session::assemble_context`.
  - `Total Tokens per Solved Task`: Measures context economy.
  - `Turn Count to Resolution`: Measures whether the harness guides the agent to a solution efficiently or traps it in retry loops.
  - `No-Action Turns`: Turns where the agent produced no diff, no tool call, or merely repeated earlier text.
  - `Total Dollar Cost`: Normalized cost based on model token rates.
* **Harness Invariants (`CLAUDE.md` §3 Alignment):**
  - `Blob Dedup Ratio`: Ensuring repeated reads/outputs share blob IDs (`blob rows = 1`).
  - `Warm Tier / Compaction Integrity`: Verifying `summary` table hits without invoking live summarizers during turn assembly.
  - `Demoted vs. Dropped Rungs`: Confirming that older turns are gracefully demoted (summarized/abbreviated) before being dropped.
* **Router Quality Metrics:**
  - `Exact Route Match`: Matching expected `(agent, model_tier)` against reference label.
  - `Over-Escalation Rate`: Choosing an expensive tier (e.g., `astra:max` / Opus / Sonnet) for tasks labeled `luna:low` / `easy`.
  - `Abstention Rate & Precision`: Correctly deferring deliberately ambiguous tasks to manual user intervention without falsely deferring clear tasks.
  - `Routing Latency (p95)`: Time taken to reach a routing decision.

### 3.3 Controlling for Model Variance
LLM generation is inherently stochastic, and provider endpoints experience latency variations and backend updates. To isolate the harness:
1. **Deterministic Decoding Parameters:** Set `temperature: 0.0` (or the minimum supported value) and fixed seeds across all eval runs.
2. **Pinned Model Versions:** Pin specific model snapshots (e.g., `codex` / specific Claude snapshots / local Ollama model digests) rather than floating aliases.
3. **Paired Evaluation Design (Crucial):**
   - Never evaluate Harness A on one set of tasks and Harness B on another.
   - Always run Harness A (baseline) and Harness B (candidate) on the **exact same task instances**, in identical workspaces, against the identical model endpoint.
   - Run trials sequentially or in controlled parallel batches on the same machine to eliminate external load discrepancies.

### 3.4 Statistical Power & Run Count
* **The Pitfall of Independent Testing:** To detect a 5% difference in binary pass rate between two unlinked systems with 80% power at $\alpha = 0.05$, an independent sample requires $N \approx 1,500$ trials. This is completely impractical for desktop harness development.
* **The Power of Paired Testing (McNemar's Test):**
  - In a paired setup where both harnesses run on the exact same task instances, task difficulty variance is eliminated from the error term.
  - If Harness A and Harness B agree on 80% of tasks (both pass or both fail), statistical power depends solely on the **discordant pairs** (tasks where A passes and B fails, or vice versa).
  - On a compact suite of $N = 24$ tasks, if a harness improvement flips 5 tasks from fail to pass with 0 regressions, McNemar's exact test yields $p = 0.031$ ($p < 0.05$), achieving statistically significant validation.
* **Continuous Metric Power (Wilcoxon Signed-Rank Test):**
  - For token consumption, wall-clock time, and turn counts, paired continuous tests (Wilcoxon signed-rank or paired Student's t-test) achieve high statistical power ($\beta > 0.80$) with as few as $N = 15–25$ paired tasks.
* **Run Count Policy:**
  - **Single Run ($k = 1$ paired):** Standard for PR smoke tests and daily development. Fast, cheap, and catches regressions immediately.
  - **Three Repeated Trials ($k = 3$):** Used prior to major architectural releases or router weight cutovers. Reports mean pass rate $\pm$ standard error and identifies flaky tasks.

### 3.5 Sandboxing & Isolation
* **Why Docker is Wrong for Chorus Eval:**
  - Running Docker on macOS incurs Linux VM virtualization overhead, requires external daemon dependencies, complicates filesystem mounting, and contradicts Chorus's lightweight native desktop ethos.
* **The Native macOS Git Worktree Strategy:**
  - Chorus already relies on Git shadow trees (`checkpoint.rs`) and plans Git worktrees for parallel agent orchestration (`map.md`).
  - **Evaluation Sandboxing Protocol:**
    1. A single shared repository of benchmark fixtures lives under `evals/harness/fixtures/`.
    2. Before each task run, Chorus creates a dedicated, throwaway Git worktree:
       ```bash
       git worktree add -b eval-tmp-<task_id>-<run_id> /tmp/chorus-eval/<task_id> <baseline_ref>
       ```
    3. The task runs in `/tmp/chorus-eval/<task_id>`.
    4. Upon task completion, the evaluation runner executes the task's verification script (e.g., `cargo test` or `python3 -m unittest`), records the `git diff`, and tears down the worktree:
       ```bash
       git worktree remove --force /tmp/chorus-eval/<task_id>
       git branch -D eval-tmp-<task_id>-<run_id>
       ```
  - **Performance:** Worktree creation and destruction take $<50\text{ ms}$, require zero containers, isolate files completely, and leave no residual artifacts.

---

## 4. Recommended Chorus Harness Eval Architecture

To judge feature changes (context assembly, warm tier compaction, diff presentation, handoff briefs) and router choices, Chorus should implement a dedicated, tiered evaluation runner integrated directly into `chorus-cli`.

### 4.1 The Real-Path Rule
Just as `chorus bench` mandates executing through `Session::assemble_context` rather than manually constructed `AssembleInput` structs (`CLAUDE.md` §3), the harness eval must run through the real `Session` engine:
```rust
// Real execution path inside the eval runner:
let mut session = Session::create(&temp_db_path, &worktree_cwd, &task.title)?;
let sess_id = session.session_id();

// Exercise real routing, ACP bridge turn execution, and ledger updates
let outcome = session.run_turn_decide(&task.prompt)?;
```
Every event, blob, token count, and router decision is recorded into the real SQLite ledger and extracted directly from `temp_db_path`.

### 4.2 Three-Tiered Evaluation Execution

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ Tier 0: Invariant Smoke Benchmark (`chorus bench`)                         │
│ • Runs in CI / commit hook · < 2 seconds · $0 cost                          │
│ • Validates blob dedup (rows=1), task state non-empty, assembly < 2s       │
└──────────────────────────────────────┬──────────────────────────────────────┘
                                       │
┌──────────────────────────────────────▼──────────────────────────────────────┐
│ Tier 1: Router & Policy Evaluation (`chorus eval --router`)                 │
│ • Runs locally via Ollama (Qwen2.5 / OpenThai) · < 30 seconds · $0 cost    │
│ • Evaluates 34 cases (evals/router/cases.json): exact route, abstention,    │
│   Thai/English balance, p95 latency                                         │
└──────────────────────────────────────┬──────────────────────────────────────┘
                                       │
┌──────────────────────────────────────▼──────────────────────────────────────┐
│ Tier 2: Real Session Engine Harness Suite (`chorus eval --live`)           │
│ • 24 paired tasks in throwaway Git worktrees · 3–5 minutes · <$1.00 cost   │
│ • Executes Session::run_turn / run_turn_decide via ACP bridge               │
│ • Measures: Pass@1, tokens/solved, turn count, no-action loops, diff clean  │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 4.3 Task Suite Composition (24 Tasks)
The recommended Tier 2 suite consists of **24 balanced tasks** (12 Thai, 12 English) distributed across 4 capability categories:

1. **Category A: Precision Bug Fix & Single-Hunk Edit (6 tasks: 3 TH, 3 EN)**
   - *Target:* Tests file viewing, search accuracy, diff patch generation, and immediate test pass.
   - *Example:* Fixing an off-by-one index error in a small Rust/TypeScript utility; verified by running `cargo test --test parser`.
2. **Category B: Multi-Turn Iterative Debugging (6 tasks: 3 TH, 3 EN)**
   - *Target:* Tests context retention across 2–4 turns, task state updates, error output ingestion, and demoted/dropped rung preservation.
   - *Example:* Initial code compiles but fails an edge-case test; agent receives compiler error feedback and must repair the implementation without re-asking basic questions.
3. **Category C: Read-Only Synthesis & Planning (6 tasks: 3 TH, 3 EN)**
   - *Target:* Tests context assembly efficiency, token budgeting under tight windows, and warm tier retrieval without file edits.
   - *Example:* "Analyze why module X deadlocks under high concurrency and output a concrete 3-step mitigation plan." Verified by checking structured output assertions.
4. **Category D: Router & Ambiguity Calibration (6 tasks: 3 TH, 3 EN)**
   - *Target:* Tests routing decisions on live turn execution. 4 tasks are clear (verifying cheap tier selection: `luna:low` / `default:medium`), and 2 tasks are intentionally ambiguous (verifying proper manual abstention instead of wasteful execution).

### 4.4 Directory & Data Structure
To maintain consistency with existing Chorus conventions:
```
evals/
├── harness/
│   ├── README.md                 # Usage instructions and baseline records
│   ├── cases.json                # Task definitions, prompts, expected routes, criteria
│   ├── fixtures/                 # Minimal reproducible test repos (Rust, TS, Python)
│   ├── run.rs (or runner crate)  # Test runner calling Session engine
│   └── baselines/
│       ├── v0.1.0-baseline.json  # Recorded baseline run for paired comparison
│       └── current.json          # Latest run results
```

Each case definition in `cases.json`:
```json
{
  "id": "fix-ts-chunk-parser",
  "language": "en",
  "category": "bug_fix",
  "prompt": "Fix parseChunkedStream to handle trailers properly without dropping final bytes.",
  "fixture": "fixtures/ts-parser-repo",
  "verify_command": "npm test -- test/chunk.test.ts",
  "max_turns": 4,
  "expected_route": "luna:medium",
  "timeout_seconds": 120
}
```

### 4.5 Output & Paired Comparison Report
When evaluating a branch or feature change against the baseline:
```bash
cargo run -p chorus-cli -- eval --compare-to evals/harness/baselines/v0.1.0-baseline.json
```
The runner executes the 24 tasks, queries the SQLite ledgers, and outputs a paired comparison table:

```
Chorus Harness Evaluation Report
======================================================================
Model: Claude 3.5 Sonnet (fixed seed) | Workspaces: Isolated Git Worktrees
Baseline: v0.1.0 (78 events)         | Candidate: branch/context-v2
----------------------------------------------------------------------
Metric                    Baseline       Candidate      Delta     Sig (p)
----------------------------------------------------------------------
Resolved (Pass@1)         18/24 (75.0%)  21/24 (87.5%)  +3 (+12%) p=0.083 (McNemar)
Resolved (Pass@2)         20/24 (83.3%)  23/24 (95.8%)  +3 (+12%) p=0.083 (McNemar)
Avg Tokens / Solved Task  14,200         8,150          -42.6%    p=0.002 (Wilcoxon)**
Avg Turns / Task          2.8            2.1            -25.0%    p=0.014 (Wilcoxon)*
No-Action Turns           4              1              -75.0%    p=0.041 (Wilcoxon)*
Hot Context (chars)       16,400         9,800          -40.2%    p=0.001 (Wilcoxon)**
Router Over-Escalations   5/24           1/24           -4        p=0.045 (Fisher)*
Wall Clock Time (total)   3m 42s         2m 18s         -37.8%    -
Est. Cost (24 tasks)      $0.84          $0.49          -41.6%    -
----------------------------------------------------------------------
Invariants:
- Blob dedup ratio: 1.0 (PASS)
- Task state non-empty: 24/24 (PASS)
- Summarizer bypass on assemble: PASS (0 calls)
======================================================================
Result: Candidate is statistically superior on token efficiency and turn count.
```

---

## 5. Conclusion & Actionable Next Steps for Ticket 10

1. **The Harness is a Primary Variable:** Harness evaluation must measure token economics, turn loops, and context retention alongside pass rate.
2. **Fast & Native over Docker:** Relying on macOS Git worktrees delivers sub-5-minute execution with zero virtualization overhead.
3. **Statistical Power via Paired Testing:** A focused 24-task suite provides rigorous statistical discrimination via McNemar and Wilcoxon tests without requiring thousands of dollars in benchmark runs.
4. **Direct Bridge to Ticket 10 (`harness-eval-design`):**
   - Ticket 10 should formally approve the 24-task matrix composition (12 TH / 12 EN).
   - Ticket 10 should define the exact regression threshold (e.g., zero invariant regressions, non-negative pass rate delta, $>15\%$ token budget compliance).
   - Ticket 10 should finalize whether Tier 2 runs on pre-release branches or nightly CI, keeping daily dev on Tiers 0 and 1.

---

## References

1. **Jimenez, C. E., Yang, J., Wettig, A., Yao, S., Pei, K., Press, O., & Narasimhan, K.** (2024). *SWE-bench: Can Language Models Resolve Real-World GitHub Issues?* International Conference on Learning Representations (ICLR 2024). [arXiv:2310.06770](https://arxiv.org/abs/2310.06770)
2. **OpenAI & Princeton NLP.** (2024). *SWE-bench Verified: Human Validation of Software Engineering Benchmarks.* OpenAI Research.
3. **Yang, J., Jimenez, C. E., Wettig, A., Lieret, K., Yao, S., Narasimhan, K., & Press, O.** (2024). *SWE-agent: Agent-Computer Interfaces Enable Automated Software Engineering.* Advances in Neural Information Processing Systems (NeurIPS 2024). [arXiv:2405.15793](https://arxiv.org/abs/2405.15793)
4. **Vats, N., & Golev, O.** (2026). *The Scaffold Effect in Coding Agents: Harness Choice as a Hidden Variable in Coding-Agent Evaluation.* [arXiv:2607.22585](https://arxiv.org/abs/2607.22585)
5. **Wang, X., Chen, Y., Yuan, L., Zhang, Y., Li, Y., Peng, H., & Ji, H.** (2024). *Executable Code Actions Elicit Better LLM Agents (CodeAct).* International Conference on Machine Learning (ICML 2024). [arXiv:2402.01030](https://arxiv.org/abs/2402.01030)
6. **OpenHands Team.** (2024). *OpenHands: An Open Platform for AI Software Developers as Generalist Agents.* [arXiv:2407.16741](https://arxiv.org/abs/2407.16741)
7. **Gauthier, P.** (2023–2026). *Aider LLM Leaderboards and Editing Benchmarks.* [aider.chat](https://aider.chat/docs/leaderboards/)
8. **Higgins, C., et al. (Anthropic).** (2026). *Demystifying evals for AI agents.* Anthropic Engineering Blog.
9. **Anthropic.** (2024). *Building Effective AI Agents.* Anthropic Research.
10. **Model Evaluation and Threat Research (METR).** (2024–2026). *The METR Task Standard.* [metr.org](https://metr.org) & [arXiv:2412.21199](https://arxiv.org/abs/2412.21199)
11. **Terminal-Bench Consortium.** (2025–2026). *Terminal-Bench: Evaluating AI Agents in Command Line Interfaces.* Artificial Analysis & Harbor Framework.
