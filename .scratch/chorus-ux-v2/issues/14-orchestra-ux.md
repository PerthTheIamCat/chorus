# Orchestra mode UX

Status: ready-for-agent
Blocked by: 05, 06, 12
Parent: ../spec.md
Decided in: [Orchestra mode](../../chorus-ux-overhaul/issues/10-orchestra-mode.md)

## What

- The composer mode menu `Auto route | Orchestra` is per session and takes effect from the next turn. Settings holds the default.
- The model pill shows "นำโดย <orchestrator>".
- The plan renders as a live `chorus_task_update` checklist in the thread, with Stop always visible. The "ask before starting the plan" setting is honoured.
- The cap pause shows as a "รอ" prompt with Continue and Stop.
- A new-role badge appears, with a "Keep" action.
- Replace the info-page `OrchestratorView` with the role library editor inside Settings › Orchestra.

## Done when

- Check in the app: switch modes mid-session, run a mock orchestration end to end, and see the cap pause and resume.
