# Live handoff eval (Claude ↔ Codex)

Agent A makes a decision and does half the work; agent B (the *other* provider, same ledger
session) is told only to continue. Checks are deterministic (files, greps, running code) —
no model grades a model, and nothing printed here is simulated (CLAUDE.md 7.24).

```bash
python3 evals/handoff/live/eval.py --dry-run          # no agents: prove every case is well-formed
python3 evals/handoff/live/eval.py                    # 16 real-work cases, one run each (≈ 2 real turns per case)
python3 evals/handoff/live/eval.py --matrix --repeat 3   # baseline + each ablation alone, 3 runs per cell
```

Needs `claude` and `codex` logged in (see `chorus doctor`), `python3`, `node` (two cases), `git`.
Real runs happen on the maintainer's Mac; the cloud box that built this can only dry-run.

## What is in the set

| suite | what | where |
|---|---|---|
| `real` (default) | 16 cases: 5 chores on a pinned snapshot of this repo (`BASE_SHA` in `real_cases.py`), 11 small projects in other stacks (Node, Python+sqlite, bash, CSV, migrations) with seeded files | `real_cases.py` |
| `toy` | the 16 original one-file cases from 23 Oct | `cases.py` (still runnable alone with `run.py`) |

9 cases go claude→codex, 7 codex→claude (`--both` runs each in both directions). 5 are *implied-pending*:
the unfinished half is only implied by turn 1 or by a rule written down in the repo — the kind
that failed in the first run (`05-todo-list`). Turn 2 never restates the decision from turn 1.

Your own repos: add cases with `source: "external:NAME"` and pass `--external NAME=/path/to/repo`
(its `HEAD` is exported with `git archive`, never modified). Cases whose repo is not given are skipped with a message.

Honest limits: the fixture projects and the chorus chores were written by the same person who wrote
the tool, from the *shape* of real tasks; they are not a sample of anyone's day. The Claude "weekly cap"
and "unknown model" messages in r02/r05 are synthetic stand-ins for the real classifier chore (7.6b).

## `--dry-run` (what it proves)

For every case: the seed passes no check by itself (a no-op must fail); an *oracle* — a hidden
solution kept next to the case, never shown to an agent — makes `a_checks` pass after "turn 1", leaves
at least one final check failing (so turn 2 has work), and makes all final checks pass after "turn 2".
`--slow` also runs the cargo checks (r02, r05; compiles `chorus-core`, minutes the first time).
It measures nothing about agents.

## Ablations

`--ablate r1,repomap` sets the flag of each named part to `off` for **both** turns; `--matrix` runs
baseline and each part alone.

| name | env var | part |
|---|---|---|
| `repomap` | `CHORUS_REPO_MAP=off` | repo map in the code tier — **exists today** |
| `r1` | `CHORUS_R1_TOOL_REFS=off` | old tool results collapse to one line + blob ref (workstream 1) |
| `r2` | `CHORUS_R2_MIN_RELEVANCE=off` | memory relevance floor (workstream 1) |
| `evidence` | `CHORUS_BRIEF_EVIDENCE=off` | brief carries commands/exit codes/unverified (workstream 1, R5) |

Whoever lands R1/R2/R5 adds a one-line guard reading that variable (like `repo_map_context`
does for `CHORUS_REPO_MAP`). Until a crate reads the variable the runner prints a warning and the
ablation is a no-op — it is recorded in the JSON as `read_by_core: false`, so a flat result is not
mistaken for "this part does not matter".

`evidence` only shows under `--via handoff`, which also runs `chorus handoff` (brief + Verify echo-back)
before turn 2; the default `--via ledger` is the path the app takes after a quota failover.

## Reading the output

```
config        pass   95% CI        a_dead  B secs  tokens in/out  cost
baseline   14/16 88%  (0.64, 0.97)     0     41.2   183200/9100    $1.84
```

* `a_dead` — A never finished its half (crash, timeout, `a_checks` unmet). Left out of the rate:
  a dead A is not a handoff failure.
* The interval is a Wilson 95% interval. With 16 runs it is wide (a 94% observed rate is compatible with roughly 70–99%);
  an ablation only means something with `--repeat 3` or more *and* the same case failing in the same direction.
* `B secs` = turn 2 (+ `chorus handoff` when `--via handoff`). Tokens and cost are the ledger's `session_usage`
  totals for the whole session (A + B). They print `—` unless **every** counted run reported them — Codex
  reports no cost, so cost is usually `—`; it is never filled in.
* Each run is a row in `runs/<timestamp>.json` (gitignored) with checks, return codes, per-phase seconds,
  usage after A and at the end, whether a relay happened, and the kept workspace of every non-pass.
* Failed workspaces stay in `$TMPDIR/chorus-live-*` — look at them before changing a case or the harness.

`run.py` and `results.json` are the 23 Oct run of the toy suite (15/16, one real failure: the pending
item lived only in A's wording). Kept as the baseline that motivated implied-pending cases.
