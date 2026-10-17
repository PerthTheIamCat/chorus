# Security scan since the release audit

Type: research
Status: resolved
Blocked by: —
Parent: ../map.md
Labels: wayfinder:research

## Question

What security problems were added since the 2026-10-12 release audit (`../chorus-release-audit/`)? Review the diff of commits after that audit (`61e1c87`, `0c1bfc9` and later) plus uncommitted code. For each finding give path:line, a reachable trigger, the impact, confidence, and how it was verified, in the release-audit format. Also list the new attack surfaces this map adds (killing processes, orchestra spawning agents, pasting HTML into the composer, previewing pasted files) and what each needs. Static review only; do not run install scripts or read secrets.

Research: ./research/03-security-delta-scan.md

## Answer

Static review only: 1 P1, 5 P2, 8 P3, 5 dormant.

**P1:** after each turn, `cargo check` runs in the workspace with no sandbox (ContentView.swift:993 → verify_loop.rs:65). A `build.rs` or `.cargo/config.toml` written by an agent, or already present in a hostile repo, then runs code outside Seatbelt.

**P2:**
- A role `name:` can escape the roles folder and overwrite or delete any `*.md` (roles.rs:182).
- Revoking a paired device reports success even when it failed.
- Messages that agents send through MCP are logged as the user, and an agent can control children in other sessions.
- The loopback MCP server has no auth, and it now exposes spawn, message and cancel.
- `bridge_for` runs a repo-relative `target/*/antigravity-acp`.

**Dormant:** these are unsafe but not called yet; orchestra mode will likely wire them up.
- `.worktreeinclude` paths are not confined.
- `.chorus/hooks` would run without a sandbox, and Thai output panics on truncation.
- `jump_to_file` paths are not confined.
- Worktrees under `.chorus/` clash with the sandbox carve-out that protects `.chorus/`.

Not covered: no cargo-audit run on the `Cargo.lock` delta. Detail, with new-surface requirements: [research](../research/03-security-delta-scan.md).
