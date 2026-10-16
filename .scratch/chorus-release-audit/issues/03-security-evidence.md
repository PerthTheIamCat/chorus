# Which daemon and packaging security defects are confirmed and corrected?

Parent: ../map.md
Labels: wayfinder:task
Type: task
Mode: AFK
Status: resolved
Assignee: codex
Blocked by: 01

## Question

Collect code, dependency and isolated-test evidence needed to decide security remediation priority. Cover current source, trace controls to live call sites, and distinguish default and optional exposure. For each candidate record exact location, prerequisites, impact, severity rationale, reproduction or missing proof, proposed smallest fix and regression check. Implement targeted confirmed fixes per user authorization on 2026-10-12. Inspect configured validation before running anything. Do not execute untrusted repository hooks or agents, read secrets or mutate the app installation.

Initial orientation targets (not confirmed vulnerabilities): release signing/notarization in build.sh and package-dmg.sh; sandbox/permissions and spawned bridges; daemon authentication/pairing and HTTP bounds; preview navigation; updater/model download integrity; filesystem confinement; sensitive data persistence and logs. Document audit coverage and blind spots.

## Scope update

The user authorized fixes on 2026-10-12. This ticket was narrowed to reachable daemon and packaging corrections; remaining source/dependency security investigation is preserved as a separate child ticket rather than represented as completed coverage.

## Answer

Resolved 2026-10-12 by codex. [Confirmed daemon and packaging corrections](../fixes.md) records findings, targeted fixes, compatibility effects and validation. Authentication/token leakage, entropy, loopback binding, SSE revocation/scoping, resource bounds and packaging failure handling are corrected. All 19 daemon tests pass; strict Clippy is blocked by untouched existing lint, and the scoped lint run passes. Real distribution signing and full application security/performance remain unverified.
