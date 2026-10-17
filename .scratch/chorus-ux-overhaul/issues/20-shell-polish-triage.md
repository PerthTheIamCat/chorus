# Shell polish triage

Type: grilling
Status: resolved
Blocked by: —
Parent: ../map.md
Labels: wayfinder:grilling

## Question

Which of the smaller gaps the audit found are straight fixes now and which wait for a redesign ticket on this map? The gaps: no composer auto-focus; Return sending during IME composition; collapsed folders re-expanding every 15 s; shortcut collisions (⇧⌘E, Esc, ⌘↩); no menu bar; mixed Thai/English copy; the permission dialog naming the wrong agent; diff keep/reject unreachable; the dead scope picker; the composer paste order; mapping large files into memory; the dead countdown sheet. See the audit and research 04.

Also from the Fix shell bugs session (perf work left over): MarkdownText parse cache; delta refresh via `events_since` instead of re-fetching the whole transcript; removing per-row transition, compositingGroup and geometryGroup; splitting the live stream off whole-Engine observation.

## Answer

Perth approved the triage table (2026-10-17).

**Fix now** (separate session):
1. Return during IME composition (Thai input) must not send.
2. Composer auto-focus.
3. Collapsed folders re-expanding every 15 s.
4. Shortcut collisions (`⇧⌘E`, `Esc`, `⌘↩`).
5. A standard macOS menu bar (File/Edit/View/Window), with every shortcut listed in it.
6. The permission dialog naming the wrong agent.
7. Composer paste order: a Finder copy must attach the file, not its icon.
8. Check the size before mapping a file into memory (20 MiB limit).
9. Delete the dead countdown sheet.
10. Replace hardcoded stale model ids in Settings/OrchestratorView with the model cache (7.19).
11. Chat perf: MarkdownText parse cache, drop per-row transition/compositingGroup/geometryGroup, and split the live stream off whole-Engine observation.

**Wait:**
- Delta refresh via `events_since` waits for Live updates.
- Diff keep/reject and the scope picker wait for Review baseline.
- Mixed Thai/English copy waits for Settings layout.
- The unread orchestration settings wait for Orchestra mode.
