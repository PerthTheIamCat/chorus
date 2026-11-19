import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

// ContentView, split out of ContentView.swift: session list, projects and agent footer.
// Moved verbatim; only `private` was dropped so the pieces can see each other.

extension ContentView {
    // MARK: sidebar — open / start a session

    // A plain leading-aligned VStack instead of `Form`: Form's two-column
    // label|control layout was the cause of the earlier truncated labels —
    // there just isn't enough width in a sidebar for a label column plus a
    // control column plus a text field. Every label sits above its control
    // instead, and gets the sidebar's full width to wrap in.
    /// Hand-built rows, each one a single full-width button: what lights up
    /// on hover is exactly what takes the click. (A `List` here drew its
    /// selection pill and our row content at different sizes.)
    var isScratchChat: Bool {
        workspace.contains("chorus_chats")
    }

    var chatsSection: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text("แชทอิสระ")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                Spacer()
                Button {
                    startScratchChat()
                } label: {
                    Image(systemName: "square.and.pencil")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.chorusPlain)
                .help("เปิดห้องแชทอิสระใหม่")
            }
            .padding(.horizontal, 10)
            .padding(.top, 4)
            .padding(.bottom, 2)

            Button {
                startScratchChat()
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "bubble.left.and.bubble.right.fill")
                        .foregroundStyle(isScratchChat ? Color.accentColor : .secondary)
                    Text("ห้องสนทนาอิสระ")
                        .font(.caption)
                    Spacer()
                    if isScratchChat {
                        Circle()
                            .fill(Color.accentColor)
                            .frame(width: 6, height: 6)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(InteractiveStyle(selected: isScratchChat))

            Divider().opacity(0.3).padding(.vertical, 4)
        }
    }

    func startScratchChat() {
        let scratchDir = FileManager.default.temporaryDirectory.appendingPathComponent("chorus_chats").path
        try? FileManager.default.createDirectory(atPath: scratchDir, withIntermediateDirectories: true)
        workspace = scratchDir
        startSession()
    }

    var sidebar: some View {
        sidebarContent
            .alert(emptyCleanup.map { $0.count == 0 ? "ไม่มี session ที่ว่าง" : "ลบ session ที่ว่าง \($0.count) รายการ?" } ?? "",
                   isPresented: Binding(get: { emptyCleanup != nil }, set: { if !$0 { emptyCleanup = nil } }),
                   presenting: emptyCleanup) { c in
                if c.count > 0 {
                    Button("ลบ \(c.count) รายการ", role: .destructive) {
                        engine.deleteEmptySessions(dbs: c.dbs) { n in hint = "ลบ session ที่ว่างแล้ว \(n) รายการ" }
                    }
                    Button("ยกเลิก", role: .cancel) {}
                } else {
                    Button("ตกลง", role: .cancel) {}
                }
            } message: { c in
                Text(c.count == 0
                     ? "ทุก session ใน \(c.project) มีข้อความหรือไฟล์แนบอยู่แล้ว"
                     : "เฉพาะ session ใน \(c.project) ที่ไม่มีข้อความ ไฟล์แนบ หรือ session ย่อยเลย — session ที่เปิดอยู่ไม่ถูกลบ")
            }
    }

    var sidebarContent: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                HStack(spacing: 5) {
                    Image(systemName: "magnifyingglass").font(.caption).foregroundStyle(.secondary)
                    TextField("ค้นหา session", text: $sessionQuery).textFieldStyle(.plain)
                    if !sessionQuery.isEmpty {
                        Button { sessionQuery = "" } label: { Image(systemName: "xmark.circle.fill") }
                            .buttonStyle(.chorusPlain).foregroundStyle(.tertiary)
                    }
                }
                .padding(.horizontal, 8).padding(.vertical, 6)
                .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                Button { editing = ChorusProject(name: "", folders: []); editingIsNew = true } label: {
                    Image(systemName: "folder.badge.plus")
                }
                .buttonStyle(InteractiveStyle(cornerRadius: 7, padding: .init(top: 5, leading: 6, bottom: 5, trailing: 6)))
                .help("สร้างโปรเจกต์ใหม่")
            }
            .padding(.horizontal, 10).padding(.top, 12).padding(.bottom, 6)

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 1) {
                    chatsSection

                    Text("โปรเจกต์")
                        .font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                        .padding(.horizontal, 10).padding(.top, 6).padding(.bottom, 4)
                    if projects.projects.isEmpty {
                        Button { editing = ChorusProject(name: "", folders: []); editingIsNew = true } label: {
                            Label("สร้างโปรเจกต์แรก", systemImage: "square.stack.3d.up.badge.plus")
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .buttonStyle(InteractiveStyle())
                    }
                    ForEach(projects.projects) { p in projectRow(p) }
                    toolsSection
                }
                .padding(.horizontal, 6).padding(.bottom, 8)
            }
            agentFooter
        }
    }

    /// Installed tools, each with its own page. Kept out of Settings so a tool
    /// is one click away while working.
    var toolsSection: some View {
        VStack(alignment: .leading, spacing: 1) {
            Text("เครื่องมือ")
                .font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                .padding(.horizontal, 10).padding(.top, 14).padding(.bottom, 4)
            ForEach(engine.catalog.filter(\.isInstalled)) { tool in
                Button { toolPage = tool.id } label: {
                    HStack(spacing: 9) {
                        IntegrationIcon(id: tool.id, size: 22)
                        Text(tool.title)
                        Spacer(minLength: 4)
                        if tool.isWorking { ProgressView().controlSize(.mini) }
                        else if tool.isHealthy == false { Circle().fill(.orange).frame(width: 6, height: 6) }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(InteractiveStyle(selected: toolPage == tool.id))
            }
            Button { settingsSection = .integrations; showSettings = true } label: {
                Label("เพิ่มเครื่องมือ", systemImage: "plus.circle")
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(InteractiveStyle())
            .foregroundStyle(.secondary)
        }
        .chorusScrollIndicators()
    }

    /// Which agents can answer right now — the relay depends on it, so it
    /// Unified Agent Status footer: opens a popover listing all providers
    /// with their brand icons, models, accounts, and usage quotas.
    var agentFooter: some View {
        VStack(spacing: 0) {
            Divider().opacity(0.35)
            HStack(spacing: 6) {
                if engine.agents.isEmpty {
                    ProgressView().controlSize(.mini)
                    Text("กำลังตรวจ agent…").font(.caption2).foregroundStyle(.secondary)
                } else {
                    Button {
                        showProvidersPopover.toggle()
                    } label: {
                        HStack(spacing: 6) {
                            ZStack(alignment: .bottomTrailing) {
                                HStack(spacing: 4) {
                                    ForEach(engine.agents) { a in
                                        ProviderIcon(provider: a.id, size: 14)
                                            .foregroundStyle(a.ready ? .primary : .secondary)
                                    }
                                }

                                Circle()
                                    .fill(overallAgentsColor)
                                    .frame(width: 5.5, height: 5.5)
                                    .offset(x: 2, y: 2)
                            }

                            // Drops the words before anything wraps: a narrow
                            // sidebar squeezed this into a tall column.
                            ViewThatFits(in: .horizontal) {
                                Text("\(readyAgentsCount) พร้อมใช้")
                                    .font(.system(size: 11, weight: .medium))
                                    .lineLimit(1)
                                    .fixedSize()
                                Text("\(readyAgentsCount)")
                                    .font(.system(size: 11, weight: .medium))
                                    .lineLimit(1)
                                    .fixedSize()
                            }

                            Image(systemName: "chevron.up")
                                .font(.system(size: 8, weight: .bold))
                                .foregroundStyle(.tertiary)
                        }
                        .padding(.horizontal, 7)
                        .padding(.vertical, 4.5)
                        .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 7, style: .continuous))
                    }
                    .buttonStyle(.chorusPlain)
                    .popover(isPresented: $showProvidersPopover, arrowEdge: .top) {
                        providersPopoverContent
                    }
                    .help("คลิกเพื่อดูรายการสถานะ agent และโควตาทั้งหมด")
                }

                Spacer(minLength: 4)

                ChorusRAMLabel(isPresented: $showSystemMonitorPopover)

                Button {
                    settingsSection = .accounts
                    showSettings = true
                } label: {
                    Image(systemName: "gearshape")
                        .font(.system(size: 13))
                        .foregroundStyle(.secondary)
                        .padding(5)
                        .background(Color.primary.opacity(0.04), in: Circle())
                }
                .buttonStyle(.chorusPlain)
                .help("ตั้งค่า (⌘,)")
            }
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, 10).padding(.vertical, 8)
        }
    }

    var readyAgentsCount: Int {
        engine.agents.filter(\.ready).count
    }

    var overallAgentsColor: Color {
        if engine.agents.isEmpty { return .secondary }
        let anyLimited = engine.accounts.filter(\.ready).contains(where: \.limited)
        if anyLimited { return .orange }
        return engine.agents.allSatisfy(\.ready) ? .green : .orange
    }

    var providersPopoverContent: some View {
        VStack(alignment: .leading, spacing: 12) {
            // Header
            HStack(spacing: 8) {
                Image(systemName: "cpu")
                    .font(.system(size: 13, weight: .bold))
                    .foregroundStyle(.secondary)
                Text("agent และโควตา")
                    .font(.system(size: 13, weight: .bold))
                Spacer()
                Text("\(readyAgentsCount)/\(engine.agents.count) พร้อมใช้")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(overallAgentsColor)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(overallAgentsColor.opacity(0.12), in: Capsule())
            }

            Divider().opacity(0.5)

            // Providers List
            VStack(spacing: 7) {
                ForEach(engine.agents) { a in
                    let accs = engine.accounts.filter { $0.agent == a.id }
                    let limited = !accs.isEmpty && accs.filter(\.ready).allSatisfy(\.limited)
                    let worst = accs.compactMap(\.worst).min()
                    let statusColor: Color = !a.ready ? Color.orange : limited ? .red : .green

                    Button {
                        showProvidersPopover = false
                        settingsSection = .accounts
                        showSettings = true
                    } label: {
                        HStack(alignment: .top, spacing: 10) {
                            ZStack(alignment: .bottomTrailing) {
                                ProviderIcon(provider: a.id, size: 22)
                                    .padding(4)
                                    .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 6))

                                Circle()
                                    .fill(statusColor)
                                    .frame(width: 6, height: 6)
                                    .offset(x: 1, y: 1)
                            }

                            VStack(alignment: .leading, spacing: 3) {
                                HStack(spacing: 6) {
                                    Text(a.label)
                                        .font(.system(size: 12, weight: .semibold))
                                        .foregroundStyle(.primary)

                                    if accs.count > 1 {
                                        Text("\(accs.count) บัญชี")
                                            .font(.system(size: 9, weight: .bold))
                                            .foregroundStyle(.tertiary)
                                            .padding(.horizontal, 4)
                                            .padding(.vertical, 1)
                                            .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 4))
                                    }

                                    Spacer()

                                    Text(!a.ready ? "ยังไม่พร้อม" : limited ? "ติดลิมิต" : "พร้อมใช้")
                                        .font(.system(size: 10.5, weight: .medium))
                                        .foregroundStyle(statusColor)
                                }

                                if !a.defaultModel.isEmpty {
                                    Text(a.defaultModel)
                                        .font(.system(size: 10, design: .monospaced))
                                        .foregroundStyle(.secondary)
                                        .lineLimit(1)
                                }

                                if let worst {
                                    VStack(alignment: .leading, spacing: 2) {
                                        HStack {
                                            Text("โควตาใช้งาน:")
                                                .font(.system(size: 9.5))
                                                .foregroundStyle(.tertiary)
                                            Spacer()
                                            Text("\(Int(worst * 100))%")
                                                .font(.system(size: 9.5, weight: .semibold, design: .monospaced))
                                                .foregroundStyle(.secondary)
                                        }
                                        UsageBar(fraction: worst, height: 3.5)
                                    }
                                    .padding(.top, 2)
                                } else if !a.ready {
                                    Text(a.detail)
                                        .font(.caption2)
                                        .foregroundStyle(.orange)
                                        .lineLimit(1)
                                }
                            }
                        }
                        .padding(8)
                        .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                    }
                    .buttonStyle(.chorusPlain)
                    .help(agentHelpTooltip(a, accs: accs, limited: limited, worst: worst))
                }
            }

            Divider().opacity(0.5)

            // Footer
            Button {
                showProvidersPopover = false
                settingsSection = .accounts
                showSettings = true
            } label: {
                HStack(spacing: 5) {
                    Image(systemName: "slider.horizontal.3")
                        .font(.system(size: 10.5))
                    Text("จัดการบัญชีและโมเดลทั้งหมด…")
                        .font(.system(size: 11, weight: .medium))
                    Spacer()
                    Text("⌘,")
                        .font(.system(size: 9.5, design: .monospaced))
                        .foregroundStyle(.tertiary)
                }
                .foregroundStyle(.secondary)
            }
            .buttonStyle(.chorusPlain)
        }
        .padding(12)
        .frame(width: 270)
    }

    /// A section header plus its content, spaced and left-aligned — the
    /// sidebar's substitute for `Form`'s `Section`.
    @ViewBuilder
    func sidebarSection<Content: View>(
        _ title: String, @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).font(.subheadline.bold())
            content()
        }
    }

    // MARK: thread — the ledger, plus the compose box

    func dbPath(for project: String) -> String {
        (project as NSString).appendingPathComponent(".chorus/store.db")
    }

    func sessions(of p: ChorusProject) -> [(folder: String, s: Engine.SessionSummary, key: String)] {
        var result: [(folder: String, s: Engine.SessionSummary, key: String)] = []
        for f in p.folders {
            let all: [Engine.SessionSummary] = engine.sessionsByDB[dbPath(for: f)] ?? []
            let allIDs: Set<Int64> = Set(all.map(\.id))
            for s in all {
                if let forked = s.forked_from, allIDs.contains(forked) { continue }
                result.append((folder: f, s: s, key: "\(f)#\(s.id)"))
            }
        }
        return result.sorted { ($0.s.started_ms + $0.s.last_ms) > ($1.s.started_ms + $1.s.last_ms) }
    }

    func checkSubagentNotifications(in dbSubs: [String: [Engine.SubagentInfo]]) {
        guard !NSApp.isActive else { return }
        for subs in dbSubs.values {
            for sub in subs {
                if sub.status == "waiting" || sub.status == "failed" {
                    let key = "\(sub.id)-\(sub.status)"
                    if !notifiedSubagents.contains(key) {
                        notifiedSubagents.insert(key)
                        postNotification(
                            id: key,
                            title: sub.status == "waiting" ? "Chorus: งานย่อยรอคำยืนยัน" : "Chorus: งานย่อยล้มเหลว",
                            body: "\(sub.role): \(sub.task)")
                    }
                }
            }
        }
    }

    func postNotification(id: String, title: String, body: String) {
        TurnNotice.post(id: id, title: title, body: body)
    }

    func loadSessions(of p: ChorusProject) {
        for f in p.folders { engine.loadSessions(db: dbPath(for: f)) }
    }

    func toggle(_ p: ChorusProject) {
        withAnimation(.easeOut(duration: 0.18)) {
            if expanded.contains(p.id.uuidString) { expanded.remove(p.id.uuidString) }
            else { expanded.insert(p.id.uuidString); loadSessions(of: p) }
        }
    }

    func selectProject(_ p: ChorusProject) {
        if let f = p.folders.first {
            workspace = f
        }
        withAnimation(.easeOut(duration: 0.18)) {
            expanded.insert(p.id.uuidString)
            loadSessions(of: p)
        }
        let sess = sessions(of: p)
        if let first = sess.first {
            if engine.sessionID < 0 || !p.folders.contains(workspace) {
                engine.reopen(db: dbPath(for: first.folder), sessionID: first.s.id)
            }
        }
    }

    func projectRow(_ p: ChorusProject) -> some View {
        let open = expanded.contains(p.id.uuidString) || !sessionQuery.isEmpty
        let rows = sessions(of: p).filter { matches($0.s) }
        let isCurrent = p.folders.contains(workspace)
        return VStack(alignment: .leading, spacing: 1) {
            HStack(spacing: 4) {
                Button { selectProject(p) } label: {
                    HStack(spacing: 7) {
                        Button { toggle(p) } label: {
                            Image(systemName: "chevron.right")
                                .font(.system(size: 9, weight: .bold)).foregroundStyle(.secondary)
                                .rotationEffect(.degrees(open ? 90 : 0))
                                .frame(width: 14, height: 14)
                        }
                        .buttonStyle(.chorusPlain)

                        Image(systemName: "square.stack.3d.up.fill").foregroundStyle(Color.accentColor)
                        Text(p.name).fontWeight(.medium).lineLimit(1)
                        Spacer(minLength: 0)
                        if isCurrent && engine.sessionID >= 0 {
                            Circle().fill(.green).frame(width: 6, height: 6)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(InteractiveStyle(cornerRadius: 8, selected: isCurrent, padding: .init(top: 6, leading: 6, bottom: 6, trailing: 8)))
                .contextMenu {
                    Button("session ใหม่") { newSession(in: p) }
                    Button("ลบ session ที่ว่าง…") { confirmEmptyCleanup(p) }
                        .disabled(p.folders.isEmpty)
                    Button("แก้ไขโปรเจกต์…") { editing = p; editingIsNew = false }
                    Menu("เปิดใน Finder") {
                        ForEach(p.folders, id: \.self) { f in
                            Button((f as NSString).lastPathComponent) {
                                NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: f)])
                            }
                        }
                    }
                    Divider()
                    Button("ลบโปรเจกต์ (ไฟล์ไม่หาย)", role: .destructive) { projects.remove(p) }
                }
                newSessionButton(p)
            }
            if open {
                if p.folders.isEmpty {
                    Button { editing = p; editingIsNew = false } label: {
                        Label("เพิ่มโฟลเดอร์…", systemImage: "folder.badge.plus").font(.caption)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 7))
                    .padding(.leading, 18)
                } else if rows.isEmpty {
                    Text(sessionQuery.isEmpty ? "ยังไม่มี session — กด ✎ เพื่อเริ่ม" : "ไม่พบ")
                        .font(.caption).foregroundStyle(.tertiary)
                        .padding(.leading, 30).padding(.vertical, 4)
                }
                if let d = engine.draft, p.folders.contains(where: { draftFolder(d) == $0 }) {
                    draftRow
                }
                ForEach(rows, id: \.key) { r in sessionRow(r.s, folder: r.folder, showFolder: p.folders.count > 1) }
            }
        }
        .padding(.bottom, open ? 4 : 0)
        .onAppear { if expanded.contains(p.id.uuidString) { loadSessions(of: p) } }
    }

    /// The project folder a draft belongs to (its store is `<folder>/.chorus/store.db`).
    func draftFolder(_ d: Engine.DraftInfo) -> String {
        ((d.db as NSString).deletingLastPathComponent as NSString).deletingLastPathComponent
    }

    /// The unsent new session: not in the store yet, so not in `rows`.
    var draftRow: some View {
        HStack(spacing: 7) {
            Image(systemName: "square.and.pencil").font(.caption).foregroundStyle(.secondary)
            Text("session ใหม่").italic().lineLimit(1)
            Spacer(minLength: 0)
            Text("ยังไม่ได้ส่ง").font(.caption2).foregroundStyle(.tertiary)
        }
        .font(.callout)
        .padding(.vertical, 5).padding(.leading, 30).padding(.trailing, 8)
        .background(Color.accentColor.opacity(engine.sessionID < 0 ? 0.12 : 0), in: RoundedRectangle(cornerRadius: 7))
        .help("session นี้จะถูกสร้างเมื่อส่งข้อความแรก")
    }

    /// Count first, then ask: nothing is deleted before the user sees how many.
    func confirmEmptyCleanup(_ p: ChorusProject) {
        let dbs = p.folders.map { dbPath(for: $0) }
        engine.emptySessionCount(dbs: dbs) { n in
            emptyCleanup = EmptyCleanup(project: p.name, dbs: dbs, count: n)
        }
    }

    func newSession(in p: ChorusProject, folder: String? = nil) {
        guard let f = folder ?? p.folders.first, !engine.busy, !engine.switching else { return }
        creatingIn = p.id
        workspace = f
        expanded.insert(p.id.uuidString)
        startSession()
    }

    /// ✎ on the project row. Its own spinner state, so switching between
    /// existing sessions never touches it (that flicker was the old label
    /// keying off the shared "opening" flag).
    @ViewBuilder
    func newSessionButton(_ p: ChorusProject) -> some View {
        let style = InteractiveStyle(cornerRadius: 7, padding: .init(top: 5, leading: 6, bottom: 5, trailing: 6))
        let label = Group {
            if creatingIn == p.id && engine.switching { ProgressView().controlSize(.mini) }
            else { Image(systemName: "square.and.pencil") }
        }
        .frame(width: 16, height: 16)
        if p.folders.count > 1 {
            Button { pickingFolderFor = p.id } label: { label }
                .buttonStyle(style)
                .help("session ใหม่ — เลือกโฟลเดอร์")
                .popover(isPresented: Binding(get: { pickingFolderFor == p.id },
                                              set: { if !$0 { pickingFolderFor = nil } })) {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(p.folders, id: \.self) { f in
                            Button { pickingFolderFor = nil; newSession(in: p, folder: f) } label: {
                                Label((f as NSString).lastPathComponent, systemImage: "folder")
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                            .buttonStyle(InteractiveStyle(cornerRadius: 7))
                        }
                    }
                    .padding(6).frame(width: 220)
                }
                .disabled(engine.busy || engine.switching)
        } else {
            Button { newSession(in: p) } label: { label }
                .buttonStyle(style)
                .help("session ใหม่")
                .disabled(p.folders.isEmpty || engine.busy || engine.switching)
        }
    }

    /// A title the user set wins; otherwise the first thing they asked.
    func displayTitle(_ s: Engine.SessionSummary) -> String {
        let t = s.title.trimmingCharacters(in: .whitespaces)
        if !t.isEmpty && t != "session" { return t }
        return s.goal.isEmpty ? "session #\(s.id)" : s.goal
    }

    func matches(_ s: Engine.SessionSummary) -> Bool {
        let q = sessionQuery.trimmingCharacters(in: .whitespaces)
        return q.isEmpty || displayTitle(s).localizedCaseInsensitiveContains(q)
            || s.goal.localizedCaseInsensitiveContains(q)
            || s.agents.contains { $0.localizedCaseInsensitiveContains(q) }
    }

    func sessionRow(_ s: Engine.SessionSummary, folder: String, showFolder: Bool) -> some View {
        let selected = folder == workspace && s.id == engine.sessionID
        let meta = ((showFolder ? [(folder as NSString).lastPathComponent] : []) + s.agents).joined(separator: " · ")
        let isRenamingThis = engine.renamingSessionIDs.contains(s.id)
        let dbSessions = engine.sessionsByDB[dbPath(for: folder)] ?? []
        let childSessions = dbSessions.filter { $0.forked_from == s.id }
        let subs = engine.subagentsByParent[s.id] ?? (engine.subagentsByDB[dbPath(for: folder)] ?? []).filter { $0.parent_sess == s.id }

        let runningSubs = subs.filter { $0.status == "running" }
        let waitingSubs = subs.filter { $0.status == "waiting" }
        let failedSubs = subs.filter { $0.status == "failed" }
        let hasActive = !runningSubs.isEmpty || !waitingSubs.isEmpty
        let isExpanded = hasActive || expandedSubParents.contains(s.id)

        let totalCost: Double? = {
            let selfCost = s.cost_usd
            let childCosts = childSessions.compactMap { $0.cost_usd }
            if selfCost == nil && childCosts.isEmpty { return nil }
            return (selfCost ?? 0.0) + childCosts.reduce(0.0, +)
        }()

        return VStack(alignment: .leading, spacing: 1) {
            Button {
                guard !selected, !engine.busy, !engine.switching else { return }
                workspace = folder
                engine.reopen(db: dbPath(for: folder), sessionID: s.id)
            } label: {
                VStack(alignment: .leading, spacing: 1) {
                    HStack(spacing: 5) {
                        if !subs.isEmpty {
                            Button {
                                withAnimation(.easeOut(duration: 0.16)) {
                                    if expandedSubParents.contains(s.id) {
                                        expandedSubParents.remove(s.id)
                                    } else {
                                        expandedSubParents.insert(s.id)
                                    }
                                }
                            } label: {
                                Image(systemName: "chevron.right")
                                    .font(.system(size: 8, weight: .bold))
                                    .foregroundStyle(.secondary)
                                    .rotationEffect(.degrees(isExpanded ? 90 : 0))
                                    .frame(width: 12, height: 12)
                            }
                            .buttonStyle(.chorusPlain)
                        }

                        Text(displayTitle(s)).lineLimit(1)
                            .fontWeight(selected ? .semibold : .regular)

                        if isRenamingThis {
                            ProgressView()
                                .controlSize(.mini)
                        }

                        // Attention badges
                        if !failedSubs.isEmpty {
                            Image(systemName: "exclamationmark.circle.fill")
                                .font(.system(size: 9))
                                .foregroundStyle(.red)
                                .help("มีงานย่อยล้มเหลว")
                        } else if !waitingSubs.isEmpty {
                            Image(systemName: "pause.circle.fill")
                                .font(.system(size: 9))
                                .foregroundStyle(.orange)
                                .help("มีงานย่อยรอคำยืนยัน")
                        }
                        // The open session is the only one that can be waiting:
                        // its agent is blocked on a question, a key or a choice.
                        if selected, let waiting = AgentPromptNotice.badge(engine.waitingForYou) {
                            Text(waiting)
                                .font(.system(size: 9, weight: .bold))
                                .foregroundStyle(.white)
                                .padding(.horizontal, 4)
                                .padding(.vertical, 1)
                                .background(Color.orange, in: Capsule())
                        }

                        Spacer(minLength: 4)

                        // Subagents count indicator on parent row
                        if !subs.isEmpty {
                            if hasActive {
                                if !waitingSubs.isEmpty {
                                    Text("\(waitingSubs.count) รอ")
                                        .font(.system(size: 9, weight: .bold))
                                        .foregroundStyle(.white)
                                        .padding(.horizontal, 4)
                                        .padding(.vertical, 1)
                                        .background(Color.orange, in: Capsule())
                                } else {
                                    Text("\(runningSubs.count) กำลังทำ")
                                        .font(.system(size: 9, weight: .medium))
                                        .foregroundStyle(Color.accentColor)
                                        .padding(.horizontal, 4)
                                        .padding(.vertical, 1)
                                        .background(Color.accentColor.opacity(0.12), in: Capsule())
                                }
                            } else {
                                Text("เสร็จแล้ว \(subs.count)")
                                    .font(.system(size: 9, weight: .regular))
                                    .foregroundStyle(.secondary)
                            }
                        }

                        if let cost = totalCost {
                            let costText = Money.format(usd: cost, currency: displayCurrency,
                                                        thbRate: Money.rate(thbRate))
                            Text(costText)
                                .font(.system(size: 9.5, weight: .medium, design: .monospaced))
                                .foregroundStyle(.secondary)
                                .padding(.horizontal, 4)
                                .padding(.vertical, 1)
                                .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 3))
                                .help(!childSessions.isEmpty ? "มูลค่ารวม parent + \(childSessions.count) งานย่อย" : "มูลค่าเทียบ API")
                        }
                    }
                    if !meta.isEmpty {
                        Text(meta).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(InteractiveStyle(cornerRadius: 8, selected: selected,
                                          padding: .init(top: 5, leading: 10, bottom: 5, trailing: 8)))
            .disabled(engine.busy && !selected)
            .padding(.leading, 18)
            .contextMenu {
                Button {
                    engine.renameSessionWithAI(db: dbPath(for: folder), id: s.id, sampleText: displayTitle(s))
                } label: {
                    Label(isRenamingThis ? "กำลังตั้งชื่อด้วย AI…" : "ตั้งชื่อด้วย AI ✨", systemImage: isRenamingThis ? "hourglass" : "sparkles")
                }
                .disabled(isRenamingThis)
                Button("เปลี่ยนชื่อ…") { renaming = (folder, s); renameText = displayTitle(s) }
                Button("เปิดโฟลเดอร์ใน Finder") {
                    NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: folder)])
                }
                Divider()
                Button("ลบ session…", role: .destructive) { deleting = (folder, s) }
            }

            // Render sub-sessions tree if expanded
            if isExpanded && !subs.isEmpty {
                VStack(alignment: .leading, spacing: 1) {
                    ForEach(subs) { sub in
                        SubagentTreeRow(
                            sub: sub,
                            siblings: subs,
                            level: 1,
                            folder: folder,
                            currentWorkspace: workspace,
                            selectedSessionID: engine.sessionID,
                            subagentsByParent: engine.subagentsByParent,
                            onSelect: { childId in
                                guard !engine.busy, !engine.switching else { return }
                                workspace = folder
                                engine.reopen(db: dbPath(for: folder), sessionID: childId)
                            },
                            onMessage: { s in
                                messagingSubagent = s
                                subagentMessageText = ""
                            },
                            onCancel: { subId in
                                engine.cancelSubagent(db: dbPath(for: folder), id: subId)
                            },
                            onCancelParent: {
                                if engine.busy {
                                    engine.cancel()
                                    queued.removeAll()
                                }
                            },
                            onKeepRole: { roleName, sub in
                                let roleToSave = Engine.AgentRole(
                                    name: roleName.lowercased(),
                                    description: "บทบาทสังเคราะห์สำหรับงาน: \(sub.task)",
                                    system_prompt: "คุณคือผู้เชี่ยวชาญในบทบาท \(roleName)\nภารกิจ: \(sub.task)",
                                    tools: (sub.worktree != nil && !sub.worktree!.isEmpty) ? ["read", "bash", "edit", "write"] : ["read", "bash"],
                                    model: sub.model
                                )
                                engine.saveRole(cwd: folder, role: roleToSave) { ok, err in
                                    if ok {
                                        hint = "บันทึก Role '\(roleName)' ลง .chorus/agents/ เรียบร้อยแล้ว"
                                    } else if let err = err {
                                        hint = "บันทึก Role ไม่สำเร็จ: \(err)"
                                    }
                                }
                            }
                        )
                    }
                }
            }
        }
    }
}
