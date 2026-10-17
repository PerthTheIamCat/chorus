# Live updates while a turn runs

Type: grilling
Status: resolved
Blocked by: —
Parent: ../map.md
Labels: wayfinder:grilling

## Question

How do panels (context gauge, tokens, sub session status, diff) update while a turn is running, when every engine call shares one serial queue that the turn holds (CLAUDE.md 7.7)? Options: a push callback from Rust carrying events and `usage_update`, a second read-only engine handle that polls `events_since`, or both. Decide which, and what each panel subscribes to. See [audit](../research/01-current-shell-audit.md).

Note (2026-10-17, from the Fix shell bugs session, uncommitted): the context meter now updates mid-turn through the existing progress callback, with a new `usage` kind sent through main.async, without touching the engine queue. The meter still resets to nil when a session is reopened; seeding it needs a new FFI field (last tok_out), and that is part of this decision.

## Answer

Perth accepted all five recommendations and approved the schema change (2026-10-17).

1. **Mechanism, push + pull:** Rust pushes small signals ("session X has events up to seq N", plus `usage` and `status`). Swift pulls the delta through a second **read-only** engine handle; SQLite WAL lets it read while the turn writes. A dropped push is harmless because the next pull catches up. No blobs cross the callback. The ledger stays the source of truth (2.2), and 7.7 holds because the second handle never writes.
2. **Addressing:** every push is typed JSON carrying `session_id` and `seq`, replacing the current global `(kind, text)` string sink. The existing kinds are kept (`thought`, `message`, `tool`, `tool_done`, `usage`) and two are added: `events` (new seq) and `status` (sub-session state).
3. **Update pacing:**
   - Streamed tokens are coalesced to at most one main-thread update per frame (~16 ms).
   - The context meter and cost update on `usage`.
   - The diff recomputes on `tool_done`, at most once per second.
   - Sub-session status updates on `status`.
4. **Persisted usage:** a new `session_usage` table (next schema migration, v7, approved; Perth approved it as "v4" before we found the code is already at `SCHEMA_VERSION = 6`), kept separate like `summary`, holds the last usage per session (context used, window, tokens, and cost where the bridge reports it). It seeds the meter on reopen and feeds Cost display. It is not a ledger event, because usage is metadata and would distort `history()` and the dedup numbers.
5. **Background sessions:** signals are pushed for every session, since sidebar sub-session badges need them. Full event pulls happen only for sessions on screen.
