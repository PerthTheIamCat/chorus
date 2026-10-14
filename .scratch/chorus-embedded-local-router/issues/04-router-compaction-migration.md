# Unified inference runtime migration for Router and Compaction

Type: task
Status: resolved
Assignee: Antigravity
Blocked by:
- [In-process GGUF engine binding and Metal integration](./01-engine-binding-selection.md)
- [Local model weight selection and GGUF quantization](./02-model-selection-and-quantization.md)

## Question

How to refactor `crates/chorus-core/src/router.rs` and `crates/chorus-core/src/compaction.rs` to abstract behind a unified `LocalInferenceEngine` trait, replacing Ollama HTTP requests with the embedded engine and wiring seamless fallback to `local_router::route_task_fast`?

Specifically investigate and design:
1. `LocalInferenceEngine` trait definition: `generate_completion(&self, prompt: &str, grammar: Option<&str>) -> Result<String>` and `tokenize(&self, text: &str) -> Result<Vec<u32>>`.
2. Seamless coexistence / migration: If the embedded model file is not yet downloaded, Router instantly delegates to `local_router::route_task_fast` (rule-based + SQLite learned memory) with 0ms delay, and Compaction delegates to extractive text truncation.
3. Decoupling and removing legacy Ollama TCP socket calls (`127.0.0.1:11434`), `/api/chat`, and `/api/pull`.
4. Testing: Unit and integration tests verifying both the mock embedded engine and the fallback path.

## Answer

- **Unified Inference Trait**: Implemented `LocalInferenceEngine` in `crates/chorus-core/src/inference.rs` with `complete()`, `tokenize()`, and `is_ready()` checks, accompanied by `InProcessEngine` and `MockInferenceEngine`.
- **Instant Pre-Dispatch Fallback**: Added `Router::route_with_fallback()` in `crates/chorus-core/src/router.rs`. If the neural model / Ollama endpoint is offline or not installed, the router automatically delegates to `local_router::route_task_fast()` in 0ms, generating a calibrated `RouteResult::Proposal` without blocking or erroring.
- **Compaction Upgrades**: Added `InProcessSummarizer` in `crates/chorus-core/src/compaction.rs` and upgraded `best()` to check for downloaded in-process weights before checking legacy Ollama or defaulting to `Extractive`.
- **FFI Integration**: Wired `chorus_route_json()` in `crates/chorus-ffi/src/lib.rs` to call `route_with_fallback()`, guaranteeing 100% resilient pre-dispatch routing across macOS shells.
- **Test Coverage**: Added `in_process_fallback_when_ollama_is_offline` test in `router.rs` and passed all 63 unit and 10 integration test suites in `chorus-core`.
