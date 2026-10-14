# Which model digest, artifact isolation and permission controls are enforced?

Parent: ../map.md
Labels: wayfinder:task
Type: task
Mode: AFK
Status: resolved
Assignee: codex
Blocked by: 07

## Question

Implement and prove trusted digest verification before curated model installation, generated HTML/SVG/LivePreview isolation and conservative provider permission-kind classification. Use synthetic fixtures and preserve existing work. Adding a production crypto dependency requires the applicable approval.

## Scope update

The original broader coverage question is retained as [Which remaining capability, integrity and release checks need proof?](10-remaining-release-coverage.md). Resolving these specific controls does not resolve application-wide security acceptance.

## Answer

[Implementation, evidence and compatibility](../digest-preview-fixes.md): after explicit sha2 approval, curated models require SHA-256 before install; generated artifacts use opaque-origin sandbox/CSP/navigation controls; Mermaid is pinned with SRI; ReadOnly denies mixed/unknown permission kinds. 217 Rust tests, FFI build, normal/fresh Swift builds and real WKWebView regressions pass. Advisory, performance and remaining trust-boundary coverage are still open.
