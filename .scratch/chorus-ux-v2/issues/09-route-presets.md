# Route presets

Status: ready-for-agent
Blocked by: —
Parent: ../spec.md
Decided in: [Route presets](../../chorus-ux-overhaul/issues/12-route-presets.md)

## What

- In `router.rs`, the preset shifts the `Want` from `wanted()`:
  - Economy: tier −1, effort −1, floored at 0. It prefers missing below.
  - Balanced: the current table with the Review floor removed.
  - Max: tier +1, effort +1, with mid tier as the minimum. It prefers missing above.
- Tie-break direction follows the preset.
- Curated model tier table in `agents.rs` with an env override. Unknown names fall back to the name heuristic.
- Record the preset with each user override and learn per preset. Stop logging the router's own picks as learned routes.
- Expose the preset per session through FFI. The route pill shows `model · effort · preset`.
- `weight` (light/normal/heavy) maps to difficulty (easy/moderate/hard) for the orchestra children used in issue 12.

## Done when

- Router unit tests: per preset, cover each kind × difficulty pick, the tie direction, and the removed Review floor.
- `scoring_tests` are updated, not deleted.
