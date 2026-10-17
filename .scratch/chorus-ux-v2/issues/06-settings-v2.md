# Settings v2 and copy pass

Status: resolved
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

## Comments

- 2026-10-17: committed 080afae, merged as 9ea6047. Conflicts: Settings.swift took the v2 layout, which already has the preset picker; `SettingsKey.defaultRoutePreset` now points at `Engine.defaultRoutePresetKey` so both read one key. ContentView kept the preset chip with the v2 copy. Not checked in the running app: pane fit, VoiceOver on (i), dark mode.
