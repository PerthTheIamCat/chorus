# Sub sessions

Type: grilling
Status: resolved
Blocked by: 01, 10, 19
Parent: ../map.md
Labels: wayfinder:grilling

## Question

How do sub sessions behave? Decide: how they are named, their status, how deep they nest, when finished ones collapse or disappear, what opening one shows, what can be done to each kind (provider subagent vs Chorus-started), and how they link to the parent in the ledger. Say which parts of harness-roadmap ticket 16 this replaces.

## Answer

Perth accepted all eight recommendations (2026-10-17). This replaces harness-roadmap ticket 16 (the Inspector tree and drawer) and the `⚡ AGENT WORKTREES` list from ticket 12.

1. **Opening:** sub sessions live in the sidebar only, and the Inspector Subagents tab is removed.
   - A Chorus-started child is a real session. It opens in the centre with a breadcrumb (`session 1 › reviewer`) and a composer for steering.
   - A provider subagent opens as a read-only transcript with no composer.
2. **Names:** a Chorus child is `role · short task` (`reviewer · ตรวจ auth`), with a number on repeats (`reviewer 2`). A provider subagent uses the Task tool's description or Codex's nickname. One line, full name on hover.
3. **Status:**
   - Each child shows a dot: `กำลังทำ` (spinner) · `รอ` (paused at a cap or waiting for permission) · `เสร็จ ✓` · `ล้มเหลว ✕` · `ยกเลิก ⊘`, plus the short name of the model the router picked.
   - The parent row shows a count (`2 กำลังทำ`).
   - `รอ` is the most prominent state.
4. **Depth:**
   - Only the orchestrator starts Chorus children. A Chorus child cannot start further Chorus children, but it may have provider subagents.
   - The sidebar indents at most 2 levels; deeper levels collapse to `…+N`.
5. **Lifecycle:**
   - The parent row auto-expands while any child runs, and collapses to `เสร็จแล้ว N` when all are done.
   - Children go with the parent on delete or archive.
   - A worktree is removed only after merge or discard.
6. **Actions:**
   - A Chorus child can be stopped, messaged, have its worktree diff opened, and be merged or discarded through Review. These controls appear only on children owned by this session.
   - A provider subagent is view-only, with "Stop parent turn" and a note on why it can't be stopped alone.
7. **Ledger:** provider subagents become child session rows linked to the parent and marked as provider-kind, the same model as Chorus children. The parent's assembled context gets only each child's final result (as the tool result), never its internal events.
   - **Prerequisite:** a subagent normalizer in `chorus-acp` that routes child events by `parentToolUseId` (Claude) and child `sessionId` (Codex). This fixes Claude child prose leaking into the parent transcript and Codex dropping child tool calls (harness-roadmap research 06).
   - **Schema impact:** the existing `subagent` table (schema.rs:149) has `parent_sess`/`child_sess` but no kind column, and its `status` CHECK lacks a waiting state. A migration is needed; ask Perth when implementing (CLAUDE.md §11).
8. **Attention:** a badge on the parent row whenever a child fails or waits for permission. A macOS notification only when the app is not frontmost.
