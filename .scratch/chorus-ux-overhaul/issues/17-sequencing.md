# Sequencing and handoff

Type: grilling
Status: resolved
Blocked by: 07, 08, 09, 10, 11, 12, 13, 14, 15, 16
Parent: ../map.md
Labels: wayfinder:grilling

## Question

In what order should the decided areas be built, and how does the spec split into implementation issues?

## Answer

Perth accepted all four recommendations (2026-10-17). Handoff: [implementation spec](../../chorus-ux-v2/spec.md) with 14 issues in `../../chorus-ux-v2/issues/`.

1. **Phases, UI first as Perth asked at the start:**
   - 0: merge shell polish.
   - 1: plumbing (migration v7, live updates).
   - 2: main UI (layout, composer, settings and copy).
   - 3: review baseline and diff.
   - 4: cost.
   - 5: multi-agent (normalizer, orchestra core, sub sessions, orchestra UX).
2. **In parallel from the start:** route presets. System monitor starts after phase 0, because it touches the new menu file.
3. **Handoff format:** one `spec.md` that indexes the decision tickets without restating them, plus one issue per session-sized piece. Each issue has a "Done when" and links back to its tickets.
4. **Done for every issue:**
   - test, bench invariants, and clippy with no new warnings;
   - `cargo build -p chorus-ffi` then `swift build`;
   - an in-app check list;
   - no commit until Perth asks.
