# Merge shell polish

Status: resolved
Blocked by: —
Parent: ../spec.md
Decided in: [Shell polish triage](../../chorus-ux-overhaul/issues/20-shell-polish-triage.md)

## What

Branch `refs/heads/claude/serene-murdock-e347dc` has all 11 triage fixes, uncommitted. Rust tests, bench, clippy, swift build and 5 new Swift tests pass. Menu bar, a menu command and folder collapse were checked in the app.

## Check in the app (Perth)

- Type Thai with the IME and press Return mid-composition: it must not send.
- The composer gets focus on session open and after sending.

Then commit on the branch and merge into main.

## Comments

- 2026-10-17: Perth chose to merge without the manual IME/focus check. Committed as 975e3d6 and merged into main as 3c39ce9. Main after merge: 287 tests pass, bench invariants hold, clippy shows 65 pre-existing warnings, swift build passes. The IME and focus behaviour are still not checked in the app.
