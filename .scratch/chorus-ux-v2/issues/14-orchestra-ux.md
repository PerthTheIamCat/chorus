# Orchestra mode UX

Status: resolved
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

## Resolution

- **Per-session mode switch**: Added `SessionMode` (`autoRoute`, `orchestra`, `manual`) with per-session state in `ContentView` and `ComposerModeMenu`. Setting `SettingsKey.defaultMode` holds the default for new sessions.
- **Model pill in composer**: When in Orchestra mode, model pill displays `นำโดย <orchestrator>` (defaults to Claude Opus 5.5, or configured model).
- **Plan Checklist**: `PlanChecklistView` renders live task checklist from `engine.checklist` (backed by FFI `chorus_get_checklist_json`), with always-visible Stop button (`chorus_cancel_orchestration`). Respects `orchestraAskBeforePlan` setting with "เริ่มทำตามแผน" action.
- **Cap Pause Prompt**: `CapPausePromptCard` renders whenever subagents are in `status == "waiting"`, offering "ทำต่อ" (Continue - calling `chorus_resume_subagent`) and "หยุด" (Stop).
- **New Role Badge & Keep**: Subagents with custom/synthesized roles display a `new role` badge and "Keep" action that saves the role to `.chorus/agents/<name>.md`.
- **Role Library in Settings**: Replaced standalone info-page `OrchestratorView` with embedded Role Library in Settings › Orchestra.
- **Verification**: All Rust and Swift tests passed (`cargo test --workspace`, `swift test`), bench invariants intact.
