# Harness eval design

Type: grilling
Status: resolved
Blocked by: 09

## Question

Using the methodology research, decide the Chorus harness eval: the task suite (count, sources, languages, Thai/English), metrics and the baseline that must not regress, which agents and models it runs against, cost budget per run, when it runs (manual, before release, per-PR), and where results live. It must also serve as the router's quality gate.

## Answer

- **Comprehensive Task Suite (60 Tasks)**: Scaled to 60 paired tasks (30 Thai, 30 English) evenly distributed across 4 real-world engineering domains:
  1. *Bug fixing* (isolated reproducing repros, logic bugs, stack trace analysis).
  2. *Refactoring & Code Review* (detecting edge cases, architectural cleanups).
  3. *Multi-turn instruction following* (2–3 turn sequential workflows with dependency changes).
  4. *Multi-file exploration & editing* (cross-directory discovery and consistent updates).
- **Core Invariant Gates & Baselines**:
  1. *Solve Rate (Pass@1)*: Primary effectiveness metric, verified by automated test assertions in the task worktree.
  2. *Token Efficiency Index*: Cumulative tokens consumed per solved task; candidate harness cannot regress >10% vs baseline.
  3. *Turn Count & Loop Detection*: Turn count tracked; **zero tolerance for no-action tool loops** (calling identical tool with identical args >2 times immediately fails the run).
  4. *Context Bloat Ratio*: Hot context sent to model must maintain strict compression ratio; demotion must strictly precede dropping.
- **Agent Matrix Evaluation**: Dual-agent benchmark matrix executing against both **OpenAI Codex** (`gpt-5.5` / `gpt-5-codex`) and **Anthropic Claude Code** (`claude-3-7-sonnet`) over ACP bridges to prevent provider-specific harness overfitting.
- **Execution Cadence & Tiering**:
  - *Tier 0 (Invariants / `chorus bench`)*: Runs in <2s on pre-commit / every PR to verify blob dedup, assembly speed, and task state.
  - *Tier 1 (Router Gate / `chorus eval --router`)*: Runs in <30s on CI/PR to verify local router accuracy, latency, and calibration on `evals/router/cases.json`.
  - *Tier 2 (Full Live Suite / `chorus eval --live`)*: Manual execution via CLI, before major version releases, or during core architectural updates.
- **Reporting & Telemetry**: Results recorded in `evals/harness/results-YYYY-MM-DD.md` with structured JSON ledger telemetry for regression tracking.
