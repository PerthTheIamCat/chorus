import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

/// ⌘K: type to filter, ↩ runs the first match.
struct PaletteSheet: View {
    let items: [(title: String, icon: String, run: () -> Void)]
    let dismiss: () -> Void
    @State private var query = ""

    private var shown: [(title: String, icon: String, run: () -> Void)] {
        query.isEmpty ? items : items.filter { $0.title.localizedCaseInsensitiveContains(query) }
    }

    var body: some View {
        VStack(spacing: 0) {
            TextField("พิมพ์คำสั่ง…", text: $query)
                .textFieldStyle(.plain).font(.title3).padding(14)
                .onSubmit { if let f = shown.first { dismiss(); f.run() } }
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(Array(shown.enumerated()), id: \.offset) { _, item in
                        Button { dismiss(); item.run() } label: {
                            Label(item.title, systemImage: item.icon)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .buttonStyle(InteractiveStyle(cornerRadius: 7, padding: .init(top: 7, leading: 12, bottom: 7, trailing: 12)))
                    }
                }.padding(6)
            }
        }
        .frame(width: 460, height: 320)
    }
}


/// An isolated timer so TimelineView does not invalidate the entire working row every second.
struct TurnTimerView: View {
    let startedAt: Date
    let isEmpty: Bool

    var body: some View {
        TimelineView(.periodic(from: startedAt, by: 1)) { ctx in
            let sec = max(0, Int(ctx.date.timeIntervalSince(startedAt)))
            Text(isEmpty ? "กำลังเริ่ม agent… \(sec) วิ" : "กำลังตอบ… \(sec) วิ")
        }
    }
}

/// A macOS-native code block with language title bar, interactive Canvas preview, and one-click copy feedback.
struct CodeBlockView: View {
    let language: String
    let code: String
    @State private var copied = false
    @State private var showCanvas: Bool = true

    private var isCanvasSupported: Bool {
        let lang = language.lowercased()
        return lang == "mermaid" || lang == "svg" || lang == "html"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Image(systemName: isCanvasSupported ? "paintpalette.fill" : "curlybraces")
                    .font(.caption2)
                    .foregroundStyle(isCanvasSupported ? .purple : .secondary)
                Text(language.isEmpty ? "code" : language.lowercased())
                    .font(.caption2.weight(.semibold).monospaced())
                    .foregroundStyle(.secondary)

                if isCanvasSupported {
                    HStack(spacing: 4) {
                        Button {
                            showCanvas = true
                        } label: {
                            Text("แคนวาส")
                                .font(.system(size: 10, weight: showCanvas ? .bold : .regular))
                                .foregroundStyle(showCanvas ? .purple : .secondary)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(showCanvas ? Color.purple.opacity(0.14) : Color.clear, in: RoundedRectangle(cornerRadius: 4))
                        }
                        .buttonStyle(.chorusPlain)

                        Button {
                            showCanvas = false
                        } label: {
                            Text("โค้ด")
                                .font(.system(size: 10, weight: !showCanvas ? .bold : .regular))
                                .foregroundStyle(!showCanvas ? Color.primary : .secondary)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(!showCanvas ? Color.primary.opacity(0.08) : Color.clear, in: RoundedRectangle(cornerRadius: 4))
                        }
                        .buttonStyle(.chorusPlain)
                    }
                    .padding(2)
                    .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 6))
                }

                Spacer()
                Button {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(code, forType: .string)
                    withAnimation(.spring(response: 0.2, dampingFraction: 0.7)) {
                        copied = true
                    }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                        withAnimation { copied = false }
                    }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: copied ? "checkmark" : "doc.on.doc")
                            .font(.caption2)
                        Text(copied ? "คัดลอกแล้ว" : "คัดลอกโค้ด")
                            .font(.caption2)
                    }
                    .foregroundStyle(copied ? .green : .secondary)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 3)
                    .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 5, style: .continuous))
                }
                .buttonStyle(.chorusPlain)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(Color.primary.opacity(0.04))

            Divider().opacity(0.4)

            if isCanvasSupported && showCanvas {
                CanvasPreviewView(language: language, code: code)
            } else {
                ScrollView(.horizontal, showsIndicators: true) {
                    Text(code)
                        .font(.system(.callout, design: .monospaced))
                        .textSelection(.enabled)
                        .padding(10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
        .background(Color(nsColor: .textBackgroundColor).opacity(0.65), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(Color.primary.opacity(0.09), lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}

/// The running turn, live: reasoning (collapsible, like the thinking
/// block in Claude/Codex), tool calls as they start and finish, and the
/// answer as it streams — then Stop.
struct WorkingRow: View {
    @ObservedObject var stream: LiveStream
    let agentID: String
    let agentLabel: String
    let startedAt: Date?
    let onStop: () -> Void
    /// Called when the streamed content grows by a visible step, so the
    /// thread can keep following the bottom.
    let onGrow: () -> Void

    private var growth: Int {
        stream.turn.message.count / 80 + stream.turn.thoughts.count / 150 + stream.turn.tools.count
    }

    var body: some View {
        let live = stream.turn
        let shape = chatBubbleShape(isUser: false)
        return HStack(alignment: .top, spacing: 10) {
            AgentAvatar(provider: agentID)
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 7) {
                    Text(agentLabel)
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.primary.opacity(0.88))
                    Spacer(minLength: 0)
                    if let t = startedAt {
                        TurnTimerView(startedAt: t, isEmpty: live.isEmpty)
                            .font(.caption2.monospacedDigit())
                            .foregroundStyle(.tertiary)
                    }
                    Button(action: onStop) {
                        HStack(spacing: 4) {
                            Image(systemName: "stop.fill").font(.system(size: 8))
                            Text("หยุด")
                        }
                        .font(.caption2.weight(.medium))
                        .foregroundStyle(.red)
                        .padding(.horizontal, 8).padding(.vertical, 3.5)
                        .background(Color.red.opacity(0.08), in: Capsule())
                    }
                    .buttonStyle(.chorusPlain)
                    .help("หยุดเทิร์นนี้ (⌘.) — ล้างคิวด้วย")
                }

                if !live.thoughts.isEmpty {
                    ThinkingBlock(text: live.thoughts, running: live.message.isEmpty)
                        .transition(.opacity.combined(with: .offset(y: 6)))
                }

                if !live.tools.isEmpty {
                    LiveToolsView(tools: live.tools)
                        .transition(.opacity.combined(with: .offset(y: 6)))
                }

                if !live.message.isEmpty {
                    StreamingMarkdown(text: live.message)
                        .padding(.horizontal, 14).padding(.vertical, 10)
                        .background(Color(nsColor: .controlBackgroundColor).opacity(0.55), in: shape)
                        .overlay(shape.stroke(Color.primary.opacity(0.07), lineWidth: 1))
                        .frame(maxWidth: 720, alignment: .leading)
                        .transition(.opacity)
                } else if live.tools.isEmpty && live.thoughts.isEmpty {
                    TypingDots()
                        .padding(.horizontal, 14).padding(.vertical, 12)
                        .background(Color(nsColor: .controlBackgroundColor).opacity(0.55), in: shape)
                        .overlay(shape.stroke(Color.primary.opacity(0.07), lineWidth: 1))
                        .transition(.opacity)
                }
            }
            .animation(.spring(response: 0.35, dampingFraction: 0.88),
                       value: (live.thoughts.isEmpty ? 0 : 1) + (live.tools.isEmpty ? 0 : 2) + (live.message.isEmpty ? 0 : 4))
            Spacer(minLength: 24)
        }
        .onChange(of: growth) { _, _ in onGrow() }
    }
}

/// Markdown for agent output: fenced code becomes a monospaced block with header,
/// the rest goes through `AttributedString` (bold, italic, inline code, links) with
/// whitespace preserved so lists and paragraphs keep their shape.
struct MarkdownText: View {
    let text: String
    /// Size to the text instead of filling the width (chat bubbles).
    var hug = false

    enum Part {
        case code(language: String, body: String)
        case prose(AttributedString)
    }

    /// Parsed parts keyed by the exact text. Rows re-render far more often than
    /// their text changes (every scroll, hover and engine publish), and both
    /// the fence split and `AttributedString(markdown:)` used to run each time.
    /// Cost is the text length, so a streamed reply's many short-lived
    /// prefixes are evicted before the stable history is.
    private final class Box { let parts: [Part]; init(_ p: [Part]) { parts = p } }
    private static let cache: NSCache<NSString, Box> = {
        let c = NSCache<NSString, Box>()
        c.totalCostLimit = 8 * 1024 * 1024
        return c
    }()

    static func parts(for text: String) -> [Part] {
        let key = text as NSString
        if let hit = cache.object(forKey: key) { return hit.parts }
        let parsed = parse(text)
        cache.setObject(Box(parsed), forKey: key, cost: key.length * 2)
        return parsed
    }

    static func parse(_ text: String) -> [Part] {
        var out: [Part] = []
        for (i, chunk) in text.components(separatedBy: "```").enumerated() {
            guard !chunk.isEmpty else { continue }
            if i % 2 == 1 {
                let lines = chunk.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: false)
                let lang = lines.first.map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) } ?? ""
                let codeBody = lines.count > 1 ? String(lines[1]).trimmingCharacters(in: .newlines) : chunk.trimmingCharacters(in: .newlines)
                out.append(.code(language: lang, body: codeBody))
            } else {
                let trimmed = chunk.trimmingCharacters(in: .newlines)
                guard !trimmed.isEmpty else { continue }
                let attributed = (try? AttributedString(
                    markdown: trimmed,
                    options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(trimmed)
                out.append(.prose(attributed))
            }
        }
        return out
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(Self.parts(for: text).enumerated()), id: \.offset) { _, p in
                switch p {
                case let .code(language, body):
                    CodeBlockView(language: language, code: body)
                case let .prose(a):
                    Text(a).textSelection(.enabled).frame(maxWidth: hug ? nil : .infinity, alignment: .leading)
                }
            }
        }
    }
}

/// Reasoning, folded by default into one quiet line (like the thinking
/// row in Claude/ChatGPT): a shimmering "กำลังคิด…" with the newest line of
/// thought while it runs, and a click opens the full text.
struct ThinkingBlock: View {
    let text: String
    let running: Bool
    @State private var open: Bool?
    @AppStorage("chorus.thinkingOpen") private var thinkingOpen = false
    @AppStorage(DisplayKey.detail) private var detail = DetailLevel.normal.rawValue

    /// The last non-empty line, read from the tail only — the text grows by a
    /// token at a time and scanning all of it each time would be O(n²).
    private var latestLine: String {
        let tail = text.suffix(400)
        let line = tail.split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .last(where: { !$0.isEmpty }) ?? ""
        return line.replacingOccurrences(of: "**", with: "")
    }

    var body: some View {
        let level = DetailLevel.from(detail)
        let isOpen = open ?? level.thinkingOpenByDefault(userPref: running && thinkingOpen)
        VStack(alignment: .leading, spacing: 6) {
            Button {
                withAnimation(.spring(response: 0.3, dampingFraction: 0.85)) {
                    open = !isOpen
                }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "brain.head.profile")
                        .foregroundStyle(.purple)
                        .symbolEffect(.pulse, isActive: running)
                    Text(running ? "กำลังคิด…" : "กระบวนการคิด")
                        .font(.caption.weight(.semibold))
                        .shimmer(running && !isOpen)
                    if running, !isOpen, level.showsPreviews, !latestLine.isEmpty {
                        Text(latestLine)
                            .font(.caption2).foregroundStyle(.tertiary)
                            .lineLimit(1).truncationMode(.head)
                    }
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.tertiary)
                        .rotationEffect(.degrees(isOpen ? 90 : 0))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.chorusPlain)

            if isOpen {
                let cleanText = text.trimmingCharacters(in: .whitespacesAndNewlines)
                Group {
                    if cleanText.count > 300 {
                        ScrollViewReader { proxy in
                            ScrollView {
                                MarkdownText(text: cleanText)
                                    .font(.caption).foregroundStyle(.secondary)
                                    .textSelection(.enabled)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                Color.clear.frame(height: 1).id("end")
                            }
                            .frame(maxHeight: 220)
                            .onChange(of: cleanText.count) { _, _ in if running { proxy.scrollTo("end") } }
                        }
                    } else if !cleanText.isEmpty {
                        MarkdownText(text: cleanText)
                            .font(.caption).foregroundStyle(.secondary)
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    } else if running {
                        Text("กำลังใคร่ครวญและวิเคราะห์ข้อมูล…")
                            .font(.caption).foregroundStyle(.tertiary)
                    }
                }
                .padding(.top, 2)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .padding(.horizontal, 10).padding(.vertical, 7)
        .background(Color.purple.opacity(0.06), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(Color.purple.opacity(0.12), lineWidth: 1))
    }
}
