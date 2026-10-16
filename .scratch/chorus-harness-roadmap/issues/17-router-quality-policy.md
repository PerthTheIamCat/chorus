# Router quality policy

Type: grilling
Status: resolved
Blocked by: 07, 10

## Question

Decide router v2. Already agreed: **remove the countdown** and stop over-picking strong models. Still open:
- The routing approach and model (from the approaches research).
- The cost-bias policy and how it trades off against quality.
- When the router asks instead of dispatching, now that there is no countdown.
- How overrides are logged and fed back.
- The accuracy and latency targets on the harness eval before Auto becomes the default.
- How the user sees the chosen model and changes it after the fact.

## Answer

- **In-Process Embedding + Calibrated Linear Probe (Hybrid Cascade)**: Replace local Ollama 0.8B SLM with an in-process Rust embedding classifier (`candle`/`ort` using multilingual MiniLM / BGE-small). Operates at **3–8 ms latency** with zero daemon dependency and 78–84% baseline accuracy (matching RouteLLM findings).
- **Cost-Biased Utility Optimization**:
  - Remove hardcoded Tier-0 floors on Review tasks in `wanted()`.
  - Add explicit cost penalty term to utility distance ($U(m) = \text{Score}(m) - \lambda \cdot \text{Cost}(m)$).
  - Bias default execution to efficient tiers (`gpt-5-codex` / low reasoning) for short queries, unit tests, typos, and exploration. Escalate to frontier reasoning (`gpt-5.5` / high reasoning) strictly when prompts trigger architectural design, multi-file refactoring, or race-condition debugging.
- **Zero-Countdown Instant Dispatch & Borderline Gating**:
  - Eliminates the 5-second countdown banner entirely. Dispatches immediately in <10ms.
  - Borderline tasks (confidence < 40% or conflicting signals) pause to display a non-blocking confirmation dialog rather than guessing incorrectly.
- **UI Visibility & Post-Turn Switching**:
  - Displays a subtle model badge above composer / on message bubble: `Auto: gpt-5-codex [low]`.
  - Includes a 1-click "Regenerate with..." dropdown allowing the user to seamlessly re-run the turn with a different model if unsatisfied.
- **Self-Learning User Override Flywheel**:
  - Explicit user model overrides or re-runs are logged into an on-disk SQLite vector table (`router_overrides`).
  - Upcoming prompt embeddings query this table via local kNN; repeated user preferences automatically take precedence without manual retraining.
- **Quality Gate Targets**: Must achieve ≥80% accuracy and <50ms p95 latency across the 60-task harness eval before becoming the default out-of-the-box routing mode.
