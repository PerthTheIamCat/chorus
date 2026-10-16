# Permission, model and preview corrections

Date: 2026-10-12. Continuation of the user-authorized fixes. This is a bounded source audit with the evidence below, not complete application security or performance acceptance.

## Confirmed findings corrected

| Priority | Evidence before correction | Resulting behavior |
|---|---|---|
| P1 | CLI one_turn_decide fell back to the original bridge on sandbox::wrap error, including read-only mode. GUI logging defaults off | A restrictive turn fails before spawning an unconfined agent. Explicit non-restrictive policy retains its existing fallback behavior |
| P1 | ReadOnly sandbox inherited writable temporary/cache allowances when the workspace was itself in one of those trees | Add an explicit workspace write denial. A real sandbox-exec regression verifies temp workspaces remain protected while Workspace mode can write |
| P2 | Model installation accepted 80% of expected bytes, ignored ModelSpec.blake3_hash, accepted non-model content and unconstrained filenames | Require a regular file, exact pinned size and GGUF magic; stream-verify BLAKE3 when a digest is supplied before rename. Reject filenames containing traversal/absolute/nested paths. Status rejects corrupt or symlinked files without hashing a large model on each UI poll |
| P2 | Curated model URLs used mutable main; Qwen size was inaccurate | Pin current official revisions; update Qwen size to 924,456,000 bytes. OpenThai remains 529,313,920 bytes. Model identities and filenames are unchanged |
| P2 | URLSession temporary download was queued for installation after its completion handler returned; successful HTTP error responses were not checked | Move the temp file into app-owned temporary storage before returning; require HTTPS and HTTP 200; retain downloading state through installation, guard against stale callbacks, clean staging files after installation/failure |
| P2 | Daemon stale-socket cleanup removed any existing file after any connect error and ignored permission/removal errors | Refuse regular files and symlinks, remove only an actual Unix socket with ConnectionRefused, propagate other errors |
| P2 | Mermaid code was inserted into HTML with JavaScript-style escaping and securityLevel loose | Escape HTML text characters, use Mermaid strict, and use a separate nonpersistent website data store for Canvas |
| Readiness | Settings reported up-to-date after a timer without contacting any release source | Report that no update source is configured; no false up-to-date result |

## Validation

- `cargo test -p chorus-core -p chorus-acp -p chorus-cli -p chorus-daemon --locked`: 214 passed, 3 ignored. The filtered-test count in the tool summary comes from the subprocess regression invoking itself with --exact.
- Regressions cover disabled sandbox fail-closed behavior, real temp-workspace denial, invalid size/header/digest, filename confinement, symlink handling, preservation of existing models and preservation of non-socket files.
- `cargo build -p chorus-ffi --locked`: passed.
- `swift build --package-path apps/macos`: passed, including changed Engine, Settings and Canvas code. Earlier full rebuild reported existing weak/strong capture warnings in unrelated Engine methods.
- [Offline Canvas regression](verify_canvas.py) compiled the actual Canvas source and loaded generated HTML in a real WKWebView. Offline Mermaid initialization verified strict mode and that a closing-div/script payload did not execute. Fixture artifacts were removed. Run with `python3 .scratch/chorus-release-audit/verify_canvas.py`.
- `cargo clippy -p chorus-core -p chorus-acp -p chorus-cli -p chorus-daemon --all-targets --locked --no-deps`: exited successfully, 0 errors and 29 warnings in existing code; no blanket lint cleanup performed.
- rustfmt check on model_manager.rs, sandbox.rs and the changed daemon files: passed. Tracked CLI files were edited narrowly without a full-file reformat.
- `git diff --check` on touched tracked files found an existing trailing-whitespace line in Engine's model response definition, outside the edits; preserved rather than performing unrelated cleanup.
- `cargo audit --version`: unavailable (cargo-audit is not installed). No advisory scan or tool installation was performed.
- No real model files were downloaded; lifecycle tests now use a small GGUF fixture rather than a 500 MB allocation. Real CDN/download cancellation and installation flow remain untested end to end. No app installation, signing or desktop benchmark was run.

## Official sources and checked facts

- [Apple URLSession download completion](https://developer.apple.com/documentation/foundation/urlsession/downloadtask(with:completionhandler:)-7cuje): move or open the temporary file before the callback returns.
- [BLAKE3 Hasher](https://docs.rs/blake3/latest/blake3/struct.Hasher.html): incremental hashing supports bounded-memory verification using the already-established dependency.
- [Rust filesystem documentation](https://doc.rust-lang.org/std/fs/): symlink_metadata inspects the link itself; metadata checks alone are not a general defense against concurrent filesystem replacement.
- [Mermaid usage/security](https://mermaid.js.org/config/usage): strict encodes label HTML and disables click functionality; this changes the previously permissive diagram behavior.
- [Apple WKWebsiteDataStore](https://developer.apple.com/documentation/webkit/wkwebsitedatastore): nonpersistent data isolates this Canvas instance from persisted website data.
- Official metadata read from [OpenThai Hub API](https://huggingface.co/api/models/iapp/OpenThai-SystemOne-GGUF?blobs=true) and [Qwen Hub API](https://huggingface.co/api/models/Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF?blobs=true) on 2026-10-12. Pinned revisions: OpenThai `020b22d01232e29783da229dbd75ee1f9fe8dba8`, Qwen `f86cb2c1fa58255f8052cc32aeede1b7482d4361`.
- Metadata also supplies SHA-256: OpenThai `9be793559b815eb73cd2478bff9944b11e9e855e9ac9ff04efbdf6a344512da2`; Qwen Q3_K_M `d281a3a0010df03c8a0e3ffebd7f9444a95244fb518f132c5475e4b48d9adb5e`. These are public artifact digests, not credentials. They were not converted into BLAKE3 or falsely claimed as verified downloads.
- The installed official macOS SDK's sys/statvfs.h and sys/_types.h were checked: Darwin fsblkcnt_t/fsfilcnt_t are unsigned int here, matching the existing Rust u32 fields. No evidence justified changing that FFI layout.

## Remaining gaps and compatibility

- Curated specs still have no trusted BLAKE3 digest; exact size, GGUF magic and revision pinning do not replace cryptographic artifact authentication. A maintained SHA-256 verification path or independently produced BLAKE3 digests is still needed before treating downloads as cryptographically authenticated.
- Installed-file status intentionally avoids a full hash scan on every UI refresh. Disk tampering after installation and adversarial concurrent file replacement were not eliminated by these checks.
- Read-only sandbox limits workspace writes but retains its existing readable/network/cache/config allowances. It does not enforce a comprehensive deny-all capability policy; provider permission metadata and shell/network mediation need further review.
- General SVG/HTML Canvas remains active content, and LivePreview's navigation/network isolation is not complete. Mermaid strict/nonpersistent data do not constitute a complete HTML sandbox.
- RemotePairingView remains a mock integration; updater.rs parses metadata but no signed update installer exists. Release channel, signer identity and remote privilege policies remain human decisions.
- Cancel is effective during transfer; once installation has been queued, cancellation is no longer offered by the method. Failed transfer callbacks cannot overwrite a newer download's state.
- Existing public-interface shapes were preserved except internal helper arguments; no production dependency, schema or lockfile changes were introduced.
