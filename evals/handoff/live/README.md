# Tier 2 — live handoff eval (Claude ↔ Codex)

16 cases, `python3 evals/handoff/live/run.py` (≈2 real requests per case, ~10 min at `--jobs 2`).
Agent A makes a decision and does half the work; agent B (the *other* provider, same
ledger session) gets only "continue and finish". Checks are deterministic (files, greps, running code) — no model grades a model.
Directions alternate; 8 cases each way.

## Result (23 Oct 2026, one run, real agents logged in on the maintainer's Mac)

**15/16 continued correctly (94%).**

| | |
|---|---|
| claude→codex | 7/8 |
| codex→claude | 8/8 |

- `05-todo-list` (claude→codex) **failed for real:** A implemented `add`/`list` and said `done` was for later; B was told "continue and finish" and did nothing — `done` was never added. The pending item lived only in A's wording.
- `14-secret-handling` first failed on a bad check (`".env"` matched `os.environ`); fixed and re-run → pass. Counted as pass above; raw first run was 14/16.

## Read this before trusting the number

- n = 16, one run each, no repeats: a 94% here is compatible with roughly 70–99% true rate.
- Tasks are tiny and the author of the cases is also the author of the tool. They test "decision survives the handoff", not hard long-horizon work.
- Turn 2 uses `chorus run --session` with a different agent (ledger + task state), which is the same context path the app's handoff uses, but not the explicit `handoff` brief + Verify echo-back.
- The instructive failures are the ones where the pending work is only implied — extend the set with more of those.
