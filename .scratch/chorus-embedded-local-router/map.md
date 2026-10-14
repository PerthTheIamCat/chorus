# Chorus zero-Ollama 1-click local router

## Destination

An implementation-ready architectural spec and execution plan to replace external Ollama dependencies in Chorus with an embedded in-process GGUF inference engine (`llama.cpp` + Metal via Rust), a 1-click in-app model download manager (Hugging Face / CDN to `~/Library/Application Support/Chorus/models/`), dual-use runtime for pre-dispatch routing and conversation compaction, and instant fail-closed fallback to fast heuristics.

## Notes

- **Platform**: macOS only, optimized for Apple Silicon (Metal acceleration).
- **Zero external daemons**: No Ollama installation, no terminal commands, no background brew services required by the user.
- **Logic in Rust, UI in SwiftUI**: Model management, download streams, checksum verification, and GGML inference live in `crates/chorus-core`. SwiftUI in `apps/macos` only presents progress, buttons, and state indicators.
- **Scope**: Router (`crates/chorus-core/src/router.rs`) and Compaction (`crates/chorus-core/src/compaction.rs`).
- **Fail-closed & Instant fallback**: If no model is installed, Chorus falls back seamlessly to in-process heuristics (`local_router.rs`) with 0ms latency.
- Research output goes to `./research/NN-<slug>.md`. Cite primary sources.

## Decisions so far

<!-- the index: one line per closed ticket, enough to judge relevance, then zoom the link for the detail the ticket holds -->

- [In-process GGUF engine binding and Metal integration](./issues/01-engine-binding-selection.md): Recommended thin static `llama.cpp` C++ FFI via `cc` crate (zero CMake dependency), `-DGGML_METAL_EMBED_LIBRARY=ON` for runtime shader compilation without `xcrun metal`, 15–30 ms `mmap` UMA zero-copy cold start, and native GBNF grammar constraints for 100% JSON schema conformity.
- [Local model weight selection and GGUF quantization](./issues/02-model-selection-and-quantization.md): `openthai-systemone:0.8b-q4_k_m` (529 MB, Apache 2.0) is the routing champion (50.0% accuracy, 380 ms p95 on Metal). Dual-use was invalidated because OpenThai is a slot head without an LM generator and 0.5B generative models fail both routing calibration and 4k-token code compaction. Retained model-free `Extractive` compaction as 0-byte default, with optional `qwen2.5-coder:1.5b` if neural compaction is enabled.
- [In-app 1-click model download manager and storage lifecycle](./issues/03-model-download-manager.md): Storage in `~/Library/Application Support/Chorus/models/` with Darwin `statvfs` pre-download disk space guard (1.5 GB minimum), native Swift `URLSessionDownloadTask` with KVO `task.progress` tracking, and atomic temp file finalization in Rust `ModelManager`.
- [Unified inference runtime migration for Router and Compaction](./issues/04-router-compaction-migration.md): Abstracted behind `LocalInferenceEngine` trait (`crates/chorus-core/src/inference.rs`), added `Router::route_with_fallback()` with 0ms delegation to in-process heuristics when offline, and upgraded `compaction::best()` to prioritize in-process weights over legacy Ollama.
- [macOS Settings and Onboarding 1-click UI](./issues/05-macos-settings-ui.md): Replaced legacy Ollama UI with native Apple HIG 1-click model card in `Settings.swift`, offering 1-click download of OpenThai System One (529 MB), animated ProgressView, and storage management.

## Not yet specified

<!-- see "Fog of war": in-scope fog you can't ticket yet; graduates as the frontier advances -->

- Dynamic quantization switching based on available Unified Memory (8GB vs 16GB+ Macs).
- Core ML / Apple Neural Engine (ANE) acceleration export as a future low-power alternative to Metal GPU.
- Extending in-process local model to offline Chat session generation.

## Out of scope

- Building a general-purpose model manager / app store (like Ollama or LM Studio CLI).
- Supporting non-macOS platforms (Windows/Linux) for local GGUF Metal execution.
- Training or fine-tuning weights locally inside Chorus.
