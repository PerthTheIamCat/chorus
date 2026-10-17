# Migration v7: session_usage, review_base, subagent kind

Status: resolved
Blocked by: 01
Parent: ../spec.md
Decided in: [Live updates](../../chorus-ux-overhaul/issues/19-live-updates.md) · [Review baseline](../../chorus-ux-overhaul/issues/13-review-baseline.md) · [Sub sessions](../../chorus-ux-overhaul/issues/11-sub-sessions.md)

## What

One migration, `SCHEMA_VERSION` 6 → 7, in `crates/chorus-core/src/schema.rs`:
- New table `session_usage` (one row per session): `context_used`, `context_window`, `tokens_in`, `tokens_out`, `cost_usd` (nullable), `updated_ms`. Perth approved it.
- `session.review_base` TEXT, nullable: a shadow-repo sha. Perth approved it.
- `subagent`: add `kind` (`chorus` | `provider`) and allow a `waiting` status. Perth approved it on 2026-10-17.

Write the usage row at turn end and on every `usage_update`. None of this goes into the ledger `event` table.

## Done when

- Migration tests cover v6 → v7 on an existing db and a fresh db, and are idempotent.
- `history()` and the dedup numbers in bench are unchanged.
- Reopening a session seeds the context meter from `session_usage`.

## Comments

- 2026-10-17: merged into main as f06adb3 (fast-forward). Not checked in the running app: the meter seeding from session_usage.
