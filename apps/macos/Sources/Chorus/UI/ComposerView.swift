import SwiftUI
import UniformTypeIdentifiers

// Design board "Session" — the composer: a status line while an agent works
// (with ⌘. to stop), notices and the prepared handoff, then one card holding the
// field and a single row of choices: attach · Auto→model · คิดลึก · ถามก่อน/ให้ทำเลย
// · วางแผนก่อน · context ring · mic · send. Picking another provider's model
// prepares the handoff; sending carries it (Rust decides all of it).

struct ComposerView: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @AppStorage(DisplayKey.currency) private var currency = "auto"
    @AppStorage(SettingsKey.thbRate) private var thbRate = ""
    @ObservedObject private var voice = VoiceDictationManager.shared
    @FocusState private var focused: Bool
    @State private var picking = false

    var body: some View {
        VStack(spacing: 6) {
            Notices()
            if !shell.queued.isEmpty { queue }
            HandoffStrip()
            if let status = shell.routeStatus {
                RouteStatusCard(status: status) { shell.routeStatus = nil }
            }
            completions
            statusLine
            card
            metrics
        }
        .padding(.horizontal, 24).padding(.top, 6).padding(.bottom, 16)
        .frame(maxWidth: 780 + 48)
        .frame(maxWidth: .infinity)
        .onChange(of: engine.sessionID) { _, _ in focused = true }
        .onAppear { focused = true }
        .fileImporter(isPresented: $picking, allowedContentTypes: [.item], allowsMultipleSelection: true) { r in
            if case .success(let urls) = r { shell.attach(urls: urls) }
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

    // MARK: status line

    @ViewBuilder
    private var statusLine: some View {
        if engine.busy {
            HStack(spacing: 8) {
                if let a = engine.liveAgent { ProviderIcon(provider: ProviderTint.provider(of: a), size: 12) }
                Text("\(shell.agentLabel(engine.liveAgent ?? "agent")) กำลังทำงาน · ข้อความถัดไปจะรอคิว")
                Spacer()
                Button { shell.stop() } label: {
                    HStack(spacing: 5) {
                        Image(systemName: "stop.fill").font(.system(size: 8))
                        Text("หยุด")
                        Kbd("⌘.")
                    }
                }
                .buttonStyle(QuietButtonStyle(small: true))
                .keyboardShortcut(".", modifiers: .command)
            }
            .font(Typo.caption).foregroundStyle(Palette.text3)
            .padding(.horizontal, 6)
        }
    }

    // MARK: the card

    private var card: some View {
        VStack(alignment: .leading, spacing: 10) {
            if !shell.attachments.isEmpty || shell.importing > 0 { attachments }
            TextField(engine.nextHint ?? "สั่งงาน · @ อ้างไฟล์หรือ agent · / คำสั่ง", text: $shell.text, axis: .vertical)
                .textFieldStyle(.plain)
                .font(Typo.body)
                .lineLimit(2...10)
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
            HStack(spacing: 6) {
                Button { picking = true } label: { Image(systemName: "plus") }
                    .buttonStyle(IconButtonStyle()).help("แนบไฟล์หรือรูป").accessibilityLabel("แนบไฟล์")
                RouteChip()
                EffortChip()
                PermissionSwitch()
                Button { shell.mode = shell.mode == .plan ? .ask : .plan } label: { Text("วางแผนก่อน") }
                    .buttonStyle(ChipStyle(on: shell.mode == .plan))
                    .help(ActMode.plan.detail)
                Spacer(minLength: 4)
                ContextRing(pct: engine.metrics?.context_pct)
                Button {
                    voice.toggleRecording { t in shell.text += (shell.text.isEmpty ? "" : " ") + t }
                } label: { Image(systemName: voice.isRecording ? "mic.fill" : "mic") }
                    .buttonStyle(IconButtonStyle(on: voice.isRecording))
                    .help("พูดแทนพิมพ์").accessibilityLabel("พูด")
                sendButton
            }
        }
        .padding(.leading, 14).padding(.trailing, 10).padding(.top, 12).padding(.bottom, 10)
        .background(Palette.surface, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous)
            .strokeBorder(focused ? Palette.accent.opacity(0.55) : Palette.line2, lineWidth: 1))
        .shadow(color: .black.opacity(0.14), radius: 12, y: 4)
    }

    private var attachments: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(shell.attachments) { a in
                    HStack(spacing: 5) {
                        Image(systemName: a.mediaType.hasPrefix("image/") ? "photo" : "doc")
                        Text(a.name).lineLimit(1)
                        Button { shell.removeAttachment(a.id) } label: { Image(systemName: "xmark.circle.fill") }
                            .buttonStyle(.plain).foregroundStyle(Palette.text3)
                            .accessibilityLabel("เอา \(a.name) ออก")
                    }
                    .font(Typo.caption).foregroundStyle(a.agentReadable ? Palette.text2 : Palette.bad)
                    .padding(.horizontal, 9).frame(height: 26)
                    .background(Palette.hover, in: Capsule())
                    .overlay(Capsule().strokeBorder(a.agentReadable ? Palette.line : Palette.bad.opacity(0.6)))
                    .help(a.agentReadable ? a.name : "agent อ่านไฟล์นี้ไม่ได้")
                }
                if shell.importing > 0 { ProgressView().controlSize(.small) }
            }
        }
    }

    @ViewBuilder
    private var sendButton: some View {
        let empty = shell.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && shell.attachments.isEmpty
        if engine.busy && empty {
            Button { shell.stop() } label: {
                Image(systemName: "stop.fill").font(.system(size: 11)).foregroundStyle(.white)
                    .frame(width: 32, height: 32)
                    .background(Palette.bad, in: RoundedRectangle(cornerRadius: 9, style: .continuous))
            }
            .buttonStyle(.plain).help("หยุด (⌘.)").accessibilityLabel("หยุด")
        } else {
            Button { shell.send() } label: {
                Image(systemName: "arrow.up").font(.system(size: 14, weight: .semibold)).foregroundStyle(.white)
                    .frame(width: 32, height: 32)
                    .background(Palette.accentButton.opacity(empty ? 0.35 : 1), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
            }
            .buttonStyle(.plain)
            .disabled(empty)
            .help(engine.busy ? "ต่อคิว ส่งเมื่อ agent ทำเสร็จ" : "ส่ง (↩)")
            .accessibilityLabel("ส่ง")
        }
    }

    // MARK: `@` and `/`

    private var suggestions: [Suggestion] { shell.suggestions(for: ShellModel.completionToken(in: shell.text)) }

    @ViewBuilder
    private var completions: some View {
        let items = suggestions
        if !items.isEmpty {
            VStack(alignment: .leading, spacing: 1) {
                ForEach(items) { s in
                    Button { pick(s) } label: {
                        HStack(spacing: 10) {
                            Image(systemName: s.icon).frame(width: 18).foregroundStyle(Palette.text3)
                            Text(s.title).font(.system(size: 13, weight: .medium)).foregroundStyle(Palette.text)
                            if !s.detail.isEmpty { Text(s.detail).font(Typo.caption).foregroundStyle(Palette.text3).lineLimit(1) }
                            Spacer(minLength: 0)
                        }
                    }
                    .buttonStyle(RowStyle(height: 32))
                }
            }
            .padding(6)
            .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Palette.line2))
            .accessibilityLabel("ตัวเลือก กด Tab เพื่อเลือกอันแรก")
        }
    }

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
                    Image(systemName: "clock").foregroundStyle(Palette.text3)
                    Text(q.send.text).lineLimit(1).font(Typo.small).foregroundStyle(Palette.text2)
                    Spacer()
                    Button { shell.dismissQueued(at: i) } label: { Image(systemName: "xmark") }
                        .buttonStyle(IconButtonStyle()).accessibilityLabel("ยกเลิกข้อความที่รอคิว")
                }
            }
        }
        .padding(.horizontal, 10).padding(.vertical, 6)
        .background(Palette.hover, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }

    @ViewBuilder
    private var metrics: some View {
        let chips = MetricsChips.make(engine.metrics, MetricsToggles(currency: currency, thbRate: Money.rate(thbRate)))
        if !chips.isEmpty {
            HStack(spacing: 14) {
                ForEach(chips) { c in
                    Label(c.label, systemImage: c.icon).font(Typo.tiny.monospacedDigit())
                        .foregroundStyle(c.warn ? Palette.warn : Palette.text3).help(c.help)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 6)
        }
    }
}

// MARK: choices

/// "Auto → [icon] luna ⌄": who picks the model, and which model.
private struct RouteChip: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel

    private var current: Engine.AgentInfo? { shell.agent(for: shell.model) }

    var body: some View {
        Menu {
            Section("ใครเลือกโมเดล") {
                ForEach(SessionMode.allCases) { m in
                    Toggle(isOn: Binding(get: { shell.sessionMode == m }, set: { if $0 { shell.sessionMode = m } })) {
                        Text(m.label)
                    }
                }
            }
            ForEach(engine.agents) { a in
                Section(a.label) {
                    if a.ready {
                        ForEach(a.baseModels, id: \.id) { m in
                            let id = ModelID.compose(m.id, ModelID(shell.model).effort.flatMap { a.efforts(for: m.id).contains($0) ? $0 : nil }
                                                     ?? a.efforts(for: m.id).first)
                            Toggle(isOn: Binding(get: { ModelID(shell.model).base == m.id && current?.id == a.id },
                                                 set: { if $0 { shell.choose(engineID(a, base: m.id, fallback: id)) } })) {
                                Text(m.label)
                            }
                        }
                    } else {
                        Text(a.auth == "needs_login" ? "ต้อง login ในตั้งค่า" : "ไม่พบ CLI")
                    }
                }
            }
        } label: {
            HStack(spacing: 6) {
                Image(systemName: shell.sessionMode == .orchestra ? "point.3.connected.trianglepath.dotted" : "arrow.triangle.branch")
                    .font(.system(size: 11))
                if shell.sessionMode != .manual {
                    Text(shell.sessionMode == .orchestra ? "Orchestra" : "Auto")
                    Image(systemName: "arrow.right").font(.system(size: 9)).foregroundStyle(Palette.text3)
                }
                if let a = current { ProviderIcon(provider: a.id, size: 12) }
                Text(modelLabel).foregroundStyle(Palette.text2).lineLimit(1)
                Image(systemName: "chevron.down").font(.system(size: 8, weight: .semibold)).foregroundStyle(Palette.text3)
            }
            .font(.system(size: 12.5)).foregroundStyle(Palette.text)
            .padding(.horizontal, 10).frame(height: 28)
            .overlay(Capsule().strokeBorder(Palette.line))
            .contentShape(Capsule())
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .help("เลือกโมเดล · เลือกของเจ้าอื่น = เตรียมส่งไม้ให้ทันที (⇧⌘I)")
    }

    /// Prefer an exact id the agent reported for this base, so the pick is never blank.
    private func engineID(_ a: Engine.AgentInfo, base: String, fallback: String) -> String {
        a.models.first { $0.id == fallback }?.id ?? a.models.first { ModelID($0.id).base == base }?.id ?? fallback
    }

    private var modelLabel: String {
        guard let a = current else { return "เลือกโมเดล" }
        let base = ModelID(shell.model).base
        return a.baseModels.first { $0.id == base }?.label ?? base
    }
}

/// "คิดลึก · high" — the reasoning level of the current model, if it has any.
private struct EffortChip: View {
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        let id = ModelID(shell.model)
        let levels = shell.agent(for: shell.model)?.efforts(for: id.base) ?? []
        if !levels.isEmpty {
            Menu {
                ForEach(levels, id: \.self) { e in
                    Toggle(isOn: Binding(get: { id.effort == e }, set: { if $0 { shell.choose(ModelID.compose(id.base, e)) } })) {
                        Text(e)
                    }
                }
            } label: {
                Text("คิดลึก · \(id.effort ?? levels.first ?? "")")
                    .font(.system(size: 12.5)).foregroundStyle(Palette.text2)
                    .padding(.horizontal, 10).frame(height: 28)
                    .overlay(Capsule().strokeBorder(Palette.line))
                    .contentShape(Capsule())
            }
            .menuStyle(.button).buttonStyle(.plain).menuIndicator(.hidden).fixedSize()
            .help("ระดับการคิด (⇧⌘E)")
        }
    }
}

/// "ถามก่อน | ให้ทำเลย" — a two-segment pill.
private struct PermissionSwitch: View {
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        HStack(spacing: 0) {
            segment(.ask)
            segment(.allow)
        }
        .padding(2)
        .background(Palette.hover, in: Capsule())
        .overlay(Capsule().strokeBorder(Palette.line))
        .accessibilityElement(children: .contain)
        .accessibilityLabel("สิทธิ์ของ agent")
    }

    private func segment(_ m: ActMode) -> some View {
        let on = shell.mode == m || (m == .ask && shell.mode == .plan)
        return Button { shell.mode = m } label: {
            Text(m.label)
                .font(.system(size: 12, weight: on ? .medium : .regular))
                .foregroundStyle(on ? Palette.text : Palette.text3)
                .padding(.horizontal, 10).frame(height: 22)
                .background(on ? Palette.elevated : .clear, in: Capsule())
                .shadow(color: .black.opacity(on ? 0.15 : 0), radius: 1, y: 1)
                .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .help(m.detail)
    }
}

/// How full the context is, as a ring.
private struct ContextRing: View {
    let pct: Double?
    var body: some View {
        if let pct {
            HStack(spacing: 6) {
                ZStack {
                    Circle().stroke(Palette.line2, lineWidth: 2.5)
                    Circle().trim(from: 0, to: min(max(pct / 100, 0), 1))
                        .stroke(pct > 80 ? Palette.warn : Palette.accent, style: StrokeStyle(lineWidth: 2.5, lineCap: .round))
                        .rotationEffect(.degrees(-90))
                }
                .frame(width: 16, height: 16)
                Text("\(Int(pct.rounded()))%").font(Typo.tiny.monospacedDigit()).foregroundStyle(Palette.text3)
            }
            .help("คอนเท็กซ์ใช้ไป \(Int(pct.rounded()))%")
            .accessibilityElement(children: .combine)
        }
    }
}

// MARK: strips above the field

private struct HandoffStrip: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @State private var showBrief = false

    var body: some View {
        switch engine.handoffPrep {
        case .none: EmptyView()
        case .preparing(let to):
            Strip(tone: .neutral) {
                ProgressView().controlSize(.small)
                Text("กำลังเตรียมส่งไม้ให้ \(shell.agentLabel(to))… พิมพ์ต่อได้เลย")
            }
        case .ready(let p):
            Strip(tone: .accent) {
                ProviderIcon(provider: p.to ?? "", size: 13)
                Text("พร้อมส่งไม้ให้ \(shell.agentLabel(p.to ?? "")) · ทำแล้ว \((p.done ?? []).count) · ค้าง \((p.pending ?? []).count)")
                Spacer()
                Button("ดู brief") { showBrief = true }
                    .buttonStyle(QuietButtonStyle(small: true))
                    .popover(isPresented: $showBrief) {
                        ScrollView {
                            Text(p.markdown ?? "").font(Typo.monoSmall).textSelection(.enabled)
                                .frame(maxWidth: .infinity, alignment: .leading).padding(14)
                        }
                        .frame(width: 460, height: 340)
                    }
                Button("ยกเลิก") { engine.cancelHandoffPrep() }.buttonStyle(QuietButtonStyle(small: true))
            }
        case .failed:
            Strip(tone: .warn) {
                Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Palette.warn)
                Text("เตรียมส่งไม้ไม่สำเร็จ ส่งข้อความได้ตามปกติ ระบบจะสรุปให้ตอนส่ง")
            }
        }
    }
}

/// Errors, plan approval, limits: a line, not a window.
private struct Notices: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        VStack(spacing: 6) {
            if let err = engine.lastError, !err.isEmpty {
                Strip(tone: .warn) {
                    Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Palette.warn)
                    Text(err).lineLimit(3).textSelection(.enabled)
                    Spacer()
                    Button { engine.lastError = nil } label: { Image(systemName: "xmark") }
                        .buttonStyle(IconButtonStyle()).accessibilityLabel("ปิด")
                }
            }
            if let hint = shell.hint {
                Strip(tone: .neutral) { Image(systemName: "info.circle"); Text(hint) }
            }
            if limited {
                Strip(tone: .warn) {
                    Image(systemName: "gauge.with.dots.needle.100percent").foregroundStyle(Palette.warn)
                    Text("\(shell.agentLabel(shell.agent(for: shell.model)?.id ?? "")) ติดลิมิต เลือกโมเดลอื่นหรือกดส่งไม้ ข้อความที่พิมพ์ไว้ไม่หาย")
                    Spacer()
                    Button("ส่งไม้…") { shell.showHandoff = true }.buttonStyle(SecondaryButtonStyle(small: true))
                }
            }
        }
    }

    private var limited: Bool {
        guard let id = shell.agent(for: shell.model)?.id else { return false }
        let own = engine.accounts.filter { $0.agent == id }
        return !own.isEmpty && own.allSatisfy(\.limited)
    }
}

/// One tinted line above the composer.
struct Strip<Content: View>: View {
    enum Tone { case neutral, accent, warn }
    let tone: Tone
    @ViewBuilder var content: Content

    var body: some View {
        HStack(spacing: 8) { content }
            .font(Typo.small)
            .foregroundStyle(Palette.text2)
            .padding(.horizontal, 12).padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(bg, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(stroke))
    }
    private var bg: Color { tone == .accent ? Palette.accentWash : tone == .warn ? Palette.warnWash : Palette.hover }
    private var stroke: Color { tone == .warn ? Palette.warn.opacity(0.35) : .clear }
}
