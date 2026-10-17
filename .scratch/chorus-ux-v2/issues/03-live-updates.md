# Live updates while a turn runs

Status: resolved
Resolved in: 5fe4eda
Blocked by: 02
Parent: ../spec.md
Decided in: [Live updates](../../chorus-ux-overhaul/issues/19-live-updates.md)

## What

- Replace the global `(kind, text)` progress sink with typed JSON pushes that each carry `session_id` and `seq`. Keep the kinds `thought/message/tool/tool_done/usage` and add `events` and `status`.
- Add a second **read-only** engine handle (SQLite WAL). Swift pulls deltas through `events_since(seq)` on it, never on the turn's serial queue (CLAUDE.md 7.7).
- Push for every session; pull only for sessions on screen.
- Swift coalesces streamed tokens to at most one main-thread update per frame. Split the live-stream state from the `Engine` observable if Shell polish has not already.
- Replace `refresh()`'s full `events_since(0)` reload with the delta pull.

## Done when

- A test drives two concurrent sessions and checks that pushes are routed by `session_id`.
- A dropped push is recovered by the next pull.
- Check in the app: during a long turn, the context meter, the thread, the file tree badges and the diff all update before the turn ends, and typing stays smooth.
