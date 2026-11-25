import AppKit
import SwiftUI

// Design board "Session — สนทนา": the thread on a readable measure (780 pt),
// the person's turns as quiet bubbles, each agent's turn led by its own mark,
// tool runs folded into one card, handoffs as a bar across the page with the
// new agent's echo-back, and permission/question cards inline where the agent
// asked. Built for long sessions: lazy rows, folding memoised per ledger
// length, Markdown parsed once per text, the streaming turn in its own view.

struct ConversationView: View {
    var body: some View {
        VStack(spacing: 0) {
            ThreadView()
            ComposerView()
        }
        .background(Palette.window)
    }
}

/// Width of an agent's avatar column; everything an agent did lines up past it.
enum Gutter {
    static let avatar: CGFloat = 32
    static let gap: CGFloat = 12
    static var text: CGFloat { avatar + gap }
}

private enum Item: Identifiable {
    case message(LedgerEvent)
    case handoff(LedgerEvent)
    case tools(ToolRun)
    case note(LedgerEvent)

    var id: String {
        switch self {
        case .message(let e), .handoff(let e), .note(let e): "e\(e.seq)"
        case .tools(let t): "t\(t.firstSeq)"
        }
    }
}

struct ToolRun {
    let firstSeq: Int64
    var calls: [String] = []
    var reads = 0
    var edits = 0
    var failed = 0

    var summary: String {
        var parts: [String] = []
        if reads > 0 { parts.append("อ่าน \(reads)") }
        if edits > 0 { parts.append("แก้ \(edits) ไฟล์") }
        let other = calls.count - reads - edits
        if other > 0 { parts.append("เครื่องมือ \(other)") }
        if failed > 0 { parts.append("ล้มเหลว \(failed)") }
        return parts.isEmpty ? "ใช้เครื่องมือ \(calls.count) ครั้ง" : parts.joined(separator: " · ")
    }

    /// File-ish words from the calls, for the one-line preview.
    var targets: String {
        let names = calls.compactMap { c -> String? in
            c.split(separator: " ").last.map(String.init).flatMap { w in
                w.contains(".") || w.contains("/") ? (w as NSString).lastPathComponent : nil
            }
        }
        var seen = Set<String>()
        return names.filter { seen.insert($0).inserted }.prefix(5).joined(separator: " · ")
    }
}

struct ThreadView: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        let items = Self.folded(engine.events, session: engine.sessionID)
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 22) {
                    ForEach(items) { row($0) }
                    LiveTurnView(stream: engine.liveStream, agent: engine.liveAgent, busy: engine.busy)
                    PromptStack()
                    Color.clear.frame(height: 1).id("end")
                }
                .padding(.horizontal, 32).padding(.top, 24).padding(.bottom, 12)
                .frame(maxWidth: 780 + 64)
                .frame(maxWidth: .infinity)
            }
            .defaultScrollAnchor(.bottom)
            .opacity(engine.switching ? 0.55 : 1)
            .animation(.easeOut(duration: 0.15), value: engine.switching)
            .onChange(of: engine.events.count) { _, _ in proxy.scrollTo("end", anchor: .bottom) }
            .onChange(of: engine.sessionID) { _, _ in proxy.scrollTo("end", anchor: .bottom) }
            .overlay { if items.isEmpty && !engine.busy { EmptySession() } }
        }
    }

    @ViewBuilder
    private func row(_ item: Item) -> some View {
        switch item {
        case .message(let e):
            if e.actor == "user" { UserBubble(event: e) } else { AgentTurn(event: e) }
        case .handoff(let e): HandoffBar(json: e.body)
        case .tools(let run): ToolRunCard(run: run)
        case .note(let e):
            Text(e.body).font(Typo.caption).foregroundStyle(Palette.text3).textSelection(.enabled)
                .padding(.leading, Gutter.text)
        }
    }

    // The engine publishes many times a turn (status, metrics, quota) while the
    // ledger only grows; fold again only when it did.
    @MainActor private static var memo: (key: String, items: [Item])?

    @MainActor private static func folded(_ events: [LedgerEvent], session: Int64) -> [Item] {
        let key = "\(session):\(events.count):\(events.last?.seq ?? -1)"
        if let m = memo, m.key == key { return m.items }
        let items = fold(events)
        memo = (key, items)
        return items
    }

    private static func fold(_ events: [LedgerEvent]) -> [Item] {
        var out: [Item] = []
        var run: ToolRun?
        func flush() { if let r = run { out.append(.tools(r)); run = nil } }
        for e in events {
            switch e.kind {
            case "tool_call", "tool_result", "diff":
                if run == nil { run = ToolRun(firstSeq: e.seq) }
                if e.kind == "tool_call" {
                    let line = String((e.body.split(whereSeparator: \.isNewline).first ?? "").prefix(160))
                    run?.calls.append(line)
                    let l = line.lowercased()
                    if l.hasPrefix("read") || l.hasPrefix("อ่าน") || l.contains("cat ") || l.hasPrefix("view") { run?.reads += 1 }
                    if l.hasPrefix("edit") || l.hasPrefix("write") || l.hasPrefix("แก้") { run?.edits += 1 }
                } else if e.kind == "diff" {
                    if run?.edits == 0 { run?.edits = 1 }
                } else if e.body.contains("failed") {
                    run?.failed += 1
                }
            case "msg": flush(); out.append(.message(e))
            case "handoff": flush(); out.append(.handoff(e))
            default:
                flush()
                if !e.body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { out.append(.note(e)) }
            }
        }
        flush()
        return out
    }
}

private struct EmptySession: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        VStack(spacing: 12) {
            ProviderStack(providers: engine.agents.filter(\.ready).map(\.id), size: 34)
            Text(shell.workspace.isEmpty ? "เลือกโปรเจกต์ทางซ้าย" : "เริ่มงานใน \((shell.workspace as NSString).lastPathComponent)")
                .font(.system(size: 15, weight: .semibold)).foregroundStyle(Palette.text)
            Text("\(engine.agents.filter(\.ready).count) agent พร้อม · ความจำ \(engine.facts.count) ข้อจะติดไปทุกเทิร์น")
                .font(Typo.small).foregroundStyle(Palette.text3)
            HStack(spacing: 6) {
                ForEach(["อธิบาย repo นี้", "แก้เทสต์ที่ล้ม", "ทำต่อจาก session ล่าสุด"], id: \.self) { s in
                    Button(s) { shell.text = s }.buttonStyle(ChipStyle())
                }
            }
        }
        .padding(24)
    }
}

// MARK: messages

private struct UserBubble: View {
    let event: LedgerEvent
    @State private var hover = false

    var body: some View {
        HStack(alignment: .bottom, spacing: 6) {
            Spacer(minLength: 120)
            if hover { CopyButton(text: event.body) }
            Text(event.body)
                .font(Typo.body).lineSpacing(3)
                .foregroundStyle(Palette.text)
                .textSelection(.enabled)
                .padding(.horizontal, 14).padding(.vertical, 10)
                .background(Palette.surface, in: UnevenRoundedRectangle(topLeadingRadius: 14, bottomLeadingRadius: 14,
                                                                        bottomTrailingRadius: 4, topTrailingRadius: 14,
                                                                        style: .continuous))
                .overlay(UnevenRoundedRectangle(topLeadingRadius: 14, bottomLeadingRadius: 14, bottomTrailingRadius: 4,
                                                topTrailingRadius: 14, style: .continuous).strokeBorder(Palette.line))
        }
        .onHover { hover = $0 }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("คุณ: \(event.body)")
    }
}

private struct AgentTurn: View {
    @EnvironmentObject private var engine: Engine
    let event: LedgerEvent
    @State private var hover = false

    var body: some View {
        let provider = ProviderTint.provider(of: event.actor)
        let label = engine.agents.first { $0.id == provider }?.label ?? provider.capitalized
        HStack(alignment: .top, spacing: Gutter.gap) {
            ProviderAvatar(provider: provider, size: Gutter.avatar)
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 8) {
                    Text(label).font(.system(size: 12, weight: .semibold)).foregroundStyle(Palette.text)
                    Text(meta).font(Typo.caption).foregroundStyle(Palette.text3)
                    Spacer(minLength: 0)
                    if hover { CopyButton(text: event.body) }
                }
                .frame(minHeight: Gutter.avatar)
                MarkdownBody(text: event.body)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
        .onHover { hover = $0 }
    }

    private var meta: String {
        var parts: [String] = []
        if let m = event.model_id { parts.append(ModelID(m).base) }
        if let c = event.cost_usd { parts.append(String(format: "$%.4f", c)) }
        return parts.joined(separator: " · ")
    }
}

private struct CopyButton: View {
    let text: String
    @State private var copied = false
    var body: some View {
        Button {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
            copied = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.4) { copied = false }
        } label: {
            Image(systemName: copied ? "checkmark" : "doc.on.doc").font(.system(size: 11))
        }
        .buttonStyle(IconButtonStyle())
        .help(copied ? "คัดลอกแล้ว" : "คัดลอก")
        .accessibilityLabel("คัดลอกข้อความ")
    }
}

/// "อ่าน 4 · แก้ 2 ไฟล์   handoff.rs · session.rs" — one card per tool run.
private struct ToolRunCard: View {
    let run: ToolRun
    @State private var open = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button { withAnimation(.snappy(duration: 0.18)) { open.toggle() } } label: {
                HStack(spacing: 8) {
                    Image(systemName: "chevron.right").font(.system(size: 9, weight: .bold))
                        .rotationEffect(.degrees(open ? 90 : 0))
                    Text(run.summary).font(Typo.small).lineLimit(1)
                    Text(run.targets).font(Typo.monoSmall).foregroundStyle(Palette.text3).lineLimit(1)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    if run.failed > 0 {
                        Image(systemName: "exclamationmark.triangle.fill").font(.system(size: 10)).foregroundStyle(Palette.warn)
                    }
                }
                .foregroundStyle(Palette.text2)
                .padding(.horizontal, 10).frame(height: 32)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if open {
                Hairline()
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(Array(run.calls.enumerated()), id: \.offset) { _, c in
                        Text(c).font(Typo.monoSmall).foregroundStyle(Palette.text2).lineLimit(1).textSelection(.enabled)
                    }
                }
                .padding(10)
            }
        }
        .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Palette.line))
        .padding(.leading, Gutter.text)
    }
}

/// The bar across the thread when work changes hands, plus the brief summary.
private struct HandoffBar: View {
    let json: String

    var body: some View {
        let h = Self.parse(json)
        VStack(spacing: 10) {
            HStack(spacing: 10) {
                Hairline()
                HStack(spacing: 8) {
                    ProviderIcon(provider: h.from, size: 13)
                    Image(systemName: "arrow.right").font(.system(size: 10)).foregroundStyle(Palette.text3)
                    ProviderIcon(provider: h.to, size: 13)
                    Text(h.auto ? "ส่งไม้อัตโนมัติ" : "ส่งไม้").font(Typo.caption).foregroundStyle(Palette.text)
                    if let r = h.reason { Text("· \(r)").font(Typo.caption).foregroundStyle(Palette.text3) }
                }
                .padding(.horizontal, 12).frame(height: 28)
                .background(Palette.surface, in: Capsule())
                .overlay(Capsule().strokeBorder(Palette.line2))
                .fixedSize()
                Hairline()
            }
            if let goal = h.goal {
                HStack(alignment: .top, spacing: 12) {
                    (Text("\(h.to.capitalized) รับงาน: ").fontWeight(.medium).foregroundColor(Palette.text)
                     + Text(goal) + Text(h.pending > 0 ? " · ค้าง \(h.pending) ข้อ" : ""))
                        .font(Typo.small).foregroundStyle(Palette.text2).lineSpacing(2)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 14).padding(.vertical, 10)
                .background(Palette.hover, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .padding(.horizontal, Gutter.text)
            }
        }
        .accessibilityElement(children: .combine)
    }

    private struct Parsed { let from: String; let to: String; let reason: String?; let auto: Bool; let goal: String?; let pending: Int }
    private static let cache = RenderCache<Parsed>(limit: 64)

    private static func parse(_ json: String) -> Parsed {
        cache.value(for: json) {
            guard let d = json.data(using: .utf8),
                  let o = try? JSONSerialization.jsonObject(with: d) as? [String: Any] else {
                return Parsed(from: "?", to: "?", reason: nil, auto: false, goal: nil, pending: 0)
            }
            let h = (o["handoff"] as? [String: Any]) ?? o
            let reason = h["reason"] as? String
            let label: String? = switch reason {
            case "rate_limit": "ติดลิมิตชั่วคราว"
            case "quota", "quota_exhausted": "โควตาหมด"
            case "context_exhausted": "context เต็ม"
            case "user", "manual", nil: nil
            case .some(let s): s
            }
            let goal = (o["goal"] as? String).flatMap { $0.isEmpty ? nil : $0 }
            return Parsed(from: (h["from"] as? String).map(ProviderTint.provider(of:)) ?? "?",
                          to: (h["to"] as? String).map(ProviderTint.provider(of:)) ?? "?",
                          reason: label, auto: reason != nil && reason != "user" && reason != "manual",
                          goal: goal, pending: (o["pending"] as? [Any])?.count ?? 0)
        }
    }
}

// MARK: streaming

/// The reply being written; observes the stream so a token re-renders only this.
private struct LiveTurnView: View {
    @ObservedObject var stream: LiveStream
    let agent: String?
    let busy: Bool

    var body: some View {
        if busy || !stream.turn.isEmpty {
            let provider = agent.map(ProviderTint.provider(of:)) ?? ""
            HStack(alignment: .top, spacing: Gutter.gap) {
                ProviderAvatar(provider: provider, size: Gutter.avatar)
                VStack(alignment: .leading, spacing: 8) {
                    HStack(spacing: 8) {
                        Text(provider.isEmpty ? "agent" : provider.capitalized)
                            .font(.system(size: 12, weight: .semibold)).foregroundStyle(Palette.text)
                        if busy { Tag(text: "กำลังทำ", tone: .accent) }
                    }
                    .frame(minHeight: Gutter.avatar)
                    if !stream.turn.tools.isEmpty {
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(Array(stream.turn.tools.suffix(4).enumerated()), id: \.offset) { i, t in
                                if i > 0 { Hairline() }
                                HStack(spacing: 8) {
                                    if t.state == "running" { Spinner(size: 11) } else {
                                        Image(systemName: t.state == "failed" ? "xmark" : "checkmark")
                                            .font(.system(size: 10, weight: .bold))
                                            .foregroundStyle(t.state == "failed" ? Palette.bad : Palette.ok)
                                    }
                                    Text(t.title).font(Typo.monoSmall).foregroundStyle(Palette.text2).lineLimit(1)
                                }
                                .padding(.horizontal, 10).frame(height: 30)
                            }
                        }
                        .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Palette.line))
                    }
                    if !stream.turn.thoughts.isEmpty {
                        ThinkingBox(text: stream.turn.thoughts, streaming: stream.turn.message.isEmpty)
                    }
                    if !stream.turn.message.isEmpty { MarkdownBody(text: stream.turn.message, cached: false) }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

private struct ThinkingBox: View {
    let text: String
    let streaming: Bool
    @AppStorage("chorus.thinkingOpen") private var openByDefault = false
    @State private var open: Bool?

    var body: some View {
        DisclosureGroup(isExpanded: Binding(get: { open ?? openByDefault }, set: { open = $0 })) {
            ScrollView {
                Text(text).font(Typo.small).foregroundStyle(Palette.text3).textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxHeight: 200)
        } label: {
            Label(streaming ? "กำลังคิด…" : "ความคิดของ agent", systemImage: "brain")
                .font(Typo.caption).foregroundStyle(Palette.text3)
        }
    }
}

// MARK: markdown

/// Paragraphs with inline Markdown and fenced code blocks, parsed once per text.
struct MarkdownBody: View {
    let text: String
    var cached = true

    private static let blockCache = RenderCache<[Block]>(limit: 400)
    fileprivate static let proseCache = RenderCache<AttributedString?>(limit: 800)

    var body: some View {
        let blocks = cached ? Self.blockCache.value(for: text) { Self.blocks(text) } : Self.blocks(text)
        VStack(alignment: .leading, spacing: 10) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { i, b in
                switch b {
                // A streaming reply re-parses only its last paragraph.
                case .prose(let s): Prose(text: s, cached: cached || i < blocks.count - 1)
                case .code(let lang, let s): CodeBlock(language: lang, code: s)
                }
            }
        }
        .textSelection(.enabled)
    }

    enum Block { case prose(String), code(String, String) }

    static func blocks(_ text: String) -> [Block] {
        var out: [Block] = []
        var prose: [Substring] = []
        var code: [Substring] = []
        var lang = ""
        var inCode = false
        // One block per paragraph, not per run between fences: while a reply
        // streams, every finished paragraph is parsed once and only the last
        // one is parsed again per token (it was the whole reply, every token).
        func flushProse() {
            let joined = prose.joined(separator: "\n")
            prose = []
            for para in joined.components(separatedBy: "\n\n") {
                let trimmed = para.trimmingCharacters(in: .newlines)
                if !trimmed.trimmingCharacters(in: .whitespaces).isEmpty { out.append(.prose(trimmed)) }
            }
        }
        for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
            let t = line.trimmingCharacters(in: .whitespaces)
            if t.hasPrefix("```") {
                if inCode {
                    out.append(.code(lang, code.joined(separator: "\n"))); code = []; inCode = false
                } else {
                    flushProse()
                    lang = String(t.dropFirst(3)); inCode = true
                }
            } else if inCode { code.append(line) } else { prose.append(line) }
        }
        if inCode { out.append(.code(lang, code.joined(separator: "\n"))) }
        flushProse()
        return out
    }

    private struct Prose: View {
        let text: String
        var cached = true
        private static func parse(_ text: String) -> AttributedString? {
            try? AttributedString(markdown: text, options: .init(allowsExtendedAttributes: true,
                                                                  interpretedSyntax: .inlineOnlyPreservingWhitespace))
        }
        var body: some View {
            let parsed = cached ? MarkdownBody.proseCache.value(for: text) { Self.parse(text) } : Self.parse(text)
            Group {
                if let a = parsed { Text(a) } else { Text(text) }
            }
            .font(Typo.body).lineSpacing(4)
            .foregroundStyle(Palette.text)
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    private struct CodeBlock: View {
        let language: String
        let code: String
        var body: some View {
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    Text(language.isEmpty ? "code" : language).font(Typo.tiny).foregroundStyle(Palette.text3)
                    Spacer()
                    CopyButton(text: code)
                }
                .padding(.leading, 12).padding(.trailing, 4).frame(height: 30)
                Hairline()
                ScrollView(.horizontal) {
                    Text(code).font(Typo.mono).foregroundStyle(Palette.text).lineSpacing(3)
                        .padding(12).fixedSize(horizontal: true, vertical: false)
                }
            }
            .background(Palette.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Palette.line))
        }
    }
}
