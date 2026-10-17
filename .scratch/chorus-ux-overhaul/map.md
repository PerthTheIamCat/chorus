# Chorus UX overhaul and orchestra mode

Labels: wayfinder:map
Status: resolved (2026-10-17): destination reached, handed off to ../chorus-ux-v2/spec.md

## Destination

A decided spec, ready to split into implementation issues, for: workbench layout v2 (file viewer beside the tree, file-type icons, sub sessions in the sidebar), orchestra mode, route presets, the composer, settings layout, the system monitor, review baseline for diffs, cost display, and the security delta since the 2026-10-12 release audit. Planning ends at the reviewed spec. Plain bugs are fixed separately (see Out of scope).

## Notes

- Perth's list of 2026-10-17 is the source. Answers from charting:
  - Orchestra mode: the orchestrator is an **agent** (default Claude Opus 5.5, medium effort, configurable) that calls Chorus's control surface. Chorus still never plans by itself (`CONTEXT.md`).
  - Orchestra and auto routing are **mutually exclusive** for the user's turn. Inside orchestra mode, the orchestrator assigns the **role**, and the router picks the agent/model/effort for each piece of work.
  - Roles come from an editable library (`.chorus/agents/*.md`); the orchestrator may add a new role when none fits, and that role is shown to the user.
  - Sub sessions in the sidebar show **both** provider subagents (view-only) and Chorus-started agents (stop/message). Different icons.
  - Review baseline: git project = `HEAD` (committed work is done); non-git = shadow repo + "Mark reviewed".
  - Monitor: every listening port/process owned by the current user, agent-spawned ones labelled; kill = confirm, then SIGTERM, then SIGKILL; never system or other users' processes.
  - Route presets: `ประหยัด · สมดุล · เต็มกำลัง` / `Economy · Balanced · Max`, default Balanced.
  - Security: scan only code since the release audit (commits `61e1c87`, `0c1bfc9` and later) plus new surfaces from this map.
- **This map overrides some decisions in [chorus-harness-roadmap](../chorus-harness-roadmap/map.md)**: workbench layout (its ticket 13), subagent UX (16), router policy (17). Prototype tickets must say what they replace.
- UI tickets (prototype) consult `anti-ui-slop`, `macos-design-guidelines`, `ui-ux-pro-max`, `swiftui-pro`. Logic stays in Rust; Swift draws (CLAUDE.md 2.3).
- Research output goes to `./research/NN-<slug>.md` in this workspace (no throwaway branches, no commits). Cite primary sources.
- Glossary terms added: Orchestra mode, Role, Sub session, Route preset, Review baseline.

## Decisions so far

<!-- the index: one line per closed ticket -->

- [File-type icon sources](./issues/06-file-type-icons.md): Catppuccin Icons subset (MIT, light/dark) with an SF Symbols fallback, resolved in Rust by file name then extension; system UTType icons are wrong for code files.
- [Chat transcript virtualisation](./issues/05-chat-virtualisation.md): keep LazyVStack and fix the feed (whole-view rebuild per token, O(n²) rows, full reload); pin to the bottom with scroll-position APIs; AppKit table only if measurements still fail.
- [Composer paste and preview options](./issues/04-composer-paste-and-preview.md): Rust converts pasted HTML to Markdown (`htmd`) and styles it (`pulldown-cmark`) inside a plain NSTextView; QuickLook/ImageIO thumbnails; attach on the left, model next to Send; the paste-order bug and the large-file check go to fixes.
- [Process and port monitoring on macOS](./issues/02-process-port-monitoring-apis.md): libproc via `libc` (no new dependency), 3–5 ms per scan, show phys_footprint not RSS; stopping needs uid + system-path denylist + pid-reuse re-check, then SIGTERM, then SIGKILL; agent labels are a hint.
- [Current macOS shell audit](./issues/01-current-shell-audit.md): panels refresh only at turn end (nothing polls); relay drops context_used; diff, highlighting, router and orchestration decisions from harness-roadmap are mostly unbuilt (spawn only writes rows); full gap list in the research.
- [Security scan since the release audit](./issues/03-security-delta-scan.md): 1 P1 (unsandboxed post-turn `cargo check` = code execution from agent-written build.rs), 5 P2 (role-name path escape, MCP actor spoofing and no auth, revoke reports false success, repo-relative bridge binary), 8 P3, 5 dormant that orchestra mode would wake.
- [Security findings triage](./issues/16-security-triage.md): fix P1 + all P2 + P3-1/2/3/5/7 now in a separate session; post-turn verify/hooks off by default and sandboxed when trusted; worktrees move to App Support; agent commits made during a turn stay in review; research §4 rules become spec requirements.
- [Orchestra mode](./issues/10-orchestra-mode.md): per-session `Auto route | Orchestra` switch; a read-only orchestrator (Opus 5.5 medium, failover backup) hands out role + task + weight and the router picks the model; children are real Chorus sessions, writers in App Support worktrees, merged on user approval; Rust-enforced caps pause and ask; new roles are saved only on "Keep".
- [Live updates while a turn runs](./issues/19-live-updates.md): typed push signals with session_id + seq, delta pull on a read-only second handle (WAL); token updates coalesced per frame; new `session_usage` table (schema v7, approved) seeds the meter and feeds cost.
- [Sub sessions](./issues/11-sub-sessions.md): sidebar-only tree (Inspector tab and worktree list removed); Chorus children open as real sessions with steering, provider subagents as read-only transcripts; status dots with `รอ` loudest; 2 levels max; provider subagents stored as child sessions after a bridge normalizer fix (needs a migration).
- [Rich paste converter dependency](./issues/18-rich-paste-dependency.md): approved `htmd` + `pulldown-cmark` in chorus-core and `rust-version` 1.85, with size, depth and time caps on conversion.
- [Route presets](./issues/12-route-presets.md): presets shift the router's wanted tier/effort by ±1 with a matching tie-break (Balanced drops the Review floor); per-session menu, global default; applies to orchestra children; learning per preset; curated tier table.
- [Review baseline for diffs](./issues/13-review-baseline.md): persisted `review_base` (migration v7) moves on Mark reviewed or on user commits between turns; agent commits during a turn stay in review; lazy per-file hunks with size caps; scope picker wired.
- [Cost display](./issues/15-cost-display.md): tokens always, money only when a bridge reports it (Claude, labelled API-equivalent), user-set THB rate with no network; the existing hardcoded price-table estimate goes; quota bars above money; totals per turn/session/sub session/orchestration.
- [System monitor](./issues/14-system-monitor.md): separate window (⌥⌘M) with Chorus and Services groups, 1 s refresh only while visible; stop = confirm, then SIGTERM, then an explicit "force" SIGKILL; disk by category; manual optimise actions with size preview.
- [Shell polish triage](./issues/20-shell-polish-triage.md): 11 small fixes now (IME Return, focus, menu bar + shortcuts, paste order, size check, stale model ids, chat perf…); delta refresh, diff controls, copy pass and orchestration settings wait for their tickets.
- [Workbench layout v2](./issues/07-workbench-layout-v2.md): variant A, four columns (sessions with sub sessions │ file tree │ viewer │ chat); under 1200 px the tree becomes a rail and an open file takes chat's place; replaces harness-roadmap 13.
- [Composer layout](./issues/08-composer-layout.md): variant A: thumbnails → live-styled Markdown → one row (attach · mode · preset · permission … dictation · model · Send); Send becomes Stop while streaming; grows to ~220 px.
- [Settings layout](./issues/09-settings-layout.md): variant A, a System Settings-style sidebar with 13 categories in 4 groups; one-line labels with (i) tooltips; copy pass included.
- [Sequencing and handoff](./issues/17-sequencing.md): UI-first phases 0–5 with route presets and the system monitor in parallel; handed off as [chorus-ux-v2 spec](../chorus-ux-v2/spec.md) + 14 issues.

## Not yet specified

- **Daily and monthly cost totals**: a usage history view, perhaps in the system monitor.
- **Monitor optimisation automation**: scheduled clean-up versus manual buttons only.
- **Menu bar extra** for the system monitor.

## Out of scope

- **Plain bug fixes** (context/token panel not refreshing, chat not opening at the latest message or after sending, diff stutter with many hunks, code viewer missing syntax colour): Perth agreed on 2026-10-17 to fix these directly in a separate session, not on this map.
- **Chorus running its own planner**: still out, per the earlier map; orchestra mode is agent-driven.
- **In-app editing**: the file viewer stays read-only.
