# Which permission, model and preview defects are confirmed and corrected?

Parent: ../map.md
Labels: wayfinder:task
Type: task
Mode: AFK
Status: resolved
Assignee: codex
Blocked by: 03

## Question

Continue the evidence-backed audit of current source/dependencies outside the corrected paths. Trace agent permission mediation and sandbox enforcement to live execution, filesystem confinement and stale socket handling, updater/model integrity, preview isolation, sensitive data storage/log redaction, and dependency advisories. Inspect the desktop RemotePairingView's mock state versus actual daemon authorization and endpoint configuration. Record exact locations, reachable conditions, severity and missing proof; implement targeted confirmed fixes under the user's 2026-10-12 authorization with regression checks. Preserve all unrelated work, avoid secrets and installation side effects. Resolve only when coverage and remaining limitations are explicitly recorded.

## Scope update

Narrowed to the confirmed permission, model download, stale socket and Mermaid/readiness corrections in this session. Unfinished coverage is preserved as a follow-up child ticket and is not counted as audited.

## Answer

Resolved 2026-10-12 by codex. [Permission, model and preview corrections](../remaining-security-fixes.md) records exact behavior, evidence and limits. Restrictive sandbox fallback, temp-workspace writes, model validation/path handling, download lifetime, unsafe socket cleanup and Mermaid HTML injection are corrected; false update readiness is removed. Rust tests and Swift build pass. Cryptographic curated-model authentication, general preview isolation, sensitive-data/advisory checks and remote integration remain open.
