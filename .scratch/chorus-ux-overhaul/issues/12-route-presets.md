# Route presets

Type: grilling
Status: resolved
Blocked by: 01
Parent: ../map.md
Labels: wayfinder:grilling

## Question

What does each route preset (Economy, Balanced, Max) change in how the router weighs cost against capability and effort? Decide where the preset is set (per session, per project, global), whether it applies to the work handed out in orchestra mode, and how it changes harness-roadmap ticket 17.

## Answer

Perth accepted all six recommendations (2026-10-17). This changes harness-roadmap ticket 17: the default is now Balanced, not a fixed cost bias.

1. **What a preset changes:** it shifts the `Want` from the existing `(kind, difficulty)` table and the direction of the tie-break.
   - **Economy (ประหยัด):** tier −1 and effort −1, floored at 0. It prefers missing below over missing above.
   - **Balanced (สมดุล, default):** the current table without the Review floor, so Review at trivial/easy becomes small tier, effort 2.
   - **Max (เต็มกำลัง):** tier +1 and effort +1, with mid tier as the minimum. It prefers missing above.
2. **Where it is set:** per session, from a composer menu next to `Auto route | Orchestra`. The default for new sessions lives in Settings (global). There is no per-project setting.
3. **Orchestra mode:** the session preset applies when routing every child. The orchestrator's `weight` maps `light/normal/heavy` to `easy/moderate/hard`. The orchestrator itself uses its own Settings model and ignores the preset.
4. **Learning:** each user override is recorded with the active preset, and learning is per preset. The router's own picks are no longer logged as learned routes.
5. **Model tier source:** a curated tier table in `agents.rs` with an env override. Names not in the table fall back to the name heuristic.
6. **UI:** the menu shows `ประหยัด · สมดุล · เต็มกำลัง` with an (i) tooltip of one line. The route pill appends the preset (`gpt-5.6-luna · low · ประหยัด`).
