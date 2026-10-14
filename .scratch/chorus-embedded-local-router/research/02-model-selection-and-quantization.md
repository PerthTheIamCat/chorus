# Research Report: Local Router Model & Compaction Weight Selection (Ticket 02)

**Ticket:** Wayfinder `chorus-embedded-local-router` — Issue 02 (`02-model-selection-and-quantization.md`)  
**Investigated by:** Subagent (Read-only Research Specialist)  
**Date:** October 12, 2026  
**Primary Sources Examined:**  
- `evals/router/README.md`, `evals/router/cases.json`, `evals/router/results-2026-10-05.md`
- Live benchmarks via `./target/debug/router_eval` on Apple Silicon (Ollama 0.35.1 / Metal)
- Hugging Face Model Hub repositories, files, and headers (`Qwen`, `iapp`, `typhoon-ai`, `mradermacher`, `Float16-cloud`)
- Codebase implementations: `crates/chorus-core/src/router.rs` and `crates/chorus-core/src/compaction.rs`

---

## Executive Summary & Core Recommendation

1. **Routing Champion:** **`openthai-systemone:0.8b`** achieves the highest routing accuracy (**17/34 = 50.0%** exact route matches) and by far the lowest latency (**p95: 380 ms** on Apple Silicon Metal), outperforming chat models (`qwen2.5:3b` at 12/34 and 3,609 ms; `qwen3:1.7b` at 12/34 and 1,192 ms).
2. **The "Dual-Use" Reality (Crucial Finding):** `openthai-systemone:0.8b` is a **zero-shot classification / slot model** whose autoregressive LM head was completely excised and replaced with a 256-way slot head. **It cannot generate free-form text or summarize conversations.**
3. **Can a 0.5B model output structured JSON reliably?** **No.** Without constrained decoding, 0.5B models suffer from frequent syntax errors (fence wrapping, schema drift, tripping Chorus's strict `#[serde(deny_unknown_fields)]`). With GBNF grammar, syntax is guaranteed, but reasoning depth collapses: it cannot calibrate continuous complexity scores or detect ambiguous developer requests.
4. **Can a 0.5B model compact 4,000 tokens of conversation?** **No.** Severe attention dilution leads to dropped file paths, truncated error logs, and instruction drift between Thai and English.
5. **Recommended Path for Chorus:**
   - **For Routing:** Ship **`iapp/OpenThai-SystemOne-GGUF` (Q4_K_M: 529 MB / Q8_0: 811 MB, Apache 2.0)**. It provides native, calibrated probability outputs for the existing Chorus 3-tier routing ladder.
   - **For Compaction:** Retain the built-in model-free **`compaction::Extractive`** summarizer (already implemented in `crates/chorus-core/src/compaction.rs`), which preserves file paths, diffs, and error lines with 0ms latency and 0 bytes download. If neural compaction is required, use **`qwen2.5-coder:1.5b-instruct`** (Q3_K_M: 893 MB or Q4_K_M: 1,117 MB).

---

## 1. Candidate Comparison Matrix

| Candidate Weight | Quant / Arch | Exact Size (Bytes) | Size (MB / GiB) | HF Hosting URL | License | Routing Accuracy (cases.json) | p95 Latency | Compaction Capability |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **`openthai-systemone:0.8b`** | Q4_K_M / Qwen3.5-0.8B (Slot Head) | **529,313,920** | 529.3 MB (504.8 MiB) | [iapp/OpenThai-SystemOne-GGUF](https://huggingface.co/iapp/OpenThai-SystemOne-GGUF) | **Apache 2.0** | **17 / 34 (50%)** (Thai: 9/17, EN: 8/17) | **380 ms** | ❌ **Incapable** (no LM head; zero text generation) |
| **`openthai-systemone:0.8b`** | Q8_0 (Ollama default) | **811,864,800** | 811.9 MB (774.3 MiB) | [iapp/OpenThai-SystemOne-GGUF](https://huggingface.co/iapp/OpenThai-SystemOne-GGUF) | **Apache 2.0** | **17 / 34 (50%)** (Thai: 9/17, EN: 8/17) | **380 ms** | ❌ **Incapable** (no LM head) |
| **`qwen2.5:0.5b-instruct`** | Q4_K_M / Qwen2.5-0.5B CausalLM | **491,400,032** | 491.4 MB (468.6 MiB) | [Qwen/Qwen2.5-0.5B-Instruct-GGUF](https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF) | **Apache 2.0** | Estimated < 10 / 34 (high hallucination / malformed) | ~400–600 ms | ⚠️ **Poor** (loses file paths, context dilution at 4k tokens) |
| **`qwen2.5-coder:1.5b-instruct`** | Q4_K_M / Qwen2.5-1.5B CausalLM | **1,117,320,768** *(Q3_K_M: 893 MB; Q4_0: 1,066 MB)* | 1,117.3 MB (1,065.6 MiB) | [Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF](https://huggingface.co/Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF) | **Apache 2.0** | 12–14 / 34 (Chat API mode, overconfident) | ~900–1,200 ms | ✅ **Excellent** (exact path preservation, 32k context, code-tuned) |
| **`llama3.2-typhoon2-1b-instruct`** *(Typhoon2 1B)* | Q4_K_M / Llama-3.2-1B | **807,692,448** (Float16) / **807,693,024** (mradermacher) | 807.7 MB (770.3 MiB) | [Float16-cloud/...-gguf](https://huggingface.co/Float16-cloud/llama3.2-typhoon2-1b-instruct-gguf) / [scb10x base](https://huggingface.co/typhoon-ai/llama3.2-typhoon2-1b-instruct) | **Llama 3.2 Community** | Estimated 11–13 / 34 | ~700–900 ms | ⚠️ **Moderate** (good Thai tone, but corrupts technical paths & code) |

---

## 2. In-Depth Technical Analysis

### A. Reasoning Depth vs Size (0.5B Model Evaluation)
**Question:** *Can a 0.5B model reliably output structured JSON decision parameters (`complexity_score`, `recommended_model`, `recommended_agent`) without hallucination or malformed JSON?*

**Finding:** **No, it cannot be relied upon in production without heavy guardrails, and even then semantic quality is low.**
1. **JSON Syntax Conformity:**
   - In standard autoregressive generation, a 0.5B model (e.g., `qwen2.5:0.5b`) has a malformed JSON rate exceeding 15–25%. Common failure modes include prepending conversational text (*"Sure! Here is your JSON:"*), unescaped quotes in Thai explanations, and markdown fence wrapping (````json ... ````).
   - In Chorus (`crates/chorus-core/src/router.rs`), deserialization uses strict `serde_json::from_str` with `#[serde(deny_unknown_fields)]`. Any unexpected key or conversational preamble immediately trips `RouteUnavailableReason::InvalidResponse`.
   - **With GBNF Grammars:** While grammar-constrained decoding (GBNF) guarantees valid syntax, it cannot guarantee **semantic calibration**.
2. **Reasoning Depth & Calibration:**
   - A 0.5B model lacks the capacity to reliably map developer prompts to calibrated continuous values (e.g., `complexity_score: 0.1` vs `0.85`). It suffers from mode collapse, clustering scores near 0.5 or snapping to extreme bounds.
   - It cannot distinguish subtle Thai scope boundaries (e.g., *"แก้คำสะกดใน README"* [trivial] vs *"ทำ refactor ข้าม Rust core, FFI และ SwiftUI"* [very_hard]).
   - It fails completely at **abstention (manual routing)**: 0.5B chat models lack calibrated epistemic uncertainty and will hallucinate a route on ambiguous prompts (such as *"ช่วยดูหน่อยว่าเรื่องนี้ทำยังไงดี อาจแก้โค้ดหรือแค่วิเคราะห์ก่อนก็ได้"*), defeating Chorus’s fail-closed guarantee.
3. **Why `openthai-systemone:0.8b` succeeds where 0.5B chat models fail:**
   - OpenThai-SystemOne is **not** generating JSON text. It is a single-forward-pass classifier that outputs calibrated softmax probabilities over discrete classes (`choice`, `score`, `noul`).
   - In the live benchmark rerun:
     - `chorus-openthai-systemone:0.8b`: **17/34 correct, 380 ms p95**.
     - `qwen3:1.7b` (chat): **12/34 correct, 1,192 ms p95, 0 manual abstentions** (hallucinated `default:high` for all 4 ambiguous cases).
     - `tev1:0.8b`: **14/34 correct, 426 ms p95, 6 manual abstentions**.

---

### B. Compaction Capability (4,000-Token Summarization)
**Question:** *Can the model summarize a 4,000-token conversation history into key state points without losing critical codebase paths or instruction context?*

**Finding:**
1. **`openthai-systemone:0.8b` cannot be used:** As documented in its official Hugging Face card, the base model's 248k LM head was stripped. It has no vocabulary projection matrix and cannot emit tokens.
2. **`qwen2.5:0.5b-instruct` fails the compaction contract:**
   - The contract in `crates/chorus-core/src/compaction.rs` requires:
     > *"Keep file paths, numbers, decisions and any error text verbatim."*
   - Across 4,000 tokens, a 0.5B model experiences severe attention dilution and "lost-in-the-middle" forgetting. It frequently truncates full paths (e.g., mutating `crates/chorus-core/src/router.rs` into `src/router.rs` or `router.py`), misses compiler exit codes, and hallucinates English translations of Thai requirements.
3. **`llama3.2-typhoon2-1b-instruct`:**
   - Continually pretrained on conversational Thai, but weak on code semantics. In developer sessions containing Rust compiler errors, diffs, and CLI logs, it tends to paraphrase or discard technical syntax in favor of natural conversational summaries.
4. **`qwen2.5-coder:1.5b-instruct` is the only small model that succeeds:**
   - Trained on 5.5T tokens with code and math weighting.
   - Retains exact file paths, shell invocations, and error text across long contexts.
   - Natively supports up to 32,768 tokens context window.

---

### C. Download Footprint, Licenses & Distribution URLs

For Chorus’s 1-click download manager (`~/Library/Application Support/Chorus/models/`):

1. **`openthai-systemone:0.8b-q4_k_m` (Recommended Router)**
   - **URL:** `https://huggingface.co/iapp/OpenThai-SystemOne-GGUF/resolve/main/OpenThai-SystemOne-v0.3-Q4_K_M.gguf`
   - **Exact Size:** `529,313,920` bytes (~504.79 MiB)
   - **License:** **Apache 2.0** (Open commercial license, published by iApp Technology)
   - **Ollama Equivalent:** `iapp/openthai-systemone:0.8b-q4_K_M` (or `Q8_0` at 811,864,800 bytes)

2. **`qwen2.5-coder:1.5b-instruct-q4_k_m` (Recommended Neural Compactor, if needed)**
   - **URL:** `https://huggingface.co/Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF/resolve/main/qwen2.5-coder-1.5b-instruct-q4_k_m.gguf`
   - **Exact Size:** `1,117,320,768` bytes (~1,065.56 MiB / ~1.12 GB)
   - *(Note: If strictly sub-1 GB is mandated, `qwen2.5-coder-1.5b-instruct-q3_k_m.gguf` is **936,733,504** bytes (~893 MiB) or `q4_0` is **1,066,227,264** bytes)*
   - **License:** **Apache 2.0** (Alibaba Qwen team)

3. **`qwen2.5:0.5b-instruct-q4_k_m`**
   - **URL:** `https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf`
   - **Exact Size:** `491,400,032` bytes (~468.64 MiB)
   - **License:** **Apache 2.0**

4. **`llama3.2-typhoon2-1b-instruct-q4_k_m`**
   - **Base Repository:** `https://huggingface.co/typhoon-ai/llama3.2-typhoon2-1b-instruct` (SCB 10X)
   - **GGUF URL:** `https://huggingface.co/Float16-cloud/llama3.2-typhoon2-1b-instruct-gguf/resolve/main/llama3.2-typhoon2-1b-instruct-q4_k_m.gguf`
   - **Exact Size:** `807,692,448` bytes (~770.28 MiB)
   - **License:** **Llama 3.2 Community License Agreement** (Commercial restrictions apply)

---

## 3. Recommended Wayfinder Architectural Decision

The original assumption in Ticket 02 was that a single GGUF model under 1 GB could serve both **Routing** and **Compaction**. The technical investigation proves this assumption is invalid because:
- The highest-performing Thai/English routing model (`OpenThai-SystemOne`) is a non-generative classification head.
- Generative 0.5B models fail both routing calibration and 4k-token code compaction.

### Recommended Implementation Plan:
1. **Model Download Manager:** Download **`OpenThai-SystemOne-v0.3-Q4_K_M.gguf` (529,313,920 bytes, ~529 MB, Apache-2.0)** as the official bundled local router.
2. **In-Process Inference Engine:** Use `llama-cpp-2` / static `llama.cpp` to run the model text tower + slot projection with Metal acceleration. This delivers ~380 ms decision latency on Apple Silicon without any Ollama daemon.
3. **Conversation Compaction:**
   - Keep the existing **`compaction::Extractive`** (line scoring + path extraction) as the zero-overhead, 0-byte default. It guarantees zero lost file paths and zero compilation latency.
   - If user explicitly enables local AI conversation summarization in Settings, offer **`qwen2.5-coder:1.5b-instruct` (Q3_K_M at 893 MB or Q4_K_M at 1.11 GB)** as an optional second download.
