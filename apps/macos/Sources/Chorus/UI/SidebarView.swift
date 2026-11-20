import AppKit
import SwiftUI

// Design component "แถบโปรเจกต์": ⌘K search, projects that fold, each session
// with its state on the leading edge and the agents that took part on the
// trailing edge, the agents a lead started nested underneath, sessions found in
// other apps, and "วงของคุณ" — every agent's quota, always visible.

struct SidebarView: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @ObservedObject var projects: ProjectStore

    @AppStorage("chorus.sidebar.folded") private var foldedRaw = ""
    @State private var editing: ChorusProject?
    @State private var messaging: Engine.SubagentInfo?
    @State private var steer = ""
    @State private var external: [String: Int] = [:]
    @State private var showAgents = false

    var body: some View {
        VStack(spacing: 0) {
            searchButton
                .padding(.horizontal, 12).padding(.top, 4).padding(.bottom, 8)
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 1) {
                    HStack {
                        GroupHeader("โปรเจกต์")
                        Spacer()
                        Button { shell.addProject() } label: { Image(systemName: "plus") }
                            .buttonStyle(IconButtonStyle()).controlSize(.small)
                            .help("เพิ่มโปรเจกต์").accessibilityLabel("เพิ่มโปรเจกต์")
                    }
                    .padding(.leading, 8).padding(.top, 4)

                    ForEach(projects.projects) { p in project(p) }

                    if !external.isEmpty {
                        GroupHeader("นำเข้าจากแอปอื่น").padding(.leading, 8).padding(.top, 16).padding(.bottom, 4)
                        ForEach(external.keys.sorted(), id: \.self) { provider in
                            Button { shell.showSearch = true } label: {
                                HStack(spacing: 10) {
                                    ProviderIcon(provider: Self.providerKey(provider), size: 14)
                                    Text(provider)
                                    Spacer()
                                    Text("\(external[provider] ?? 0)").font(Typo.tiny).foregroundStyle(Palette.text3)
                                }
                            }
                            .buttonStyle(RowStyle())
                        }
                    }
                }
                .padding(.horizontal, 8).padding(.bottom, 12)
            }
            .scrollIndicators(.automatic)
            footer
        }
        .background(Palette.sidebar)
        .task { loadExternal() }
        .sheet(item: $editing) { p in
            ProjectEditorSheet(project: p, onSave: { saved in
                projects.upsert(saved)
                if engine.sessionID >= 0, saved.folders.contains(shell.workspace) {
                    engine.setProjectFolders(saved.folders)
                }
            }, onDelete: { projects.remove(p) })
        }
        .alert("ส่งข้อความถึง agent ย่อย", isPresented: Binding(get: { messaging != nil }, set: { if !$0 { messaging = nil } }),
               presenting: messaging) { sub in
            TextField("ข้อความ", text: $steer)
            Button("ส่ง") {
                let t = steer.trimmingCharacters(in: .whitespacesAndNewlines)
                if !t.isEmpty { engine.messageSubagent(db: shell.currentDB, id: sub.id, message: t) }
                messaging = nil
            }
            Button("ยกเลิก", role: .cancel) { messaging = nil }
        } message: { sub in Text(sub.task) }
    }

    // MARK: search

    private var searchButton: some View {
        Button { shell.showPalette = true } label: {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass").font(.system(size: 12))
                Text("ค้นหา หรือสั่งงาน").frame(maxWidth: .infinity, alignment: .leading)
                Kbd("⌘K")
            }
            .font(Typo.ui)
            .foregroundStyle(Palette.text3)
            .padding(.horizontal, 10).frame(height: 32)
            .background(Palette.surface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Palette.line))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("ค้นหา หรือสั่งงาน")
    }

    // MARK: projects

    private var folded: Set<String> { Set(foldedRaw.split(separator: ",").map(String.init)) }

    private func toggleFold(_ p: ChorusProject) {
        var f = folded
        if f.contains(p.id.uuidString) { f.remove(p.id.uuidString) } else { f.insert(p.id.uuidString) }
        foldedRaw = f.joined(separator: ",")
    }

    @ViewBuilder
    private func project(_ p: ChorusProject) -> some View {
        let open = !folded.contains(p.id.uuidString)
        let rows = rows(of: p)
        HStack(spacing: 6) {
            Button { withAnimation(.snappy(duration: 0.18)) { toggleFold(p) } } label: {
                HStack(spacing: 6) {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundStyle(Palette.text3)
                        .rotationEffect(.degrees(open ? 90 : 0))
                        .frame(width: 12)
                    Text(p.name).font(.system(size: 13, weight: .medium)).foregroundStyle(Palette.text)
                    Spacer(minLength: 4)
                    Text(open ? (p.folders.count > 1 ? "\(p.folders.count) โฟลเดอร์" : "") : "\(rows.count)")
                        .font(Typo.tiny).foregroundStyle(Palette.text3)
                }
            }
            .buttonStyle(RowStyle())
            .contextMenu {
                Button("แก้ไขโปรเจกต์…") { editing = p }
                Button("session ใหม่") { shell.newSession(in: p.folders.first) }
            }
            Button { shell.newSession(in: p.folders.first) } label: { Image(systemName: "square.and.pencil") }
                .buttonStyle(IconButtonStyle())
                .help("session ใหม่ใน \(p.name) (⌘N)")
                .accessibilityLabel("session ใหม่ใน \(p.name)")
                .disabled(p.folders.isEmpty || engine.busy)
        }
        .padding(.top, 4)

        if open {
            if engine.draft.map({ p.folders.contains(draftFolder($0)) }) == true {
                Button {} label: {
                    HStack(spacing: 10) {
                        Image(systemName: "square.and.pencil").font(.system(size: 11)).foregroundStyle(Palette.text3).frame(width: 14)
                        Text("session ใหม่").italic()
                        Spacer()
                        Text("ยังไม่ได้ส่ง").font(Typo.tiny).foregroundStyle(Palette.text3)
                    }
                }
                .buttonStyle(RowStyle(selected: true, height: 34))
                .padding(.leading, 18)
            }
            ForEach(rows) { r in
                sessionRow(r, showsFolder: p.folders.count > 1)
                ForEach(subagents(of: r.session, folder: r.folder)) { sub in
                    subagentRow(sub, folder: r.folder)
                }
            }
            if rows.isEmpty && engine.draft == nil {
                Text(shell.query.isEmpty ? "ยังไม่มี session" : "ไม่เจอที่ค้น")
                    .font(Typo.caption).foregroundStyle(Palette.text3)
                    .padding(.leading, 38).padding(.vertical, 4)
            }
        }
    }

    private func sessionRow(_ r: Row, showsFolder: Bool) -> some View {
        let selected = shell.selection == SessionKey(folder: r.folder, id: r.session.id)
        let status = status(of: r.session, folder: r.folder)
        return Button { shell.open(SessionKey(folder: r.folder, id: r.session.id)) } label: {
            HStack(spacing: 10) {
                StatusGlyph(status: status).frame(width: 14)
                VStack(alignment: .leading, spacing: 1) {
                    Text(shell.title(of: r.session)).lineLimit(1)
                        .foregroundStyle(selected ? Palette.text : Palette.text2)
                    if showsFolder || status.isAttention {
                        Text([status.isAttention ? status.text : nil,
                              showsFolder ? (r.folder as NSString).lastPathComponent : nil]
                            .compactMap { $0 }.joined(separator: " · "))
                            .font(Typo.tiny).foregroundStyle(status.isAttention ? status.color : Palette.text3)
                            .lineLimit(1)
                    }
                }
                Spacer(minLength: 4)
                if engine.renamingSessionIDs.contains(r.session.id) { ProgressView().controlSize(.mini) }
                if !r.session.agents.isEmpty { ProviderStack(providers: r.session.agents, size: 17) }
            }
        }
        .buttonStyle(RowStyle(selected: selected, height: 34))
        .padding(.leading, 18)
        .help(status.text)
        .contextMenu {
            Button("ตั้งชื่อด้วย AI") {
                engine.renameSessionWithAI(db: shell.dbPath(for: r.folder), id: r.session.id, sampleText: shell.title(of: r.session))
            }
            Button("เปิดโฟลเดอร์ใน Finder") {
                NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: r.folder)])
            }
            Divider()
            Button("ลบ session…", role: .destructive) { shell.pendingDelete = r.session }
        }
    }

    private func subagentRow(_ sub: Engine.SubagentInfo, folder: String) -> some View {
        let key = sub.child_sess.map { SessionKey(folder: folder, id: $0) }
        let selected = key != nil && shell.selection == key
        return Button { if let key { shell.open(key) } } label: {
            HStack(spacing: 8) {
                Text("↳").foregroundStyle(Palette.text3)
                Text(sub.task.isEmpty ? sub.role : "\(sub.role) · \(sub.task)").lineLimit(1)
                Spacer(minLength: 4)
                switch sub.status {
                case "running": Spinner(size: 10)
                case "waiting": Text("รอ").font(Typo.tiny).foregroundStyle(Palette.warn)
                case "failed": Image(systemName: "exclamationmark.triangle.fill").font(.system(size: 10)).foregroundStyle(Palette.bad)
                default: Text("เสร็จ").font(Typo.tiny).foregroundStyle(Palette.text3)
                }
            }
            .font(Typo.small)
        }
        .buttonStyle(RowStyle(selected: selected, height: 28))
        .padding(.leading, 36)
        .help(sub.summary ?? sub.task)
        .contextMenu {
            if sub.status == "running" || sub.status == "waiting" {
                Button("ส่งข้อความถึง agent ย่อย…") { steer = ""; messaging = sub }
                Button("หยุด", role: .destructive) { engine.cancelSubagent(db: shell.dbPath(for: folder), id: sub.id) }
            }
        }
    }

    // MARK: footer — วงของคุณ

    private var footer: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button { showAgents.toggle() } label: {
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        GroupHeader("วงของคุณ")
                        Spacer()
                        if engine.agents.isEmpty {
                            ProgressView().controlSize(.mini)
                        } else {
                            Text("5 ชม.").font(Typo.tiny).foregroundStyle(Palette.text3)
                        }
                    }
                    ForEach(engine.agents) { a in
                        QuotaRow(agent: a, accounts: engine.accounts.filter { $0.agent == a.id })
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .popover(isPresented: $showAgents, arrowEdge: .trailing) { AgentsPopover() }
            .help("บัญชีและโควตาของแต่ละเจ้า")

            HStack(spacing: 4) {
                Button { AppCommandCenter.shared.send(.systemMonitor) } label: {
                    Label("System Monitor", systemImage: "waveform.path.ecg").labelStyle(.iconOnly)
                }
                .buttonStyle(IconButtonStyle())
                .help("System Monitor (⌥⌘M)")
                Spacer()
                Button { shell.addProject() } label: { Image(systemName: "folder.badge.plus") }
                    .buttonStyle(IconButtonStyle()).help("เพิ่มโปรเจกต์").accessibilityLabel("เพิ่มโปรเจกต์")
                Button { RootView.openSettings() } label: { Image(systemName: "gearshape") }
                    .buttonStyle(IconButtonStyle()).help("ตั้งค่า (⌘,)").accessibilityLabel("ตั้งค่า")
            }
        }
        .padding(.horizontal, 14).padding(.top, 12).padding(.bottom, 10)
        .overlay(alignment: .top) { Hairline() }
    }

    // MARK: data

    private struct Row: Identifiable {
        let folder: String
        let session: Engine.SessionSummary
        var id: String { "\(folder)#\(session.id)" }
    }

    private func allSubagents(folder: String) -> [Engine.SubagentInfo] {
        var seen = Set<Int64>()
        return (engine.subagentsByParent.values.flatMap { $0 } + (engine.subagentsByDB[shell.dbPath(for: folder)] ?? []))
            .filter { seen.insert($0.id).inserted }
    }

    private func subagents(of s: Engine.SessionSummary, folder: String) -> [Engine.SubagentInfo] {
        allSubagents(folder: folder).filter { $0.parent_sess == s.id && $0.child_sess != nil }
            .sorted { $0.created_ms < $1.created_ms }
    }

    /// Compare lanes and a lead's children show under their parent, not as rows.
    private func rows(of p: ChorusProject) -> [Row] {
        var out: [Row] = []
        let q = shell.query.trimmingCharacters(in: .whitespaces)
        for f in p.folders {
            let all = shell.sessions(in: f)
            let ids = Set(all.map(\.id))
            let children = Set(allSubagents(folder: f).compactMap(\.child_sess))
            for s in all where !children.contains(s.id) {
                if let parent = s.forked_from, ids.contains(parent) { continue }
                if q.isEmpty || shell.title(of: s).localizedCaseInsensitiveContains(q) || s.goal.localizedCaseInsensitiveContains(q) {
                    out.append(Row(folder: f, session: s))
                }
            }
        }
        return out.sorted { ($0.session.started_ms + $0.session.last_ms) > ($1.session.started_ms + $1.session.last_ms) }
    }

    private func draftFolder(_ d: Engine.DraftInfo) -> String {
        ((d.db as NSString).deletingLastPathComponent as NSString).deletingLastPathComponent
    }

    private func status(of s: Engine.SessionSummary, folder: String) -> SessionStatus {
        let open = folder == shell.workspace && s.id == engine.sessionID
        if open && engine.waitingForYou > 0 { return .waiting }
        let subs = engine.subagentsByParent[s.id]
            ?? (engine.subagentsByDB[shell.dbPath(for: folder)] ?? []).filter { $0.parent_sess == s.id }
        if subs.contains(where: { $0.status == "waiting" }) { return .waiting }
        if subs.contains(where: { $0.status == "failed" }) { return .failed }
        if open && engine.busy { return .running }
        switch engine.sessionStatus[s.id] {
        case "running": return .running
        case "failed": return .failed
        default: break
        }
        if subs.contains(where: { $0.status == "running" }) { return .running }
        return .idle(Date(timeIntervalSince1970: Double(s.started_ms + s.last_ms) / 1000))
    }

    private func loadExternal() {
        engine.externalSessions(query: "", limit: 200) { sessions, _ in
            var counts: [String: Int] = [:]
            for s in sessions { counts[s.provider, default: 0] += 1 }
            external = counts
        }
    }

    static func providerKey(_ display: String) -> String {
        let d = display.lowercased()
        if d.contains("claude") { return "claude" }
        if d.contains("codex") { return "codex" }
        if d.contains("gemini") { return "gemini" }
        return d
    }
}

enum SessionStatus: Equatable {
    case running, waiting, failed
    case idle(Date)

    var isAttention: Bool { self == .waiting || self == .failed }

    var text: String {
        switch self {
        case .running: "กำลังทำ"
        case .waiting: "รอคุณ"
        case .failed: "ติด"
        case .idle(let d): Self.relative.localizedString(for: d, relativeTo: Date())
        }
    }

    var color: Color {
        switch self {
        case .running: Palette.accent
        case .waiting: Palette.warn
        case .failed: Palette.bad
        case .idle: Palette.text3
        }
    }

    private static let relative: RelativeDateTimeFormatter = {
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .short
        return f
    }()
}

/// Leading glyph of a session row: spinner, hand, triangle, or a quiet ring.
struct StatusGlyph: View {
    let status: SessionStatus
    var body: some View {
        switch status {
        case .running: Spinner(size: 12)
        case .waiting: Image(systemName: "exclamationmark.triangle").font(.system(size: 11, weight: .semibold)).foregroundStyle(Palette.warn)
            .accessibilityLabel("รอคุณ")
        case .failed: Image(systemName: "xmark.octagon").font(.system(size: 11, weight: .semibold)).foregroundStyle(Palette.bad)
            .accessibilityLabel("ติด")
        case .idle: Circle().strokeBorder(Palette.text3.opacity(0.6), lineWidth: 1.5).frame(width: 8, height: 8)
            .accessibilityHidden(true)
        }
    }
}

/// One line of "วงของคุณ".
struct QuotaRow: View {
    let agent: Engine.AgentInfo
    let accounts: [Engine.Account]

    var body: some View {
        HStack(spacing: 8) {
            ProviderIcon(provider: agent.id, size: 13).opacity(agent.ready ? 1 : 0.45)
            Text(agent.label).lineLimit(1).frame(width: 74, alignment: .leading)
                .foregroundStyle(agent.ready ? Palette.text : Palette.text3)
            if !agent.ready {
                Text(agent.auth == "needs_login" ? "ต้อง login" : "ไม่พบ CLI").foregroundStyle(Palette.text3)
                Spacer(minLength: 0)
            } else if let used = best {
                Meter(value: used, tint: limited ? Palette.warn : (used > 0.8 ? Palette.warn : Palette.text2))
                Text(trailing(used)).monospacedDigit()
                    .foregroundStyle(limited ? Palette.warn : Palette.text3)
                    .frame(width: 40, alignment: .trailing)
            } else {
                Spacer(minLength: 0)
                Text(limited ? "ติดลิมิต" : "พร้อม").foregroundStyle(limited ? Palette.warn : Palette.text3)
            }
        }
        .font(Typo.caption)
        .accessibilityElement(children: .combine)
    }

    private var limited: Bool { !accounts.isEmpty && accounts.allSatisfy(\.limited) }
    /// The best-off account decides: relay moves to it before leaving the provider.
    private var best: Double? { accounts.compactMap(\.worst).min() }

    private func trailing(_ used: Double) -> String {
        if limited {
            for a in accounts {
                if let l = a.rate_limits?.primary?.resets_label ?? a.hit?.resets {
                    let t = l.replacingOccurrences(of: "resets", with: "").trimmingCharacters(in: .whitespaces)
                    return String(t.split(separator: " ").first ?? Substring(t))
                }
            }
        }
        return "\(Int((used * 100).rounded()))%"
    }
}

/// Every agent and account, opened from "วงของคุณ".
struct AgentsPopover: View {
    @EnvironmentObject private var engine: Engine

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("agent และบัญชี").font(Typo.headline)
            ForEach(engine.agents) { a in
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 8) {
                        ProviderAvatar(provider: a.id, size: 22)
                        Text(a.label).font(.system(size: 13, weight: .medium))
                        Spacer()
                        Tag(text: a.ready ? "พร้อม" : (a.auth == "needs_login" ? "ต้อง login" : "ไม่พบ CLI"),
                            tone: a.ready ? .ok : .warn)
                    }
                    ForEach(engine.accounts.filter { $0.agent == a.id }) { acc in
                        HStack {
                            Text(acc.title).font(Typo.caption).foregroundStyle(Palette.text2)
                            Spacer()
                            if acc.limited {
                                Text("ติดลิมิต").font(Typo.caption).foregroundStyle(Palette.warn)
                            } else if let used = acc.rate_limits?.primary?.used_percent {
                                Text("ใช้ไป \(Int(used))%").font(Typo.caption.monospacedDigit()).foregroundStyle(Palette.text3)
                            }
                        }
                        .padding(.leading, 30)
                    }
                    if !a.ready, !a.hint.isEmpty {
                        Text(a.hint).font(Typo.monoSmall).foregroundStyle(Palette.text3).textSelection(.enabled)
                            .padding(.leading, 30)
                    }
                }
            }
            Button("จัดการบัญชี…") { RootView.openSettings() }.buttonStyle(QuietButtonStyle(small: true))
        }
        .padding(16)
        .frame(width: 300)
    }
}

/// Name and folders of a project.
struct ProjectEditorSheet: View {
    @State var project: ChorusProject
    let onSave: (ChorusProject) -> Void
    let onDelete: () -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            SheetHeader(title: "โปรเจกต์", subtitle: "หลายโฟลเดอร์เป็นโปรเจกต์เดียวได้ agent อ่านและแก้ได้ทุกโฟลเดอร์") { dismiss() }
            Form {
                TextField("ชื่อ", text: $project.name, prompt: Text("เช่น Chorus, งานลูกค้า A"))
                Section {
                    ForEach(project.folders, id: \.self) { f in
                        HStack {
                            Image(systemName: "folder").foregroundStyle(Palette.text3)
                            VStack(alignment: .leading) {
                                Text((f as NSString).lastPathComponent)
                                Text(f).font(Typo.caption).foregroundStyle(Palette.text3).lineLimit(1).truncationMode(.middle)
                            }
                            Spacer()
                            Button { project.folders.removeAll { $0 == f } } label: { Image(systemName: "minus.circle") }
                                .buttonStyle(.borderless).help("เอาออกจากโปรเจกต์ (ไม่ลบไฟล์)")
                        }
                    }
                } header: {
                    HStack {
                        Text("โฟลเดอร์")
                        Spacer()
                        Button("เพิ่มโฟลเดอร์…") {
                            for f in ProjectStore.pickFolders(message: "เลือกโฟลเดอร์ของ \(project.name)")
                            where !project.folders.contains(f) { project.folders.append(f) }
                            if project.name.isEmpty, let f = project.folders.first { project.name = (f as NSString).lastPathComponent }
                        }
                    }
                }
            }
            .formStyle(.grouped)
            .scrollContentBackground(.hidden)
            Hairline()
            HStack {
                Button("ลบโปรเจกต์") { onDelete(); dismiss() }.buttonStyle(QuietButtonStyle(danger: true))
                    .help("ลบแค่รายการในแอป ไฟล์และ session ยังอยู่ครบ")
                Spacer()
                Button("ยกเลิก") { dismiss() }.buttonStyle(SecondaryButtonStyle())
                Button("บันทึก") { onSave(project); dismiss() }
                    .buttonStyle(PrimaryButtonStyle())
                    .keyboardShortcut(.defaultAction)
                    .disabled(project.name.trimmingCharacters(in: .whitespaces).isEmpty)
            }
            .padding(14)
        }
        .frame(width: 520, height: 460)
        .background(Palette.elevated)
    }
}
