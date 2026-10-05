import AppKit
import SwiftUI

/// Mirrors `Policy::from_level` in chorus-cli/session.rs — the FFI takes
/// this as a plain int, not a bool, since read-only sits between deny
/// and allow (auto-approves reads, still denies writes/commands/network).
/// What the agent may do. Two answers, not four.
///
/// There were four: ask / deny / read-only / allow. Two of them stopped being
/// distinguishable once the engine learned that agents do not reliably ask
/// permission at all — deny and read-only now produce the same thing (the
/// restricted ACP mode, plus a revert of anything written anyway), and
/// deny-everything was never usable: an agent that may not read cannot answer.
/// A control whose options differ only in theory is a control that makes people
/// guess, so it is down to the two answers that behave differently.
enum PermissionLevel: Int32, CaseIterable {
    /// Routes to the interactive path: each request pops a dialog, and if the
    /// agent writes without asking, the turn is reverted.
    case ask = -1
    /// Everything allowed, nothing reverted.
    case allowAll = 2

    var label: String {
        switch self {
        case .ask: "ถามก่อน"
        case .allowAll: "ให้ทำเลย"
        }
    }

    var hint: String {
        switch self {
        case .ask: "เด้งถามก่อนเขียนไฟล์หรือรันคำสั่ง — ถ้า agent เขียนโดยไม่ถาม Chorus ย้อนกลับให้"
        case .allowAll: "ให้ทำได้ทุกอย่างโดยไม่ถาม — ใช้กับโฟลเดอร์ที่ลบทิ้งได้เท่านั้น"
        }
    }
}

/// sessions (left) · thread (centre) · context (right) — the layout from
/// design §7, at its smallest. Every action goes straight to `Engine`,
/// which forwards to Rust.
struct ContentView: View {
    @EnvironmentObject var engine: Engine

    // Empty on purpose: defaulting to the home directory made "Start
    // session" look hung — it snapshots the whole tree (checkpoint.rs does
    // `git add -A`) before the first turn, which is minutes-to-forever over
    // a home folder (worse still if it's iCloud-synced). Force a real
    // project path instead of silently picking something enormous.
    @State private var workspace = ""
    @State private var recents: [String] =
        UserDefaults.standard.stringArray(forKey: "chorus.recentWorkspaces") ?? []
    /// Holds the composer text outside `ContentView`'s own state: a `@State`
    /// string here re-evaluated this 1000-line body on every keystroke, which
    /// is what made typing lag. Only `ComposerField` observes the box.
    @StateObject private var draftBox = DraftBox()
    @State private var agent = "codex"
    @State private var permission = PermissionLevel.ask
    @State private var model = ""
    @State private var showSettings = false
    @State private var compareOutput: String?
    @State private var inspected: (text: String, tokens: Int64)?
    @State private var lastPrompt = ""

    @State private var rightTab = RightTab.context
    @State private var showPalette = false
    @State private var showInspector = true
    @AppStorage("chorus.failoverAccounts") private var failoverAccounts = true
    @AppStorage("chorus.failoverProviders") private var failoverProviders = true

    @State private var showSidebar = true
    @State private var expanded: Set<String> = []
    @StateObject private var projects = ProjectStore()
    @StateObject private var fileIndex = FileIndex()
    @State private var editing: ChorusProject?
    @State private var editingIsNew = false
    @State private var creatingIn: UUID?
    @State private var pickingFolderFor: UUID?
    @State private var sessionQuery = ""
    @State private var renaming: (folder: String, s: Engine.SessionSummary)?
    @State private var renameText = ""
    @State private var deleting: (folder: String, s: Engine.SessionSummary)?
    @State private var queued: [(model: String, text: String)] = []
    @State private var hint: String?
    @State private var confirmRevert = false
    @State private var rejected: Set<String> = []

    enum RightTab: String, CaseIterable {
        case context = "คอนเท็กซ์", changes = "ที่แก้ไป", memory = "ความจำ", subagents = "งานย่อย"

        var icon: String {
            switch self {
            case .context: "gauge.with.dots.needle.50percent"
            case .changes: "plusminus"
            case .memory: "brain.head.profile"
            case .subagents: "point.3.connected.trianglepath.dotted"
            }
        }
    }

    private func badge(_ t: RightTab) -> Int {
        switch t {
        case .context: 0
        case .changes: Set(parseHunks(engine.workingDiff).map(\.file)).count
        case .memory: engine.pending.count
        case .subagents: engine.events.filter { $0.kind == "tool_call" && Self.isSubagentCall($0.body) }.count
        }
    }

    /// Icon tabs with counts, so the panel says where something needs you
    /// (files changed, proposals waiting) before you open the tab.
    private var inspectorTabs: some View {
        HStack(spacing: 2) {
            ForEach(RightTab.allCases, id: \.self) { t in
                let n = badge(t)
                Button { rightTab = t } label: {
                    VStack(spacing: 3) {
                        ZStack(alignment: .topTrailing) {
                            Image(systemName: t.icon).font(.system(size: 14, weight: .medium))
                                .frame(width: 26, height: 18)
                            if n > 0 {
                                Text("\(n)").font(.system(size: 9, weight: .bold)).foregroundStyle(.white)
                                    .padding(.horizontal, 4).padding(.vertical, 1)
                                    .background(t == .memory ? Color.orange : Color.accentColor, in: Capsule())
                                    .offset(x: 8, y: -5)
                            }
                        }
                        Text(t.rawValue).font(.caption2)
                    }
                    .frame(maxWidth: .infinity)
                    .foregroundStyle(rightTab == t ? Color.accentColor : .secondary)
                }
                .buttonStyle(InteractiveStyle(cornerRadius: 9, selected: rightTab == t,
                                              padding: .init(top: 6, leading: 2, bottom: 5, trailing: 2)))
            }
        }
        .padding(8)
    }

    var body: some View {
        // Three floating glass cards — sidebar, thread, inspector — over a
        // soft backdrop, instead of edge-to-edge columns split by dividers.
        HStack(spacing: 10) {
            if showSidebar {
                sidebar
                    .frame(width: 272)
                    .floatingPanel(cornerRadius: 20)
                    .transition(.move(edge: .leading).combined(with: .opacity))
            }
            thread
            if showInspector {
                VStack(spacing: 0) {
                    inspectorTabs
                    Divider().opacity(0.5)
                    switch rightTab {
                    case .context: contextPane
                    case .subagents: subagentsPane
                    case .changes: changesPane
                    case .memory: memoryPane
                    }
                }
                .frame(width: 312)
                .floatingPanel(cornerRadius: 20)
                .transition(.move(edge: .trailing).combined(with: .opacity))
            }
        }
        .padding(10)
        .background(Backdrop())
        .animation(.spring(response: 0.32, dampingFraction: 0.86), value: showSidebar)
        .animation(.spring(response: 0.32, dampingFraction: 0.86), value: showInspector)
        .toolbar {
            ToolbarItem(placement: .navigation) {
                Button { showSidebar.toggle() } label: {
                    Label("แถบโปรเจกต์", systemImage: "sidebar.left")
                }
                .keyboardShortcut("s", modifiers: [.command, .control])
                .help("เปิด/ปิดแถบโปรเจกต์ (⌃⌘S)")
            }
            ToolbarItem(placement: .navigation) {
                Text(engine.sessionID < 0 ? "ยังไม่ได้เปิดเซสชัน"
                     : (engine.sessionsByDB[dbPath]?.first { $0.id == engine.sessionID }).map(displayTitle)
                        ?? "เซสชัน \(engine.sessionID)")
                    .lineLimit(1)
                    .font(.headline)
            }
            // Push everything after this to the window's trailing edge.
            if #available(macOS 26, *) {
                ToolbarSpacer(.flexible)
            }
            ToolbarItemGroup(placement: .primaryAction) {
                // Handing over is one click per agent. Everything the next
                // agent needs is already in the ledger, so asking the user to
                // fill in a form would be asking them to retype what we know.
                if engine.sessionID >= 0 {
                    ForEach(engine.agents.filter { $0.ready && $0.id != currentAgentID }) { a in
                        Button {
                            engine.handoffAuto(to: a.id, level: max(permission.rawValue, 0))
                        } label: {
                            Label("ส่งต่อให้ \(a.label)", systemImage: "arrow.left.arrow.right")
                        }
                        .disabled(engine.busy)
                        .help("ส่งงานต่อให้ \(a.label) พร้อมเป้าหมาย สิ่งที่ทำไปแล้ว และ diff — ไม่ต้องพิมพ์อะไร")
                    }
                }
            }
            // Declared last so it sits at the far right, over the panel it opens.
            ToolbarItem(placement: .primaryAction) {
                Button { showInspector.toggle() } label: {
                    Label("แผงข้อมูล", systemImage: "sidebar.right")
                }
                .keyboardShortcut("i", modifiers: [.command, .option])
                .help("เปิด/ปิดแผงคอนเท็กซ์ ที่แก้ไป ความจำ (⌘⌥I)")
            }
        }
        .onAppear {
            engine.loadAgents(); engine.registerProgress(); engine.loadUsage()
            if let raw = UserDefaults.standard.object(forKey: "chorus.defaultPermission") as? Int,
               let p = PermissionLevel(rawValue: Int32(raw)) { permission = p }
        }
        .onChange(of: workspace) { _, w in fileIndex.load(w); engine.loadSkills(cwd: w) }
        .onChange(of: engine.sessionID) { _, id in
            guard !workspace.isEmpty else { return }
            engine.loadSessions(db: dbPath)
            if id >= 0, let p = projects.project(containing: workspace) {
                engine.setMemoryDB(ProjectStore.memoryPath(p), projectName: p.name)
            }
        }
        .onChange(of: engine.busy) { _, _ in flushQueue() }
        .onChange(of: engine.agents) { _, _ in syncModelSelection(); engine.refreshModels() }
        .onChange(of: model) { _, m in applyModel(m) }
        .sheet(isPresented: $showPalette) { palette }
        .alert("เปลี่ยนชื่อเซสชัน", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField("ชื่อ", text: $renameText)
            Button("บันทึก") {
                if let r = renaming {
                    engine.renameSession(db: dbPath(for: r.folder), id: r.s.id,
                                         title: renameText.trimmingCharacters(in: .whitespaces))
                }
                renaming = nil
            }
            Button("ยกเลิก", role: .cancel) { renaming = nil }
        }
        .confirmationDialog(
            "ลบเซสชันนี้?",
            isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
            presenting: deleting
        ) { d in
            Button("ลบถาวร", role: .destructive) {
                let open = d.folder == workspace && d.s.id == engine.sessionID
                engine.deleteSession(db: dbPath(for: d.folder), id: d.s.id, isOpen: open)
                deleting = nil
            }
            Button("ยกเลิก", role: .cancel) { deleting = nil }
        } message: { d in
            Text("“\(displayTitle(d.s))” และประวัติทั้งหมดจะหายถาวร กู้คืนไม่ได้ ไฟล์ในโฟลเดอร์ไม่ถูกแตะ")
        }
        .sheet(item: $editing) { p in
            ProjectEditor(project: p, isNew: editingIsNew,
                          onSave: { saved in
                              projects.upsert(saved)
                              expanded.insert(saved.id.uuidString)
                              loadSessions(of: saved)
                          },
                          onDelete: editingIsNew ? nil : { projects.remove(p) })
        }
        .background(
            ZStack {
                Button("") { showPalette = true }
                    .keyboardShortcut("k", modifiers: .command)
                Button("") {
                    if workspace.isEmpty || engine.busy {
                        editing = ChorusProject(name: "", folders: []); editingIsNew = true
                    } else { startSession() }
                }
                .keyboardShortcut("n", modifiers: .command)
            }
            .frame(width: 0, height: 0)
            .opacity(0)
            .allowsHitTesting(false)
        )
        .sheet(isPresented: $showSettings) {
            SettingsSheet(workspace: workspace).environmentObject(engine)
        }
        .sheet(isPresented: Binding(get: { compareOutput != nil }, set: { if !$0 { compareOutput = nil } })) {
            CompareResultsSheet(json: compareOutput ?? "")
        }
        .sheet(isPresented: Binding(get: { inspected != nil }, set: { if !$0 { inspected = nil } })) {
            contextInspector
        }
        .sheet(item: $engine.pendingAsk) { ask in
            VStack(alignment: .leading, spacing: 14) {
                Text("ขออนุญาต").font(.title3.bold())
                Text("\(agent) wants to: **\(ask.tool)**")
                if !ask.kind.isEmpty {
                    Text("kind: \(ask.kind)").font(.caption).foregroundStyle(.secondary)
                }
                if !ask.detail.isEmpty {
                    Text(ask.detail).font(.callout).textSelection(.enabled)
                }
                HStack {
                    Spacer()
                    Button("Deny", role: .destructive) { engine.answerPermission(false) }
                    Button("Allow") { engine.answerPermission(true) }
                        .keyboardShortcut(.defaultAction)
                }
            }
            .padding(20)
            .frame(width: 420)
        }
    }

    // MARK: sidebar — open / start a session

    // A plain leading-aligned VStack instead of `Form`: Form's two-column
    // label|control layout was the cause of the earlier truncated labels —
    // there just isn't enough width in a sidebar for a label column plus a
    // control column plus a text field. Every label sits above its control
    // instead, and gets the sidebar's full width to wrap in.
    /// Hand-built rows, each one a single full-width button: what lights up
    /// on hover is exactly what takes the click. (A `List` here drew its
    /// selection pill and our row content at different sizes.)
    private var sidebar: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                HStack(spacing: 5) {
                    Image(systemName: "magnifyingglass").font(.caption).foregroundStyle(.secondary)
                    TextField("ค้นหาเซสชัน", text: $sessionQuery).textFieldStyle(.plain)
                    if !sessionQuery.isEmpty {
                        Button { sessionQuery = "" } label: { Image(systemName: "xmark.circle.fill") }
                            .buttonStyle(.plain).foregroundStyle(.tertiary)
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
                }
                .padding(.horizontal, 6).padding(.bottom, 8)
            }
            agentFooter
        }
    }

    /// Which agents can answer right now — the relay depends on it, so it
    /// stays visible instead of hiding in a settings form.
    private var agentFooter: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                if engine.agents.isEmpty {
                    ProgressView().controlSize(.mini)
                    Text("กำลังตรวจเอเจนต์…").font(.caption2).foregroundStyle(.secondary)
                }
                ForEach(engine.agents) { a in
                    let accs = engine.accounts.filter { $0.agent == a.id }
                    let limited = !accs.isEmpty && accs.filter(\.ready).allSatisfy(\.limited)
                    let worst = accs.compactMap(\.worst).min()
                    Button { showSettings = true } label: {
                        VStack(alignment: .leading, spacing: 3) {
                            HStack(spacing: 4) {
                                Circle().fill(!a.ready ? Color.orange : limited ? .red : .green).frame(width: 6, height: 6)
                                Text(a.label).font(.caption2)
                                if accs.count > 1 {
                                    Text("×\(accs.count)").font(.caption2).foregroundStyle(.tertiary)
                                }
                            }
                            if let worst { UsageBar(fraction: worst, height: 3).frame(width: 54) }
                        }
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 6, padding: .init(top: 3, leading: 5, bottom: 3, trailing: 5)))
                    .help(limited ? "\(a.label): ทุกบัญชีติดลิมิต"
                          : worst.map { "\(a.label): บัญชีที่ว่างที่สุดใช้ไป \(Int($0 * 100))%" } ?? (a.ready ? "\(a.label) พร้อมใช้" : a.detail))
                }
                Spacer()
                Button { showSettings = true } label: { Image(systemName: "gearshape") }
                    .buttonStyle(InteractiveStyle(cornerRadius: 6, padding: .init(top: 4, leading: 6, bottom: 4, trailing: 6)))
                    .keyboardShortcut(",", modifiers: .command)
                    .help("ตั้งค่า (⌘,)")
            }
            .padding(.horizontal, 14).padding(.vertical, 9)
        }
    }

    /// The agent whose model id is currently selected.
    private func agentFor(_ modelID: String) -> Engine.AgentInfo? {
        engine.agents.first { $0.models.contains(where: { $0.id == modelID }) || $0.id == modelID }
    }
    /// Keep the model picker pointed at something that actually exists.
    ///
    /// The picker's tags are *model* ids (`gpt-5.1-codex`), and it was seeded
    /// with an *agent* id (`"codex"`). SwiftUI renders a Picker whose selection
    /// matches no tag as blank — so the app opened with an empty agent box and
    /// no way to tell whether detection had failed or the list was just not
    /// drawn. Seed from the loaded agents instead: a ready one first, because a
    /// picker should not default to something that cannot answer a turn.
    /// Tell the engine which model the picked one is. Without this the
    /// selection is decoration — the agent runs whatever its own CLI defaults
    /// to, and the label lies.
    private func applyModel(_ modelID: String) {
        guard let a = agentFor(modelID) else { return }
        engine.setModel(agent: a.id, model: modelID)
    }

    private func syncModelSelection() {
        let known = engine.agents.flatMap { $0.models.map(\.id) }
        guard !known.isEmpty else { return }
        if known.contains(model) {
            applyModel(model)
            return
        }
        model = engine.agents.first(where: \.ready)?.defaultModel
            ?? engine.agents[0].defaultModel
        applyModel(model)
    }


    private var currentAgentID: String { agentFor(model)?.id ?? "" }

    private func pickFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Select"
        panel.message = "เลือกโฟลเดอร์โปรเจกต์ให้ Chorus"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        addRecent(url.path)
        let p = projects.project(containing: url.path)
            ?? ChorusProject(name: url.lastPathComponent, folders: [url.path])
        projects.upsert(p)
        expanded.insert(p.id.uuidString)
        loadSessions(of: p)
    }

    private func addRecent(_ path: String) {
        var list = recents.filter { $0 != path }
        list.insert(path, at: 0)
        if list.count > 30 { list = Array(list.prefix(30)) }
        recents = list
        persistRecents()
    }

    private func persistRecents() {
        UserDefaults.standard.set(recents, forKey: "chorus.recentWorkspaces")
    }

    private func startSession() {
        let db = (workspace as NSString).appendingPathComponent(".chorus/store.db")
        try? FileManager.default.createDirectory(
            atPath: (db as NSString).deletingLastPathComponent,
            withIntermediateDirectories: true)
        engine.open(db: db, cwd: workspace, title: "session")
    }

    /// A section header plus its content, spaced and left-aligned — the
    /// sidebar's substitute for `Form`'s `Section`.
    @ViewBuilder
    private func sidebarSection<Content: View>(
        _ title: String, @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).font(.subheadline.bold())
            content()
        }
    }

    // MARK: thread — the ledger, plus the compose box

    private var thread: some View {
        Group {
            if engine.sessionID < 0 {
                emptyState
            } else {
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 12) {
                            ForEach(engine.events) { ev in
                                EventRow(event: ev).id(ev.seq)
                            }
                            if engine.busy { workingRow.id("working") }
                            else if !engine.lastThoughts.isEmpty {
                                ThinkingBlock(text: engine.lastThoughts, running: false)
                            }
                        }
                        .frame(maxWidth: 820)
                        .frame(maxWidth: .infinity)
                        .padding(.horizontal, 20).padding(.vertical, 16)
                    }
                    .onChange(of: engine.events.count) { _, _ in
                        if let last = engine.events.last { proxy.scrollTo(last.seq, anchor: .bottom) }
                    }
                    .onChange(of: engine.live.message.count + engine.live.thoughts.count / 200 + engine.live.tools.count) { _, _ in
                        if engine.busy { proxy.scrollTo("working", anchor: .bottom) }
                    }
                    .onChange(of: engine.busy) { _, b in
                        if b { withAnimation { proxy.scrollTo("working", anchor: .bottom) } }
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        // Floating composer: the thread scrolls underneath it instead of
        // stopping at a divider, the way Claude/Codex/ChatGPT do.
        .safeAreaInset(edge: .bottom, spacing: 0) {
            composer
                .frame(maxWidth: 820)
                .padding(.horizontal, 16).padding(.bottom, 14).padding(.top, 4)
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            if let err = engine.lastError {
                HStack(spacing: 8) {
                    Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.red)
                    Text(err).font(.callout).lineLimit(3).textSelection(.enabled)
                    Spacer(minLength: 0)
                    Button { engine.lastError = nil } label: { Image(systemName: "xmark") }
                        .buttonStyle(.borderless)
                }
                .padding(.horizontal, 14).padding(.vertical, 9)
                .floatingPanel(cornerRadius: 14, tint: .red)
                .padding(.horizontal, 16).padding(.top, 8)
            }
        }
        .frame(minWidth: 420)
        .clipShape(RoundedRectangle(cornerRadius: 20))
    }

    /// The running turn, live: reasoning (collapsible, like the thinking
    /// block in Claude/Codex), tool calls as they start and finish, and the
    /// answer as it streams — then Stop.
    private var workingRow: some View {
        let live = engine.live
        return VStack(alignment: .leading, spacing: 8) {
            if !live.thoughts.isEmpty {
                ThinkingBlock(text: live.thoughts, running: live.message.isEmpty)
            }
            ForEach(live.tools) { t in
                HStack(spacing: 6) {
                    switch t.state {
                    case "running": ProgressView().controlSize(.mini)
                    case "failed": Image(systemName: "xmark.circle.fill").foregroundStyle(.red)
                    default: Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                    }
                    Text(t.title).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            if !live.message.isEmpty {
                MarkdownText(text: live.message)
            }
            HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                if let t = engine.turnStartedAt {
                    TimelineView(.periodic(from: t, by: 1)) { ctx in
                        Text(live.isEmpty ? "กำลังเริ่ม agent… \(Int(ctx.date.timeIntervalSince(t))) วิ"
                                          : "กำลังทำงาน… \(Int(ctx.date.timeIntervalSince(t))) วิ")
                    }
                } else {
                    Text("กำลังทำงาน…")
                }
                Spacer()
                Button { engine.cancel(); queued.removeAll() } label: {
                    Label("หยุด", systemImage: "stop.fill")
                }
                .controlSize(.small)
                .keyboardShortcut(".", modifiers: .command)
                .help("หยุดเทิร์นนี้ (⌘.) — ล้างคิวด้วย")
            }
            .font(.callout).foregroundStyle(.secondary)
        }
        .padding(10)
        .background(.secondary.opacity(0.06), in: RoundedRectangle(cornerRadius: 10))
    }

    private var emptyState: some View {
        VStack(spacing: 10) {
            Spacer()
            Image(systemName: "bubble.left.and.bubble.right")
                .font(.system(size: 36)).foregroundStyle(.tertiary)
            Text("ยังไม่ได้เปิดเซสชัน").font(.title3.weight(.semibold))
            Text("เลือกโฟลเดอร์โปรเจกต์ทางซ้าย แล้วกดเริ่มเซสชัน")
                .font(.callout).foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var composer: some View {
        VStack(spacing: 6) {
            if !queued.isEmpty {
                VStack(alignment: .leading, spacing: 3) {
                    ForEach(Array(queued.enumerated()), id: \.offset) { i, q in
                        HStack(spacing: 6) {
                            Image(systemName: "clock").font(.caption2)
                            Text(q.text).font(.caption).lineLimit(1)
                            Spacer()
                            Button { queued.remove(at: i) } label: { Image(systemName: "xmark.circle.fill") }
                                .buttonStyle(.plain).foregroundStyle(.secondary)
                        }
                    }
                    Text("รอคิว — จะส่งต่อเมื่อเทิร์นนี้จบ").font(.caption2).foregroundStyle(.tertiary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            if let hint { Text(hint).font(.caption).foregroundStyle(.orange).frame(maxWidth: .infinity, alignment: .leading) }
            ComposerField(box: draftBox, suggestions: suggest, onSubmit: submit)
            HStack(spacing: 8) {
                ModelPicker(model: $model, onNeedsLogin: { showSettings = true })
                ChipPicker(
                    icon: permission == .ask ? "hand.raised" : "bolt.fill",
                    value: "\(permission.rawValue)",
                    options: PermissionLevel.allCases.map { "\($0.rawValue)" },
                    labels: Dictionary(uniqueKeysWithValues: PermissionLevel.allCases.map { ("\($0.rawValue)", $0.label) }),
                    selected: "\(permission.rawValue)"
                ) { v in if let p = PermissionLevel(rawValue: Int32(v) ?? 0) { permission = p } }
                .help(permission.hint)
                Spacer()
                Button(action: submit) {
                    Image(systemName: engine.busy ? "text.badge.plus" : "arrow.up")
                        .font(.system(size: 14, weight: .bold))
                        .foregroundStyle(.white)
                        .frame(width: 30, height: 30)
                        .background(Color.accentColor, in: Circle())
                }
                .buttonStyle(InteractiveStyle(cornerRadius: 99, padding: .init()))
                .keyboardShortcut(.return, modifiers: .command)
                .disabled(engine.sessionID < 0)
                .help(engine.busy ? "เข้าคิว (↩)" : "ส่ง (↩ · ⇧↩ ขึ้นบรรทัดใหม่)")
            }
        }
        .padding(.horizontal, 14).padding(.top, 12).padding(.bottom, 10)
        .floatingPanel(cornerRadius: 22)
    }

    /// `@codex fix this` → ("codex's default model", "fix this"). Matches agent
    /// id or label, case-insensitively; anything else is sent untouched.
    private func route(_ text: String) -> (model: String, text: String) {
        guard text.hasPrefix("@") else { return (model, text) }
        let parts = text.dropFirst().split(separator: " ", maxSplits: 1, omittingEmptySubsequences: true)
        guard let tag = parts.first?.lowercased(),
              let a = engine.agents.first(where: { $0.ready && ($0.id.lowercased() == tag || $0.label.lowercased() == tag) })
        else { return (model, text) }
        return (a.defaultModel, parts.count > 1 ? String(parts[1]) : "")
    }

    private func submit() {
        let raw = draftBox.text.trimmingCharacters(in: .whitespacesAndNewlines)
        let (target, text) = route(raw)
        guard engine.sessionID >= 0, !raw.isEmpty else { return }
        guard !text.isEmpty else {
            hint = "พิมพ์ข้อความต่อจาก \(raw) ด้วย"
            return
        }
        hint = nil
        draftBox.text = ""
        // Busy: line it up behind the running turn (Cursor-style queue)
        // instead of dropping the keystroke on a disabled button.
        if engine.busy { queued.append((target, text)); return }
        dispatch(target, text)
    }

    /// The relay chain for a turn, as *agent names* (with accounts), never
    /// model ids: the engine picks the bridge from the name, so sending
    /// "opus" or "default" used to start the Codex bridge for a Claude pick.
    /// Order: the chosen agent's accounts (chosen one first), then every
    /// other ready agent's accounts.
    private func chain(for agentID: String) -> [String] {
        let usable = engine.accounts.filter { $0.ready && !$0.limited }
        func names(_ agent: String) -> [String] {
            let own = usable.filter { $0.agent == agent }.map(\.name)
            return own.isEmpty ? [agent] : own
        }
        // An agent with no logged-in account is skipped up front: a revoked
        // Claude login used to cost ~3 minutes of retries before failing over.
        let chosenReady = engine.agents.first { $0.id == agentID }?.ready ?? true
        var out = chosenReady ? names(agentID) : []
        if let pick = UserDefaults.standard.string(forKey: "chorus.preferred.\(agentID)"),
           let i = out.firstIndex(of: pick) {
            out.insert(out.remove(at: i), at: 0)
        }
        if !failoverAccounts { out = Array(out.prefix(1)) }
        if failoverProviders {
            for a in engine.agents where a.ready && a.id != agentID { out += names(a.id) }
        }
        return out
    }

    private func accounts_split(_ name: String) -> String {
        String(name.split(separator: "@", maxSplits: 1).first ?? Substring(name))
    }

    private func dispatch(_ target: String, _ text: String) {
        lastPrompt = text
        let agentID = agentFor(target)?.id ?? target
        // `@codex …` picked another agent's default model; pin it for the turn.
        if target != model { engine.setModel(agent: agentID, model: target) }
        var names = chain(for: agentID)
        var notice: String?
        if names.first.map({ accounts_split($0) }) != agentID {
            let label = agentFor(target)?.label ?? agentID
            if let next = names.first {
                notice = "\(label) ยังไม่ได้ login หรือติดลิมิตทุกบัญชี — ส่งให้ \(next) แทน (login ได้ที่ ตั้งค่า → บัญชี)"
            } else {
                notice = "\(label) ยังไม่ได้ login — ไปที่ ตั้งค่า → บัญชี แล้วกด Login"
                names = [agentID]
            }
        }
        // After the send: starting a turn clears the banner.
        defer { if let notice { engine.lastError = notice } }
        if permission == .ask {
            engine.sendInteractive(text, agent: names.first ?? agentID)
        } else {
            // Always the relay path, even for one agent: a chain of one
            // behaves exactly like a single turn, and the user never has
            // to know the word "relay" to get failover.
            engine.sendRelay(text, agents: names.joined(separator: ","), level: permission.rawValue)
        }
    }

    private func flushQueue() {
        guard !engine.busy, !queued.isEmpty, engine.lastError == nil else { return }
        let next = queued.removeFirst()
        dispatch(next.model, next.text)
    }

    // MARK: `/` and `@` completion

    private func suggest(_ trigger: Character, _ query: String) -> [Suggestion] {
        let q = query.lowercased()
        func hit(_ t: String) -> Bool { q.isEmpty || t.lowercased().contains(q) }
        if trigger == "@" {
            var out: [Suggestion] = engine.agents.filter { $0.ready && hit($0.id) }.map { a in
                Suggestion(title: "@" + a.id, detail: "ส่งข้อความนี้ให้ \(a.label)", icon: "person.crop.circle",
                           insert: "@\(a.id) ")
            }
            out += fileIndex.match(query, limit: 8 - min(out.count, 3)).map { f in
                Suggestion(title: (f as NSString).lastPathComponent, detail: f, icon: "doc.text", insert: "@\(f) ")
            }
            return out
        }
        var out: [Suggestion] = []
        for a in engine.agents where a.ready && a.id != currentAgentID && engine.sessionID >= 0 {
            out.append(.init(title: "/handoff \(a.id)", detail: "ส่งงานต่อให้ \(a.label) พร้อม brief", icon: "arrow.left.arrow.right") {
                engine.handoffAuto(to: a.id, level: max(permission.rawValue, 0))
            })
        }
        for a in engine.agents where a.ready {
            for m in a.baseModels {
                out.append(.init(title: "/model \(m.id)", detail: "\(a.label) · \(m.label)", icon: "cpu") {
                    let efforts = a.efforts(for: m.id)
                    model = ModelID.compose(m.id, efforts.contains("high") ? "high" : efforts.first)
                })
            }
        }
        if let a = agentFor(model) {
            for e in a.efforts(for: ModelID(model).base) {
                out.append(.init(title: "/effort \(e)", detail: "ระดับการคิด", icon: "brain") {
                    model = ModelID.compose(ModelID(model).base, e)
                })
            }
        }
        out.append(.init(title: "/new", detail: "เซสชันใหม่ในโฟลเดอร์นี้", icon: "square.and.pencil") {
            if !workspace.isEmpty { startSession() }
        })
        out.append(.init(title: "/compare", detail: "ถามทุกเอเจนต์แล้วเทียบ", icon: "square.split.2x1") { runCompare() })
        out.append(.init(title: "/snapshot", detail: "บันทึกจุดย้อนกลับตอนนี้", icon: "camera") { engine.snapshot() })
        out.append(.init(title: "/ask", detail: "ถามก่อนเขียนไฟล์", icon: "hand.raised") { permission = .ask })
        out.append(.init(title: "/auto", detail: "ให้ทำเลยไม่ต้องถาม", icon: "bolt.fill") { permission = .allowAll })
        out.append(.init(title: "/settings", detail: "ตั้งค่าและ skills", icon: "gearshape") { showSettings = true })
        // Skills go in as text: Claude loads them by name, the others are told
        // where they live by the projected AGENTS.md index.
        for sk in engine.skills {
            out.append(.init(title: "/" + sk.name, detail: sk.description, icon: "sparkles", insert: "/\(sk.name) "))
        }
        return Array(out.filter { hit($0.title) || hit($0.detail) }.prefix(9))
    }

    private func runCompare() {
        let others = engine.agents.filter(\.ready).map(\.id)
        guard others.count > 1, engine.sessionID >= 0 else { return }
        engine.compare(
            lastPrompt.isEmpty ? "ทบทวนงานที่ค้างอยู่แล้วเสนอขั้นต่อไป" : lastPrompt,
            agents: others.joined(separator: ","),
            level: max(permission.rawValue, 0)
        ) { compareOutput = $0 }
    }

    // MARK: command palette (⌘K)

    private struct PaletteItem: Identifiable {
        let id = UUID()
        let title: String
        let icon: String
        let run: () -> Void
    }

    private var paletteItems: [PaletteItem] {
        var items: [PaletteItem] = []
        for a in engine.agents where a.ready {
            items.append(.init(title: "สลับไปใช้ \(a.label)", icon: "person.crop.circle") { model = a.defaultModel })
            if engine.sessionID >= 0 && a.id != currentAgentID {
                items.append(.init(title: "ส่งต่องานให้ \(a.label)", icon: "arrow.left.arrow.right") {
                    engine.handoffAuto(to: a.id, level: max(permission.rawValue, 0))
                })
            }
        }
        items.append(.init(title: "เลือกโฟลเดอร์โปรเจกต์…", icon: "folder") { pickFolder() })
        if engine.sessionID >= 0 {
            items.append(.init(title: "Snapshot ตอนนี้", icon: "camera") { engine.snapshot() })
        }
        items.append(.init(title: "ตั้งค่า", icon: "gearshape") { showSettings = true })
        return items
    }

    private var palette: some View {
        PaletteSheet(items: paletteItems.map { ($0.title, $0.icon, $0.run) }) { showPalette = false }
    }

    // MARK: context pane — budget + last agent

    private var changesPane: some View {
        let hunks = parseHunks(engine.workingDiff)
        let files = Dictionary(grouping: hunks, by: \.file)
        let order = hunks.map(\.file).reduce(into: [String]()) { if !$0.contains($1) { $0.append($1) } }
        return VStack(spacing: 0) {
            PaneHeader(title: "ที่แก้ไปในเซสชันนี้",
                       subtitle: hunks.isEmpty ? nil
                           : "\(order.count) ไฟล์ · +\(hunks.map(\.addCount).reduce(0, +)) −\(hunks.map(\.delCount).reduce(0, +))") {
                Button { engine.snapshot() } label: { Image(systemName: "camera") }
                    .buttonStyle(InteractiveStyle(cornerRadius: 6, padding: .init(top: 4, leading: 6, bottom: 4, trailing: 6)))
                    .disabled(engine.sessionID < 0)
                    .help("บันทึกจุดย้อนกลับตอนนี้")
            }
            if hunks.isEmpty {
                EmptyPane(icon: "checkmark.seal", title: "ยังไม่มีไฟล์ถูกแก้",
                          hint: "เมื่อเอเจนต์แก้ไฟล์ จะเห็นทีละก้อนที่นี่ เก็บหรือตีกลับได้ทีละก้อน")
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(order, id: \.self) { f in
                            FileChanges(file: f, hunks: files[f] ?? [], rejected: $rejected)
                        }
                    }
                    .padding(10)
                }
                VStack(spacing: 6) {
                    if !rejected.isEmpty {
                        Button {
                            let patch = hunks.filter { rejected.contains($0.id) }.map(\.patch).joined()
                            engine.rejectHunks(patch)
                            rejected.removeAll()
                        } label: {
                            Label("ตีกลับ \(rejected.count) ก้อนที่เลือก", systemImage: "arrow.uturn.backward")
                                .frame(maxWidth: .infinity)
                                .foregroundStyle(.white)
                        }
                        .buttonStyle(InteractiveStyle(cornerRadius: 8))
                        .background(Color.red.opacity(0.85), in: RoundedRectangle(cornerRadius: 8))
                    }
                    Button { confirmRevert = true } label: {
                        Label("ย้อนทุกอย่างกลับไปตอนเริ่มเซสชัน", systemImage: "clock.arrow.circlepath")
                            .frame(maxWidth: .infinity)
                            .foregroundStyle(.red)
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 8))
                    .confirmationDialog(
                        "ทิ้งทุกอย่างที่เอเจนต์แก้ในเซสชันนี้?",
                        isPresented: $confirmRevert, titleVisibility: .visible
                    ) {
                        Button("ย้อนทั้งหมด", role: .destructive) { engine.revertToStart() }
                        Button("ยกเลิก", role: .cancel) {}
                    }
                }
                .padding(10)
            }
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }

    private var memoryPane: some View {
        VStack(spacing: 0) {
            PaneHeader(title: engine.memoryScope.map { "ความจำร่วม · \($0)" } ?? "ความจำของโฟลเดอร์นี้",
                       subtitle: "ทุกเอเจนต์ในเซสชันนี้อ่านชุดเดียวกัน") { EmptyView() }
            if engine.facts.isEmpty && engine.pending.isEmpty {
                EmptyPane(icon: "brain", title: "ยังไม่มีความจำ",
                          hint: "เอเจนต์เสนอเรื่องที่ควรจำผ่าน memory_propose แล้วจะมารอให้คุณอนุมัติที่นี่")
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 8) {
                        if !engine.pending.isEmpty {
                            SectionLabel("รออนุมัติ", count: engine.pending.count, tint: .orange)
                            ForEach(engine.pending) { p in
                                HStack(alignment: .top, spacing: 8) {
                                    Text(p.text).font(.callout).frame(maxWidth: .infinity, alignment: .leading)
                                    Button { engine.review(p.id, approve: false) } label: { Image(systemName: "xmark") }
                                        .buttonStyle(InteractiveStyle(cornerRadius: 6, padding: .init(top: 4, leading: 6, bottom: 4, trailing: 6)))
                                        .foregroundStyle(.red).help("ปฏิเสธ")
                                    Button { engine.review(p.id, approve: true) } label: { Image(systemName: "checkmark") }
                                        .buttonStyle(InteractiveStyle(cornerRadius: 6, padding: .init(top: 4, leading: 6, bottom: 4, trailing: 6)))
                                        .foregroundStyle(.green).help("อนุมัติ")
                                }
                                .padding(10)
                                .background(Color.orange.opacity(0.10), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                            }
                        }
                        SectionLabel("จำไว้แล้ว", count: engine.facts.count, tint: .secondary)
                        ForEach(Array(engine.facts.enumerated()), id: \.offset) { _, f in
                            HStack(alignment: .top, spacing: 8) {
                                Circle().fill(Color.accentColor.opacity(0.6)).frame(width: 5, height: 5).padding(.top, 6)
                                Text(f).font(.callout).textSelection(.enabled)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                            .padding(.horizontal, 8).padding(.vertical, 6)
                            .hoverHighlight(cornerRadius: 8)
                        }
                    }
                    .padding(10)
                }
            }
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }

    /// The app's honesty surface. Everything else in this window *describes*
    /// what gets carried between agents; this shows the bytes.
    private var contextInspector: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text("สิ่งที่ agent ตัวถัดไปจะได้เห็น").font(.headline)
                    Text("ประกอบสดจาก ledger ไม่ใช่คำอธิบาย — นี่คือข้อความจริงที่จะถูกส่ง")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                if let n = inspected?.tokens, n > 0 {
                    Text("\(n) token").font(.caption.monospaced()).foregroundStyle(.secondary)
                }
                Button("Done") { inspected = nil }.keyboardShortcut(.defaultAction)
            }
            ScrollView {
                Text(inspected?.text ?? "")
                    .font(.caption.monospaced())
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(16)
        .frame(minWidth: 640, minHeight: 460)
    }

    // MARK: projects — several folders at once, each with its own sessions

    private func dbPath(for project: String) -> String {
        (project as NSString).appendingPathComponent(".chorus/store.db")
    }

    private func sessions(of p: ChorusProject) -> [(folder: String, s: Engine.SessionSummary, key: String)] {
        p.folders.flatMap { f in (engine.sessionsByDB[dbPath(for: f)] ?? []).map { (f, $0, "\(f)#\($0.id)") } }
            .sorted { ($0.s.started_ms + $0.s.last_ms) > ($1.s.started_ms + $1.s.last_ms) }
    }

    private func loadSessions(of p: ChorusProject) {
        for f in p.folders { engine.loadSessions(db: dbPath(for: f)) }
    }

    private func toggle(_ p: ChorusProject) {
        withAnimation(.easeOut(duration: 0.18)) {
            if expanded.contains(p.id.uuidString) { expanded.remove(p.id.uuidString) }
            else { expanded.insert(p.id.uuidString); loadSessions(of: p) }
        }
    }

    private func projectRow(_ p: ChorusProject) -> some View {
        let open = expanded.contains(p.id.uuidString) || !sessionQuery.isEmpty
        let rows = sessions(of: p).filter { matches($0.s) }
        return VStack(alignment: .leading, spacing: 1) {
            HStack(spacing: 2) {
                // The whole row toggles, not just the chevron.
                Button { toggle(p) } label: {
                    HStack(spacing: 7) {
                        Image(systemName: "chevron.right")
                            .font(.system(size: 9, weight: .bold)).foregroundStyle(.secondary)
                            .rotationEffect(.degrees(open ? 90 : 0))
                            .frame(width: 10)
                        Image(systemName: "square.stack.3d.up.fill").foregroundStyle(Color.accentColor)
                        Text(p.name).fontWeight(.medium).lineLimit(1)
                        Spacer(minLength: 0)
                        if p.folders.contains(workspace) && engine.sessionID >= 0 {
                            Circle().fill(.green).frame(width: 6, height: 6)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(InteractiveStyle(cornerRadius: 8, padding: .init(top: 6, leading: 6, bottom: 6, trailing: 8)))
                .contextMenu {
                    Button("เซสชันใหม่") { newSession(in: p) }
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
                    Text(sessionQuery.isEmpty ? "ยังไม่มีเซสชัน — กด ✎ เพื่อเริ่ม" : "ไม่พบ")
                        .font(.caption).foregroundStyle(.tertiary)
                        .padding(.leading, 30).padding(.vertical, 4)
                }
                ForEach(rows, id: \.key) { r in sessionRow(r.s, folder: r.folder, showFolder: p.folders.count > 1) }
            }
        }
        .padding(.bottom, open ? 4 : 0)
        .onAppear { if expanded.contains(p.id.uuidString) { loadSessions(of: p) } }
    }

    private func newSession(in p: ChorusProject, folder: String? = nil) {
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
    private func newSessionButton(_ p: ChorusProject) -> some View {
        let style = InteractiveStyle(cornerRadius: 7, padding: .init(top: 5, leading: 6, bottom: 5, trailing: 6))
        let label = Group {
            if creatingIn == p.id && engine.switching { ProgressView().controlSize(.mini) }
            else { Image(systemName: "square.and.pencil") }
        }
        .frame(width: 16, height: 16)
        if p.folders.count > 1 {
            Button { pickingFolderFor = p.id } label: { label }
                .buttonStyle(style)
                .help("เซสชันใหม่ — เลือกโฟลเดอร์")
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
                .help("เซสชันใหม่")
                .disabled(p.folders.isEmpty || engine.busy || engine.switching)
        }
    }

    /// A title the user set wins; otherwise the first thing they asked.
    private func displayTitle(_ s: Engine.SessionSummary) -> String {
        let t = s.title.trimmingCharacters(in: .whitespaces)
        if !t.isEmpty && t != "session" { return t }
        return s.goal.isEmpty ? "เซสชัน #\(s.id)" : s.goal
    }

    private func matches(_ s: Engine.SessionSummary) -> Bool {
        let q = sessionQuery.trimmingCharacters(in: .whitespaces)
        return q.isEmpty || displayTitle(s).localizedCaseInsensitiveContains(q)
            || s.goal.localizedCaseInsensitiveContains(q)
            || s.agents.contains { $0.localizedCaseInsensitiveContains(q) }
    }

    private func sessionRow(_ s: Engine.SessionSummary, folder: String, showFolder: Bool) -> some View {
        let selected = folder == workspace && s.id == engine.sessionID
        let meta = ((showFolder ? [(folder as NSString).lastPathComponent] : []) + s.agents).joined(separator: " · ")
        return Button {
            guard !selected, !engine.busy, !engine.switching else { return }
            workspace = folder
            engine.reopen(db: dbPath(for: folder), sessionID: s.id)
        } label: {
            VStack(alignment: .leading, spacing: 1) {
                Text(displayTitle(s)).lineLimit(1)
                    .fontWeight(selected ? .semibold : .regular)
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
            Button("เปลี่ยนชื่อ…") { renaming = (folder, s); renameText = displayTitle(s) }
            Button("เปิดโฟลเดอร์ใน Finder") {
                NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: folder)])
            }
            Divider()
            Button("ลบเซสชัน…", role: .destructive) { deleting = (folder, s) }
        }
    }

    /// Delegated work, read back from the ledger: every tool call that looks
    /// like a spawned subagent (Task / Agent / delegate), the result that
    /// followed it, and who ran it. This is the ledger's view — it fills in
    /// when a turn ends, not while it streams.
    private var subagentsPane: some View {
        let evs = engine.events
        let calls: [(call: LedgerEvent, result: LedgerEvent?)] = evs.enumerated().compactMap { i, e in
            guard e.kind == "tool_call", Self.isSubagentCall(e.body) else { return nil }
            let r = evs[(i + 1)...].first { $0.kind == "tool_result" }
            return (e, r)
        }
        let byAgent = Dictionary(grouping: evs.filter { $0.actor != "user" && $0.kind == "tool_call" }, by: \.actor)
        return VStack(spacing: 0) {
          PaneHeader(title: "งานย่อย", subtitle: calls.isEmpty ? nil : "\(calls.count) งานที่เอเจนต์แตกออกไป") { EmptyView() }
          if calls.isEmpty && byAgent.isEmpty {
            EmptyPane(icon: "point.3.connected.trianglepath.dotted", title: "ยังไม่มีงานย่อย",
                      hint: "เมื่อเอเจนต์แตกงานให้ subagent (Task/Agent) จะขึ้นที่นี่พร้อมผลลัพธ์")
          } else {
          ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                ForEach(calls, id: \.call.seq) { c in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(spacing: 6) {
                            Image(systemName: c.result == nil ? "hourglass" : "checkmark.circle.fill")
                                .foregroundStyle(c.result == nil ? Color.orange : Color.green)
                            Text(c.call.actor).font(.caption.bold())
                            Spacer()
                            Text("#\(c.call.seq)").font(.caption2).foregroundStyle(.tertiary)
                        }
                        Text(c.call.body).font(.caption).lineLimit(3)
                        if let r = c.result {
                            DisclosureGroup("ผลลัพธ์") {
                                Text(r.body).font(.caption2.monospaced()).textSelection(.enabled)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }.font(.caption2)
                        }
                    }
                    .padding(10)
                    .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    .hoverHighlight(cornerRadius: 10)
                }
                SectionLabel("กิจกรรมต่อเอเจนต์", count: nil, tint: .secondary)
                ForEach(byAgent.keys.sorted(), id: \.self) { a in
                    HStack {
                        Text(a).font(.caption)
                        Spacer()
                        Text("\(byAgent[a]?.count ?? 0) tool calls").font(.caption2).foregroundStyle(.secondary)
                    }
                }
            }
            .padding(12)
          }
          }
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }

    private static func isSubagentCall(_ body: String) -> Bool {
        let t = body.lowercased()
        return ["task:", "agent:", "subagent", "delegate", "spawn"].contains { t.contains($0) }
    }

    private var dbPath: String {
        (workspace as NSString).appendingPathComponent(".chorus/store.db")
    }

    private var contextPane: some View {
        VStack(spacing: 0) {
            PaneHeader(title: "คอนเท็กซ์", subtitle: "สิ่งที่เอเจนต์ถัดไปจะได้รับ") { EmptyView() }
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    HStack(spacing: 14) {
                        let frac: Double? = {
                            guard let u = engine.contextUsed, let w = engine.contextWindow, w > 0 else { return nil }
                            return Double(u) / Double(w)
                        }()
                        ZStack {
                            Circle().stroke(Color.primary.opacity(0.08), lineWidth: 7)
                            Circle().trim(from: 0, to: frac ?? 0)
                                .stroke(frac ?? 0 > 0.8 ? Color.orange : Color.accentColor,
                                        style: StrokeStyle(lineWidth: 7, lineCap: .round))
                                .rotationEffect(.degrees(-90))
                            Text(frac.map { "\(Int($0 * 100))%" } ?? "—")
                                .font(.callout.weight(.semibold)).monospacedDigit()
                        }
                        .frame(width: 64, height: 64)
                        .animation(.easeOut, value: frac)
                        VStack(alignment: .leading, spacing: 3) {
                            if let u = engine.contextUsed, let w = engine.contextWindow {
                                Text("\(u.formatted()) / \(w.formatted())").font(.callout.monospacedDigit())
                                Text("token ในหน้าต่างของเอเจนต์").font(.caption).foregroundStyle(.secondary)
                                if (frac ?? 0) > 0.8 {
                                    Label("ใกล้เต็ม — ส่งต่องานได้", systemImage: "exclamationmark.triangle.fill")
                                        .font(.caption).foregroundStyle(.orange)
                                }
                            } else {
                                Text("ยังไม่มีเทิร์น").font(.callout)
                                Text("ตัวเลขขึ้นหลังเอเจนต์ตอบครั้งแรก").font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 12, style: .continuous))

                    Button {
                        engine.assembledContext { text, tokens in inspected = (text, tokens) }
                    } label: {
                        Label("ดูสิ่งที่เอเจนต์เห็นจริง ๆ", systemImage: "doc.text.magnifyingglass")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 8))
                    .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 8))
                    .disabled(engine.sessionID < 0)

                    // The tier the design never trims. Showing it is also the
                    // cheapest way to notice if it ever goes empty again.
                    if !engine.taskState.isEmpty {
                        SectionLabel("สถานะงาน", count: nil, tint: .secondary)
                        Text(engine.taskState)
                            .font(.caption.monospaced())
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(10)
                            .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }
                }
                .padding(10)
            }
            // The warm tier catches up after every turn; this keeps a backlog
            // visible rather than mysterious.
            HStack(spacing: 6) {
                Label("\(engine.events.count) events", systemImage: "list.bullet")
                if engine.uncompacted > 0 {
                    Text("·")
                    Text("รอสรุป \(engine.uncompacted)")
                    Spacer()
                    Button("สรุปตอนนี้") { engine.compactNow(limit: 500) }
                        .buttonStyle(InteractiveStyle(cornerRadius: 6, padding: .init(top: 2, leading: 6, bottom: 2, trailing: 6)))
                        .disabled(engine.busy)
                } else {
                    Spacer()
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .padding(.horizontal, 12).padding(.vertical, 8)
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }
}

/// Title + optional subtitle + trailing actions — the top of every inspector tab.
struct PaneHeader<Trailing: View>: View {
    let title: String
    var subtitle: String?
    @ViewBuilder var trailing: () -> Trailing

    var body: some View {
        HStack(alignment: .center) {
            VStack(alignment: .leading, spacing: 1) {
                Text(title).font(.headline).lineLimit(1)
                if let subtitle { Text(subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(1) }
            }
            Spacer(minLength: 4)
            trailing()
        }
        .padding(.horizontal, 12).padding(.top, 10).padding(.bottom, 6)
    }
}

struct SectionLabel: View {
    let text: String
    let count: Int?
    let tint: Color
    init(_ text: String, count: Int?, tint: Color) { self.text = text; self.count = count; self.tint = tint }

    var body: some View {
        HStack(spacing: 5) {
            Text(text).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            if let count {
                Text("\(count)").font(.caption2.weight(.bold)).foregroundStyle(tint)
                    .padding(.horizontal, 5).padding(.vertical, 1)
                    .background(tint.opacity(0.15), in: Capsule())
            }
        }
        .padding(.top, 4)
    }
}

/// An empty tab that says what will appear and how it gets there.
struct EmptyPane: View {
    let icon: String
    let title: String
    let hint: String

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: icon).font(.system(size: 28)).foregroundStyle(.tertiary)
            Text(title).font(.callout.weight(.medium))
            Text(hint).font(.caption).foregroundStyle(.secondary).multilineTextAlignment(.center)
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// One file's hunks, collapsible, each hunk with its own keep/reject pill.
struct FileChanges: View {
    let file: String
    let hunks: [DiffHunk]
    @Binding var rejected: Set<String>
    @State private var open = true

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button { withAnimation(.easeOut(duration: 0.15)) { open.toggle() } } label: {
                HStack(spacing: 6) {
                    Image(systemName: "chevron.right").font(.system(size: 9, weight: .bold))
                        .rotationEffect(.degrees(open ? 90 : 0)).foregroundStyle(.secondary)
                    Image(systemName: "doc.text").foregroundStyle(.secondary)
                    Text((file as NSString).lastPathComponent).font(.callout.weight(.medium)).lineLimit(1)
                    Spacer(minLength: 4)
                    Text("+\(hunks.map(\.addCount).reduce(0, +))").foregroundStyle(.green)
                    Text("−\(hunks.map(\.delCount).reduce(0, +))").foregroundStyle(.red)
                }
                .font(.caption.monospacedDigit())
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(InteractiveStyle(cornerRadius: 7, padding: .init(top: 5, leading: 6, bottom: 5, trailing: 6)))
            .help(file)
            if open {
                ForEach(hunks) { h in
                    HunkView(hunk: h, rejected: rejected.contains(h.id)) {
                        if rejected.contains(h.id) { rejected.remove(h.id) } else { rejected.insert(h.id) }
                    }
                }
            }
        }
        .padding(6)
        .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}

struct EventRow: View {
    let event: LedgerEvent

    private var isUser: Bool { event.actor == "user" }

    var body: some View {
        switch event.kind {
        case "msg":
            message
        case "handoff":
            if let b = HandoffBrief(json: event.body) {
                card(tint: .purple) { handoffBody(b) }
            } else {
                note(event.body)
            }
        case "tool_call", "tool_result", "diff":
            toolRow
        default:
            note(event.body)
        }
    }

    // User: right-aligned bubble. Agent: plain text under its name, like the
    // chat panes people already know from Claude / Codex / Cursor.
    private var message: some View {
        VStack(alignment: isUser ? .trailing : .leading, spacing: 4) {
            if !isUser {
                HStack(spacing: 5) {
                    Circle().fill(agentColor).frame(width: 7, height: 7)
                    Text(event.actor).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                }
            }
            MarkdownText(text: event.body)
                .padding(.horizontal, isUser ? 12 : 0).padding(.vertical, isUser ? 8 : 0)
                .background(isUser ? Color.accentColor.opacity(0.14) : .clear,
                            in: RoundedRectangle(cornerRadius: 12))
        }
        .frame(maxWidth: .infinity, alignment: isUser ? .trailing : .leading)
    }

    /// One collapsed line per tool call; click to see the payload.
    private var toolRow: some View {
        DisclosureGroup {
            Text(event.body)
                .font(.caption2.monospaced())
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, 4)
        } label: {
            HStack(spacing: 6) {
                Image(systemName: event.kind == "diff" ? "plusminus" : "wrench.and.screwdriver")
                    .font(.caption2).foregroundStyle(.secondary)
                Text(firstLine).font(.caption).lineLimit(1).foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, 8).padding(.vertical, 5)
        .background(.secondary.opacity(0.07), in: RoundedRectangle(cornerRadius: 8))
        .hoverHighlight(cornerRadius: 8)
    }

    private var firstLine: String {
        let l = event.body.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: true).first.map(String.init) ?? event.kind
        return l.count > 110 ? String(l.prefix(110)) + "…" : l
    }

    private func note(_ t: String) -> some View {
        Text(t).font(.caption).foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func card<C: View>(tint: Color, @ViewBuilder _ c: () -> C) -> some View {
        c().padding(10)
            .background(tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 10))
    }

    private var agentColor: Color {
        let palette: [Color] = [.green, .orange, .teal, .pink, .indigo]
        return palette[abs(event.actor.hashValue) % palette.count]
    }

    /// The handoff is the product's whole argument, so it gets read as a
    /// moment — who fell over, why, and what the next agent was told — rather
    /// than as the brief's raw JSON.
    @ViewBuilder
    private func handoffBody(_ b: HandoffBrief) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 6) {
                Text(b.from).font(.caption.bold())
                Image(systemName: "arrow.right").font(.caption2)
                Text(b.to).font(.caption.bold())
                Text(b.reason)
                    .font(.caption2)
                    .padding(.horizontal, 5).padding(.vertical, 1)
                    .background(.orange.opacity(0.25), in: Capsule())
            }
            if !b.goal.isEmpty {
                Text("goal · \(b.goal)").font(.caption)
            }
            HStack(spacing: 10) {
                if !b.done.isEmpty { Text("done \(b.done.count)") }
                if !b.pending.isEmpty { Text("pending \(b.pending.count)") }
                if !b.files.isEmpty { Text("files \(b.files.count)") }
                if b.diffBytes > 0 { Text("diff \(b.diffBytes)B") }
            }
            .font(.caption2).foregroundStyle(.secondary)
            DisclosureGroup("ดู brief ที่ส่งต่อ") {
                Text(event.body)
                    .font(.caption2.monospaced())
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .font(.caption2)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

}

struct CompareResultsSheet: View {
    let json: String
    @Environment(\.dismiss) private var dismiss

    struct Branch: Codable, Identifiable {
        let agent: String
        let session_id: Int64
        let message: String
        let stop_reason: String
        let error: String?
        var id: Int64 { session_id }
    }
    struct Payload: Codable { let ok: Bool?; let branches: [Branch]? ; let error: String? }

    private var payload: Payload? {
        guard let d = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(Payload.self, from: d)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Compare").font(.title3.bold())
            if let err = payload?.error {
                Text(err).foregroundStyle(.red)
            }
            ScrollView {
                HStack(alignment: .top, spacing: 12) {
                    ForEach(payload?.branches ?? []) { b in
                        VStack(alignment: .leading, spacing: 6) {
                            Text(b.agent).font(.headline)
                            Text("resume: --session \(b.session_id)")
                                .font(.caption2).foregroundStyle(.secondary)
                            Divider()
                            if let e = b.error {
                                Text(e).font(.caption).foregroundStyle(.red)
                            } else {
                                Text(b.message).font(.callout).textSelection(.enabled)
                                Text("[\(b.stop_reason)]").font(.caption2).foregroundStyle(.tertiary)
                            }
                        }
                        .frame(width: 260, alignment: .leading)
                        .padding(10)
                        .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 8))
                    }
                }
            }
            HStack { Spacer(); Button("Done") { dismiss() }.keyboardShortcut(.defaultAction) }
        }
        .padding(20)
        .frame(minWidth: 560, minHeight: 320)
    }
}

/// A minimal unified-diff model: each hunk carries the file preamble it
/// needs to be a valid standalone patch.
struct DiffHunk: Identifiable {
    /// Stable across re-parses: the diff is re-parsed on every render, and a
    /// fresh UUID each time silently dropped every keep/reject the user set.
    var id: String { file + "|" + header }
    let file: String
    let preamble: String   // "diff --git … / --- … / +++ …"
    let header: String     // "@@ -a,b +c,d @@ …"
    let lines: [String]    // body lines (context / +/-)

    var patch: String { preamble + header + "\n" + lines.joined(separator: "\n") + "\n" }
    var addCount: Int { lines.filter { $0.hasPrefix("+") }.count }
    var delCount: Int { lines.filter { $0.hasPrefix("-") }.count }
}

func parseHunks(_ diff: String) -> [DiffHunk] {
    var hunks: [DiffHunk] = []
    var file = ""
    var preamble = ""
    var inPreamble = false
    var curHeader: String?
    var curLines: [String] = []

    func flush() {
        if let h = curHeader {
            hunks.append(DiffHunk(file: file, preamble: preamble, header: h, lines: curLines))
        }
        curHeader = nil
        curLines = []
    }

    for line in diff.split(separator: "\n", omittingEmptySubsequences: false).map(String.init) {
        if line.hasPrefix("diff --git") {
            flush()
            preamble = line + "\n"
            inPreamble = true
            if let r = line.range(of: " b/") { file = String(line[r.upperBound...]) }
        } else if inPreamble && (line.hasPrefix("index ") || line.hasPrefix("--- ") || line.hasPrefix("+++ ")
                                 || line.hasPrefix("new file") || line.hasPrefix("deleted file")
                                 || line.hasPrefix("similarity") || line.hasPrefix("rename ")) {
            preamble += line + "\n"
        } else if line.hasPrefix("@@") {
            flush()
            inPreamble = false
            curHeader = line
        } else if curHeader != nil {
            curLines.append(line)
        }
    }
    flush()
    return hunks
}

struct HunkView: View {
    let hunk: DiffHunk
    let rejected: Bool
    let toggle: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(hunk.header.components(separatedBy: "@@").dropFirst().first?.trimmingCharacters(in: .whitespaces) ?? "")
                    .font(.caption2.monospaced()).foregroundStyle(.tertiary).lineLimit(1)
                Spacer()
                Button(action: toggle) {
                    Label(rejected ? "ตีกลับ" : "เก็บ", systemImage: rejected ? "xmark" : "checkmark")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(rejected ? .red : .green)
                }
                .buttonStyle(InteractiveStyle(cornerRadius: 99, selected: rejected,
                                              padding: .init(top: 2, leading: 7, bottom: 2, trailing: 7)))
                .help(rejected ? "กดอีกครั้งเพื่อเก็บไว้" : "กดเพื่อเลือกตีกลับก้อนนี้")
            }
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(hunk.lines.prefix(60).enumerated()), id: \.offset) { _, l in
                    Text(l.isEmpty ? " " : l).font(.system(size: 10.5, design: .monospaced))
                        .foregroundStyle(l.hasPrefix("+") ? Color.green : l.hasPrefix("-") ? .red : .secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 6)
                        .background(l.hasPrefix("+") ? Color.green.opacity(0.08)
                                    : l.hasPrefix("-") ? Color.red.opacity(0.08) : .clear)
                }
                if hunk.lines.count > 60 {
                    Text("… อีก \(hunk.lines.count - 60) บรรทัด").font(.caption2).foregroundStyle(.tertiary).padding(6)
                }
            }
            .padding(.vertical, 4)
            .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 6))
            .opacity(rejected ? 0.4 : 1)
            .strikethrough(rejected)
        }
        .padding(.horizontal, 4)
    }
}

/// Just enough of the handoff brief to render the moment it describes.
struct HandoffBrief {
    let goal: String
    let done: [String]
    let pending: [String]
    let files: [String]
    let diffBytes: Int
    let from: String
    let to: String
    let reason: String

    init?(json: String) {
        guard let data = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        let meta = obj["handoff"] as? [String: Any] ?? [:]
        goal = obj["goal"] as? String ?? ""
        done = obj["done"] as? [String] ?? []
        pending = obj["pending"] as? [String] ?? []
        files = obj["files"] as? [String] ?? []
        diffBytes = (obj["diff"] as? String)?.utf8.count ?? 0
        from = meta["from"] as? String ?? "?"
        to = meta["to"] as? String ?? "?"
        reason = (meta["reason"] as? String ?? "handoff").replacingOccurrences(of: "_", with: " ")
    }
}

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


/// Markdown for agent output: fenced code becomes a monospaced block, the rest
/// goes through `AttributedString` (bold, italic, inline code, links) with
/// whitespace preserved so lists and paragraphs keep their shape.
struct MarkdownText: View {
    let text: String

    private var parts: [(code: Bool, body: String)] {
        var out: [(Bool, String)] = []
        for (i, chunk) in text.components(separatedBy: "```").enumerated() {
            guard !chunk.isEmpty else { continue }
            if i % 2 == 1 {
                // Drop the language tag on the first line.
                let lines = chunk.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: false)
                out.append((true, lines.count > 1 ? String(lines[1]) : chunk))
            } else {
                out.append((false, chunk.trimmingCharacters(in: .newlines)))
            }
        }
        return out
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(parts.enumerated()), id: \.offset) { _, p in
                if p.code {
                    ScrollView(.horizontal) {
                        Text(p.body.trimmingCharacters(in: .newlines))
                            .font(.callout.monospaced()).textSelection(.enabled)
                            .padding(10)
                    }
                    .background(.secondary.opacity(0.12), in: RoundedRectangle(cornerRadius: 8))
                } else if let a = try? AttributedString(
                    markdown: p.body,
                    options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)) {
                    Text(a).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                } else {
                    Text(p.body).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
    }
}



/// Collapsible reasoning. Open while the agent is still thinking, collapsed
/// to one line once it starts answering.
struct ThinkingBlock: View {
    let text: String
    let running: Bool
    @State private var open: Bool?
    @AppStorage("chorus.thinkingOpen") private var thinkingOpen = true

    var body: some View {
        let isOpen = open ?? (running && thinkingOpen)
        VStack(alignment: .leading, spacing: 6) {
            Button { open = !isOpen } label: {
                HStack(spacing: 6) {
                    Image(systemName: "brain").foregroundStyle(.purple)
                    Text(running ? "กำลังคิด…" : "ความคิด").font(.caption.weight(.semibold))
                    Text("\(text.count) ตัวอักษร").font(.caption2).foregroundStyle(.tertiary)
                    Image(systemName: isOpen ? "chevron.up" : "chevron.down").font(.caption2)
                    Spacer()
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if isOpen {
                ScrollViewReader { proxy in
                    ScrollView {
                        Text(text)
                            .font(.caption).foregroundStyle(.secondary)
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Color.clear.frame(height: 1).id("end")
                    }
                    .frame(maxHeight: 160)
                    .onChange(of: text.count) { _, _ in if running { proxy.scrollTo("end") } }
                }
            }
        }
        .padding(8)
        .background(Color.purple.opacity(0.06), in: RoundedRectangle(cornerRadius: 8))
    }
}


extension View {
    /// A floating panel: Liquid Glass on macOS 26+, a material card with a
    /// soft shadow before that.
    @ViewBuilder
    func floatingPanel(cornerRadius: CGFloat, tint: Color? = nil) -> some View {
        if #available(macOS 26, *) {
            self.glassEffect(tint.map { Glass.regular.tint($0.opacity(0.25)) } ?? .regular,
                             in: .rect(cornerRadius: cornerRadius))
        } else {
            self
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: cornerRadius))
                .overlay(RoundedRectangle(cornerRadius: cornerRadius).stroke(.quaternary))
                .shadow(color: .black.opacity(0.12), radius: 12, y: 4)
        }
    }
}


/// What the glass panels float over: the window colour with a faint wash of
/// the accent, so the panels read as separate layers.
struct Backdrop: View {
    var body: some View {
        ZStack {
            Color(nsColor: .windowBackgroundColor)
            LinearGradient(colors: [Color.accentColor.opacity(0.10), .clear, Color.purple.opacity(0.06)],
                           startPoint: .topLeading, endPoint: .bottomTrailing)
        }
        .ignoresSafeArea()
    }
}
