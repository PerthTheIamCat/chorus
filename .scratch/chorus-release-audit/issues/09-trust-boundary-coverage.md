# Which Git metadata paths must the agent sandbox protect?

Parent: ../map.md
Labels: wayfinder:task
Type: task
Mode: AFK
Status: resolved
Assignee: codex
Blocked by: 08

## Question

Verify and correct Git/worktree sandbox protection, including gitfile/commondir, shared metadata, nested working directories and symlink aliases. Use synthetic fixtures and preserve existing work.

## Scope update

The original broader remaining-trust-boundaries question is retained in [Which remaining capability, integrity and release checks need proof?](10-remaining-release-coverage.md). This resolution covers the confirmed fixed-path Git metadata write defects; it does not close application security acceptance.

## Answer

[Evidence and compatibility](../git-sandbox-fixes.md): actual Git admin/common config/hooks and routing entries are protected; parent directory relocation and symlink alias bypasses are denied. Invalid discovery/path inputs fail closed. Ordinary workspace/index writes remain allowed. 221 Rust tests and FFI build pass; remaining capability, configurable Git paths, advisory and performance checks stay open.
