# Current macOS shell audit

**Ticket:** `../issues/01-current-shell-audit.md`
**Date:** 2026-10-17
**Method:** I read the source at `main` (`0c1bfc9` plus uncommitted `CONTEXT.md`) and traced the Rust FFI and core calls behind each panel. **I did not run the app.** Every claim below comes from code, with `file:line` evidence. Where a behaviour can only be confirmed at runtime, the claim says so.

Paths are relative to the repo root. `CV` = `apps/macos/Sources/Chorus/ContentView.swift`, `EN` = `apps/macos/Sources/Chorus/Engine.swift`, `FFI` = `crates/chorus-ffi/src/lib.rs`, `SES` = `crates/chorus-cli/src/session.rs`.

---

## 0. Cross-cutting causes (read first)

Four structural facts explain most of the reported bugs:

1. **Nothing polls the engine or pushes events to the UI.** Panels only update when `Engine.refresh()` runs (`EN:1237-1269`), and that happens only at turn end, session open, snapshot or reject (`EN:256, 1104, 1141, 1171, 1193, 1224, 1507, 1649, 1661, 1674`). `chorus_events_since` exists (`FFI:837`) but Swift never calls it. `refresh()` pulls the **whole** transcript through `chorus_transcript_json` (which is `events_since(0)`, `FFI:827-829`) and replaces `events` wholesale (`EN:1249`). CLAUDE.md §3 says "shell ยัง poll `events_since`", which is not true of the current code.
2. **One serial queue carries every engine call, and a running turn holds it.** `send`, `sendRelay` and `sendInteractive` block `queue` for the whole turn (`EN:1085, 1122, 1157`). Every other handle-bound call goes onto the same `queue`: `refresh`, `fetchWorktrees`, `fetchRoles`, `fetchAlignedDiff`, review comments and `assembledContext` (`EN:1239, 1857, 1978, 1759, 1797-1832, 1540`). While a turn runs, none of them can return. This is intentional (CLAUDE.md 7.7), but it means **no panel can update mid-turn** without push or a second handle.
3. **`ContentView` observes the whole `Engine`** (`@EnvironmentObject`, `CV:64`), and `live` is `@Published` and changes on every streamed chunk (`EN:432, 451-466`). Each chunk therefore re-evaluates the roughly 3,900-line `ContentView.body`. That includes:
   - `badge(.changes)`, which re-parses the full `workingDiff` (`CV:409`);
   - the thread `ForEach`, which runs two linear scans per row: `engine.attachments.filter` and `engine.events.last(where:)` (`CV:1602, 1611-1614`). That makes each render O(n²) in events.

   `DiffReviewerView` and `WorkspaceExplorerView` are also `@ObservedObject var engine` (`DiffReviewerView.swift:8`, `WorkspaceExplorerView.swift:8`), so they re-render on every chunk as well.
4. **Several settings and controls are stored but never reach the engine** (see §5 and §7). They change nothing.

---

## 1. Workbench layout, file tree, file viewer

| | Evidence |
|---|---|
| **Implemented** | `HStack` with up to 7 slots: sidebar, explorer (leading or trailing), inspector (leading or trailing), center stage, code viewer, then the trailing explorer/inspector (`CV:621-674`). Resizable grips, widths in `@AppStorage` (`CV:119-122`). Maximize mode for one panel (`CV:677-716`). Center stage switches between thread, orchestrator and live preview (`CV:577-603`), with a terminal drawer (`CV:605-616`). Shortcuts: ⌃⌘S sidebar, ⌘B explorer, ⌥⌘I inspector, ⌘⌥1–4 tabs, ⌘⌥O orchestrator, ⌘⌥P preview, ⌘\` terminal, ⇧⌘M focus (`CV:747-896`, `CV:442`). |
| **File tree** | `WorkspaceExplorerView.swift`. Rust builds the tree (`chorus_file_tree_json`, `EN:1704-1720`). It shows git badges and an `is_focus` dot (`WorkspaceExplorerView.swift:283-300`), plus a filter and a worktrees footer (`:121-163`). |
| **Code viewer** | Clicking a file sets `selectedFile` (`CV:472-474`), which opens `CodeViewerView` as a column to the **right of the thread**, not beside the tree (`CV:650-656`). It has an external-editor button (`CodeViewerView.swift:244-260`). |
| **Missing vs ticket 13/14** | The tree is not `NSOutlineView`. It is a recursive SwiftUI `VStack`, so only the root level is lazy (`WorkspaceExplorerView.swift:89-99, 308-317`). There is no TextKit 2 viewer (§2). There is no menu bar: `ChorusApp` has no `.commands` (`ChorusApp.swift:18-37`), so shortcuts live only on toolbar buttons. There are no sub sessions in the sidebar (§3). File-type icons cover only 5 extensions (`WorkspaceExplorerView.swift:333-340`). The code viewer has its own separate 7-extension map (`CodeViewerView.swift:25-36`). |
| **Broken** | (a) Every refresh re-inserts all top-level folders into `expandedPaths` (`WorkspaceExplorerView.swift:204-208`). Refresh fires every 15 s (`:178-182`), on every `workingDiff` change and on app activation (`:172-177`), so **a folder the user collapsed springs open again**. (b) `CodeViewerView` loads synchronously on the main thread with no size cap (`CodeViewerView.swift:219-228`), and only when `filePath` changes, so it never reloads after an agent edits the open file. Non-UTF-8 files show an error line. (c) The minimum window width is 820 (`ChorusApp.swift:22`). The default columns add up to 260 + 260 + 330 (420 on Changes, `CV:519`) + 420 thread minimum (`CV:1740`) + 520 viewer = about 1,800 pt, with nothing collapsing automatically. (d) `onSelectWorktree` is wired to `{ _ in }` at every call site (`CV:475, 586, 695`; `Settings.swift:813`), so clicking a worktree does nothing. |
| **Shortcut collisions** | ⇧⌘E is bound both to "cycle effort" (`ModelPicker.swift:107`) and to "open in editor" (`CodeViewerView.swift:181`). Esc is bound both to "back to chat" (`CodeViewerView.swift:102`) and to "restore from maximize" (`CV:566`). ⌘F is cross-session search (`CV:821`), which shadows Find in text views. |

## 2. Code highlighting — root cause

There is **no real highlighter**. `SyntaxHighlighter` is a per-line regex colourer in Swift (`CodeViewerView.swift:321-379`). This breaks rule 2.3 and contradicts ticket 14, which called for Rust `syntect`/`tree-sitter` spans. Neither crate appears in any `Cargo.toml`.

Likely reasons a user sees "no colour":
- **Only `rs`, `swift`, `ts` and `js` get keywords; `toml`, `yaml` and `ini` get keys** (`:338-364`). `tsx`, `jsx`, `py`, `go`, `svelte`, `css`, `html`, `c`, `sh` and `md` fall through to plain text, apart from `#`/`//` whole-line comments and quoted strings.
- It works line by line, so block comments, multi-line strings and anything with state are impossible.
- It compiles `NSRegularExpression` 4–5 times **per line per render** (`:369-372`). Every hover changes `hoveredLine` on the parent (`CodeViewerView.swift:58-63`), which re-renders the visible rows, so the work repeats on mouse movement.
- **Runtime check needed:** `Range(range, in: attr)` maps `String` indices from a *copy* (`String(attr.characters)`) into the `AttributedString` (`:371-376`). If that conversion returns `nil`, the `continue` silently drops every colour, and the viewer would show only comment and TOML-header styling. This needs confirming in the running app.
- Chat code blocks get **no colour at all**: `CodeBlockView` renders `Text(code)` (`CV:3766-3772`).

## 3. Sub agent display

| | Evidence |
|---|---|
| **Implemented** | `subagentsPane` builds a tree from the ledger: it uses `subagent_spawned`/`subagent_state_update` JSON payloads and otherwise falls back to keyword matching on `tool_call` bodies (`CV:2714-2831`, keywords at `CV:2789-2792`). `OrchestratorView` has a second, near-duplicate parser (`OrchestratorView.swift:43-70, 308-314, 727-729`) with **different** matching rules (`"\"subagent\""`/`"\"Task\""`/`chorus_spawn_agent`). |
| **Broken** | **`subagentsPane` is never shown.** `RightTab` has only context, changes, memory and checkpoints (`CV:384-386`), and `subagentsPane` has no call site apart from its own definition (`CV:2714`). Subagents appear only inside the Orchestrator center-stage page. They fill in only after the turn ends, because the data comes from the ledger after `refresh()` (comment at `CV:2712-2713`). |
| **Missing vs ticket 16 and map** | There is no live state while the turn streams. The `live.tools` row is flat (`CV:1784-1795`). There are no per-child transcript, tokens, or stop/message controls, and no sub sessions in the sidebar. Child sessions that `agent_control::spawn_agent` creates (`crates/chorus-core/src/agent_control.rs:169-206`, title `child-agent-<role>`, `forked_from = parent`) would show as flat, unlabelled rows, because Swift never reads `forked_from` (`EN:1317`). |

## 4. Orchestrator UI and agent control surface

| | Evidence |
|---|---|
| **Implemented (UI)** | `OrchestratorView` is an info page, not a control surface. It has an explainer card with emoji and stale model advice (`OrchestratorView.swift:178-230`), a worktree list (`:235-305`), the subagent list, a role library with CRUD and AI synthesis over `.chorus/agents/*.md` (`:416-660`, `RoleEditorSheet :733+`), and a "Spawn Parallel Agents" button. That button only **pastes a canned Thai prompt into the composer** and switches back to the thread (`:668-670`; `CV:587-590`). |
| **Implemented (core)** | The MCP server lists 10 control tools: spawn, send_message, wait, collect, cancel, checklist, jump_to_file, request_review, synthesize_role, notify (`crates/chorus-core/src/mcp.rs:274-291`). |
| **Broken** | **Spawned agents never run.** `spawn_agent` only INSERTs a `session` row and a `subagent` row with status `running` and a worktree *path string*; it creates no worktree and starts no process (`agent_control.rs:169-206`). `complete_agent` is called only from tests (`crates/chorus-core/tests/mcp_control_surface.rs`). `wait_agent` ignores `_timeout_secs` and returns at once (`agent_control.rs:285-323`). An orchestrating agent that calls `chorus_spawn_agent` therefore gets a child that sits in "running" forever. |
| **Missing** | The UI has no read path for checklist, jump_to_file, reviews or the subagent table. None of `list_subagents`, `get_checklist`, `get_latest_jump` or `list_reviews` is exposed through the FFI; their only callers are the CLI (`crates/chorus-cli/src/main.rs:1364, 1444`) and tests. There is no way to choose an orchestrator agent per session (needed for orchestra mode). Hardcoded model ids such as `claude-3-7-sonnet-20250219` and `gpt-5.1-codex` contradict CLAUDE.md 7.19 (`OrchestratorView.swift:633-641`, `Settings.swift:722-728`). |

## 5. Router UI and policy

| | Evidence |
|---|---|
| **Implemented** | An Auto toggle on the composer (`CV:1909-1915`). `routeOrDispatch` builds candidates and calls `proposeRoute`, then dispatches immediately with no countdown (`CV:2115-2198`). `RouteStatusCard` shows "thinking", then "Auto chose agent/model/effort + reason" (`RouteStatus.swift`). When the router fails, a manual-pick sheet appears (`CV:1076-1093`). A battery-saver fallback switches to `in-process-heuristic` (`CV:2160-2161`). |
| **Dead code** | `RouteReview` (the 5-second countdown, `RouteReview.swift:4, 16-24, 55-79`) is reachable only when `pendingRoute.automatic == true` (`CV:1067`). Both constructors pass `automatic: false` (`CV:2151, 2192`), so the countdown is gone in practice but the file remains. |
| **Policy vs ticket 17** | `wanted()` still has a tier floor for Review: Easy goes to tier 1, Moderate goes to tier 2 (`crates/chorus-core/src/router.rs:1042-1046`). `router.rs` has **no cost term** (no `cost` in the file). There are no route presets. |
| **Broken (learning signal)** | `recordLearnedRoute` runs for **the router's own pick** (`CV:2179-2185`) as well as for manual choices (`CV:2117`, `2215`). The "learned routes" store therefore mostly learns from itself, which is the opposite of ticket 17's "log overrides". |
| **Missing** | There is no badge on the message showing which model ran. `EventRow` shows `modelLabel`, but only where `model_id` is stored; I have not verified it is filled on the relay path. There is no "Regenerate with…" and no borderline-confidence gate in the UI. |

## 6. Composer

| | Evidence |
|---|---|
| **Implemented** | The field is a native `NSTextView` (`Composer.swift:420-487`). It supports `/` and `@` completion (`:100-117, 386+`), paste of image, file or URL (`:322-357, 492-535`), drag and drop (`:178, 576-605`), a file picker, attachment chips with a "not agent-readable" marker (`:238-275`), dictation (`:189-213`) and a queue while busy (`CV:1881-1895`). The chip row holds Auto, model and effort, permission, and "ขนาน" (`CV:1908-1957`). |
| **Broken** | (a) The `@FocusState focused` variable is set on appear (`Composer.swift:97, 235`), but no `.focused($focused)` modifier exists anywhere, and the NSTextView is never made first responder. **The composer does not auto-focus.** (b) Return sends even during IME composition: `keyDown` never checks `hasMarkedText()` (`:553-571`). Keypad Enter (keyCode 76) is not handled. (c) Height is guessed as `chars/55` lines (`:119-128`). That figure is wrong for Thai, which has no spaces and different glyph widths, and for any panel width other than the one assumed. (d) Dictation **replaces** the draft instead of inserting (`:191-194`). |
| **Missing vs ticket 19** | No Quick Look or thumbnails; chips use generic `photo`/`doc` icons. No ⌘L to attach a selection from the viewer or diff. No per-model vision indicator. No limit toasts in the composer. |
| **No-op control** | "ขนาน (เปิด/ปิด)" toggles `@AppStorage("chorus.orchestrationEnabled")` (`CV:1927-1944`), but the value is never passed to the engine (it appears only in ContentView and Settings). The defaults also disagree: `false` in `CV:110`, `true` in `Settings.swift:714`. |

## 7. Settings structure

The settings are a sheet, not a `Settings` scene (`CV:1063-1065`, opened with ⌘, at `CV:1336`). Sections: General, Accounts and usage, Agents and models, Orchestra, Mobile Companion, Skills, Integrations (`Settings.swift:13-31`). The pane is a fixed 760×560 frame (`:107`).

- **General** mixes behaviour, window/scrollbar layout, failover and the whole local-router setup (`Settings.swift:214, 230, 269, 279`).
- **Orchestration settings are stored but never read.** `orchestrationLimit`, `orchestrationBasePort`, `autoMergeWorktrees` and `orchestratorLeadModel` appear only in `Settings.swift` (`:714-718`). The section also embeds a full `OrchestratorView` (`:809-815`), so that page exists twice.
- There are no route presets, and no place to set the orchestrator agent's model and effort as the map specifies.

## 8. Context / token panel — root cause of "doesn't refresh"

The panel reads `engine.contextUsed` / `engine.contextWindow` (`CV:2842-2894`). Those are set **only** from a `TurnResult` in `send` and `sendInteractive` (`EN:1099-1100, 1136-1137`). Causes, in order of impact:

1. **The relay path never reports usage.** The app sends every non-"ask" turn through `sendRelay` (`CV:2259-2265`). `RelayJson` has no `context_used`/`context_window` fields (`FFI:1079-1085`), even though `rr.turn` is a `TurnReport` that carries them (`SES:92-99`). `sendRelay` does not set them either (`EN:1166-1173`). **With permission "ให้ทำเลย" the gauge stays at "ยังไม่มีเทิร์น" forever.**
2. **Reset on every session open.** `swap()` sets both to `nil` (`EN:244-245`), and nothing derives them from the ledger. Reopening a long session shows 0%, even though the ledger holds usage: `ne.tok_out = outcome.context_used` (`SES:1681`).
3. **No mid-turn update.** ACP sends `usage_update` (CLAUDE.md 8, Q3), but the progress callback handles only thought, message, tool and tool_done (`EN:451-466`). The serial queue also blocks `refresh()` (§0.2).
4. The "รอสรุป" warm-tier count is updated by `compactNow` (`EN:1574-1576`). `taskState` and memory are updated only by `refresh()`, so a `memory_propose` made mid-turn appears only after the turn ends.
5. Cost is refreshed on `busy → false` (`CV:993-997`) on a separate connection (`EN:1925-1937`). It works, but it is a separate path from tokens.

## 9. Diff viewer and baseline — root cause of stutter

**Which view ships.** `diffReviewMode` is `true` and never changes (`CV:112, 493`), so `DiffReviewerView` always renders. The older `changesPane`, which holds the per-hunk keep/reject buttons (`CV:2376-2434`, `HunkView` at `CV:3566-3607`), is **unreachable**. **Shipped UI has no per-hunk keep/reject.**

**Stutter causes:**
1. `allHunks` is a computed property that re-runs `parseHunks(engine.workingDiff)`. It is called by `filesWithDiffs`, `activeHunks`, `totalAdd`, `totalDel` and `onAppear` (`DiffReviewerView.swift:37-52, 85-86, 152, 179`), so the diff is parsed **at least 4 times per body evaluation**. `ContentView.badge` adds a 5th (`CV:409`). `DiffHunk.addCount`/`delCount` re-filter lines on every access (`CV:3523-3524`).
2. The body re-evaluates on **every streamed chunk**, because the view observes the whole engine (§0.3).
3. `ScrollView([.vertical, .horizontal])` wraps a `LazyVStack` (`DiffReviewerView.swift:181-182`). A two-axis scroll view has to measure content width, and every line is `.fixedSize(horizontal: true)` (`:470, 514, 534`). Only hunks are lazy: each hunk's lines come from an eager `ForEach` keyed by `\.offset` (`:201, 226`). A big hunk is laid out in full the moment it scrolls near.
4. Every row has its own `@State isHovered` plus `.onHover` (`:449, 487, 496, 551`). Moving the mouse across the diff invalidates row after row.

**Other defects:**
- The scope picker (Latest Turn / Current Agent / Full Session, `:31-35, 68-74`) is never read. It changes nothing.
- "Split" is not side by side: each row shows the same line on both sides according to its +/- prefix (`:491-553`). The Rust aligner `fetchAlignedDiff` (`EN:1758`) has no caller.
- Line numbers are the hunk-relative index (`lineNum: idx + 1`, `:204`), not file line numbers. Review comments are saved against that index (`:215, 325-336`).
- `commentingLine` is one `Int` shared by all hunks (`:18, 210, 235`). Clicking "comment" on line 3 opens an editor under line 3 of **every** hunk.
- `onAppear` picks the first file (`:313-315`), so the default view shows one file, not all. If that file later leaves the diff, `activeHunks` is empty and the pane says "ยังไม่มีการแก้ไข" while other files have changes.
- Hunk ids are `file|header` (`CV:3517`). Two hunks with the same header in one file collide.

**Baseline:**
- `working_diff()` diffs against `base_ckpt` (`SES:456-461`), using `diff_since_user` (`crates/chorus-core/src/checkpoint.rs:138-155`). That function runs `git add -A` in the shadow repo on every `refresh()`.
- `base_ckpt` comes from `open_checkpoint`: a **new snapshot only if the shadow repo is fresh; otherwise the shadow repo's HEAD** (`SES:328-344`). Turn checkpoints advance HEAD, so **reopening a session moves the baseline up to the latest checkpoint**, and earlier sessions' and turns' changes disappear from Changes. The baseline also has no link to the user's own git: work the user already committed keeps showing until a new session. Both are the gap that the map's "Review baseline" decision (`HEAD` in git projects, "Mark reviewed" otherwise) is meant to close.

## 10. Chat scrolling and list rendering — root cause

Code: `CV:1637-1686`. The thread is a `ScrollViewReader` → `ScrollView` → `LazyVStack` over `engine.events`, with a 1 pt `chatBottom` sentinel. Auto-scroll triggers on `events.count`, on a coarse `live` length bucket, and on `busy` becoming true.

1. **Opening a session does not land at the bottom.** There is no `.onAppear`, and no `.defaultScrollAnchor(.bottom)`. `onChange` does not fire for the initial value. When the thread view is created fresh (coming from `emptyState` or `sessionWelcomeView`, `CV:1632-1635`), the events are already loaded, so no change fires and the view starts at the top. When switching between two sessions with the **same event count**, no change fires either. **Runtime check needed:** confirm whether the `ScrollView` identity survives a session switch.
2. **After sending, it often stops short.** `scrollTo` targets a sentinel at the end of a `LazyVStack` whose rows above have not been measured yet: tall markdown and code blocks get estimated heights. The scroll is also wrapped in a spring animation (`CV:1666-1668`) while insertion transitions run (`CV:1604-1610, 1660`). The target offset is computed before layout settles.
3. **Rows are swapped, not appended.** `echoUser` inserts a placeholder row with seq `Int64.min + n` (`EN:412-418`). `refresh()` replaces the whole array (`EN:1249`), so the placeholder's id disappears and the real row is inserted. That plays the removal and insertion transitions again and resets the scroll target.
4. **Streaming cost.** Every chunk re-renders `ContentView` (§0.3). `MarkdownText` re-splits on ```` ``` ```` and rebuilds `AttributedString` on each render (`CV:3793-3826`). A partial `html`/`svg`/`mermaid` block opens in canvas mode by default (`showCanvas = true`, `CV:3686, 3763-3764`), which reloads the preview on every chunk.
5. Markdown is inline-only (`.inlineOnlyPreservingWhitespace`, `CV:3819`), so headings, lists and tables are not rendered as structure.

## 11. Other UX gaps worth a ticket

- **Copy mixes Thai and English with marketing labels**: "Zero-Ollama Auto Router" (`Settings.swift:279`), "High-Density Repo Map (Personalized PageRank)" (`CV:3045`), "Self-Heal ⚡" (`CV:3005`), "Swarm Orchestrator", emoji 👑/👷 (`OrchestratorView.swift:198, 213`). The Context pane holds unrelated cards: cost, compiler verification and repo map (`CV:2900-3078`).
- **Two verification systems.** The Context pane uses `engine.verifyWorkspace(ws:)`, and the Diff pane uses `verifyWorkspace(workspace:)` with its own state (`EN:2250, 2270`; `DiffReviewerView.swift:371-378`). Verification also runs automatically after every turn (`CV:997-999`), which is a cost that has no setting.
- **The permission sheet** says "\(agent) wants to:", where `agent` is the composer `@State` default "codex" (`CV:79, 1100`), not the agent that is asking. This is a mislabel during relay or auto routing.
- **Accessibility**: inspector tabs have labels (`CV:441`), but diff and code rows are bare `Text` with hover-only buttons, so there is no keyboard path to "comment" or "keep/reject". The empty state tells the user to "เลือกโฟลเดอร์ทางซ้าย", but the sidebar can be on either side (`CV:1820`).
- **Keyboard**: there is no menu bar (§1). ⌘↩ is bound both to send (`CV:1954`) and to "Send to Agent" in the diff (`DiffReviewerView.swift:304`), so both are live when the inspector shows Changes.
- **Scrollbars default to hidden** (`@AppStorage("chorus.scrollbarStyle") = "hidden"`, `CV:116`), which hides position in long diffs and threads.
- **Docs drift**: CLAUDE.md §3 says the shell polls `events_since` and that the Context pane is live. Both are untrue today (§0.1, §8).

---

## Not verified

- Runtime behaviour of `SyntaxHighlighter` range conversion (§2), and whether the ScrollView is recreated on session switch (§10). Both need a run of `apps/macos/build.sh`.
- Whether `model_id` is filled for relay-path events, which decides whether a model badge appears (§5).
- I did not profile anything. The stutter causes are deduced from the render structure, not measured with Instruments.
