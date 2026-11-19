import AppKit
import SwiftUI

// The conversation of the v0.4 window. Built for long chats: a LazyVStack so
// only what is on screen is laid out, tool calls folded into one line per run,
// and the streaming reply in its own view that observes `LiveStream`, so a
// token re-renders that view and not the whole transcript.

enum ProviderTint {
    static func color(_ agent: String) -> Color {
        switch agent.lowercased() {
        case "claude": Color(red: 0.80, green: 0.45, blue: 0.30)
        case "codex": Color(red: 0.25, green: 0.55, blue: 0.45)
        case "antigravity", "agy": Color(red: 0.30, green: 0.45, blue: 0.85)
        default: Color.secondary
        }
    }

    /// `claude@work:opus` → `claude`.
    static func provider(of actor: String) -> String {
        let base = String(actor.split(separator: ":", maxSplits: 1).first ?? Substring(actor))
        return String(base.split(separator: "@", maxSplits: 1).first ?? Substring(base))
    }
}

/// What the transcript draws: ledger events, with each run of tool events
/// folded into a single item.
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

private struct ToolRun {
    let firstSeq: Int64
    var calls: [String] = []
    var edits = 0
    var failed = 0

    var summary: String {
        var parts = ["ใช้เครื่องมือ \(calls.count) ครั้ง"]
        if edits > 0 { parts.append("แก้ \(edits) ไฟล์") }
        if failed > 0 { parts.append("ล้มเหลว \(failed)") }
        return parts.joined(separator: " · ")
    }
}

struct ShellTranscript: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        let items = Self.fold(engine.events)
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 16) {
                    ForEach(items) { item in
                        row(item)
                    }
                    LiveTurnView(stream: engine.liveStream, agent: engine.liveAgent, busy: engine.busy)
                    Color.clear.frame(height: 1).id("end")
                }
                .padding(.horizontal, 24).padding(.vertical, 16)
                // A readable measure: very long lines are tiring on a wide window.
                .frame(maxWidth: 860)
                .frame(maxWidth: .infinity)
            }
            .defaultScrollAnchor(.bottom)
            .onChange(of: engine.events.count) { _, _ in proxy.scrollTo("end", anchor: .bottom) }
            .onChange(of: engine.sessionID) { _, _ in proxy.scrollTo("end", anchor: .bottom) }
            .overlay {
                if items.isEmpty && !engine.busy { EmptyConversation() }
            }
        }
    }

    @ViewBuilder
    private func row(_ item: Item) -> some View {
        switch item {
        case .message(let e):
            if e.actor == "user" { UserMessage(text: e.body) } else { AgentMessage(event: e) }
        case .handoff(let e):
            HandoffDivider(json: e.body)
        case .tools(let run):
            ToolRunRow(run: run)
        case .note(let e):
            Text(e.body).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
        }
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
                    run?.calls.append(String((e.body.split(whereSeparator: \.isNewline).first ?? "").prefix(140)))
                } else if e.kind == "diff" {
                    run?.edits += 1
                } else if e.body.contains("failed") {
                    run?.failed += 1
                }
            case "msg":
                flush(); out.append(.message(e))
            case "handoff":
                flush(); out.append(.handoff(e))
            default:
                flush()
                if !e.body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { out.append(.note(e)) }
            }
        }
        flush()
        return out
    }
}

private struct EmptyConversation: View {
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        ContentUnavailableView {
            Label("พร้อมเริ่มงาน", systemImage: "sparkles")
        } description: {
            Text(shell.workspace.isEmpty ? "เพิ่มโปรเจกต์ที่แถบซ้ายเพื่อเริ่ม"
                                         : "บอก agent ว่าอยากให้ทำอะไรใน \((shell.workspace as NSString).lastPathComponent)")
        }
    }
}

// MARK: messages

private struct UserMessage: View {
    let text: String

    var body: some View {
        HStack {
            Spacer(minLength: 80)
            Text(text)
                .textSelection(.enabled)
                .padding(.horizontal, 12).padding(.vertical, 8)
                .background(Color.accentColor.opacity(0.14), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        }
        .accessibilityLabel("คุณ")
    }
}

private struct AgentMessage: View {
    @EnvironmentObject private var engine: Engine
    let event: LedgerEvent

    var body: some View {
        let provider = ProviderTint.provider(of: event.actor)
        let label = engine.agents.first { $0.id == provider }?.label ?? provider
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Circle().fill(ProviderTint.color(provider)).frame(width: 8, height: 8)
                Text(label).font(.callout.weight(.semibold))
                if let m = event.model_id {
                    Text(ModelID(m).base).font(.caption).foregroundStyle(.secondary)
                }
            }
            MarkdownBody(text: event.body)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct HandoffDivider: View {
    let json: String

    var body: some View {
        HStack(spacing: 10) {
            line
            Text(label).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            line
        }
    }

    private var line: some View { Rectangle().fill(Color.primary.opacity(0.12)).frame(height: 1) }

    private var label: String {
        guard let d = json.data(using: .utf8),
              let o = try? JSONSerialization.jsonObject(with: d) as? [String: Any] else { return "ส่งต่อ" }
        let h = (o["handoff"] as? [String: Any]) ?? o
        let from = (h["from"] as? String).map(ProviderTint.provider(of:)) ?? "?"
        let to = (h["to"] as? String).map(ProviderTint.provider(of:)) ?? "?"
        return "ส่งต่อ \(from) → \(to)"
    }
}

private struct ToolRunRow: View {
    let run: ToolRun
    @State private var open = false

    var body: some View {
        DisclosureGroup(isExpanded: $open) {
            VStack(alignment: .leading, spacing: 3) {
                ForEach(Array(run.calls.enumerated()), id: \.offset) { _, c in
                    Text(c).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            .padding(.top, 4)
        } label: {
            Label(run.summary, systemImage: "wrench.and.screwdriver")
                .font(.caption).foregroundStyle(.secondary)
        }
    }
}

// MARK: streaming

/// The reply being written. Observes the stream directly so the transcript
/// above it does not re-render per token.
private struct LiveTurnView: View {
    @ObservedObject var stream: LiveStream
    let agent: String?
    let busy: Bool

    var body: some View {
        if busy || !stream.turn.isEmpty {
            let provider = agent.map(ProviderTint.provider(of:)) ?? ""
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 6) {
                    Circle().fill(ProviderTint.color(provider)).frame(width: 8, height: 8)
                    Text(provider.isEmpty ? "agent" : provider.capitalized).font(.callout.weight(.semibold))
                    if busy { ProgressView().controlSize(.mini) }
                }
                if let tool = stream.turn.tools.last(where: { $0.state == "running" }) {
                    Label(tool.title, systemImage: "wrench.and.screwdriver")
                        .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                if !stream.turn.message.isEmpty {
                    MarkdownBody(text: stream.turn.message)
                } else if !stream.turn.thoughts.isEmpty {
                    Text(String(stream.turn.thoughts.suffix(240)))
                        .font(.callout).foregroundStyle(.secondary).italic().lineLimit(3)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

// MARK: markdown

/// Paragraphs with inline Markdown and fenced code blocks. Not a full
/// Markdown renderer: it covers what agents write in a chat.
struct MarkdownBody: View {
    let text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(Self.blocks(text).enumerated()), id: \.offset) { _, b in
                switch b {
                case .prose(let s): Prose(text: s)
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
        for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
            let t = line.trimmingCharacters(in: .whitespaces)
            if t.hasPrefix("```") {
                if inCode {
                    out.append(.code(lang, code.joined(separator: "\n")))
                    code = []; inCode = false
                } else {
                    let p = prose.joined(separator: "\n")
                    if !p.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { out.append(.prose(p)) }
                    prose = []; lang = String(t.dropFirst(3)); inCode = true
                }
            } else if inCode {
                code.append(line)
            } else {
                prose.append(line)
            }
        }
        if inCode { out.append(.code(lang, code.joined(separator: "\n"))) }
        let p = prose.joined(separator: "\n")
        if !p.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { out.append(.prose(p)) }
        return out
    }

    private struct Prose: View {
        let text: String
        var body: some View {
            if let a = try? AttributedString(
                markdown: text,
                options: .init(allowsExtendedAttributes: true,
                               interpretedSyntax: .inlineOnlyPreservingWhitespace)) {
                Text(a).fixedSize(horizontal: false, vertical: true)
            } else {
                Text(text).fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private struct CodeBlock: View {
        let language: String
        let code: String
        @State private var copied = false

        var body: some View {
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    Text(language.isEmpty ? "code" : language).font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    Button {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(code, forType: .string)
                        copied = true
                        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { copied = false }
                    } label: {
                        Label(copied ? "คัดลอกแล้ว" : "คัดลอก", systemImage: copied ? "checkmark" : "doc.on.doc")
                            .font(.caption)
                    }
                    .buttonStyle(.borderless)
                }
                .padding(.horizontal, 10).padding(.vertical, 5)
                Divider()
                ScrollView(.horizontal) {
                    Text(code).font(.system(.callout, design: .monospaced))
                        .padding(10)
                        .fixedSize(horizontal: true, vertical: false)
                }
            }
            .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        }
    }
}
