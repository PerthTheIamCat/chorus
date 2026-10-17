# Settings v2 and copy pass

Status: ready-for-agent
Blocked by: 01
Parent: ../spec.md
Decided in: [Settings layout](../../chorus-ux-overhaul/issues/09-settings-layout.md) · prototype `09-settings-layout.html?variant=A`

## What

- Rebuild Settings as a sidebar of 13 categories in 4 groups (see the ticket).
- Each row: a short label, its control, and an (i) tooltip that works on hover **and** keyboard focus.
- Move every existing `@AppStorage` key into its category.
- Remove the stale hardcoded model ids and the unread `chorus.orchestrationEnabled` key.
- Add the new keys as controls with their decided defaults, even before the features behind them ship: default mode and preset, orchestrator and backup, caps, ask-before-plan, auto-merge (off), `.env` copy (off), trusted projects, Spotlight (off), on-device dictation, THB rate (empty), show Chorus-written files, Catppuccin credit.
- Copy pass over the whole app: one-line Thai labels, with English kept for product terms (Orchestra, Auto route, MCP, Skills).

## Done when

- Check in the app: every category opens, no pane scrolls past one screen at the default window size, VoiceOver reads the (i) content, and dark mode works.
