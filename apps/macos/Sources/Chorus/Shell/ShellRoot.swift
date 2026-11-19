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

    var body: some View {
        NavigationSplitView(columnVisibility: $columns) {
            ShellSidebar(projects: shell.projects) { shell.showSettings = true }
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
        .onChange(of: engine.sessionID) { _, id in shell.sessionChanged(id); syncCommands() }
        .onChange(of: engine.busy) { _, busy in shell.busyChanged(busy); syncCommands() }
        .onChange(of: engine.draft != nil) { _, _ in syncCommands() }
        .onChange(of: engine.agents) { _, _ in shell.syncModel(); engine.refreshModels() }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            engine.loadAgents()
        }
        .onReceive(AppCommandCenter.shared.fire) { handle($0) }
        .sheet(isPresented: $shell.showSettings) {
            SettingsSheet(workspace: shell.workspace).environmentObject(engine)
        }
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
        case .settings: shell.showSettings = true
        case .toggleSidebar:
            columns = columns == .detailOnly ? .all : .detailOnly
        case .toggleInspector: shell.showInspector.toggle()
        case .inspectorTab(let i):
            if InspectorTab.allCases.indices.contains(i) { shell.inspectorTab = InspectorTab.allCases[i] }
            shell.showInspector = true
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
