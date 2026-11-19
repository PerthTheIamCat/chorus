import AppKit
import SwiftUI

// The v0.4 window: sidebar, conversation, and an inspector that is closed
// until asked for. Standard macOS structure on purpose: `NavigationSplitView`
// for the sidebar, a unified toolbar, `.inspector` for the side panel,
// materials and SF Symbols. Nothing here decides anything about a turn; that
// is `ShellModel` calling `Engine` calling Rust.

struct ShellRoot: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @State private var columns: NavigationSplitViewVisibility = .all
    @State private var tool: ShellTool?

    var body: some View {
        NavigationSplitView(columnVisibility: $columns) {
            ShellSidebar(projects: shell.projects) { ShellRoot.openSettings() }
                .navigationSplitViewColumnWidth(min: 220, ideal: 260, max: 380)
        } detail: {
            Conversation()
                .inspector(isPresented: $shell.showInspector) {
                    ShellInspector()
                        .inspectorColumnWidth(min: 300, ideal: 360, max: 560)
                }
        }
        .navigationTitle(title)
        .navigationSubtitle(subtitle)
        .toolbar { toolbar }
        .frame(minWidth: 820, minHeight: 560)
        .onAppear {
            shell.start()
            syncCommands()
            NSApp.setActivationPolicy(.regular)
            NSApp.activate(ignoringOtherApps: true)
        }
        .onChange(of: shell.workspace) { _, w in shell.fileIndex.load(w) }
        .onChange(of: engine.sessionID) { _, id in shell.sessionChanged(id); syncCommands() }
        .onChange(of: engine.busy) { _, busy in shell.busyChanged(busy); syncCommands() }
        .onChange(of: engine.draft != nil) { _, _ in syncCommands() }
        .onChange(of: engine.agents) { _, _ in shell.syncModel(); engine.refreshModels() }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            engine.loadAgents()
        }
        .onReceive(Timer.publish(every: 4, on: .main, in: .common).autoconnect()) { _ in
            // Agents a lead started change state on their own.
            if !shell.workspace.isEmpty && (engine.busy || engine.sessionID >= 0) {
                engine.loadSubagents(db: shell.currentDB)
            }
        }
        .onReceive(AppCommandCenter.shared.fire) { handle($0) }
        .sheet(isPresented: $shell.showPalette) {
            ShellPalette(items: shell.paletteItems { tool = $0 }) { shell.showPalette = false }
        }
        .sheet(isPresented: $shell.confirmCompare) { CompareConfirm().environmentObject(engine).environmentObject(shell) }
        .sheet(isPresented: Binding(get: { shell.compareOutput != nil }, set: { if !$0 { shell.compareOutput = nil } })) {
            CompareResultsSheet(json: shell.compareOutput ?? "")
        }
        .sheet(item: $shell.pendingRoute) { RouteReview(proposal: $0).environmentObject(engine).environmentObject(shell) }
        .sheet(item: $tool) { ToolSheet(tool: $0).environmentObject(engine).environmentObject(shell) }
        .sheet(item: $engine.pendingSecret) { SecretSheet(request: $0) }
        .confirmationDialog(
            "ลบ session นี้?",
            isPresented: Binding(get: { shell.pendingDelete != nil }, set: { if !$0 { shell.pendingDelete = nil } }),
            presenting: shell.pendingDelete
        ) { s in
            Button("ลบถาวร", role: .destructive) {
                let open = s.id == engine.sessionID
                engine.deleteSession(db: shell.currentDB, id: s.id, isOpen: open)
                shell.pendingDelete = nil
            }
            Button("ยกเลิก", role: .cancel) { shell.pendingDelete = nil }
        } message: { s in
            Text("“\(shell.title(of: s))” และประวัติทั้งหมดจะหายถาวร ไฟล์ในโฟลเดอร์ไม่ถูกแตะ")
        }
    }

    static func openSettings() {
        NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
    }

    // MARK: title and toolbar

    private var title: String {
        if engine.sessionID < 0 {
            return engine.draft != nil ? "session ใหม่" : "Chorus"
        }
        return shell.sessions(in: shell.workspace).first { $0.id == engine.sessionID }.map(shell.title(of:))
            ?? "session \(engine.sessionID)"
    }

    private var subtitle: String {
        shell.workspace.isEmpty ? "" : (shell.workspace as NSString).lastPathComponent
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItem(placement: .primaryAction) {
            Button { shell.newSession() } label: { Label("session ใหม่", systemImage: "square.and.pencil") }
                .help("session ใหม่ (⌘N)")
                .disabled(engine.busy)
        }
        ToolbarItem(placement: .primaryAction) {
            Menu {
                ForEach([ShellTool.files, .terminal, .preview, .ship, .checkpoints, .search, .orchestra, .monitor]) { t in
                    Button { tool = t } label: { Label(t.title, systemImage: t.symbol) }
                }
            } label: { Label("เครื่องมือ", systemImage: "wrench.and.screwdriver") }
            .disabled(shell.workspace.isEmpty)
            .help("ไฟล์ เทอร์มินัล commit/PR checkpoint ค้นแชท Orchestra")
        }
        ToolbarItem(placement: .primaryAction) {
            Button { shell.showInspector.toggle() } label: {
                Label("แผงข้อมูล", systemImage: "sidebar.right")
                    .overlay(alignment: .topTrailing) {
                        if !shell.showInspector && (engine.diffFiles.count + engine.pending.count) > 0 {
                            Circle().fill(Color.accentColor).frame(width: 7, height: 7).offset(x: 3, y: -2)
                        }
                    }
            }
            .help("แผงข้อมูล: คอนเท็กซ์ ที่แก้ ความจำ (⌥⌘I)")
        }
    }

    /// The menu bar enables Send / Stop from this.
    private func syncCommands() {
        let c = AppCommandCenter.shared
        c.hasSession = engine.sessionID >= 0 || engine.draft != nil
        c.busy = engine.busy
    }

    private func handle(_ c: AppCommand) {
        switch c {
        case .newSession: shell.newSession()
        case .settings: ShellRoot.openSettings()
        case .toggleSidebar:
            columns = columns == .detailOnly ? .all : .detailOnly
        case .toggleInspector: shell.showInspector.toggle()
        case .inspectorTab(let i):
            if InspectorTab.allCases.indices.contains(i) { shell.inspectorTab = InspectorTab.allCases[i] }
            shell.showInspector = true
        case .toggleExplorer: tool = .files
        case .toggleTerminal: tool = .terminal
        case .toggleLivePreview: tool = .preview
        case .toggleOrchestrator: tool = .orchestra
        case .searchSessions: tool = .search
        case .systemMonitor: tool = .monitor
        case .commandPalette: shell.showPalette = true
        case .compareAgents: shell.runCompare()
        case .stop: shell.stop()
        case .send: shell.send()
        default: break
        }
    }
}

/// The middle of the window: the conversation, then what the person can do
/// next. Everything that needs an answer sits right above the field.
private struct Conversation: View {
    var body: some View {
        VStack(spacing: 0) {
            ShellTranscript()
            ShellComposer()
        }
        .background(Color(nsColor: .textBackgroundColor))
    }
}


// MARK: sheets

/// ⌘K: type to filter, ↩ runs the first match.
private struct ShellPalette: View {
    let items: [ShellModel.PaletteItem]
    let dismiss: () -> Void
    @State private var query = ""
    @FocusState private var focused: Bool

    private var shown: [ShellModel.PaletteItem] {
        query.isEmpty ? items : items.filter { $0.title.localizedCaseInsensitiveContains(query) }
    }

    var body: some View {
        VStack(spacing: 0) {
            TextField("พิมพ์คำสั่ง…", text: $query)
                .textFieldStyle(.plain).font(.title3).padding(14)
                .focused($focused)
                .onSubmit { if let f = shown.first { dismiss(); f.run() } }
            Divider()
            List(shown) { item in
                Button { dismiss(); item.run() } label: {
                    Label(item.title, systemImage: item.icon).frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(.plain)
            }
            .listStyle(.plain)
        }
        .frame(width: 460, height: 340)
        .onAppear { focused = true }
    }
}

private struct CompareConfirm: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        let agents = engine.agents.filter(\.ready)
        let source = shell.comparePrompt
        VStack(alignment: .leading, spacing: 14) {
            Label("เทียบ agent", systemImage: "square.split.2x1").font(.title3.bold())
            Text("ส่งคำถามเดียวกันให้ทุก agent ที่พร้อม แยกกันทำใน fork ของตัวเอง แล้วเทียบคำตอบ ไฟล์ที่แก้ เวลา และค่าใช้จ่าย แต่ละตัวใช้โควตาของตัวเองหนึ่งเทิร์น")
                .font(.callout).foregroundStyle(.secondary)
            GroupBox(source.fromComposer ? "คำถาม (จากช่องพิมพ์ จะถูกล้างหลังส่ง)" : "คำถาม (ข้อความล่าสุดที่ส่ง)") {
                Text(source.text).lineLimit(5).font(.callout)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            Text("agent ที่จะถูกเรียก (\(agents.count)): " + agents.map(\.label).joined(separator: " · "))
                .font(.callout)
            Toggle("ตัดสินด้วยหลักฐาน: รันเช็คและเทสต์ในแต่ละสำเนา แล้วให้อีกเจ้าจัดอันดับ", isOn: $shell.compareJudge)
                .help("ใช้เวลารันเทสต์และโควตาของผู้ตัดสินอีกหนึ่งเทิร์น เปิดได้เมื่อโปรเจกต์ trust แล้ว คุณเลือกผู้ชนะเองเสมอ")
            Toggle("เทียบ agent เดี่ยว กับ orchestra ของ agent เดียวกัน", isOn: $shell.compareOrchestra)
            HStack {
                Button("ยกเลิก") { shell.confirmCompare = false }.keyboardShortcut(.cancelAction)
                Spacer()
                Button("เทียบเลย") { shell.confirmCompare = false; shell.performCompare() }
                    .buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 460)
    }
}

/// Auto route could not decide: the person picks the model and confirms.
private struct RouteReview: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    let proposal: ShellModel.PendingRoute

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Label("เลือกโมเดลก่อนส่ง", systemImage: "arrow.triangle.branch").font(.title3.bold())
            Text(proposal.reason).font(.callout).foregroundStyle(.secondary)
            Text(proposal.send.text).lineLimit(4).font(.callout)
                .padding(8).frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            Picker("โมเดล", selection: $shell.reviewModel) {
                ForEach(engine.agents.filter(\.ready)) { a in
                    Section(a.label) { ForEach(a.models, id: \.id) { m in Text(m.label).tag(m.id) } }
                }
            }
            HStack {
                Button("ยกเลิก") { shell.cancelReviewedRoute() }.keyboardShortcut(.cancelAction)
                Spacer()
                Button("ส่ง") { shell.sendReviewedRoute() }
                    .buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                    .disabled(shell.agent(for: shell.reviewModel)?.ready != true)
            }
        }
        .padding(20)
        .frame(width: 460)
    }
}
