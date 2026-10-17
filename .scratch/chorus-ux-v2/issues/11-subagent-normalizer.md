# Subagent normalizer in chorus-acp

Status: ready-for-agent
Blocked by: 02
Parent: ../spec.md
Decided in: [Sub sessions](../../chorus-ux-overhaul/issues/11-sub-sessions.md) Q7 · [harness research 06](../../chorus-harness-roadmap/issues/06-subagent-visibility-in-bridges.md)

## What

- Route child events by `_meta.claudeCode.parentToolUseId` (Claude) and by child `sessionId` (Codex, `subagent_spawned`/`subagent_state_update`).
- Store provider subagents as child session rows (`kind=provider`) with their own events.
- The parent's assembled context gets only each child's final result, as the tool result.
- Forward spawn and state through the typed push (`status`).
- Fixes Claude child prose leaking into the parent transcript and Codex dropping child tool calls.

## Done when

- Fixture tests replay recorded Claude and Codex streams and check that the parent transcript contains no child prose and that each child transcript is complete.
- Bench numbers are unchanged.
