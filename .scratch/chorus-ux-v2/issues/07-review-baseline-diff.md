# Review baseline and diff rewrite

Status: ready-for-agent
Blocked by: 02, 03
Parent: ../spec.md
Decided in: [Review baseline](../../chorus-ux-overhaul/issues/13-review-baseline.md)

## What

- Persist `review_base` per session.
- Record git `HEAD` at each turn end. If `HEAD` changes between turns, the user committed, and the baseline moves to that commit's tree. A commit made during a turn counts as the agent's and the baseline stays.
- "Mark reviewed" snapshots the shadow repo as the new baseline. It works in git and non-git projects and never touches the user's git.
- Untracked files show; `.gitignore`d files never show; Chorus-written files are hidden behind a toggle.
- Scopes: Latest turn · This agent · Session (default) · Child worktree. Wire up the scope picker.
- Performance: Rust returns the file list with `+/-` first, and hunks per file on expand. Above 300 files or 5,000 lines, collapse to a summary with "show all". Binary files and lockfiles show stats only.
- Make per-hunk keep/reject reachable again. Line numbers count within the file, not the hunk. Comments anchor to the right hunk.
- Restore and `revert_if_touched` are unchanged.

## Done when

- Tests: baseline movement for a user commit between turns, an agent commit during a turn, and Mark reviewed. The baseline survives a reopen.
- Check in the app: a 500-file diff scrolls smoothly; committing in a terminal clears those files from the Session scope.
