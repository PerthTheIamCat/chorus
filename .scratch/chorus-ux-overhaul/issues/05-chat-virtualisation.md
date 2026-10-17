# Chat transcript virtualisation

Type: research
Status: resolved
Blocked by: —
Parent: ../map.md
Labels: wayfinder:research

## Question

What is the best way on macOS 14+ to render a long chat transcript (thousands of messages with Markdown and code blocks) so it opens at the latest message, stays there while streaming, and scrolls smoothly? Compare SwiftUI `LazyVStack` with `defaultScrollAnchor(.bottom)`/`ScrollPosition`, `List`, and an AppKit `NSTableView`/`NSCollectionView` with variable row heights. Cover known bugs, memory for each approach, and how to measure 120 Hz smoothness.

Research: ./research/05-chat-virtualisation.md

## Answer

Keep the `ScrollView` + `LazyVStack` thread and fix how it is fed:
- Stream updates must stop rebuilding the whole `ContentView`.
- Do the O(n) work once, not inside every row.
- Cache the parsed Markdown.
- Load only new events with the existing delta call instead of `events_since(0)`.
- Drop the per-row transitions.

Pin to the bottom with `.defaultScrollAnchor(.bottom)` + `scrollPosition(id:anchor:)` on macOS 14, and `ScrollPosition`/`isPositionedByUser` on 15+, instead of `scrollTo` to a sentinel row. Don't use `List`. Move to AppKit `NSTableView` only if a 2,000-event Release test still hitches more than 10 ms/s. Loading only the last N events would need a new FFI call, so ask first. Detail: [research](../research/05-chat-virtualisation.md).
