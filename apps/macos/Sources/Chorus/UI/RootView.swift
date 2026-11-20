import AppKit
import SwiftUI

// The main window (design board "Session"): projects on the left, the stage in
// the middle (conversation · workbench · Orchestra), an inspector on the right.
// Native structure — NavigationSplitView, a unified toolbar with a segmented
// control, `.inspector` — drawn in the Chorus palette. Every decision lives in
// `ShellModel` → `Engine` → Rust; this file only lays things out and routes
// menu commands.

struct RootView: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @State private var columns: NavigationSplitViewVisibility = .all
    @State private var tool: ShellTool?
    @Environment(\.openSettings) private var openSettingsWindow

    var body: some View {
        NavigationSplitView(columnVisibility: $columns) {
            SidebarView(projects: shell.projects)
                .navigationSplitViewColumnWidth(min: 230, ideal: 252, max: 360)
        } detail: {
            VStack(spacing: 0) {
                stage
                if shell.showTerminal, !shell.workspace.isEmpty {
                    Hairline()
                    TerminalDrawer(workspace: shell.workspace)
                        .frame(height: 220)
                        .transition(.move(edge: .bottom))
                }
            }
            .pane(Palette.window)
            .inspector(isPresented: $shell.showInspector) {
                InspectorView()
                    .inspectorColumnWidth(min: 300, ideal: 330, max: 520)
            }
        }
        .navigationTitle(title)
        .navigationSubtitle(subtitle)
        .toolbar { toolbar }
        .frame(minWidth: 900, minHeight: 600)
        .background(Palette.window)
        .tint(Palette.accent)
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
            if !shell.workspace.isEmpty && (engine.busy || engine.sessionID >= 0) {
                engine.loadSubagents(db: shell.currentDB)
            }
        }
        .onReceive(AppCommandCenter.shared.fire) { handle($0) }
        .overlay {
            if shell.showPalette {
                CommandPalette { shell.showPalette = false } openTool: { tool = $0 }
            }
        }
        .sheet(isPresented: $shell.showHandoff) { HandoffSheet() }
        .sheet(isPresented: $shell.showSearch) { SearchSheet() }
        .sheet(isPresented: $shell.confirmCompare) { CompareConfirmSheet() }
        .sheet(isPresented: Binding(get: { shell.compareOutput != nil }, set: { if !$0 { shell.compareOutput = nil } })) {
            CompareSheet(json: shell.compareOutput ?? "")
        }
        .sheet(item: $shell.pendingRoute) { RouteReviewSheet(proposal: $0) }
        .sheet(item: $tool) { ToolWindow(tool: $0) }
        .sheet(item: $engine.pendingSecret) { SecretSheet(request: $0) }
        .confirmationDialog(
            "ลบ session นี้?",
            isPresented: Binding(get: { shell.pendingDelete != nil }, set: { if !$0 { shell.pendingDelete = nil } }),
            presenting: shell.pendingDelete
        ) { s in
            Button("ลบถาวร", role: .destructive) {
                engine.deleteSession(db: shell.currentDB, id: s.id, isOpen: s.id == engine.sessionID)
                shell.pendingDelete = nil
            }
            Button("ยกเลิก", role: .cancel) { shell.pendingDelete = nil }
        } message: { s in
            Text("“\(shell.title(of: s))” และประวัติทั้งหมดจะหายถาวร ไฟล์ในโฟลเดอร์ไม่ถูกแตะ")
        }
    }

    // MARK: stage

    @ViewBuilder
    private var stage: some View {
        if shell.projects.projects.isEmpty {
            WelcomeView()
        } else {
            switch shell.stage {
            case .conversation: ConversationView()
            case .workbench: WorkbenchView()
            case .orchestra: OrchestraView()
            }
        }
    }

    /// Buttons anywhere ask for Settings through the command center; the root
    /// opens it with `openSettings`. The old `showSettingsWindow:` selector is
    /// ignored on macOS 14+ (Apple only honours SettingsLink/openSettings).
    static func openSettings() {
        AppCommandCenter.shared.send(.settings)
    }

    // MARK: title and toolbar

    private var title: String {
        if engine.sessionID < 0 { return engine.draft != nil ? "session ใหม่" : "Chorus" }
        return shell.sessions(in: shell.workspace).first { $0.id == engine.sessionID }.map(shell.title(of:))
            ?? "session \(engine.sessionID)"
    }

    /// "chorus · 3 ไฟล์ +48 −6" — the folder and what this session changed.
    private var subtitle: String {
        guard !shell.workspace.isEmpty else { return "" }
        var parts = [(shell.workspace as NSString).lastPathComponent]
        if !engine.diffFiles.isEmpty {
            let add = engine.diffHunks.reduce(0) { $0 + $1.addCount }
            let del = engine.diffHunks.reduce(0) { $0 + $1.delCount }
            parts.append("\(engine.diffFiles.count) ไฟล์ +\(add) −\(del)")
        }
        return parts.joined(separator: " · ")
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItem(placement: .principal) {
            Picker("มุมมอง", selection: $shell.stage) {
                ForEach(Stage.allCases) { Text($0.title).tag($0) }
            }
            .pickerStyle(.segmented)
            .fixedSize()
            .help("สนทนา · โต๊ะงาน (⌘B) · Orchestra (⌥⌘O)")
            .disabled(shell.projects.projects.isEmpty)
        }
        ToolbarItemGroup(placement: .primaryAction) {
            Button { shell.showHandoff = true } label: {
                Label("ส่งไม้", systemImage: "arrow.left.arrow.right")
            }
            .help("ส่งงานต่อให้ agent อื่น พร้อมทุกอย่างที่คุยมา")
            .disabled(engine.sessionID < 0 || engine.agents.filter(\.ready).count < 2)

            Button { shell.runCompare() } label: {
                Label("เทียบ agent", systemImage: "rectangle.split.2x1")
            }
            .help("ส่งคำถามเดียวกันให้ทุก agent แล้วเทียบ (⇧⌘K)")
            .disabled(engine.agents.filter(\.ready).count < 2 || !shell.canCompare || shell.comparing)

            Button { withAnimation(.snappy) { shell.showTerminal.toggle() } } label: {
                Label("เทอร์มินัล", systemImage: "terminal")
            }
            .help("เทอร์มินัล (⌃`)")
            .disabled(shell.workspace.isEmpty)

            Menu {
                ForEach([ShellTool.ship, .search, .monitor]) { t in
                    Button { open(t) } label: { Label(t.title, systemImage: t.symbol) }
                }
                Divider()
                Button { ShellRoot.openSettingsCompat() } label: { Label("ตั้งค่า…", systemImage: "gearshape") }
            } label: {
                Label("เพิ่มเติม", systemImage: "ellipsis.circle")
            }
            .help("Commit & PR · ค้นทุก session · System Monitor")

            Button { shell.showInspector.toggle() } label: {
                Label("แผงข้อมูล", systemImage: "sidebar.right")
                    .overlay(alignment: .topTrailing) {
                        if !shell.showInspector && (engine.diffFiles.count + engine.pending.count) > 0 {
                            Circle().fill(Palette.accent).frame(width: 7, height: 7).offset(x: 3, y: -2)
                        }
                    }
            }
            .help("แผงข้อมูล: งาน คอนเท็กซ์ ที่แก้ ความจำ ย้อน (⌥⌘I)")
        }
    }

    private func open(_ t: ShellTool) {
        if t == .search { shell.showSearch = true } else { tool = t }
    }

    private func syncCommands() {
        let c = AppCommandCenter.shared
        c.hasSession = engine.sessionID >= 0 || engine.draft != nil
        c.busy = engine.busy
    }

    private func toggle(_ s: Stage) { shell.stage = shell.stage == s ? .conversation : s }

    private func handle(_ c: AppCommand) {
        switch c {
        case .newSession: shell.newSession()
        case .settings: openSettingsWindow()
        case .toggleSidebar: columns = columns == .detailOnly ? .all : .detailOnly
        case .toggleInspector: shell.showInspector.toggle()
        case .inspectorTab(let i):
            if InspectorTab.allCases.indices.contains(i) { shell.inspectorTab = InspectorTab.allCases[i] }
            shell.showInspector = true
        case .toggleExplorer: shell.stage = .workbench; shell.workbenchTab = .files
        case .toggleTerminal: withAnimation(.snappy) { shell.showTerminal.toggle() }
        case .toggleLivePreview: shell.stage = .workbench; shell.workbenchTab = .preview
        case .toggleOrchestrator: toggle(.orchestra)
        case .searchSessions: shell.showSearch = true
        case .systemMonitor: tool = .monitor
        case .commandPalette: shell.showPalette.toggle()
        case .compareAgents: shell.runCompare()
        case .stop: shell.stop()
        case .send: shell.send()
        default: break
        }
    }
}

/// Kept so model code that predates the redesign can still open Settings.
enum ShellRoot {
    static func openSettings() { RootView.openSettings() }
    static func openSettingsCompat() { RootView.openSettings() }
}

/// Things that open in their own sheet (not a stage).
enum ShellTool: String, Identifiable {
    case ship, terminal, search, monitor
    var id: String { rawValue }

    var title: String {
        switch self {
        case .ship: "Commit & PR"
        case .terminal: "เทอร์มินัล"
        case .search: "ค้นทุก session"
        case .monitor: "System Monitor"
        }
    }

    var symbol: String {
        switch self {
        case .ship: "arrow.triangle.branch"
        case .terminal: "terminal"
        case .search: "magnifyingglass"
        case .monitor: "gauge.with.dots.needle.50percent"
        }
    }
}

/// A sheet hosting one tool, with the design's header (title + close).
struct ToolWindow: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @Environment(\.dismiss) private var dismiss
    let tool: ShellTool

    var body: some View {
        switch tool {
        case .ship:
            ShipSheet()
        case .terminal:
            VStack(spacing: 0) {
                SheetHeader(title: tool.title) { dismiss() }
                TerminalDrawer(workspace: shell.workspace)
            }
            .frame(minWidth: 760, minHeight: 480)
        case .search:
            SearchSheet()
        case .monitor:
            VStack(spacing: 0) {
                SheetHeader(title: tool.title) { dismiss() }
                MonitorTool()
            }
            .frame(minWidth: 900, idealWidth: 1000, minHeight: 600, idealHeight: 680)
            .background(Palette.window)
        }
    }
}

/// Title + subtitle + close button, the top of every sheet in the design.
struct SheetHeader: View {
    let title: String
    var subtitle: String?
    let close: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.system(size: 18, weight: .semibold))
                if let subtitle { Text(subtitle).font(Typo.small).foregroundStyle(Palette.text3) }
            }
            Spacer()
            Button(action: close) { Image(systemName: "xmark") }
                .buttonStyle(IconButtonStyle())
                .keyboardShortcut(.cancelAction)
                .accessibilityLabel("ปิด")
        }
        .padding(.horizontal, 20).padding(.top, 18).padding(.bottom, 12)
    }
}
