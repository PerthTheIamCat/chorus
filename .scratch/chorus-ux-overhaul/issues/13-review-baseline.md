# Review baseline for diffs

Type: grilling
Status: resolved
Blocked by: 01
Parent: ../map.md
Labels: wayfinder:grilling

## Question

How does the review baseline work? Decide: git projects measure from `HEAD`, and what happens when an agent or the user commits mid-session; how untracked and ignored files are shown; how "Mark reviewed" works outside git; how per-turn, per-agent and session scopes sit on top of the baseline; and how this interacts with the shadow-repo checkpoints and `revert_if_touched`.

Constraint from Security findings triage: the baseline is `HEAD` at session start or the user's last "Mark reviewed". Commits made while a turn runs count as the agent's and stay in review. Commits made between turns count as the user's and move the baseline forward.

## Answer

Perth accepted all seven recommendations and approved a `review_base` column in migration v7 (2026-10-17).

1. **Storage:** the `session` table gets a `review_base` column holding a shadow-repo sha. It goes in the same v7 migration as `session_usage`. Reopening a session keeps its baseline. It is not a ledger event, because it is metadata.
2. **Detecting the user's commits:**
   - At each turn end, record the real git `HEAD`.
   - If `HEAD` has changed before the next turn or on app focus, and no turn ran in between, the user committed. The baseline moves to that commit's tree, and uncommitted changes still show.
   - A commit made while a turn runs counts as the agent's and does not move the baseline.
   - If the user later commits over the agent's commits, those leave review too, because committing means accepting.
3. **Mark reviewed:** available in git and non-git projects. It snapshots the shadow repo as the new baseline and never touches the user's git.
4. **What files show:**
   - Untracked files show as new.
   - `.gitignore`d files never show (7.3).
   - Chorus-written files (`CLAUDE.md`/`AGENTS.md`/`GEMINI.md`/`.claude/skills/`) are hidden, with a "show files Chorus wrote" toggle (7.11).
5. **Scopes:**
   - Latest turn: the turn-start snapshot to now, independent of the baseline.
   - This agent: that agent's turns since the baseline.
   - Session: baseline to now. This is the default.
   - Child worktree: the child's branch to now.
   - The scope picker gets wired up, since today it does nothing.
6. **Performance:**
   - Rust returns the file list with `+/-` first, and computes hunks per file when that file is expanded.
   - Above 300 files, or 5,000 lines in one file, the diff collapses to a summary with "show all".
   - Binary files and lockfiles show stats only.
   - The thresholds get tuned after measuring.
7. **Checkpoints:** the baseline is only a pointer to a shadow sha. Restore and `revert_if_touched` keep using the per-turn snapshots as before.
