# In-process GGUF engine binding and Metal integration

Type: research
Status: resolved
Blocked by:

## Question

Which Rust in-process runtime (e.g. `llama-cpp-2` / `llama_cpp_rs` vs `candle-transformers` vs static `llama.cpp` C++ FFI with Metal) best balances compilation time, Apple Silicon Metal acceleration, Universal Binary compatibility, and binary footprint for Chorus?

Specifically investigate:
1. Compilation requirements on macOS (Xcode Command Line Tools, Metal shading language compilers `xcrun metal`, `cmake`).
2. Cold start latency to load a ~500MB–1GB GGUF model into Unified Memory via `mmap`.
3. In-process inference overhead, memory footprint, and how it handles concurrency / thread safety with Tokio and Rust async runtime.
4. Support for structured output / grammar constraints (GBNF) for JSON routing outputs (`{"recommended_agent": "...", "recommended_model": "..."}`).

## Answer

- **Recommended Runtime**: Thin static `llama.cpp` C++ FFI via the Rust `cc` crate (or pinned `llama-cpp-2` with vendored C++).
- **Zero CMake Requirement**: Standard macOS Xcode CLT lacks `cmake`. Using the Rust `cc` crate compiles directly with `/usr/bin/clang`, eliminating CMake/Homebrew dependencies.
- **Embedded Metal Shaders**: Compiling with `-DGGML_METAL_EMBED_LIBRARY=ON` embeds the Metal shader source directly in the binary string, compiling at runtime via `[MTLDevice newLibraryWithSource:options:error:]`. This completely avoids `xcrun metal` build requirements and missing `.metallib` bundle crashes.
- **Cold Start**: POSIX `mmap` + `newBufferWithBytesNoCopy` in Apple Silicon UMA maps 500MB–1GB weights into virtual memory in **15–30 ms**.
- **Structured JSON (GBNF)**: Built-in `llama_sampler_init_grammar` guarantees 100% JSON schema conformity and cuts generated token count from ~80 to ~25.
- **Universal Binary**: Apple Silicon targets Metal GPU; Intel x86_64 targets AVX2 CPU SIMD fallback.

Detailed findings: [01-engine-binding-selection.md](../research/01-engine-binding-selection.md).
