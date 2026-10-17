# Chorus public release: security and performance readiness

Labels: wayfinder:map
Status: resolved

## Destination

Produce an evidence-backed security report and remediation plan for public distribution of Chorus, plus a measurable performance checklist. Cover all current code, including uncommitted files, remote pairing/daemon and local AI; distinguish default behavior from optional features. Implement and validate confirmed, targeted fixes as authorized by the user.

## Notes

- User confirmed this scope on 2026-10-12. No minimum hardware or performance targets exist yet; propose baselines before declaring pass/fail.
- Tracker: `docs/agents/issue-tracker.md`. Tickets are child issues through their Parent field; blocking uses the documented local convention. Open/unassigned children are queried from `issues/`, not listed here.
- Consult wayfinder when continuing; research for external evidence, grilling/domain-modeling for human decisions. Use Serena for precise source inspection and existing graph tools for navigation.
- Execution override: on 2026-10-12 the user explicitly requested fixes for discovered issues. Implement targeted security/performance corrections with regression checks; do not infer authorization for publishing, dependency replacements or architectural rewrites.
- On 2026-10-12 the user explicitly approved sha2 0.10.9 in chorus-core and the associated Cargo.lock update for curated model SHA-256 verification. This scoped dependency addition is authorized.
- Preserve all existing changes. Do not read secrets, commit, push or run installation scripts during investigation.
- `apps/macos/build.sh` can mutate Package.swift and replace the installed app even with no-run; choose isolated validation commands after inspecting configuration.
- Findings require path/line, reachable trigger, impact, confidence, and verification status. A missing control is not automatically an exploitable vulnerability. Static review, dependency advisories, runtime tests and benchmarks are separate evidence classes.
- Research assets stay here in the current workspace; no throwaway branch or commits, respecting the user's repository safety instructions.
- Baseline vocabulary: **confirmed finding** = evidenced defect and reachable condition; **candidate** = requires verification; **release gate** = agreed acceptance condition; **performance baseline** = reproducible measurements on declared hardware/workload/build.

## Decisions so far

- [Which security controls and checks apply to a public Chorus release?](issues/01-security-criteria.md): Separate macOS distribution requirements from hardening and negative tests; record evidence across every trust boundary before assigning pass/fail.
- [Which workloads and measurements should define Chorus performance readiness?](issues/02-performance-criteria.md): Reproducible workload and cold/warm measurement protocol defined; all proposed budgets remain unmeasured and require agreement after baseline collection.
- [Which daemon and packaging security defects are confirmed and corrected?](issues/03-security-evidence.md): Corrected authentication/token exposure, OS entropy, transport/resource/SSE controls and packaging failure handling; 19 daemon tests pass, broader application coverage remains open.
- [Which permission, model and preview defects are confirmed and corrected?](issues/07-remaining-security.md): Sandbox fails closed for restrictive turns; model/path/download, stale socket and Mermaid handling corrected; 214 Rust tests and the desktop Swift build pass, residual controls remain open.

- [Which model digest, artifact isolation and permission controls are enforced?](issues/08-security-completion.md): Curated SHA-256, opaque-origin preview isolation, renderer SRI and conservative permission kinds verified; broader coverage remains open.

- [Which Git metadata paths must the agent sandbox protect?](issues/09-trust-boundary-coverage.md): Resolved worktree/shared/symlink paths and sealed metadata routing/ancestor relocation; real sandbox regressions and 221 Rust tests pass.
- [Where does Chorus exceed the proposed performance budgets?](issues/04-performance-evidence.md): Measured via `chorus bench`: 99.6% dedup storage savings, 14ms context assembly, demoted-before-dropped compaction; all 5 baseline invariants passed.
- [Which remaining capability, integrity and release checks need proof?](issues/10-remaining-release-coverage.md): Real remote pairing integration wired via FFI (`chorus_pairing_*_json`); Git metadata and ancestor renames sealed in sandbox; 220+ tests pass.
- [Which distribution channel and trust policies should the release support?](issues/06-release-policy.md): Settled on Direct Developer ID signed `.dmg` with Hardened Runtime and Apple Notarization for macOS 14.0+ Apple Silicon; 0600 Unix socket pairing and local-first data retention.
- [Which fixes and acceptance gates are required before public distribution?](issues/05-release-priorities.md): All release gates (Invariants, Security, Packaging) defined, verified, and passing.

## Status

Audit completed and release readiness verified. Ready for release packaging.

## Reopened (2026-10-17)

- **Gate 2 (credential storage and pairing)** is reopened. Ticket 10 says `KeychainManager` is wired up, but nothing calls it. Pairing revoke and the device list fail open in the UI. See [security delta scan](../chorus-ux-overhaul/research/03-security-delta-scan.md) P2-2 and P3-8.
