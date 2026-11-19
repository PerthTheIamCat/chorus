import AppKit
import SwiftUI

// The sidebar of the v0.4 window: every project's sessions, one status line
// each (running, waiting for you, failed, or how long ago), and the agents'
// state at the bottom. A native source-list `List`: it is lazy, it owns
// selection and keyboard navigation, and it looks like every other Mac app.

struct ShellSidebar: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @ObservedObject var projects: ProjectStore
    var openSettings: () -> Void

    @State private var showAgents = false

    var body: some View {
        List(selection: Binding(
            get: { shell.selection },
            set: { if let k = $0 { shell.open(k) } }
        )) {
            if projects.projects.isEmpty {
                ContentUnavailableView {
                    Label("ยังไม่มีโปรเจกต์", systemImage: "folder.badge.plus")
                } description: {
                    Text("เลือกโฟลเดอร์ที่จะให้ agent ทำงานด้วย")
                } actions: {
                    Button("เพิ่มโปรเจกต์…") { shell.addProject() }
                }
                .listRowSeparator(.hidden)
            }
            ForEach(projects.projects) { p in
                Section {
                    if engine.draft.map({ p.folders.contains(draftFolder($0)) }) == true {
                        DraftRow()
                    }
                    ForEach(rows(of: p)) { r in
                        SessionRow(session: r.session, title: shell.title(of: r.session),
                                   status: status(of: r.session, folder: r.folder),
                                   showsFolder: p.folders.count > 1 ? (r.folder as NSString).lastPathComponent : nil)
                            .tag(SessionKey(folder: r.folder, id: r.session.id))
                    }
                } header: {
                    HStack {
                        Text(p.name)
                        Spacer()
                        Button { shell.newSession(in: p.folders.first) } label: {
                            Image(systemName: "square.and.pencil")
                        }
                        .buttonStyle(.borderless)
                        .help("session ใหม่ใน \(p.name)")
                        .disabled(p.folders.isEmpty || engine.busy)
                    }
                }
            }
        }
        .listStyle(.sidebar)
        .searchable(text: $shell.query, placement: .sidebar, prompt: "ค้นหา session")
        .safeAreaInset(edge: .bottom, spacing: 0) { footer }
    }

    // MARK: rows

    private struct Row: Identifiable {
        let folder: String
        let session: Engine.SessionSummary
        var id: String { "\(folder)#\(session.id)" }
    }

    /// Forks that belong to a session shown in the same folder are hidden:
    /// they are the lanes of a compare, not sessions of their own.
    private func rows(of p: ChorusProject) -> [Row] {
        var out: [Row] = []
        for f in p.folders {
            let all = shell.sessions(in: f)
            let ids = Set(all.map(\.id))
            for s in all {
                if let parent = s.forked_from, ids.contains(parent) { continue }
                if matches(s) { out.append(Row(folder: f, session: s)) }
            }
        }
        return out.sorted { ($0.session.started_ms + $0.session.last_ms) > ($1.session.started_ms + $1.session.last_ms) }
    }

    private func matches(_ s: Engine.SessionSummary) -> Bool {
        let q = shell.query.trimmingCharacters(in: .whitespaces)
        return q.isEmpty
            || shell.title(of: s).localizedCaseInsensitiveContains(q)
            || s.goal.localizedCaseInsensitiveContains(q)
            || s.agents.contains { $0.localizedCaseInsensitiveContains(q) }
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

    // MARK: footer

    private var footer: some View {
        HStack(spacing: 8) {
            Button { showAgents.toggle() } label: {
                HStack(spacing: 6) {
                    Circle().fill(agentsColor).frame(width: 8, height: 8)
                    Text(engine.agents.isEmpty ? "กำลังตรวจ agent…" : "\(ready) พร้อมใช้")
                        .font(.callout)
                }
            }
            .buttonStyle(.plain)
            .popover(isPresented: $showAgents, arrowEdge: .top) { AgentsPopover() }
            .help("สถานะ agent และโควตา")
            Spacer()
            Button { shell.addProject() } label: { Image(systemName: "folder.badge.plus") }
                .buttonStyle(.borderless)
                .help("เพิ่มโปรเจกต์")
            Button(action: openSettings) { Image(systemName: "gearshape") }
                .buttonStyle(.borderless)
                .help("ตั้งค่า (⌘,)")
        }
        .padding(.horizontal, 12).padding(.vertical, 8)
        .background(.bar)
    }

    private var ready: Int { engine.agents.filter(\.ready).count }

    private var agentsColor: Color {
        if engine.agents.isEmpty { return .secondary }
        if ready == 0 { return .red }
        let limited = engine.accounts.contains { $0.limited }
        return limited || ready < engine.agents.count ? .orange : .green
    }
}

enum SessionStatus: Equatable {
    case running, waiting, failed
    case idle(Date)

    var text: String {
        switch self {
        case .running: "กำลังทำ"
        case .waiting: "รอคุณ"
        case .failed: "ล้มเหลว"
        case .idle(let d): Self.relative.localizedString(for: d, relativeTo: Date())
        }
    }

    var color: Color {
        switch self {
        case .running: Color.accentColor
        case .waiting: Color.orange
        case .failed: Color.red
        case .idle: Color.secondary
        }
    }

    private static let relative: RelativeDateTimeFormatter = {
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .short
        return f
    }()
}

private struct SessionRow: View {
    let session: Engine.SessionSummary
    let title: String
    let status: SessionStatus
    let showsFolder: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).lineLimit(1)
            HStack(spacing: 4) {
                if status == .running { ProgressView().controlSize(.mini) }
                Text(status.text).foregroundStyle(status.color)
                if let showsFolder { Text("· \(showsFolder)").foregroundStyle(.secondary) }
            }
            .font(.caption)
            .lineLimit(1)
        }
        .padding(.vertical, 1)
        .contextMenu { SessionMenu(session: session) }
    }
}

private struct SessionMenu: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    let session: Engine.SessionSummary

    var body: some View {
        Button("ตั้งชื่อด้วย AI") {
            engine.renameSessionWithAI(db: shell.currentDB, id: session.id, sampleText: shell.title(of: session))
        }
        Button("เปิดโฟลเดอร์ใน Finder") {
            NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: session.cwd)])
        }
        Divider()
        Button("ลบ session…", role: .destructive) { shell.pendingDelete = session }
    }
}

private struct DraftRow: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text("session ใหม่").italic()
            Text("ยังไม่ได้ส่ง").font(.caption).foregroundStyle(.secondary)
        }
        .padding(.vertical, 1)
    }
}

/// Every agent, whether it can answer a turn, and how full its quota is.
private struct AgentsPopover: View {
    @EnvironmentObject private var engine: Engine

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("agent").font(.headline)
            ForEach(engine.agents) { a in
                VStack(alignment: .leading, spacing: 4) {
                    HStack {
                        Image(systemName: a.ready ? "checkmark.circle.fill" : "exclamationmark.circle")
                            .foregroundStyle(a.ready ? Color.green : Color.orange)
                        Text(a.label).fontWeight(.medium)
                        Spacer()
                        if !a.ready { Text(a.auth == "needs_login" ? "ต้อง login" : "ไม่พบ CLI").foregroundStyle(.secondary) }
                    }
                    ForEach(engine.accounts.filter { $0.agent == a.id }) { acc in
                        HStack {
                            Text(acc.title).font(.caption).foregroundStyle(.secondary)
                            Spacer()
                            if acc.limited {
                                Text("ติดลิมิต").font(.caption).foregroundStyle(.orange)
                            } else if let used = acc.rate_limits?.primary?.used_percent {
                                Text("ใช้ไป \(Int(used))%").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
        }
        .padding(14)
        .frame(width: 280)
    }
}
