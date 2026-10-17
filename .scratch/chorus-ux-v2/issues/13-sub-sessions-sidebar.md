# Sub sessions in the sidebar

Status: resolved
Blocked by: 04, 11, 12
Parent: ../spec.md
Decided in: [Sub sessions](../../chorus-ux-overhaul/issues/11-sub-sessions.md)

## What

- A tree under each session in the sidebar, at most 2 levels, deeper levels shown as `…+N`.
- Names: `role · short task` with a number on repeats.
- Status dots: กำลังทำ, รอ (most prominent), เสร็จ, ล้มเหลว, ยกเลิก. Plus the model chip.
- The parent row shows a count, auto-expands while children run, and collapses to `เสร็จแล้ว N` when they are done.
- A Chorus child opens in the centre as a full session with a breadcrumb and composer.
- A provider child opens as a read-only transcript, with "Stop parent turn".
- Stop and Message appear only on Chorus children owned by this session.
- A badge shows on failure or a permission wait. A macOS notification fires only when the app isn't frontmost.
- Remove the Inspector Subagents pane and the duplicate subagent parser in `OrchestratorView.swift`.

## Done when

- Check in the app with the mock orchestration from issue 12: the tree updates live, and every state is reachable.
