# Model digest, artifact isolation and permission corrections

Date: 2026-10-12. Targeted implementation authorized by the user; sha2 0.10.9 and Cargo.lock changes explicitly approved after review. Public-release acceptance remains open.

## Confirmed findings and corrections

| Priority | Source / reachable trigger before correction | Impact and resulting control | Evidence |
|---|---|---|---|
| P1 | `crates/chorus-core/src/model_manager.rs:250`: curated model install checked size/header but supplied no trusted digest | A same-sized modified GGUF could pass validation. Curated OpenThai and Qwen now require pinned SHA-256 before rename; malformed or mismatched hashes fail and preserve the installed model | Synthetic same-size tampering, malformed digest and tampering beyond the first buffer rejected; full real downloads untested |
| P1 | Canvas/LivePreview generated HTML and SVG accepted active content without a complete origin/network boundary | Untrusted preview content could attempt network requests and navigation. Shared `ArtifactPreviewSecurity.swift:10` wraps it in an opaque-origin iframe with allow-scripts only, restrictive CSP, nonpersistent WK storage and navigation filtering | Real WKWebView tests deny parent access/storage/fetch/form/image/frame/script/CSS network attempts, top navigation and popup attempts |
| P2 | `crates/chorus-cli/src/session.rs:78`: ReadOnly matched kind text containing read | Mixed or unknown kinds such as read/write or thread could receive Allow. Exact normalized read/fetch/search kinds are now permitted; others denied | Permanent regression covers mixed/unknown kinds and case normalization; provider metadata trust and actual capabilities still require review |
| P2 | Mermaid renderer fetched a mutable @10 CDN URL with no integrity check | CDN bytes could change without validation. `ArtifactPreviewSecurity.swift:7` pins the already-served 10.9.8 bytes with SHA-384 SRI | Actual Mermaid SVG renders; intentionally wrong SRI blocks execution in WKWebView |

These are confirmed control defects with synthetic reachable triggers, not evidence of exploitation in a user's installation.

## Implementation and compatibility

- Added approved `sha2 = "=0.10.9"` to chorus-core and its lockfile dependencies. Incremental SHA-256/BLAKE3 verification uses one 64 KiB buffer and a single pass. ModelSpec has an optional serde-default sha256_hash; existing serialized custom specs remain readable. Curated models always supply SHA-256. Custom hashless specs remain compatible and are not claimed authenticated.
- Official pinned revisions/sizes/digests match the prior report. Installed-file status does not hash the whole model on each UI refresh. Post-install tampering and concurrent filesystem replacement remain separate open questions.
- Canvas and generated LivePreview HTML share the sandbox. Self-contained inline JavaScript and SVG continue to work. External scripts/styles/fonts/network, storage, forms, popups and embedding are restricted. Mermaid alone may fetch the exact pinned renderer. This deliberately changes previews that depended on external resources.
- LivePreview web mode still supports HTTP/HTTPS dev servers, including localhost ports. Empty, file, javascript, data, custom-scheme and credential-bearing URLs are rejected. Generated artifact mode has no localhost base origin.
- Canvas avoids an unused WKWebView instance and reloads only when HTML changes. This is a targeted reduction of redundant work, not a measured performance result.
- SRI pins bytes; it does not establish that Mermaid has no known vulnerabilities. Infinite loops or expensive scripts in active previews are not fully bounded by these controls.

## Validation

- `cargo test -p chorus-core -p chorus-cli -p chorus-acp -p chorus-daemon --locked`: 217 passed, 3 ignored. Filtered counts include a subprocess regression invoking itself with --exact.
- `cargo build -p chorus-ffi --locked`: passed.
- `cargo clippy -p chorus-core -p chorus-cli --all-targets --locked --no-deps`: passed with 0 errors and 24 existing warnings. No unrelated warning cleanup.
- `rustfmt --edition 2021 --check crates/chorus-core/src/model_manager.rs`: passed.
- `swift build --package-path apps/macos`: passed. A separate fresh temporary scratch-path build also passed and linked the rebuilt Rust FFI. Existing unrelated Engine weak/strong capture warnings remain.
- `python3 .scratch/chorus-release-audit/verify_canvas.py --real-renderer`: passed using actual Swift preview source and real WKWebView. Loopback recorder observed only the permitted dev-server positive control; actual pinned Mermaid renders and wrong SRI is rejected. Default invocation provides an offline renderer stub. Fixtures/temp builds removed.
- Pinned @10.9.8 and previously served @10 returned identical 3,337,857-byte renderer content and SHA-384; this pins the current renderer without a version upgrade.
- No real large-model download, app installation, signing, minimum OS/toolchain matrix, dependency advisory scan or performance benchmark was performed. cargo-audit was unavailable at the prior check. Do not infer those passes from build/test success.

## Sources checked

- [sha2 0.10.9](https://docs.rs/sha2/0.10.9/sha2/): incremental Digest API supports bounded-memory hashing.
- [Pinned OpenThai metadata](https://huggingface.co/api/models/iapp/OpenThai-SystemOne-GGUF/revision/020b22d01232e29783da229dbd75ee1f9fe8dba8?blobs=true) and [pinned Qwen metadata](https://huggingface.co/api/models/Qwen/Qwen2.5-Coder-1.5B-Instruct-GGUF/revision/f86cb2c1fa58255f8052cc32aeede1b7482d4361?blobs=true): exact SHA-256 and byte sizes used by curated specs.
- [HTML iframe sandbox](https://html.spec.whatwg.org/multipage/iframe-embed-object.html#attr-iframe-sandbox) and [CSP](https://www.w3.org/TR/CSP3/): sandbox without allow-same-origin gives an opaque origin; CSP restricts subresource/network execution. WK behavior was tested, including inherited policy for srcdoc.
- [Subresource Integrity](https://www.w3.org/TR/sri/): cross-origin renderer uses integrity and crossorigin; mismatch blocks execution, verified in WKWebView.

## Remaining release work

[Remaining trust-boundary coverage](issues/10-remaining-release-coverage.md) tracks git/worktree sandbox paths, filesystem races/post-install integrity, provider shell/network mediation, credential callers/log retention, dependency advisories and real remote pairing integration. Performance baseline evidence and release-policy decisions also remain open. Existing Keychain code alone does not prove provider credentials use it.
