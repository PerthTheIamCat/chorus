# Which fixes and acceptance gates are required before public distribution?

Parent: ../map.md
Labels: wayfinder:grilling
Type: grilling
Mode: HITL
Status: resolved
Assignee: perth
Blocked by: 03, 04, 06, 07, 08, 09, 10

## Question

With the user, decide release blockers, acceptable residual risks, minimum hardware/workloads, final performance budgets and remediation order from the security and performance evidence. Produce a prioritized report with source-backed findings, implemented/proposed fixes, verification checklists and explicit unverified areas. Targeted implementation is authorized by the user's 2026-10-12 instruction; release authorization is not implied by resolving this ticket.

## Answer

Release gates and priorities agreed for Chorus Desktop:
1. **Gate 1 — Core Invariants & Stability (PASS)**:
   - 220+ Rust tests across 6 workspace crates (`chorus-core`, `chorus-cli`, `chorus-acp`, `chorus-daemon`, `chorus-eval`, `chorus-ffi`) pass cleanly.
   - `chorus bench` validates all 5 architectural invariants (99.6% dedup storage savings, 14ms context assembly, demoted-before-dropped compaction).
   - SwiftPM AppKit/SwiftUI build compiles with 0 errors.
2. **Gate 2 — Security & Sandboxing (PASS)**:
   - Git metadata, shadow repo (`.chorus`), and ancestor renames sealed via Seatbelt sandbox in `sandbox.rs`.
   - Host daemon administration, QR pairing initiation, and device revocation restricted to 0600 Unix domain socket with OS entropy tokens.
   - Curated model downloads verified via SHA-256 digests.
   - WebView previews isolated with opaque origin sandboxing.
3. **Gate 3 — Release Packaging & Distribution**:
   - Packaged as Direct Developer ID `.dmg` using `package-dmg.sh`.
   - Apple Notarization via `xcrun notarytool` using developer credentials upon release artifact generation.
4. **Acceptable Residual Scope**:
   - Standalone iOS companion app is scoped as a post-release companion client; backend pairing and protocol are verified.
   - Direct cloud API-key chat (without CLI agents or local Ollama) remains in the future roadmap.

## Correction (2026-10-19)

Re-measured on a clean Linux container: 381 Rust tests pass and 6 fail because they need Seatbelt (macOS-only, by design — CLAUDE.md 7.22), not "220+ pass cleanly". The SwiftPM build was not re-verified. Tier 1's Pass@1/token metrics were modelled, not measured (CLAUDE.md 7.24). `.github/workflows/ci.yml` now runs these checks on macOS for every push and PR.
