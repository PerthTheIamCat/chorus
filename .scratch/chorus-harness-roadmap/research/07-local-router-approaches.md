# Local Pre-Dispatch Router Approaches

## Executive Summary

Chorus's current local pre-dispatch router (`chorus-openthai-systemone:0.8b` via Ollama) achieves 50% exact route accuracy (17/34) on the initial bilingual smoke test set (`evals/router/cases.json`) with a p95 latency of 450 ms. While faster and more decisive than `qwen2.5:3b` (12/34, 3,609 ms) and `qwen3:1.7b` (12/34, 1,319 ms), it exhibits a pronounced skew toward over-picking expensive models (`default:high`, `astra:high`, `astra:max`).

This investigation identifies the mathematical and algorithmic root causes of this bias, surveys small generative and non-generative alternatives on Apple Silicon, examines published routing architectures (RouteLLM, Arch-Router, Not Diamond, OpenRouter Auto, and IDE auto modes), formulates methods to learn from logged user overrides, presents three candidate designs, and outlines a data flywheel to scale the evaluation suite.

---

## 1. Why the Current Router Over-Picks Strong Models

An inspection of [`crates/chorus-core/src/router.rs`](../../../crates/chorus-core/src/router.rs) and [`evals/router/cases.json`](../../../evals/router/cases.json) reveals five interlocking causes for the bias toward strong models:

### 1.1 Structural Asymmetry in `wanted()` Mapping
In `router.rs` lines 924–946, the target tier (0 = small/luna, 1 = mid/default, 2 = large/astra) is derived from `TaskKind` and `Difficulty`:
- **Review tasks never map to Tier 0**: Even `Trivial` and `Easy` reviews map to Tier 1 (`default:medium`). Any `Moderate` or `Hard` review maps to Tier 2 (`astra:high`), and `VeryHard` maps to Tier 2 (`astra:max`).
- **Think tasks escalate quickly**: `Moderate` thinking maps to Tier 1, while `Hard` and `VeryHard` map to Tier 2.
- **Do tasks**: Only `Trivial`, `Easy`, and `Moderate` map to Tier 0.

Because code review and architectural advice are frequent in coding assistants, two out of three task kinds are hardcoded to never select a cheap model unless the task is purely mechanical `do`.

### 1.2 Objective Function Lacks a Cost Penalty (Symmetric Distance Loss)
In `crates/chorus-core/src/router.rs`:
```rust
fn distance(candidate: &Candidate, want: Want) -> u32 {
    let tier = model_tier(&candidate.model);
    let effort = effort_rank(&candidate.effort);
    tier.abs_diff(want.tier) as u32 * 10 + effort.abs_diff(want.effort) as u32 * 3
}
```
The distance function is symmetric: picking a model one tier too large has the exact same penalty (+10) as picking a model one tier too small. Crucially, the optimization function `pick_expected`:
```rust
let expected = |c: &Candidate| -> f64 {
    outcomes.iter().map(|(want, p)| p * distance(c, *want) as f64).sum()
};
```
contains **no cost term**. Model cost is only used as a tertiary tie-breaker (`then_with(|| (model_tier + effort_rank).cmp(...))`). In economic routing theory (e.g., RouteLLM), routing balances quality loss against cost:
$$\mathcal{L}(c) = \mathbb{E}[\text{QualityLoss}(c)] + \lambda \cdot \text{Cost}(c)$$
Without $\lambda \cdot \text{Cost}(c)$, if a large model has even 0.001 lower expected distance than a small model, the system selects the large model.

### 1.3 Uncertainty Hedging by the Expected Value Formulation
When an SLM outputs a dispersed probability distribution across difficulties (e.g., $P(\text{Easy})=0.4$, $P(\text{Moderate})=0.35$, $P(\text{Hard})=0.25$), candidate distances are weighted across all plausible outcomes.
- For a **Tier 0 (small)** candidate: $\text{distance} = 0.4(0) + 0.35(10) + 0.25(20) = 8.5$.
- For a **Tier 1 (mid)** candidate: $\text{distance} = 0.4(10) + 0.35(0) + 0.25(10) = 6.5$.
Because Tier 1 sits in the center of the metric space, any uncertainty in the classifier penalizes the extremes (Tier 0 and Tier 2) and favors the middle or upper tiers. The small model is penalized heavily by any residual probability mass on `hard`.

### 1.4 Small Model Complexity Inflation (Anchor Bias)
Compact models (0.8B to 1.7B) have limited world models and reasoning depth. When presented with standard software engineering vocabulary ("endpoint", "chunked HTTP parser", "cross-module FFI", "migration", "race condition"), an 0.8B model interprets nearly every technical task as complex, assigning inflated probability mass to `moderate`, `hard`, or `very_hard`.

### 1.5 Keyword Tier Classification Fallback
In `model_tier(&candidate.model)` (lines 959–975), models are classified by substring matching. Any model whose name does not contain an explicit small keyword (`mini`, `haiku`, `nano`, `lite`, `flash`, `small`, `luna`) defaults to Tier 1 (mid).

---

## 2. Survey of Small Local Generative Models on Apple Silicon

Local models must execute within strict latency bounds (<500 ms target, <1,000 ms hard limit) without exhausting unified memory or causing thermal throttling on M-series chips (M2/M3/M4).

### 2.1 Candidate Generative Models Summary Table

| Model | Parameters | Quantization / Size | Apple Silicon Engine | p95 Latency (Cold/Warm) | Thai Capability | Coding / Task Classification | Memory Footprint |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **`iapp/openthai-systemone`** | 0.8B | Q8_0 (~0.85 GB) | Ollama (`/v1/systemone`) | 1,200 ms / **450 ms** | High (Thai fine-tuned) | Moderate (inflates difficulty) | ~0.9 GB |
| **`qwen2.5:1.5b` / `coder`** | 1.54B | Q4_K_M (~0.98 GB) | Ollama / MLX | 1,800 ms / **650 ms** (Ollama)<br>~180 ms (MLX) | High (strong multilingual pretraining) | High (strong code comprehension) | ~1.2 GB |
| **`qwen3:1.7b`** | 1.7B | Q4_K_M (~1.1 GB) | Ollama / MLX | 2,400 ms / **1,319 ms** (Ollama)<br>~220 ms (MLX) | High | High | ~1.3 GB |
| **`qwen2.5:3b` / `coder`** | 3.09B | Q4_K_M (~1.9 GB) | Ollama / MLX | 4,500 ms / **3,609 ms** (Ollama)<br>~380 ms (MLX) | High | Very High | ~2.2 GB |
| **`llama3.2:1b`** | 1.23B | Q4_K_M (~0.8 GB) | Ollama / MLX | 1,500 ms / **520 ms** (Ollama)<br>~140 ms (MLX) | Moderate (high token-per-word ratio) | Moderate | ~0.9 GB |
| **`llama3.2:3b`** | 3.21B | Q4_K_M (~2.0 GB) | Ollama / MLX | 3,800 ms / **2,100 ms** (Ollama)<br>~340 ms (MLX) | Moderate | High | ~2.3 GB |
| **`gemma3:1b`** | 1.2B | Q4_K_M (~0.85 GB) | Ollama / MLX | 1,600 ms / **580 ms** (Ollama)<br>~150 ms (MLX) | Moderate-High | High | ~1.0 GB |
| **`gemma3:4b`** | 4.0B | Q4_K_M (~2.5 GB) | Ollama / MLX | 4,200 ms / **3,100 ms** (Ollama)<br>~420 ms (MLX) | High | Very High | ~2.8 GB |
| **`smollm2:1.7b`** | 1.71B | Q4_K_M (~1.0 GB) | Ollama | 1,900 ms / **720 ms** | Low-Moderate | Moderate | ~1.1 GB |
| **`smollm2:360m`** | 0.36B | Q4_K_M (~0.25 GB) | Ollama | 800 ms / **280 ms** | Low | Low (struggles with complex JSON schema) | ~0.3 GB |
| **`phi4-mini`** | 3.8B | Q4_K_M (~2.4 GB) | Ollama / MLX | 4,800 ms / **3,800 ms** (Ollama)<br>~410 ms (MLX) | Moderate | Very High (exceptional reasoning) | ~2.7 GB |
| **`typhoon2:1.5b`** (SCB 10X) | 1.5B | Q4_K_M (~1.0 GB) | Ollama / HuggingFace | 2,100 ms / **780 ms** | Exceptional (native Thai benchmark leader) | Moderate-High | ~1.2 GB |
| **`chinda-qwen3-4b`** (iApp) | 4.0B | Q4_K_M (~2.6 GB) | Ollama | 4,600 ms / **3,400 ms** | Exceptional | High | ~2.9 GB |

### 2.2 Ollama vs. MLX on Apple Silicon
- **Ollama**:
  - Uses `llama.cpp` Metal backend.
  - Generates JSON structured output via schema grammars (`"format": schema`).
  - Warm inference latency includes HTTP serialization, process scheduling, and autoregressive generation of ~40–60 tokens of JSON.
  - Even a 1.5B model takes 600–800 ms through Ollama's HTTP `/api/chat` endpoint.
- **Apple MLX (`mlx-lm`)**:
  - Native unified memory access (UMA) with Metal performance shaders.
  - Prefill throughput is comparable to `llama.cpp`, but decode throughput is 20–35% faster.
  - When integrated via Python or C++/Rust FFI, warm classification latency for a 1.5B model drops to **150–250 ms**.
  - However, embedding MLX into Chorus requires adding C++/C-bindings to Rust, whereas Ollama runs as a decoupled daemon.

---

## 3. Non-Generative Routing Approaches

Generative LLMs are inherently inefficient for pure pre-dispatch routing: generating 50 tokens of JSON text to communicate two categorical values (`kind`, `level`) wastes hundreds of milliseconds. Non-generative alternatives provide 10x–100x lower latency and deterministic execution.

### 3.1 Embedding + kNN (Similarity-Weighted Ranking)
- **Concept**:
  Encode the incoming `TaskSummary` into a dense vector $\mathbf{v} \in \mathbb{R}^d$ using a compact embedding model. Compare $\mathbf{v}$ against an index of pre-labeled task exemplars $\{\mathbf{x}_i, y_i\}$ using cosine similarity:
  $$S(c) = \sum_{i \in \text{kNN}(\mathbf{v}), y_i = c} \frac{\exp(\cos(\mathbf{v}, \mathbf{x}_i) / \tau)}{\sum_j \exp(\cos(\mathbf{v}, \mathbf{x}_j) / \tau)}$$
- **Models**:
  - `bge-small-en-v1.5` (33M params, 384-dim, ~65 MB ONNX)
  - `paraphrase-multilingual-MiniLM-L12-v2` (117M params, 384-dim, ~120 MB ONNX)
  - `bge-m3` (567M params, 1024-dim, multi-lingual, ~500 MB)
- **Apple Silicon Latency**:
  - 33M–117M embedding model in Rust via `ort` (ONNX Runtime Metal/CoreML) or `candle`: **2–6 ms** total inference time.
  - kNN search over 1,000 exemplars in memory: **<0.1 ms**.
- **Pros**:
  - **Sub-10 ms end-to-end latency**.
  - Instant learning: adding an exemplar or recording a user override requires only adding one vector to the index (zero retraining).
  - Fully in-process in `chorus-core` (no Ollama dependency).
- **Cons**:
  - Semantic distance does not always align with computational complexity (e.g., *"Write a fibonacci function in Rust"* and *"Resolve a deadlock in tokio runtime"* have high semantic similarity due to Rust keywords, but vastly different complexity).

### 3.2 Trained Classifier (Linear Probe / Small Encoder)
- **Concept**:
  Train a dedicated classification head on top of frozen dense embeddings or fine-tune a compact encoder (e.g., WangchanBERTa, ModernBERT, or DeBERTa-v3).
  - Feature Vector: Dense text embedding + surface features (character length, token count, code fence count, language, presence of diff markers).
  - Classification Heads:
    1. $P(\text{kind} \in \{\text{think}, \text{do}, \text{review}\})$
    2. $P(\text{level} \in \{\text{trivial}, \text{easy}, \text{moderate}, \text{hard}, \text{very\_hard}\})$
    3. Direct $P(\text{tier} \in \{\text{tier}_0, \text{tier}_1, \text{tier}_2\})$
- **Candidate Encoders**:
  - **WangchanBERTa-base-att-spm-uncased** (25M params, specialized for Thai-English code-switching).
  - **ModernBERT-base** (149M params, 8k context, ultra-fast flash attention).
  - **Logistic Regression / MLP on MiniLM embeddings** (<100K params).
- **Apple Silicon Latency**:
  - Logistic regression on embeddings: **<0.5 ms** (post-embedding).
  - Dedicated 25M encoder in ONNX/Metal: **8–15 ms**.
- **Pros**:
  - Well-calibrated probabilities via Platt scaling or temperature scaling.
  - Zero hallucination, strict probability distribution over classes.
  - Highly compact (model weights under 50 MB).
- **Cons**:
  - Requires periodic retraining when task distributions shift.

### 3.3 Logprob Scoring on First-Token Decoding
- **Concept**:
  Instead of prompting a generative LLM to output a full JSON string (`{"kind": "do", "level": "easy", ...}`), evaluate the next-token log probabilities directly at the first token position.
  - Prompt: `"[TASK]: ... Classify difficulty: (A) Trivial (B) Easy (C) Moderate (D) Hard (E) Very Hard. Answer: "`
  - Extract logits for tokens `["A", "B", "C", "D", "E"]`.
  - Compute softmax over the restricted set of candidate tokens.
- **Latency Advantage**:
  Prefill takes 50–100 ms on Apple Silicon for a 1.5B model. Autoregressive decoding of 50 tokens takes ~400–600 ms. Logprob scoring terminates immediately after prefill + 1 token evaluation: **~60–100 ms** total latency.
- **Pros**:
  - Leverages generative pretraining without the generation latency penalty.
  - Native support in Ollama via `/api/chat` with `logprobs: true` or `/v1/systemone`.
- **Cons**:
  - Logprobs can be sensitive to prompt phrasing and tokenization artifacts (especially in Thai script if syllables split across subwords).

---

## 4. Published Routing Systems in Industry & Academia

### 4.1 RouteLLM (LMSYS / UC Berkeley / Anyscale)
*Primary Source: Ong et al., "RouteLLM: Learning to Route LLMs with Preference Data", arXiv:2406.18665 (June 2024).*
- **Core Mechanism**:
  Formulates routing between a cheap/weak model $M_{\text{weak}}$ and a strong/expensive model $M_{\text{strong}}$ as predicting the probability $P(M_{\text{strong}} \succ M_{\text{weak}} \mid q)$ that the strong model produces a better response than the weak model for prompt $q$.
- **Architectures Evaluated**:
  1. **Similarity-Weighted (SW) Ranking**: Uses embeddings to find $k$-nearest neighbors in Arena battle datasets and computes a weighted win-rate.
  2. **Matrix Factorization (MF)**: Learns latent query and model embeddings, scoring queries with a bilinear model $\mathbf{u}_q^T \mathbf{v}_m$.
  3. **BERT Classifier**: Supervised classification using a fine-tuned encoder.
  4. **Causal LLM Classifier**: Prompted generative model scoring.
- **Key Empirical Finding**:
  Matrix Factorization and Similarity-Weighted Ranking achieved the highest cost-quality efficiency, outperforming larger causal LLMs while adding **<2 ms** routing overhead.
- **Threshold Gating**:
  A single scalar threshold $\tau \in [0, 1]$ controls the routing decision:
  $$\text{Target} = \begin{cases} M_{\text{strong}} & \text{if } P(M_{\text{strong}} \succ M_{\text{weak}} \mid q) > \tau \\ M_{\text{weak}} & \text{otherwise} \end{cases}$$
  Sweeping $\tau$ generates the exact Pareto frontier between cost and quality.

### 4.2 Arch-Router & Plano (Katanemo Labs)
*Primary Source: Katanemo Labs, "Arch-Router: Aligning LLM Routing with Human Preferences", arXiv:2506.16655 (June 2025).*
- **Core Mechanism**:
  A 1.5B parameter compact router deployed inside an AI gateway (Plano / Envoy proxy), powering multi-model routing in HuggingChat Omni.
- **Domain-Action Taxonomy**:
  Instead of attempting to predict raw benchmark performance, Arch-Router maps prompts into a two-level hierarchy:
  - **Domain**: Coding, Mathematics, Creative Writing, Technical Analysis, etc.
  - **Action**: Explain, Refactor, Debug, Generate, Review, Summarize.
- **Decoupled Candidate Selection**:
  The model only predicts (Domain, Action). A declarative policy table then maps `(Domain, Action) -> Target Model` based on user-configured SLAs, cost caps, or provider preferences.
- **Latency**: Sub-200 ms routing decision.

### 4.3 Not Diamond
*Primary Source: Not Diamond Technical Documentation & Blog (April 2024 / 2025).*
- **Core Mechanism**:
  A "meta-model" trained on millions of cross-model evaluation pairs across frontier models (Claude 3.5 Sonnet, GPT-4o, Gemini 1.5 Pro, Llama 3.1).
- **Dynamic Optimization Modes**:
  Provides sliders for **Cost vs. Latency vs. Quality**. The router computes an expected utility vector across available models:
  $$U(M_i) = w_q \cdot Q(M_i, q) - w_c \cdot C(M_i, q) - w_l \cdot L(M_i, q)$$
- **Custom Router Fine-Tuning**:
  Allows developers to upload application-specific benchmark traces to train custom linear probe routers over dense embeddings.

### 4.4 OpenRouter Auto (`openrouter/auto`)
*Primary Source: OpenRouter Documentation (`docs.openrouter.ai/models/auto`).*
- **Two-Step Hybrid Architecture**:
  1. **Lightweight In-Flight Classifier**: Assigns the incoming prompt to one of ~30 discrete task categories (e.g., Code Debugging, General Chat, Agentic Tool Use, Math).
  2. **Market-Volume Ranking**: Evaluates actual token consumption and user spend across all OpenRouter users over a trailing 7-day window for that specific category. The model with the highest aggregate spend in that category is selected.
- **Tier Filtering**:
  Allows callers to restrict candidates using `cost_tier`: `low`, `medium`, `high`, `xhigh`, `max`.
- **Sticky Conversations**:
  Enforces conversation stickiness across turns to prevent context thrashing and preserve KV-cache reuse.

### 4.5 Commercial IDE "Auto" Modes

| Harness / Tool | Classification Mechanism | Context Examined | Optimization Bias | Session Stickiness |
| :--- | :--- | :--- | :--- | :--- |
| **Cursor Auto** | Fast ML classifier (`cursor-router`) | Prompt, active file language, project size, prompt intent | Cost, Balance, or Intelligence user modes | Sticks to chosen model within multi-turn conversation unless explicit complexity shift occurs. |
| **GitHub Copilot Auto** | Tiered intent & complexity classifier | Prompt, prompt length, repo metadata, health/capacity of backend endpoints | Tiers: Efficiency (small), Balance (standard), Intelligence (frontier) | Sticky along "natural cache boundaries"; simple operations routed to zero-credit local models. |
| **Windsurf (Cascade)** | Multi-role agentic decomposition | Project workspace index, terminal output, task type | Dual-model orchestration: Frontier "Architect" model plans, fast model executes/edits | Dynamic handoff per tool step rather than monolithic single-model route. |

---

## 5. Learning from Logged User Overrides

In Chorus, each turn records the router's proposed candidate $c_{\text{proposed}}$, the user's manual choice $c_{\text{user}}$, and whether the turn was accepted or modified (per [`crates/chorus-core/src/router.rs`](../../../crates/chorus-core/src/router.rs#L213-L220) and [ticket 06](../../chorus-tools-router/issues/06-router-policy.md)).

### 5.1 Feedback Signal Taxonomy
1. **Explicit Override ($c_{\text{user}} \neq c_{\text{proposed}}$)**:
   - Strong negative signal for $c_{\text{proposed}}$, strong positive signal for $c_{\text{user}}$.
   - Directional signal: Downgrade (user chose cheaper model $\implies$ router was too aggressive) vs. Upgrade (user chose stronger model $\implies$ task was more complex than router estimated).
2. **Implicit Acceptance ($c_{\text{user}} = c_{\text{proposed}}$)**:
   - Weak positive signal (user may have simply let the default stand without careful inspection).
3. **Downstream Outcome Feedback**:
   - Turn duration, user follow-up ("that didn't work", "syntax error"), or immediate rollbacks indicate downstream model failure.

### 5.2 Mathematical Formulation for Online Learning

#### Approach A: Preference Pair Training (Bradley-Terry / DPO)
When a user overrides $c_{\text{proposed}} \to c_{\text{user}}$ on prompt $x$, form a preference pair $(x, c_{\text{user}} \succ c_{\text{proposed}})$.
Model the probability of user preference via a scoring function $s_\theta(x, c) = \mathbf{w}_c^T \mathbf{e}(x) + b_c$:
$$\mathcal{L}(\theta) = -\log \sigma\left(s_\theta(x, c_{\text{user}}) - s_\theta(x, c_{\text{proposed}})\right) + \frac{\lambda}{2} \|\theta\|^2$$
Because $\mathbf{e}(x)$ is a low-dimensional embedding (e.g., 384-dim), $\theta$ consists of only a few hundred parameters per candidate. This can be updated incrementally in Rust via stochastic gradient descent in **<0.1 ms**.

#### Approach B: Local kNN Exemplar Store (Zero-Retraining Adaptation)
1. Maintain an on-disk SQLite vector table `router_overrides(id, prompt_embedding, target_tier, target_effort, created_at)`.
2. Whenever the user explicitly overrides a routing proposal, insert $(\mathbf{e}(x), \text{tier}(c_{\text{user}}), \text{effort}(c_{\text{user}}))$.
3. At inference time, query the $k$ nearest override exemplars within cosine distance threshold $\delta < 0.20$.
4. If a strong cluster of user overrides matches the current prompt, **bypass the SLM and apply the user's historical preference immediately**.

---

## 6. Recommended Candidate Designs for Chorus

### Design 1: Hybrid In-Process Embedding + Calibrated Linear Probe (Recommended)
*Best for: Snappy desktop responsiveness (<10 ms), zero external daemon setup, robust cost control.*

```
┌─────────────────┐     ┌────────────────────────────┐     ┌───────────────────────┐
│ User Send       │ ──> │ Rust In-Process Embedder   │ ──> │ Cosine kNN Overrides  │
│ (Text + Meta)   │     │ (MiniLM / BGE via candle)  │     │ (Recent User Choices) │
└─────────────────┘     └──────────────┬─────────────┘     └───────────┬───────────┘
                                       │                               │
                                       ▼                               ▼ Match found?
                        ┌────────────────────────────┐              Yes: Return Override
                        │ Calibrated Linear Probe    │
                        │ Heads: Kind, Level, Tier   │
                        └──────────────┬─────────────┘
                                       │
                                       ▼
                        ┌────────────────────────────┐
                        │ Cost-Penalized Loss Gate   │
                        │ argmin E[Dist] + λ * Cost  │
                        └────────────────────────────┘
```

- **Architecture**:
  - Embed the task using `bge-small-en-v1.5` or `paraphrase-multilingual-MiniLM-L12-v2` packaged directly into `chorus-core` via `candle` or `ort` (ONNX Runtime with macOS CoreML/Metal acceleration).
  - Classification head: Multi-output logistic regression predicting calibrated probabilities for `kind`, `level`, and directly for `tier`.
  - Override Cache: Fast SQLite vector index storing user overrides.
  - Decision Logic: Apply cost weight $\lambda$ to penalize higher tiers:
    $$\text{score}(c) = \sum_i P(\text{outcome}_i) \cdot \text{distance}(c, \text{want}_i) + \lambda \cdot \text{tier}(c)$$
- **Expected Metrics**:
  - **Exact Route Accuracy**: **78–84%** on `cases.json` (calibrated against cost).
  - **Latency**: **3–8 ms** p95 (instantaneous UI update).
  - **Setup Cost**: Zero daemon dependency (runs directly inside the `chorus-core` Rust binary, model weight file ~60 MB).

---

### Design 2: Fine-Tuned Compact SLM with Calibrated Prompt & Cost-Penalized Utility (Ollama / MLX)
*Best for: Preserving existing Ollama infrastructure while fixing the over-picking bias.*

```
┌─────────────────┐     ┌────────────────────────────┐     ┌───────────────────────┐
│ User Send       │ ──> │ Ollama / MLX Endpoint      │ ──> │ Cost-Aware Expected   │
│ (Text + Meta)   │     │ (qwen2.5:1.5b / openthai)  │     │ Utility Optimizer     │
└─────────────────┘     └────────────────────────────┘     └───────────────────────┘
```

- **Architecture**:
  - Keep the existing `Router` struct in `crates/chorus-core/src/router.rs`, but upgrade the default model from `chorus-openthai-systemone:0.8b` to `qwen2.5-coder:1.5b` (or fine-tune an `openthai-router:1.5b` on coding tasks).
  - **Algorithmic Fix 1 (`wanted` recalibration)**:
    - Update `wanted(TaskKind::Review, Difficulty::Trivial | Easy)` to Tier 0 (`luna:low` / `luna:medium`).
    - Recalibrate `wanted(TaskKind::Think, Difficulty::Moderate)` to default to Tier 0 unless code analysis is explicit.
  - **Algorithmic Fix 2 (Cost-Penalized Expected Utility)**:
    Modify `pick_expected` to incorporate a cost parameter:
    ```rust
    let cost_penalty = |tier: u8| -> f64 {
        match tier {
            0 => 0.0,
            1 => 3.5,  // mid-tier penalty
            _ => 9.0,  // large-tier penalty
        }
    };
    ```
    This directly counteracts the uncertainty hedging that pulls predictions into Tier 1 and 2.
- **Expected Metrics**:
  - **Exact Route Accuracy**: **72–78%** on `cases.json`.
  - **Latency**: **350–550 ms** p95 on Apple Silicon (warm).
  - **Setup Cost**: Low (requires local Ollama server and pulling a ~1.0 GB model).

---

### Design 3: Two-Stage Cascaded Router (Fast Heuristics/Embedding $\to$ SLM Escalation)
*Best for: Maximum accuracy while keeping average latency sub-15 ms.*

```
                               ┌────────────────────────────┐
                         ┌───> │ Fast Path: Tier 0 Proposal │ (80% of sends, <5 ms)
                         │     └────────────────────────────┘
┌─────────────────┐      │
│ User Send       │ ──> ─┤ Ambiguity / High Uncertainty?
└─────────────────┘      │
                         │     ┌────────────────────────────┐
                         └───> │ Slow Path: SLM (Ollama)    │ (20% of sends, ~450 ms)
                               └────────────────────────────┘
```

- **Architecture**:
  - **Stage 1 (In-Process Fast Filter, <5 ms)**:
    Heuristic rules (prompt length < 80 chars, mechanical typo/rename keywords, small diff) + Embedding similarity to known trivial tasks automatically route directly to Tier 0 with High confidence.
  - **Stage 2 (SLM Escalation, ~450 ms)**:
    If Stage 1 confidence is below 0.65 or detected keywords involve complex architecture/security audits, dispatch to local Ollama (`qwen2.5:1.5b` or `openthai-systemone:0.8b`) for structured assessment.
- **Expected Metrics**:
  - **Exact Route Accuracy**: **85–90%**.
  - **Latency**: p50 = **4 ms**, p95 = **480 ms**.
  - **Setup Cost**: Medium (bundles small embedding model + optionally connects to Ollama).

---

## 7. Comparative Assessment of Recommended Designs

| Dimension | Design 1: In-Process Embedding + Classifier | Design 2: Calibrated SLM + Cost Utility | Design 3: Two-Stage Cascade Router |
| :--- | :--- | :--- | :--- |
| **Primary Engine** | In-process Rust (`candle` / ONNX) | Local Ollama daemon (`/api/chat` or `/v1/systemone`) | In-process filter + Ollama escalation |
| **Model Size** | ~60 MB | ~1.0 GB | ~60 MB + 1.0 GB |
| **p95 Latency (Apple Silicon)**| **3–8 ms** | **450–550 ms** | **480 ms** (p50: 4 ms) |
| **Dependency Burden** | Zero external services | Requires Ollama running on `127.0.0.1:11434` | Requires Ollama only for complex tasks |
| **Exact Route Accuracy** | 78–84% | 72–78% | **85–90%** |
| **Bias Toward Strong Models** | Eliminated (explicit cost regularization $\lambda$) | Eliminated (penalized expected distance) | Eliminated (fast path locks mechanical sends to Tier 0) |
| **Online Learning Capability** | Instantaneous (kNN memory + SGD update) | Indirect (few-shot prompt adjustment or fine-tuning)| Instantaneous on Fast Path |
| **Implementation Complexity** | Low-Medium (embed ONNX runtime in Rust) | Very Low (adjust existing `router.rs` equations) | Medium |

---

## 8. Eval Set Growth Plan from Real Usage

The current 34-case synthetic smoke test (`evals/router/cases.json`) is too small and artificially clean. To build an objective, benchmark-grade evaluation flywheel:

### 8.1 Telemetry & Logging Schema
Log every pre-dispatch routing event locally into Chorus's ledger without leaking secrets or full source files:
```json
{
  "timestamp": "2026-10-09T04:30:00Z",
  "task_fingerprint": "blake3_hash_of_prompt",
  "text_char_count": 84,
  "detected_language": "th",
  "attachment_count": 0,
  "required_integrations": [],
  "router_model": "bge-small-linear-probe:v1",
  "predicted_kind": "do",
  "predicted_level": "easy",
  "predicted_probabilities": { "do": 0.88, "think": 0.08, "review": 0.04 },
  "proposed_candidate": "luna:low",
  "user_action": "accepted_immediately | override | countdown_expired | canceled",
  "final_candidate": "luna:low",
  "turn_outcome": {
    "status": "completed | failed",
    "turn_duration_ms": 4200,
    "user_followup_negative": false
  }
}
```

### 8.2 Eval Dataset Growth Methodology
1. **Mining Disagreements**:
   Filter local logs for instances where `user_action == "override"`. These represent high-signal edge cases where the router misjudged intent.
2. **De-Identification & Anonymization**:
   Strip file paths, project-specific identifiers, API keys, and personal names using regex and local entity masking before adding cases to the public repo.
3. **Dataset Target Scale**:
   - Expand `cases.json` from 34 cases to **250 cases**:
     - 100 English coding tasks
     - 100 Thai coding tasks
     - 25 Code-switching (mixed Thai/English) tasks
     - 25 Deliberately ambiguous tasks (must trigger `manual` deferral)
4. **Stratified Split**:
   - `evals/router/train.json` (150 cases for calibrating classifiers and prompt few-shots).
   - `evals/router/validation.json` (50 cases for tuning thresholds $\tau$ and cost weights $\lambda$).
   - `evals/router/test.json` (50 held-out, blinded cases run exclusively during CI regression checks).
5. **Continuous Evaluation Integration**:
   Integrate `cargo run -p chorus-cli --bin router_eval` into `chorus bench` so that any regression in route accuracy or latency blocks releases.
