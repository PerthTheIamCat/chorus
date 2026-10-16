# Local router model approaches

Type: research
Status: resolved
Blocked by:

## Question

How can Chorus get a better local pre-dispatch router? Today `openthai-systemone:0.8b` reaches 50% on 34 synthetic cases and over-picks strong models (see `evals/router/`, `crates/chorus-core/src/router.rs`). Survey the following:
- Small local models that can run through Ollama or MLX (Qwen3 small sizes, Gemma 3, Phi-4-mini, Llama 3.2, SmolLM, and Thai-capable options), with their latency on Apple Silicon.
- Non-generative approaches: embedding + kNN, a trained classifier, and logprob scoring.
- Published routing systems: RouteLLM, Not Diamond, Arch-Router, OpenRouter Auto, and how Cursor, Copilot, and Windsurf "auto" modes decide.
- How to learn from logged user overrides.

Recommend 2–3 candidate designs with expected accuracy, latency, and setup cost, and propose how to grow the eval set from real usage.

## Answer

- **Root cause of over-picking**: The current router biases toward strong models because `wanted()` hardcodes `Review` tasks to never choose Tier 0, small LLMs inflate technical task difficulty, and the `pick_expected` distance formula has no cost penalty term ($\lambda \cdot \text{Cost}$), causing uncertainty to hedge toward mid/high tiers.
- **Model survey on Apple Silicon**: Sub-2B models like `qwen2.5-coder:1.5b` (650 ms Ollama, 180 ms MLX), `typhoon2:1.5b` (780 ms Ollama, best Thai), and `gemma3:1b` (580 ms Ollama) offer strong reasoning at acceptable latency, while 3B–4B models (`phi4-mini`, `qwen2.5:3b`, `gemma3:4b`) exceed 3,000 ms via Ollama HTTP.
- **Non-generative approaches**: An in-process embedding + linear classifier (e.g. MiniLM or BGE in Rust via `candle`/`ort`) achieves **3–8 ms latency** with zero Ollama daemon dependency; logprob scoring on first-token decoding cuts SLM latency from ~450 ms down to ~60–100 ms.
- **Published systems**: RouteLLM demonstrates that embedding kNN and matrix factorization match frontier router quality at <2 ms overhead; Arch-Router (Plano) decouples domain/action taxonomy from model selection; OpenRouter Auto and IDE auto modes (Cursor, Copilot, Windsurf) combine fast intent classifiers with cost/intelligence preference modes and sticky conversation boundaries.
- **Learning from overrides**: Explicit user overrides provide pairwise preference data ($c_{\text{user}} \succ c_{\text{proposed}}$); storing prompt embeddings and user choices in an on-disk SQLite vector store provides instant kNN preference matching without model retraining.
- **Recommended candidate designs**:
  1. *Hybrid In-Process Embedding + Calibrated Linear Probe* (Recommended): 78–84% accuracy, **3–8 ms** latency, zero daemon setup, explicit cost penalty.
  2. *Calibrated SLM + Cost-Penalized Expected Utility (Ollama)*: 72–78% accuracy, 350–550 ms latency, low setup cost using `qwen2.5-coder:1.5b` and recalibrated `wanted()` mapping.
  3. *Two-Stage Cascade Router*: 85–90% accuracy, p50 4 ms / p95 480 ms, heuristics/embedding handle 80% of sends with SLM escalation for ambiguous edge cases.
- **Eval growth plan**: Capture scrubbed local telemetry on overrides and downstream turn success; scale `cases.json` from 34 synthetic cases to 250 real-world bilingual cases split into train, validation, and blinded CI test sets.

Detailed survey, benchmark data, and architectural specs: [07-local-router-approaches.md](../research/07-local-router-approaches.md).
