# Chorus Harness Evaluation Suite

The Chorus Harness Evaluation Suite provides a systematic empirical benchmark for judging agent scaffolding, context management, router quality, and invariant stability across releases.

## Structure

- `tasks/`: 60 paired evaluation tasks (30 Thai, 30 English) across 4 engineering domains:
  - **Rust** (backend/systems): 16 tasks (8 TH, 8 EN)
  - **Swift** (macOS UI): 14 tasks (7 TH, 7 EN)
  - **TypeScript** (web/APIs): 16 tasks (8 TH, 8 EN)
  - **Python** (data/scripts): 14 tasks (7 TH, 7 EN)
- `results-YYYY-MM-DD.md`: Telemetry ledger recording Pass@1, tokens, context bloat ratio, and tool loop rates.

## Cadence and CLI Commands

Run via `cargo run -p chorus-eval -- <command>`:

- **Tier 0 Invariants Runner (<2s SLA)**:
  ```bash
  cargo run -p chorus-eval -- tier0
  ```
  Validates ledger invariants: content-addressed blob deduplication, context assembly priority, and zero tool loops.

- **Tier 1 Scaffolding & Router Gate Runner (<30s CI Gate)**:
  ```bash
  cargo run -p chorus-eval -- tier1
  ```
  Evaluates task complexity classification accuracy (>=80% target), post-edit verification loop triggering, prompt assembly budgeting, and telemetry.

- **Run All**:
  ```bash
  cargo run -p chorus-eval -- all
  ```
