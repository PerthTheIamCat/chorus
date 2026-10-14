# Harness eval methodology

Type: research
Status: resolved
Blocked by:

## Question

How do others measure whether a harness (as opposed to a model) makes agents more effective? Read SWE-bench (Verified/Lite), Terminal-Bench, Aider's benchmarks, OpenHands evals, METR task suites, and first-party harness-evaluation write-ups. Record task selection, metrics (success, tokens, wall time, human interventions, cost), how to control for model variance, run count and statistical power, and sandboxing. Recommend a small, cheap, repeatable Chorus harness eval that runs through the real `Session` engine (like `chorus bench`'s real-path rule). It should be able to judge feature changes and router choices.

## Answer

- **Harness as Primary Multiplier (The "Scaffold Effect"):** Research (Vats & Golev 2026, SWE-agent 2024, Anthropic 2026) demonstrates that agents are `(model, harness)` pairs. Holding model weights constant, the harness (context assembly, diff presentation, tool feedback) swings token cost by up to 40× and solve rate by 10–20%, exhibiting harness-specific "failure fingerprints".
- **Primary Methodologies & Sandboxing:** While SWE-bench and METR rely on heavy, expensive Docker containers ($100–$400/run), Aider demonstrates fast, lightweight evaluation using native Git repositories. For macOS Chorus, native Git worktrees (`/tmp/chorus-eval/<task>`) provide instant (<50ms) sandboxing with zero Docker overhead.
- **Controlling Model Variance & Statistical Power:** Independent sampling requires ~1,500 trials to detect small differences, whereas **paired testing** (evaluating candidate vs. baseline harnesses on the identical task instances with fixed seed/temperature) eliminates task difficulty variance; McNemar's test and paired Wilcoxon achieve statistical significance ($p < 0.05$) on just 20–30 tasks.
- **Recommended 3-Tiered Chorus Architecture:**
  - *Tier 0 (Invariant Smoke / `chorus bench`):* <2s, $0, validates core ledger invariants (blob dedup rows=1, task state non-empty, assemble <2s).
  - *Tier 1 (Router Gate / `chorus eval --router`):* <30s, $0, evaluates local Ollama against `evals/router/cases.json` for routing accuracy, latency, and abstention.
  - *Tier 2 (Real Session Suite / `chorus eval --live`):* 24 paired tasks (12 Thai, 12 English) running in throwaway Git worktrees through real `Session::run_turn` / `run_turn_decide` via ACP bridges. Completes in 3–5 minutes for <$1.00.
- **Key Metrics & Real-Path Adherence:** Telemetry is read directly from the SQLite ledger: Pass@1/Pass@2, hot context size, total tokens per solved task, turn count, no-action loops, router over-escalation, and ledger bloat.

Full findings and literature citations: [09-harness-eval-methodology.md](../research/09-harness-eval-methodology.md)
