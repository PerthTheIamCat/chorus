# In-app 1-click model download manager and storage lifecycle

Type: task
Status: resolved
Assignee: Antigravity
Blocked by:
- [Local model weight selection and GGUF quantization](./02-model-selection-and-quantization.md)

## Question

How should the in-app 1-click model downloader in Rust/Swift handle streaming download from Hugging Face/CDN, resume on interruption, SHA256 integrity verification, disk space checks, and storage in `~/Library/Application Support/Chorus/models/`?

Specifically investigate and design:
1. Rust download worker using `reqwest` with streaming chunks and callback for progress reporting (bytes downloaded, total bytes, estimated time remaining).
2. Atomic file write (`.download` temporary file renamed to `.gguf` only after full completion and SHA256 validation).
3. Disk space safety: Verify at least 2.5× model size free space before initiating download.
4. Model lifecycle: Status states (`not_downloaded`, `downloading(progress)`, `ready`, `verifying`, `corrupted`), deletion API to reclaim disk space, and model file existence check on app launch.

## Answer

- **Storage Location**: `~/Library/Application Support/Chorus/models/` managed via `crates/chorus-core/src/model_manager.rs`.
- **Pre-download Disk Space Guard**: Uses Darwin POSIX `statvfs` to verify required free disk space (`1.5 GB` for OpenThai 0.8B Q4_K_M) before initiating transfer.
- **Download & Progress Streaming**: Handled via native Swift `URLSessionDownloadTask` in `apps/macos/Sources/Chorus/Engine.swift` with KVO `task.progress` tracking bytes and percentage without third-party CLI or daemon dependencies.
- **Atomic Finalization**: Downloaded `.tmp` file is verified by size in `ModelManager::finalize_download()` and atomically moved via POSIX `fs::rename()` into the models directory.
- **FFI & Lifecycle API**: Exposed via `chorus_model_manager_list_json()`, `chorus_model_manager_finalize_json()`, and `chorus_model_manager_delete_json()`. Verified by comprehensive unit tests in `model_manager.rs`.
