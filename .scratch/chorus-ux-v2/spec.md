# Chorus UX v2 and orchestra mode: implementation spec

Status: ready-for-agent
Source map: [Chorus UX overhaul and orchestra mode](../chorus-ux-overhaul/map.md). All 20 tickets are resolved; each one's `## Answer` is the authority for its area. This spec orders the work and splits it into issues. It does not restate the decisions.

## Goal

Make Chorus feel like a finished native Mac app for Perth's daily work:

- a four-column workbench that resizes well;
- a composer that follows common conventions;
- clear settings;
- readable code and diffs that stay fast;
- live context, cost and quota numbers;
- an agent-driven orchestra mode whose children appear as sub sessions;
- route presets;
- a system monitor.

Every change keeps CLAUDE.md rules 2.1–2.3: storage is separate from context, the ledger is the single source of truth, and logic lives in Rust while Swift only draws.

## Decisions, by area

| Area | Decided in | One line |
|---|---|---|
| Workbench layout | [Workbench layout v2](../chorus-ux-overhaul/issues/07-workbench-layout-v2.md) | Four columns: sessions │ tree │ viewer │ chat. Below 1200 px the tree becomes a rail and the viewer takes chat's place. |
| File icons | [File-type icon sources](../chorus-ux-overhaul/issues/06-file-type-icons.md) | Catppuccin subset, resolved in Rust. **Already merged into main.** |
| Composer | [Composer layout](../chorus-ux-overhaul/issues/08-composer-layout.md) · [Composer paste and preview options](../chorus-ux-overhaul/issues/04-composer-paste-and-preview.md) · [Rich paste converter dependency](../chorus-ux-overhaul/issues/18-rich-paste-dependency.md) | Variant A. `htmd` + `pulldown-cmark`, rust-version 1.85. QuickLook/ImageIO thumbnails. |
| Settings | [Settings layout](../chorus-ux-overhaul/issues/09-settings-layout.md) | Sidebar with 13 categories in 4 groups, (i) tooltips, and a copy pass. |
| Chat performance | [Chat transcript virtualisation](../chorus-ux-overhaul/issues/05-chat-virtualisation.md) | Keep LazyVStack and fix the feed. The scroll-to-bottom part is **already merged**. |
| Live updates | [Live updates while a turn runs](../chorus-ux-overhaul/issues/19-live-updates.md) | Typed push signals (`session_id`, `seq`), delta pull on a read-only second handle, per-frame coalescing. |
| Review baseline and diff | [Review baseline for diffs](../chorus-ux-overhaul/issues/13-review-baseline.md) | Persisted `review_base`. Mark reviewed, or a user commit between turns, moves it forward. Lazy per-file hunks. |
| Cost | [Cost display](../chorus-ux-overhaul/issues/15-cost-display.md) | Quota bars, money only when a bridge reports it, a user-set THB rate. Remove the price-table estimate. |
| Route presets | [Route presets](../chorus-ux-overhaul/issues/12-route-presets.md) | Shift `Want` by ±1 with a matching tie-break, drop the Review floor, curated tier table, learning per preset. |
| Orchestra mode | [Orchestra mode](../chorus-ux-overhaul/issues/10-orchestra-mode.md) | A read-only orchestrator assigns role, task and weight; the router picks each model; children are real sessions; Rust-enforced caps. |
| Sub sessions | [Sub sessions](../chorus-ux-overhaul/issues/11-sub-sessions.md) | A tree in the sidebar only. Provider subagents are stored as child sessions after a bridge normalizer is added. |
| System monitor | [System monitor](../chorus-ux-overhaul/issues/14-system-monitor.md) · [Process and port monitoring on macOS](../chorus-ux-overhaul/issues/02-process-port-monitoring-apis.md) | Separate window `⌥⌘M` with Chorus and Services groups. libproc through `libc`. Safe stop flow. Manual optimise actions. |
| Security | [Security findings triage](../chorus-ux-overhaul/issues/16-security-triage.md) | The fix batch is **already merged**. The §4 new-surface rules in [research 03](../chorus-ux-overhaul/research/03-security-delta-scan.md) bind every issue below. |
| Small fixes | [Shell polish triage](../chorus-ux-overhaul/issues/20-shell-polish-triage.md) | 11 fixes done on branch `claude/serene-murdock-e347dc`, waiting to merge (issue 01). |

## Approved changes that need no further asking

Perth approved these on 2026-10-17:

- **Schema:** migration v7, adding the `session_usage` table, the `session.review_base` column, and `subagent.kind` with a `waiting` status (the `subagent` part was approved later the same day).
- **Dependencies:** `htmd`, `pulldown-cmark`, and workspace `rust-version` raised to 1.85.

## Still needs Perth's approval when reached

- **New dependencies:** any not listed above.
- **New FFI:** a "last N events" FFI call, if chat paging turns out to need one.

## Phases

| Phase | Issues | Waits for |
|---|---|---|
| 0 | 01 Merge shell polish | Perth's manual IME/focus check |
| 1 · Plumbing | 02 Migration v7 · 03 Live updates | 01 |
| 2 · Main UI | 04 Workbench layout v2 · 05 Composer v2 · 06 Settings v2 + copy pass | 03 (04), 01 (05, 06) |
| 3 · Review | 07 Review baseline and diff rewrite | 02, 03 |
| 4 · Cost | 08 Cost display | 02, 03 |
| 5 · Multi-agent | 11 Subagent normalizer · 12 Orchestra core · 13 Sub sessions sidebar · 14 Orchestra UX | see each issue |
| Parallel | 09 Route presets · 10 System monitor | none / 01 |

## Definition of done (every issue)

- `cargo test --workspace` passes.
- `cargo run -p chorus-cli --bin chorus -- bench` holds every CLAUDE.md §3 invariant.
- `cargo clippy --workspace` adds no new warnings.
- `cargo build -p chorus-ffi && (cd apps/macos && swift build)` succeeds. Rebuild the FFI first, because the staticlib goes stale.
- The issue's own "Check in the app" list is done in the running app, or the report says what could not be checked.
- `rustfmt --edition 2021 <file>` runs only on the files touched; never `cargo fmt` a whole crate.
- No commit, push or merge. Perth asks for those.
