# Research: chat transcript virtualisation on macOS 14+

**Ticket**: [05-chat-virtualisation](../issues/05-chat-virtualisation.md)
**Date**: 2026-10-17
**Sources**: Apple developer documentation (fetched as `.md` from developer.apple.com, which carries the availability metadata), WWDC sessions, Apple Developer Forums threads, and two engineering write-ups. Each claim below links to the source it comes from. Anything marked **unverified** has no primary source behind it.

---

## TL;DR / recommendation

1. **Keep `ScrollView` + `LazyVStack`.** Do not move to `List` or `NSTableView` yet. The current thread is slow because of how it is fed and invalidated, not because of the container. The container is the expensive change; the feed fixes are cheap.
2. **Use the scroll APIs for "open at bottom / stay at bottom", not `ScrollViewReader` plus `onChange`.**
   - macOS 14 (our deployment target, `apps/macos/Package.swift` `platforms: [.macOS(.v14)]`): `.defaultScrollAnchor(.bottom)` plus `.scrollPosition(id:anchor: .bottom)` with `.scrollTargetLayout()`.
   - macOS 15+, behind `if #available`: `ScrollPosition` (`scrollTo(edge: .bottom)`, `isPositionedByUser`), `defaultScrollAnchor(_:for: .sizeChanges)`, and `onScrollGeometryChange` for an "is at bottom" flag.
3. **Before changing the container, fix five feed and invalidation problems** (see §1): streaming tokens invalidate the whole `ContentView`; every refresh replaces the whole events array; each row does O(n) work, so the list is O(n²); Markdown is re-parsed inside `body`; and every row gets its own transition, `compositingGroup` and `geometryGroup`.
4. **Measure, then decide.** Target the Hitches metric's "good" band (≤ 10 ms/s) while scrolling and streaming a 2,000-event fixture with code blocks, on a 120 Hz display (8.3 ms per frame). Use the SwiftUI and Hitches instruments. If the fixed `LazyVStack` still misses the target, the fallback is an `NSTableView` hosting SwiftUI rows through `NSViewRepresentable` (§4). That fallback is a gated follow-up, not part of this map.

---

## 0. How the thread is built today (file:line)

All paths are relative to `apps/macos/Sources/Chorus/`.

| What | Where | Note |
|---|---|---|
| Thread container | `ContentView.swift:1627-1688` | `ScrollViewReader { ScrollView { LazyVStack(spacing: 14) { ForEach(engine.events) … workingRow … Color.clear.id("chatBottom") } } }` |
| Stick-to-bottom logic | `ContentView.swift:1661-1686` | Three `onChange` handlers (events count, a bucketed live-text length, `busy`). Each calls `proxy.scrollTo("chatBottom")`, mostly inside `withAnimation(.spring)`. **Nothing scrolls on first appear**, and nothing tracks whether the user has scrolled away. The map lists "chat not opening at the latest message" as a bug to fix separately; this is its cause. |
| Whole-stack animation | `ContentView.swift:1659` | `.animation(.spring…, value: engine.events.count)` on the whole `LazyVStack` |
| Per-row wrapper | `ContentView.swift:1600-1625` (`rowForEvent`) | `engine.attachments.filter { $0.event_seq == ev.seq }` (1602) and `engine.events.last(where:)` (1611-1614) run **for every row**, so each body pass is O(n·m) + O(n²). Each row also gets an asymmetric `.transition`, `.compositingGroup()` and `.geometryGroup()` (1622-1624). |
| Row view | `ContentView.swift:3210` (`EventRow`) | Branches on `kind`, giving a variable view count and variable height |
| Markdown | `ContentView.swift:3784-3826` (`MarkdownText`) | `parts` is a computed property (3793). It splits on "```" and parses `AttributedString(markdown:)` **on every body evaluation**. Code blocks go to `CodeBlockView` (3682). |
| Live streaming row | `ContentView.swift:1748` (`workingRow`) | Renders `engine.live.message` through the same `MarkdownText`, so the whole streamed message is re-parsed on every token |
| Observation model | `Engine.swift:138` (`final class Engine: ObservableObject`), `ContentView.swift:64` (`@EnvironmentObject var engine`) | `events` (139) and `live` (432) are both `@Published`. `absorb()` (451-454) does `live.message += text` for each token, which fires `objectWillChange` on the **one** object that the ~3,900-line `ContentView` depends on. |
| Feed | `Engine.swift:1237-1249` (`refresh()`), `Engine.swift:254` (session open) | Calls `chorus_transcript_json`, which is `chorus_events_since(e, 0)` (`crates/chorus-ffi/src/lib.rs:827`), so every refresh pulls the **whole session** and replaces `self.events` wholesale. The delta API `chorus_events_since(after)` exists but the thread doesn't use it. |

---

## 1. The real costs, ranked (independent of container)

These come from Apple's performance guidance applied to the code above. Fixing them is a precondition for a fair container comparison.

1. **Broad dependency on fast-changing state.** WWDC25 *Optimize SwiftUI performance with Instruments* says to "be extra careful with dependencies that change very frequently". Its worked example shows that observing a whole collection makes every view depend on all of it, and the fix is per-item observable state ([WWDC25-306](https://developer.apple.com/videos/play/wwdc2025/306/)). Here, each streamed token invalidates `ContentView` and re-evaluates the whole thread body. **Fix:** move `LiveTurn` into its own observable object that only `workingRow` reads. `@Observable` is available on macOS 14. Then token deltas touch one view.
2. **Per-row O(n) work in `body`.** WWDC23 *Demystify SwiftUI performance*: "The inline filter here is linear over the collection… It's better to move it out to the model", and "It's important that body itself is as cheap as possible" ([WWDC23-10160](https://developer.apple.com/videos/play/wwdc2023/10160/)). **Fix:** precompute `attachmentsBySeq: [Int64: [SessionAttachment]]` and `lastAgentMsgSeq` once per refresh in `Engine`.
3. **Markdown parsed in `body`.** The WWDC25 fix for slow bodies is to compute once and read a cache in `body` ([WWDC25-306](https://developer.apple.com/videos/play/wwdc2025/306/)). **Fix:** cache parsed `[Part]` / `AttributedString` keyed by `seq`. Ledger events are immutable, so the cache never invalidates. For the live message, re-parse only the trailing open block.
4. **Whole-array replacement on every refresh.** `chorus_events_since(after:)` already exists for deltas (`crates/chorus-ffi/src/lib.rs:832-835`). **Fix:** append only `seq > last.seq`, so `ForEach` diffs a tail instead of the whole array. This is a Swift-only change and keeps CLAUDE.md 2.3.
5. **Per-row transitions, `compositingGroup` and `geometryGroup`, plus an animation on `events.count` for the whole stack.** Each `compositingGroup` adds an offscreen compositing pass, which costs render-server time ("render hitches", [Understanding hitches](https://developer.apple.com/documentation/xcode/understanding-hitches-in-your-app)). Animating the stack on every count change also animates the layout of every realised row. **Fix:** animate only the newly inserted row, and drop `compositingGroup`/`geometryGroup` from historical rows. **Unverified:** how much each modifier costs on its own. Confirm with the instrument before and after.

---

## 2. The three candidates

### 2a. SwiftUI `ScrollView` + `LazyVStack` (current, recommended)

**APIs and availability**

| API | macOS | Source |
|---|---|---|
| `defaultScrollAnchor(_:)`: "control both which part of the scroll view's content should be visible initially and how the scroll view handles content size changes". `.bottom` "start[s] at the bottom of its content". | **14.0** | [docs](https://developer.apple.com/documentation/swiftui/view/defaultscrollanchor(_:).md) |
| `scrollPosition(id:anchor:)` + `scrollTargetLayout()`: the binding tracks the top-most view (or the bottom-most with `anchor: .bottom`). Writing the binding scrolls to that view. SwiftUI "will attempt to keep the view with the identity… visible" across data re-orders and window resizes. | **14.0** | [docs](https://developer.apple.com/documentation/swiftui/view/scrollposition(id:anchor:).md) |
| `defaultScrollAnchor(_:for:)` with roles `initialOffset` / `sizeChanges` / `alignment` | **15.0** | [docs](https://developer.apple.com/documentation/swiftui/view/defaultscrollanchor(_:for:).md) |
| `ScrollPosition`: `scrollTo(edge:)`, `scrollTo(id:anchor:)`, `isPositionedByUser`, `edge`. "For an edge, that means keeping a top aligned scroll view scrolled to the top if the content size changes." | **15.0** | [docs](https://developer.apple.com/documentation/swiftui/scrollposition.md) |
| `onScrollGeometryChange(for:of:action:)`: "avoid updating large parts of your app whenever the scroll geometry changes", so transform to a `Bool` first | **15.0** | [docs](https://developer.apple.com/documentation/swiftui/view/onscrollgeometrychange(for:of:action:).md) |

**Pattern that fits the codebase**
- Apply `.defaultScrollAnchor(.bottom)` to the `ScrollView`. This fixes "open at latest" on macOS 14 with no `onAppear` hack. Apple's own example for this modifier is a `LazyVStack` message list ([docs](https://developer.apple.com/documentation/swiftui/view/defaultscrollanchor(_:).md)).
- Stay pinned only while the user is at the bottom. On 15+, read `ScrollPosition.isPositionedByUser` / `edge == .bottom`, or derive `isAtBottom` with `onScrollGeometryChange` (`contentOffset.y + containerSize.height >= contentSize.height - ε`). On 14, approximate with `scrollPosition(id:anchor: .bottom)`: if the tracked id is the last event, the user is at the bottom. Add a "jump to latest" pill when they are not.
- Under `.defaultScrollAnchor(.bottom)`, streaming growth of the last row is a content-size change, which the anchor handles ("When the content size of the scroll view changes, it may consult the anchor", [docs](https://developer.apple.com/documentation/swiftui/view/defaultscrollanchor(_:).md)). That removes the bucketed `onChange` at `ContentView.swift:1677`. **Unverified on macOS:** whether the size-change anchoring holds while the user has scrolled up. On 15+, `defaultScrollAnchor(.top, for: .sizeChanges)` plus an explicit `scrollTo(edge: .bottom)` when pinned is the documented way to split the cases ([docs](https://developer.apple.com/documentation/swiftui/view/defaultscrollanchor(_:for:).md)).

**Memory**: Lazy stacks "load and render their subviews on-demand". Apple says "lazy stacks trade some degree of layout correctness for performance, because the system only calculates the geometry for subviews as they become visible" ([Creating performant scrollable stacks](https://developer.apple.com/documentation/swiftui/creating-performant-scrollable-stacks.md)). Apple does **not** document whether realised rows are released after they scroll off. **Unverified**: retention after scrolling the whole history once. Measure it with the Allocations instrument (§5). Whatever the container, `engine.events` holds every body string in memory. That cost is roughly the size of the transcript and is the same for all three options.

**Known bugs (variable-height rows)**
- Stutter or jump with variable-height rows in `LazyVStack`. Reported since 2021 and intermittent across releases; the original poster says it no longer reproduced on iOS 17.4. No Apple engineer reply ([forum 685461](https://developer.apple.com/forums/thread/685461)).
- iOS 26: dynamic-height rows with `scrollPosition(id:)` + `defaultScrollAnchor(.bottom)` jump when the keyboard toggles. DTS could not reproduce on 26.1 beta 3. FB20979569 ([forum 805306](https://developer.apple.com/forums/thread/805306)).
- iOS 27: `ScrollViewProxy.scrollTo(lastId, anchor: .bottom)` stops 2–3 rows short when row heights vary, and position is not kept when older rows are prepended. `defaultScrollAnchor(.bottom)` fixed the initial open but re-snapped on prepend. FB24968838, no Apple reply ([forum 848608](https://developer.apple.com/forums/thread/848608)).
- All three are iOS reports. **Unverified on macOS.** Their shared lesson: `scrollTo` to an **unrealised** lazy row lands by estimated height. That is exactly what the current `scrollTo("chatBottom")` does after a long jump, so prefer the anchor/edge APIs over id-scrolling to a sentinel.
- WWDC25 (macOS 26) improved list and scroll scheduling on all platforms: "reduces the chance of your app dropping a frame while scrolling quickly at high frame rates" ([WWDC25-256](https://developer.apple.com/videos/play/wwdc2025/256/)). Behaviour on the macOS 14 baseline may be worse than on the dev machine, so profile on both.

### 2b. SwiftUI `List`

- On macOS, `List` "attempts to implement self-sizing on top of `NSTableView`" ([kean, *Not List*](https://github.com/kean/articles/blob/main/2021-03-01-not-list.markdown)). That is a third-party inference, not an Apple statement.
- "All the IDs of List and Table are gathered eagerly." The number of views per `ForEach` element must be constant, "or SwiftUI has to build the views in addition to the identifiers". `if`/`AnyView` inside the row breaks this ([WWDC23-10160](https://developer.apple.com/videos/play/wwdc2023/10160/)). `EventRow.body` switches on `kind` across different view types (`ContentView.swift:3239-3253`). It still returns one view per element, but care is needed.
- WWDC25: on macOS 26, "lists of over 100,000 items now load 6x faster… update up to 16x faster" ([WWDC25-256](https://developer.apple.com/videos/play/wwdc2025/256/)). That is not available on the macOS 14 baseline.
- Downsides for a chat: `List` row chrome (insets, selection highlight, separators) has to be undone. One developer reports reloads were slow because `List`'s diffing can't be disabled, at about 150k rows ([kean](https://github.com/kean/articles/blob/main/2021-03-01-not-list.markdown)). **Unverified:** whether `defaultScrollAnchor` applies to `List` on macOS 14. The docs show it only on `ScrollView`.
- **Verdict:** no clear win over a fixed `LazyVStack`. It keeps SwiftUI's sizing problems and adds styling fights. Not recommended.

### 2c. AppKit `NSTableView` (or `NSCollectionView`) with variable row heights

- Use a view-based `NSTableView` in `NSViewRepresentable`. Size rows either with `usesAutomaticRowHeights` (macOS 10.13+, "uses autolayout to calculate the height of rows", [docs](https://developer.apple.com/documentation/appkit/nstableview/usesautomaticrowheights.md)) or with `tableView(_:heightOfRow:)` plus `noteHeightOfRows(withIndexesChanged:)` when the streaming row grows. The latter "will animate" for view-based tables; wrap it in an `NSAnimationContext` with duration 0 to suppress that ([docs](https://developer.apple.com/documentation/appkit/nstableview/noteheightofrows(withindexeschanged:).md)).
- Rows can be SwiftUI via `NSHostingView`. Its `sizingOptions` (macOS 13+) defaults to `.standardBounds`, and "setting this to a value with fewer options can improve performance as it reduces the amount of layout measurements" ([docs](https://developer.apple.com/documentation/swiftui/nshostingview/sizingoptions.md)).
- Since macOS 13, `NSTableView` **estimates** heights of off-screen variable-height rows. In 13.0 `reloadData()` left stale cached heights. The workarounds are the `NSTableViewCanEstimateRowHeights` default, or `noteHeightOfRows` for all rows. A 2025 update reports blurry text from non-integral row heights on 15.3 ([Christian Tietze](https://christiantietze.de/posts/2022/11/nstableview-variable-row-heights-broken-macos-ventura-13-0)). Snap heights to whole points.
- **Memory**: cell reuse through `makeView(withIdentifier:owner:)` keeps realised views bounded by the viewport. That is the strongest guarantee of the three options.
- **Pinning to the bottom**: done by hand (`scrollRowToVisible(numberOfRows-1)`, or observing `NSScrollView.contentView` bounds to compute "at bottom"). Full control, but all code we would own.
- Engineering reports: replacing `List` with `NSTableView` made a 150k-row macOS app "blazing fast" for about 300 lines of AppKit ([kean](https://github.com/kean/articles/blob/main/2021-03-01-not-list.markdown)). [Michael Tsai](https://mjtsai.com/blog/2024/04/15/nstableview-with-swiftui) collects reports from developers who moved to `NSTableView` for smoother scrolling. Both describe fixed or near-fixed row heights, not streaming Markdown.
- **CLAUDE.md 2.3 fit**: this is all view glue with no business logic, so it is allowed. But it is the largest Swift surface of the three, and it would be redone for WinUI anyway.

---

## 3. Side note: text selection

Each `MarkdownText` paragraph is a separate `Text(...).textSelection(.enabled)` (`ContentView.swift:3820`). **Unverified (no Apple statement found):** whether SwiftUI selection can span multiple `Text` views on macOS. In practice it does not, in any of the three containers above. Cross-message selection would need a single TextKit 2 `NSTextView` document. TextKit 2 layout "is always noncontiguous", meaning only the viewport plus overscroll is laid out ([WWDC21-10061](https://developer.apple.com/videos/play/wwdc2021/10061/)), and `NSTextView` uses TextKit 2 by default since macOS 13 unless `layoutManager` is touched ([WWDC22-10090](https://developer.apple.com/videos/play/wwdc2022/10090/)). That would be a fourth design: no row views at all, with code blocks as attachments. It is out of scope unless Perth asks for transcript-wide selection.

---

## 4. Decision gate for the AppKit fallback

Open an `NSTableView` follow-up only if **all** of these hold after the §1 fixes land:
- The hitch rate while scrolling the 2,000-event fixture is above 10 ms/s, or above 10 ms/s while streaming at the bottom, on a 120 Hz display.
- The SwiftUI instrument attributes the hitches to lazy-stack layout or measurement ("Other Long Updates"), not to our `body`s.
- Memory after scrolling the full history once and back is not bounded. Allocations should show roughly flat growth after the first pass.

If the gate trips, a cheaper step comes before AppKit: **windowing**. Realise only the last N events (for example 200) and load older ones on demand at the top. That step needs either a Swift-side slice of `engine.events`, or an "events before seq" FFI call. The FFI call is a Rust change and must be asked about first (CLAUDE.md §11).

---

## 5. How to measure 120 Hz smoothness

- **Frame budget**: at 120 Hz "the app needs to update the UI every 8.3 ms" ([Understanding hitches](https://developer.apple.com/documentation/xcode/understanding-hitches-in-your-app)). Confirm the panel's rate with `NSScreen.maximumFramesPerSecond` (macOS 12+, [docs](https://developer.apple.com/documentation/appkit/nsscreen/maximumframespersecond.md)).
- **Pass/fail metric**: hitch rate in ms of pause per second. "At or below 10 ms/s is good; at or below 25 ms/s is a warning; at or below 50 ms/s is critical" ([Understanding hitches](https://developer.apple.com/documentation/xcode/understanding-hitches-in-your-app)). Track commit hitches (our main thread) separately from render hitches (render server, for example from `compositingGroup` offscreen passes).
- **Instruments**:
  - SwiftUI template. In Xcode 26 it has the lanes *Update Groups / Long View Body Updates / Long Representable Updates / Other Long Updates*, and a *Cause & Effect Graph* that answers "what marked my view body as outdated?" ([WWDC25-306](https://developer.apple.com/videos/play/wwdc2025/306/)). Use it to prove that token streaming no longer invalidates `ContentView`.
  - Hitches instrument. The article excludes only visionOS from it ([Understanding hitches](https://developer.apple.com/documentation/xcode/understanding-hitches-in-your-app)).
  - Allocations or a memgraph for the memory gate.
  - Profile a Release build: `Package.swift` links the debug Rust lib by default (`let target = "debug"`), so switch both.
- **In-app probe (debug only)**: `NSView.displayLink(target:selector:)` (macOS 14+, [docs](https://developer.apple.com/documentation/appkit/nsview/displaylink(target:selector:).md)) gives a `CADisplayLink` on the view's display. Log intervals longer than 1.5× the expected interval while a scripted scroll runs.
- **Automated regression (optional)**: `XCTHitchMetric(application:)` measures hitches in a UI performance test, but it needs **macOS 26+** ([docs](https://developer.apple.com/documentation/xctest/xcthitchmetric.md)), so the test can't run on the 14 baseline.
- **Fixture**: a session db with about 2,000 events (mixed user and agent messages, about 30% with fenced code blocks, some tool rows) plus a long streaming turn. Scripted actions: open the session (it should land at the bottom), fling to the top and back, then stream 5k tokens while pinned and while scrolled up. Run on both macOS 14 and the current macOS.

---

## Sources

- Apple. [`defaultScrollAnchor(_:)`](https://developer.apple.com/documentation/swiftui/view/defaultscrollanchor(_:).md) · [`defaultScrollAnchor(_:for:)`](https://developer.apple.com/documentation/swiftui/view/defaultscrollanchor(_:for:).md) · [`scrollPosition(id:anchor:)`](https://developer.apple.com/documentation/swiftui/view/scrollposition(id:anchor:).md) · [`ScrollPosition`](https://developer.apple.com/documentation/swiftui/scrollposition.md) · [`onScrollGeometryChange`](https://developer.apple.com/documentation/swiftui/view/onscrollgeometrychange(for:of:action:).md)
- Apple. [Creating performant scrollable stacks](https://developer.apple.com/documentation/swiftui/creating-performant-scrollable-stacks.md) · [Understanding hitches in your app](https://developer.apple.com/documentation/xcode/understanding-hitches-in-your-app)
- Apple AppKit. [`usesAutomaticRowHeights`](https://developer.apple.com/documentation/appkit/nstableview/usesautomaticrowheights.md) · [`noteHeightOfRows(withIndexesChanged:)`](https://developer.apple.com/documentation/appkit/nstableview/noteheightofrows(withindexeschanged:).md) · [`NSHostingView.sizingOptions`](https://developer.apple.com/documentation/swiftui/nshostingview/sizingoptions.md) · [`NSView.displayLink`](https://developer.apple.com/documentation/appkit/nsview/displaylink(target:selector:).md) · [`NSScreen.maximumFramesPerSecond`](https://developer.apple.com/documentation/appkit/nsscreen/maximumframespersecond.md) · [`XCTHitchMetric`](https://developer.apple.com/documentation/xctest/xcthitchmetric.md)
- WWDC. [WWDC23-10159 Beyond scroll views](https://developer.apple.com/videos/play/wwdc2023/10159/) · [WWDC23-10160 Demystify SwiftUI performance](https://developer.apple.com/videos/play/wwdc2023/10160/) · [WWDC25-256 What's new in SwiftUI](https://developer.apple.com/videos/play/wwdc2025/256/) · [WWDC25-306 Optimize SwiftUI performance with Instruments](https://developer.apple.com/videos/play/wwdc2025/306/) · [WWDC21-10061 Meet TextKit 2](https://developer.apple.com/videos/play/wwdc2021/10061/) · [WWDC22-10090 What's new in TextKit and text views](https://developer.apple.com/videos/play/wwdc2022/10090/)
- Apple Developer Forums. [685461](https://developer.apple.com/forums/thread/685461) · [805306 (FB20979569)](https://developer.apple.com/forums/thread/805306) · [848608 (FB24968838)](https://developer.apple.com/forums/thread/848608)
- Engineering write-ups. [kean, *Not List*](https://github.com/kean/articles/blob/main/2021-03-01-not-list.markdown) · [Christian Tietze, NSTableView variable row heights on Ventura](https://christiantietze.de/posts/2022/11/nstableview-variable-row-heights-broken-macos-ventura-13-0) · [Michael Tsai, NSTableView with SwiftUI](https://mjtsai.com/blog/2024/04/15/nstableview-with-swiftui)
