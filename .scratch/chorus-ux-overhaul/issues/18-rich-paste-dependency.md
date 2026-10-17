# Rich paste converter dependency

Type: grilling
Status: resolved
Blocked by: —
Parent: ../map.md
Labels: wayfinder:grilling

## Question

Should `htmd` (Apache-2.0, HTML to Markdown) and `pulldown-cmark` be added to the Rust core for rich paste and composer styling? `htmd` needs Rust 1.85+ (edition 2024), so this also means raising the workspace `rust-version` from 1.82. If not, which alternative is acceptable? See [research 04](../research/04-composer-paste-and-preview.md).

## Answer

Perth approved option (a) on 2026-10-17:
- Add `htmd` (Apache-2.0, HTML to Markdown) and `pulldown-cmark` (MIT, Markdown spans) to `chorus-core`.
- Raise the workspace `rust-version` from 1.82 to 1.85, because `htmd` uses edition 2024. The local toolchain is rustc 1.92. Check the release CI toolchain when implementing.
- The security triage rules apply: cap the pasteboard HTML size, the output size, nesting depth and conversion time, and fall back to plain text past a limit.
