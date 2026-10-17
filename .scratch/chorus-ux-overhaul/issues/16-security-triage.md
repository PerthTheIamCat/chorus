# Security findings triage

Type: grilling
Status: resolved
Blocked by: 03
Parent: ../map.md
Labels: wayfinder:grilling

## Question

Which findings from the security scan get fixed, in what order, and which new-surface rules become part of the spec?

## Answer

Perth accepted every recommendation (2026-10-17). See [research 03](../research/03-security-delta-scan.md) for the finding IDs.

1. **P1, verify after a turn:** off by default. A per-project "trust this project" switch turns it on, and even then it runs inside the turn's Seatbelt profile. Opening a repo or ending a turn runs nothing without consent. The same rule covers hooks (D2) and the biome/ruff/swiftlint linters. Sandboxing alone is not enough, because Seatbelt only limits writes and a hostile `build.rs` can still read and exfiltrate.
2. **Fix now, as one security batch:**
   - **P1.**
   - **P2-1:** role names must match `[a-z0-9-]{1,64}` on load.
   - **P2-2:** revoke and the device list show failure honestly, with no fallback to an empty manager.
   - **P2-3 and P2-4:**
     - Every MCP call carries a per-session secret through `mcpServers[].headers`.
     - The scope comes from that credential, never from arguments, and each `agent_id` must belong to the caller's session tree.
     - Ledger writes go through `ledger::append` as the agent's actor, never as `user`.
   - **P2-5:** remove the cwd-relative `target/*` bridge paths.
3. **P3-1:** remove the CSRF token scraping and replay. Antigravity quota shows "unknown".
4. **P3s in the same batch:**
   - P3-2: the prompt goes over stdin or a file, not argv.
   - P3-3: the QR advertises the port the daemon actually bound.
   - P3-5: the client caps `Content-Length`.
   - P3-7: Spotlight indexing is off by default, dictation sets `requiresOnDeviceRecognition`, and Info.plist gains the usage keys plus `LSMinimumSystemVersion` 14.0.

   P3-4 (attempt budget, constant-time compare) and P3-6 (agent notifications) wait for orchestra mode.
5. **Review baseline:** the baseline is `HEAD` at session start, or the user's last "Mark reviewed". Commits made while a turn runs count as the agent's and stay in review. Commits made between turns count as the user's and move the baseline forward. Passed to Review baseline for diffs.
6. **All new-surface requirements in research §4 become spec requirements:** killing processes, orchestra spawning, HTML paste, file previews. The key rules are that a child is never wider than its parent, and agents get no MCP tool to kill processes.
7. **D4:** worktrees move out of `.chorus/` to `~/Library/Application Support/Chorus/worktrees/`. This overrides harness-roadmap ticket 12.
8. **P3-8:** reopen release-audit Gate 2 (credential storage and pairing).
9. **Execution:**
   - Separate security-fix session in its own worktree, with a regression test for each fix and no commit.
   - Perth approved installing `cargo-audit` to check `Cargo.lock` against RustSec.

## Comments

- 2026-10-17: installed `cargo-audit` and ran `cargo audit` on the main checkout's `Cargo.lock`: 1334 advisories loaded, 66 crates scanned, 0 vulnerabilities, 0 warnings (exit 0). This closes the "not checked" dependency gap in research 03 §5 for the current lockfile.
