import SwiftUI
import UniformTypeIdentifiers

// The composer of the v0.4 window: a text field and two choices, nothing
// else. Permission (ask / plan / allow) and model. Choosing another
// provider's model prepares the handoff, and sending is the handoff, so there
// is no hand-over button (board 2).

struct ShellComposer: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @AppStorage(DisplayKey.currency) private var currency = "auto"
    @AppStorage(SettingsKey.thbRate) private var thbRate = ""
    @FocusState private var focused: Bool
    @State private var picking = false

    var body: some View {
        VStack(spacing: 8) {
            Notices()
            PromptStack()
            if !shell.queued.isEmpty { queue }
            HandoffStrip()
            if let status = shell.routeStatus {
                RouteStatusCard(status: status) { shell.routeStatus = nil }
            }
            completions
            field
            metrics
        }
        .padding(.horizontal, 24).padding(.bottom, 12).padding(.top, 4)
        .frame(maxWidth: 860 + 48)
        .frame(maxWidth: .infinity)
        .onChange(of: engine.sessionID) { _, _ in focused = true }
        .onAppear { focused = true }
        .fileImporter(isPresented: $picking, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            if case .success(let urls) = result { shell.attach(urls: urls) }
        }
        .onDrop(of: [.fileURL], isTargeted: nil) { providers in
            for p in providers {
                _ = p.loadObject(ofClass: URL.self) { url, _ in
                    guard let url else { return }
                    Task { @MainActor in shell.attach(urls: [url]) }
                }
            }
            return true
        }
    }

    private var field: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !shell.attachments.isEmpty || shell.importing > 0 {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(shell.attachments) { a in
                            HStack(spacing: 4) {
                                Image(systemName: a.mediaType.hasPrefix("image/") ? "photo" : "doc")
                                Text(a.name).lineLimit(1)
                                Button { shell.removeAttachment(a.id) } label: { Image(systemName: "xmark.circle.fill") }
                                    .buttonStyle(.borderless)
                            }
                            .font(.caption)
                            .padding(.horizontal, 8).padding(.vertical, 4)
                            .background(Color.primary.opacity(a.agentReadable ? 0.06 : 0.0), in: Capsule())
                            .overlay(Capsule().strokeBorder(a.agentReadable ? Color.clear : Color.red.opacity(0.6)))
                        }
                        if shell.importing > 0 { ProgressView().controlSize(.small) }
                    }
                }
            }
            TextField(engine.nextHint ?? "ขอให้ทำอะไร…  (@ เลือก agent หรือไฟล์ · / คำสั่ง)", text: $shell.text, axis: .vertical)
                .textFieldStyle(.plain)
                .lineLimit(1...10)
                .focused($focused)
                .onSubmit { shell.send() }
                .onKeyPress(.tab) {
                    if let first = suggestions.first { pick(first); return .handled }
                    if shell.text.isEmpty, let hint = engine.nextHint { shell.text = hint; engine.nextHint = nil; return .handled }
                    return .ignored
                }
                .onPasteCommand(of: [.image]) { providers in
                    for p in providers {
                        _ = p.loadDataRepresentation(for: .image) { data, _ in
                            guard let data else { return }
                            Task { @MainActor in shell.attach(image: data, mime: "image/png") }
                        }
                    }
                }
            HStack(spacing: 8) {
                Button { picking = true } label: { Image(systemName: "paperclip") }
                    .buttonStyle(.borderless)
                    .help("แนบไฟล์หรือรูป")
                ModeMenu()
                RouteMenu()
                ModelMenu()
                Button { shell.runCompare() } label: { Image(systemName: "square.split.2x1") }
                    .buttonStyle(.borderless)
                    .disabled(engine.agents.filter(\.ready).count < 2 || !shell.canCompare || shell.comparing)
                    .help("เทียบ agent: ถามทุกตัวด้วยคำถามเดียวกัน (⇧⌘K)")
                Spacer()
                sendButton
            }
        }
        .padding(12)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous)
            .strokeBorder(focused ? Color.accentColor.opacity(0.6) : Color.primary.opacity(0.12), lineWidth: 1))
    }

    @ViewBuilder
    private var sendButton: some View {
        let empty = shell.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && shell.attachments.isEmpty
        if engine.busy && empty {
            Button { shell.stop() } label: {
                Image(systemName: "stop.circle.fill").font(.title2)
            }
            .buttonStyle(.borderless)
            .keyboardShortcut(".", modifiers: .command)
            .help("หยุด (⌘.)")
        } else {
            Button { shell.send() } label: {
                Image(systemName: "arrow.up.circle.fill").font(.title2)
            }
            .buttonStyle(.borderless)
            .disabled(empty)
            .help(engine.busy ? "ต่อคิว ส่งเมื่อ agent ทำเสร็จ" : "ส่ง (↩)")
        }
    }

    // MARK: `@` and `/`

    private var suggestions: [Suggestion] {
        shell.suggestions(for: ShellModel.completionToken(in: shell.text))
    }

    @ViewBuilder
    private var completions: some View {
        let items = suggestions
        if !items.isEmpty {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(items) { s in
                    Button { pick(s) } label: {
                        HStack(spacing: 8) {
                            Image(systemName: s.icon).frame(width: 18).foregroundStyle(.secondary)
                            Text(s.title).font(.callout.weight(.medium))
                            if !s.detail.isEmpty {
                                Text(s.detail).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                            }
                            Spacer(minLength: 0)
                        }
                        .padding(.horizontal, 10).padding(.vertical, 5)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.vertical, 4)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Color.primary.opacity(0.1)))
            .accessibilityLabel("ตัวเลือกที่พิมพ์ค้างอยู่ กด Tab เพื่อเลือกอันแรก")
        }
    }

    /// Replaces the word being typed with the choice, or runs it and clears the word.
    private func pick(_ s: Suggestion) {
        var words = shell.text.split(separator: " ", omittingEmptySubsequences: false).map(String.init)
        if let insert = s.insert {
            if !words.isEmpty { words.removeLast() }
            words.append(insert)
            shell.text = words.joined(separator: " ")
        } else if let action = s.action {
            if !words.isEmpty { words.removeLast() }
            shell.text = words.joined(separator: " ")
            action()
        }
    }

    private var queue: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(Array(shell.queued.enumerated()), id: \.offset) { i, q in
                HStack(spacing: 8) {
                    Image(systemName: "clock").foregroundStyle(.secondary)
                    Text(q.send.text).lineLimit(1).font(.callout)
                    Spacer()
                    Button { shell.dismissQueued(at: i) } label: { Image(systemName: "xmark.circle.fill") }
                        .buttonStyle(.borderless).foregroundStyle(.secondary)
                }
            }
        }
        .padding(8)
        .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }

    @ViewBuilder
    private var metrics: some View {
        let chips = MetricsChips.make(engine.metrics, MetricsToggles(currency: currency, thbRate: Money.rate(thbRate)))
        if !chips.isEmpty {
            HStack(spacing: 12) {
                ForEach(chips) { c in
                    Label(c.label, systemImage: c.icon)
                        .font(.caption2.monospacedDigit())
                        .foregroundStyle(c.warn ? Color.orange : Color.secondary)
                        .help(c.help)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 4)
        }
    }
}

// MARK: the two choices

private struct ModeMenu: View {
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        Menu {
            ForEach(ActMode.allCases) { m in
                Button {
                    shell.mode = m
                } label: {
                    if shell.mode == m {
                        Label("\(m.label) · \(m.detail)", systemImage: "checkmark")
                    } else {
                        Text("\(m.label) · \(m.detail)")
                    }
                }
            }
        } label: {
            Label(shell.mode.label, systemImage: shell.mode.symbol).font(.callout)
        }
        .menuStyle(.button)
        .buttonStyle(.bordered)
        .controlSize(.small)
        .menuIndicator(.visible)
        .fixedSize()
        .help("agent ทำอะไรได้เอง แค่ไหน")
    }
}

/// Who decides the model for a message: the router, the Orchestra lead, or the
/// picker. Picking a model by hand switches this to "เลือกเอง".
private struct RouteMenu: View {
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        Menu {
            ForEach(SessionMode.allCases) { m in
                Button { shell.sessionMode = m } label: {
                    if shell.sessionMode == m { Label(m.label, systemImage: "checkmark") } else { Text(m.label) }
                }
            }
        } label: {
            Label(shell.sessionMode.label, systemImage: shell.sessionMode == .orchestra
                  ? "point.3.connected.trianglepath.dotted" : "arrow.triangle.branch").font(.callout)
        }
        .menuStyle(.button)
        .buttonStyle(.bordered)
        .controlSize(.small)
        .fixedSize()
        .help("Auto route ให้ Chorus เลือกโมเดลให้ · Orchestra ให้ lead แตกงานให้ agent ย่อย · เลือกเอง ใช้โมเดลที่เลือกไว้")
    }
}

private struct ModelMenu: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel

    private var current: Engine.AgentInfo? { shell.agent(for: shell.model) }

    var body: some View {
        Menu {
            ForEach(engine.agents) { a in
                Section(a.label) {
                    if a.ready {
                        ForEach(bases(of: a), id: \.base) { entry in
                            if entry.variants.count == 1, let only = entry.variants.first {
                                pick(only.id, title: entry.title)
                            } else {
                                Menu(entry.title) {
                                    ForEach(entry.variants, id: \.id) { v in pick(v.id, title: v.label) }
                                }
                            }
                        }
                    } else {
                        Text(a.auth == "needs_login" ? "ต้อง login ใน ตั้งค่า" : "ไม่พบ CLI")
                    }
                }
            }
        } label: {
            HStack(spacing: 6) {
                Circle().fill(ProviderTint.color(current?.id ?? "")).frame(width: 8, height: 8)
                Text(label).font(.callout)
            }
        }
        .menuStyle(.button)
        .buttonStyle(.bordered)
        .controlSize(.small)
        .menuIndicator(.visible)
        .fixedSize()
        .help("เลือกโมเดล เลือกของเจ้าอื่น = เตรียมส่งต่อให้ทันที")
    }

    private var label: String {
        guard let a = current else { return "เลือกโมเดล" }
        let m = ModelID(shell.model)
        let match = a.models.first { ModelID($0.id).base == m.base }
        let name = match.map(Self.shortLabel) ?? m.base
        return "\(a.label) · \(name)" + (m.effort.map { " · \($0)" } ?? "")
    }

    /// "gpt-5.5 · high" → "gpt-5.5": the level is shown on its own.
    private static func shortLabel(_ c: Engine.ModelChoice) -> String {
        let head = c.label.split(separator: "·").first.map(String.init) ?? c.label
        return head.trimmingCharacters(in: .whitespaces)
    }

    private struct Entry { let base: String; let title: String; let variants: [Engine.ModelChoice] }

    /// One entry per model, with its reasoning levels as a submenu.
    private func bases(of a: Engine.AgentInfo) -> [Entry] {
        var order: [String] = []
        var groups: [String: [Engine.ModelChoice]] = [:]
        for m in a.models {
            let b = ModelID(m.id).base
            if groups[b] == nil { order.append(b) }
            groups[b, default: []].append(m)
        }
        return order.map { b in
            let vs = groups[b] ?? []
            return Entry(base: b, title: vs.first.map(Self.shortLabel) ?? b, variants: vs)
        }
    }

    @ViewBuilder
    private func pick(_ id: String, title: String) -> some View {
        Button {
            shell.choose(id)
        } label: {
            if id == shell.model { Label(title, systemImage: "checkmark") } else { Text(title) }
        }
    }
}

// MARK: strips above the field

/// Handoff prepared in the background; the next message carries it.
private struct HandoffStrip: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @State private var showBrief = false

    var body: some View {
        switch engine.handoffPrep {
        case .none:
            EmptyView()
        case .preparing(let to):
            strip(tint: .secondary) {
                ProgressView().controlSize(.small)
                Text("กำลังเตรียมส่งต่อให้ \(shell.agentLabel(to))… พิมพ์ต่อได้เลย")
            }
        case .ready(let p):
            strip(tint: .accentColor) {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(Color.accentColor)
                Text("พร้อมส่งต่อให้ \(shell.agentLabel(p.to ?? "")) · ทำแล้ว \((p.done ?? []).count) · ค้าง \((p.pending ?? []).count)")
                Spacer()
                Button("ดู brief") { showBrief = true }
                    .buttonStyle(.link)
                    .popover(isPresented: $showBrief) { Brief(markdown: p.markdown ?? "") }
                Button("ยกเลิก") { engine.cancelHandoffPrep() }.buttonStyle(.link)
            }
        case .failed:
            strip(tint: .orange) {
                Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
                Text("เตรียมส่งต่อไม่สำเร็จ ส่งข้อความได้ตามปกติ ระบบจะสรุปให้ตอนส่ง")
            }
        }
    }

    private func strip<C: View>(tint: Color, @ViewBuilder _ content: () -> C) -> some View {
        HStack(spacing: 8) { content() }
            .font(.callout)
            .padding(.horizontal, 12).padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(tint.opacity(0.1), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }

    private struct Brief: View {
        let markdown: String
        var body: some View {
            ScrollView {
                Text(markdown).font(.callout.monospaced()).textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12)
            }
            .frame(width: 440, height: 320)
        }
    }
}

/// Errors, plan approval, limits: things that need a line, not a window.
private struct Notices: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        VStack(spacing: 8) {
            if let err = engine.lastError, !err.isEmpty {
                row(symbol: "exclamationmark.triangle.fill", tint: .orange) {
                    Text(err).lineLimit(3).textSelection(.enabled)
                    Spacer()
                    Button { engine.lastError = nil } label: { Image(systemName: "xmark") }
                        .buttonStyle(.borderless)
                }
            }
            if let hint = shell.hint {
                row(symbol: "info.circle", tint: .secondary) { Text(hint) }
            }
            if limited {
                row(symbol: "gauge.with.dots.needle.100percent", tint: .orange) {
                    Text("\(shell.agentLabel(shell.agent(for: shell.model)?.id ?? "")) ติดลิมิตของบัญชีนี้ เลือกโมเดลอื่นเพื่อทำต่อ ข้อความที่พิมพ์ไว้ไม่หาย")
                }
            }
            if let agents = engine.pendingPlanAgents, !engine.busy {
                row(symbol: "list.bullet.clipboard", tint: .accentColor) {
                    Text("แผนพร้อมแล้ว กดทำตามแผน หรือพิมพ์บอกว่าจะแก้ตรงไหน")
                    Spacer()
                    Button("ทำตามแผน") { shell.approvePlan(agents: agents) }
                        .buttonStyle(.borderedProminent).controlSize(.small)
                    Button { engine.pendingPlanAgents = nil } label: { Image(systemName: "xmark") }
                        .buttonStyle(.borderless)
                }
            }
        }
    }

    private var limited: Bool {
        guard let id = shell.agent(for: shell.model)?.id else { return false }
        let own = engine.accounts.filter { $0.agent == id }
        return !own.isEmpty && own.allSatisfy(\.limited)
    }

    private func row<C: View>(symbol: String, tint: Color, @ViewBuilder _ content: () -> C) -> some View {
        HStack(spacing: 8) {
            Image(systemName: symbol).foregroundStyle(tint)
            content()
        }
        .font(.callout)
        .padding(.horizontal, 12).padding(.vertical, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(tint.opacity(0.1), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}
