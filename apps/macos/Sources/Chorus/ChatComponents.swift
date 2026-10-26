import SwiftUI

// Building blocks of the chat thread: row grouping, the snapshot cache that
// keeps the thread's body cheap, and the small animated pieces (shimmer,
// typing dots, avatar, tool group).

// MARK: - Grouping

/// One row of the thread. Consecutive tool events fold into a single
/// collapsed group so a long agent run reads as one line, not forty.
enum ChatItem: Identifiable {
    case event(LedgerEvent)
    case tools([LedgerEvent])

    /// A group keeps the seq of its first event, so it stays the same row
    /// while more tool calls are appended to it.
    var id: Int64 {
        switch self {
        case .event(let e): return e.seq
        case .tools(let t): return t[0].seq
        }
    }

    static func isTool(_ e: LedgerEvent) -> Bool {
        e.kind == "tool_call" || e.kind == "tool_result" || e.kind == "diff"
    }
}

struct ChatSnapshot {
    var items: [ChatItem] = []
    var attachments: [Int64: [SessionAttachment]] = [:]
    var lastAgentMsg: Int64? = nil
    /// Only a few rows changed (a new message, a tool call): animate them in.
    /// A whole session loading at once should not animate.
    var animate = false
}

/// The thread's body runs on every engine publish (a streamed token, a hover,
/// a scroll). Grouping, the attachment index and the last-message search are
/// O(events), so they are redone only when the events actually changed.
final class ChatSnapshotCache {
    private struct Key: Equatable {
        var count: Int
        var first: Int64?
        var last: Int64?
        var lastBytes: Int
        var attachments: Int
    }
    private var key: Key?
    private var snap = ChatSnapshot()

    func snapshot(events: [LedgerEvent], attachments: [SessionAttachment]) -> ChatSnapshot {
        let k = Key(count: events.count,
                    first: events.first?.seq,
                    last: events.last?.seq,
                    lastBytes: events.last?.body.utf8.count ?? 0,
                    attachments: attachments.hashValue)
        if k == key { return snap }

        var items: [ChatItem] = []
        items.reserveCapacity(events.count)
        var run: [LedgerEvent] = []
        func flush() {
            switch run.count {
            case 0: break
            case 1: items.append(.event(run[0]))
            default: items.append(.tools(run))
            }
            run.removeAll(keepingCapacity: true)
        }
        for e in events {
            if ChatItem.isTool(e) { run.append(e) } else { flush(); items.append(.event(e)) }
        }
        flush()

        var bySeq: [Int64: [SessionAttachment]] = [:]
        for a in attachments { if let s = a.event_seq { bySeq[s, default: []].append(a) } }
        let lastAgent = events.last(where: { $0.actor != "user" && $0.kind == "msg" })?.id
        let delta = abs(items.count - snap.items.count)
        let next = ChatSnapshot(items: items, attachments: bySeq, lastAgentMsg: lastAgent,
                                animate: !snap.items.isEmpty && delta <= 4)
        key = k
        snap = next
        return next
    }
}

// MARK: - Motion

/// A soft highlight sweeping across the text while something is in progress.
/// Static under Reduce Motion.
private struct ShimmerModifier: ViewModifier {
    let active: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var phase: CGFloat = -0.6

    func body(content: Content) -> some View {
        if active && !reduceMotion {
            content
                .overlay(
                    GeometryReader { g in
                        LinearGradient(colors: [.clear, Color.white.opacity(0.55), .clear],
                                       startPoint: .leading, endPoint: .trailing)
                            .frame(width: max(24, g.size.width * 0.45))
                            .offset(x: g.size.width * phase)
                    }
                    .mask(content)
                    .allowsHitTesting(false)
                )
                .onAppear {
                    withAnimation(.linear(duration: 1.5).repeatForever(autoreverses: false)) { phase = 1.2 }
                }
        } else {
            content
        }
    }
}

extension View {
    func shimmer(_ active: Bool = true) -> some View { modifier(ShimmerModifier(active: active)) }
}

/// Three dots breathing in sequence: the agent has the turn but has not
/// produced anything yet.
struct TypingDots: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        if reduceMotion {
            Text("…").foregroundStyle(.secondary)
        } else {
            TimelineView(.animation(minimumInterval: 1.0 / 20)) { ctx in
                let t = ctx.date.timeIntervalSinceReferenceDate
                HStack(spacing: 4) {
                    ForEach(0..<3, id: \.self) { i in
                        let p = (sin(t * 5.0 - Double(i) * 0.8) + 1) / 2
                        Circle()
                            .fill(Color.secondary)
                            .frame(width: 6, height: 6)
                            .opacity(0.3 + 0.7 * p)
                            .scaleEffect(0.7 + 0.3 * p)
                    }
                }
            }
            .accessibilityLabel("กำลังประมวลผล")
        }
    }
}

// MARK: - Pieces

/// The agent's mark in a quiet circle, the left rail of an agent turn.
struct AgentAvatar: View {
    let provider: String
    var size: CGFloat = 28

    var body: some View {
        ProviderIcon(provider: provider, size: size * 0.54)
            .frame(width: size, height: size)
            .background(Color.primary.opacity(0.06), in: Circle())
            .overlay(Circle().stroke(Color.primary.opacity(0.08), lineWidth: 1))
    }
}

/// Bubble outline with the corner nearest the speaker squared off, like the
/// chat apps people already know: user on the right, agent on the left.
func chatBubbleShape(isUser: Bool) -> UnevenRoundedRectangle {
    UnevenRoundedRectangle(
        cornerRadii: .init(topLeading: isUser ? 16 : 5,
                           bottomLeading: 16,
                           bottomTrailing: 16,
                           topTrailing: isUser ? 5 : 16),
        style: .continuous)
}

/// A run of finished tool calls from the ledger, collapsed to one line.
struct ToolGroupRow: View {
    let events: [LedgerEvent]
    @State private var open = false

    private var calls: Int {
        let n = events.filter { $0.kind == "tool_call" }.count
        return n > 0 ? n : events.count
    }

    private var preview: String {
        let source = events.last(where: { $0.kind == "tool_call" }) ?? events[events.count - 1]
        let line = source.body.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: true).first.map(String.init) ?? ""
        return line.count > 70 ? String(line.prefix(70)) + "…" : line
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button {
                withAnimation(.spring(response: 0.3, dampingFraction: 0.85)) { open.toggle() }
            } label: {
                HStack(spacing: 7) {
                    Image(systemName: "wrench.and.screwdriver").font(.caption2).foregroundStyle(.secondary)
                    Text("ใช้เครื่องมือ \(calls) ครั้ง").font(.caption.weight(.medium))
                    if !open, !preview.isEmpty {
                        Text(preview)
                            .font(.caption2).foregroundStyle(.tertiary)
                            .lineLimit(1).truncationMode(.tail)
                    }
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right")
                        .font(.caption2.weight(.semibold)).foregroundStyle(.tertiary)
                        .rotationEffect(.degrees(open ? 90 : 0))
                }
                .foregroundStyle(.secondary)
                .contentShape(Rectangle())
            }
            .buttonStyle(.chorusPlain)

            if open {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(events) { EventRow(event: $0) }
                }
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .padding(.horizontal, 10).padding(.vertical, 6)
        .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .padding(.leading, 38) // aligned with the agent's text, past the avatar rail
    }
}

/// The running turn's tool calls: one line showing what is happening now,
/// the full list a click away.
struct LiveToolsView: View {
    let tools: [Engine.LiveTurn.Tool]
    @State private var open = false

    var body: some View {
        let current = tools.last(where: { $0.state == "running" })
        let shown = current ?? tools.last
        VStack(alignment: .leading, spacing: 6) {
            Button {
                withAnimation(.spring(response: 0.3, dampingFraction: 0.85)) { open.toggle() }
            } label: {
                HStack(spacing: 7) {
                    if current != nil {
                        ProgressView().controlSize(.mini)
                    } else if shown?.state == "failed" {
                        Image(systemName: "xmark.circle.fill").foregroundStyle(.red).font(.caption)
                    } else {
                        Image(systemName: "checkmark.circle.fill").foregroundStyle(.green).font(.caption)
                    }
                    Text(shown?.title ?? "")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .shimmer(current != nil)
                        .id(shown?.id)
                        .transition(.push(from: .bottom).combined(with: .opacity))
                    Text("\(tools.count) ขั้นตอน")
                        .font(.caption2.monospacedDigit()).foregroundStyle(.secondary)
                        .padding(.horizontal, 6).padding(.vertical, 1.5)
                        .background(Color.primary.opacity(0.06), in: Capsule())
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right")
                        .font(.caption2.weight(.semibold)).foregroundStyle(.tertiary)
                        .rotationEffect(.degrees(open ? 90 : 0))
                }
                .contentShape(Rectangle())
                .clipped()
            }
            .buttonStyle(.chorusPlain)
            .animation(.spring(response: 0.3, dampingFraction: 0.85), value: shown?.id)

            if open {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(tools) { t in
                        HStack(spacing: 7) {
                            switch t.state {
                            case "running": ProgressView().controlSize(.mini)
                            case "failed": Image(systemName: "xmark.circle.fill").foregroundStyle(.red).font(.caption2)
                            default: Image(systemName: "checkmark.circle.fill").foregroundStyle(.green).font(.caption2)
                            }
                            Text(t.title).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                }
                .padding(.leading, 2)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .padding(.horizontal, 10).padding(.vertical, 6)
        .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}

/// A streamed reply, split at its last blank line so the finished paragraphs
/// are a cache hit in `MarkdownText` and only the paragraph still being
/// written is parsed again for each new token.
struct StreamingMarkdown: View {
    let text: String

    static func split(_ text: String) -> (stable: String, tail: String) {
        guard let r = text.range(of: "\n\n", options: .backwards) else { return ("", text) }
        let head = String(text[..<r.lowerBound])
        // Inside an unclosed code fence the split would cut the block in two.
        if head.components(separatedBy: "```").count % 2 == 0 { return ("", text) }
        return (head, String(text[r.upperBound...]))
    }

    var body: some View {
        let (stable, tail) = Self.split(text)
        VStack(alignment: .leading, spacing: 12) {
            if !stable.isEmpty { MarkdownText(text: stable, hug: true) }
            if !tail.isEmpty { MarkdownText(text: tail, hug: true) }
        }
    }
}
