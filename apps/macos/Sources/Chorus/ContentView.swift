import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

struct SubagentLedgerPayload: Decodable {
    let event: String
    let sessionId: String
    let parentSessionId: String?
    let name: String?
    let task: String?
    let state: String?
    let message: String?
}

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
    @Environment(\.accessibilityReduceMotion) var reduceMotion

    // Empty on purpose: defaulting to the home directory made "Start
    // session" look hung — it snapshots the whole tree (checkpoint.rs does
    // `git add -A`) before the first turn, which is minutes-to-forever over
    // a home folder (worse still if it's iCloud-synced). Force a real
    // project path instead of silently picking something enormous.
    @State var workspace = ""
    @State var recents: [String] =
        UserDefaults.standard.stringArray(forKey: "chorus.recentWorkspaces") ?? []
    /// Holds the composer text outside `ContentView`'s own state: a `@State`
    /// string here re-evaluated this 1000-line body on every keystroke, which
    /// is what made typing lag. Only `ComposerField` observes the box.
    @StateObject var draftBox = DraftBox()
    @State var agent = "codex"
    @State var permission = PermissionLevel.ask
    /// Plan mode: the next messages get a read-only plan first; the agent
    /// only changes anything after "ทำตามแผน".
    @AppStorage("chorus.planFirst") var planFirst = false
    @AppStorage("chorus.suggestNext") var suggestNext = true
    @State var model = ""
    @AppStorage("chorus.autoRoute") var autoRoute = true
    @State var sessionModes: [Int64: SessionMode] = [:]
    /// The mode picked before a session exists (a fresh launch opens a draft
    /// with no id). Without it the pick was dropped and the menu snapped back
    /// to the default — only changing the default in Settings "worked".
    @State var draftSessionMode: SessionMode?
    @AppStorage(SettingsKey.defaultMode) var defaultSessionMode = "auto"
    @AppStorage(SettingsKey.orchestratorModel) var configuredOrchestrator = ""
    @AppStorage(SettingsKey.orchestratorBackup) var configuredOrchestratorBackup = ""
    @AppStorage(SettingsKey.orchestraConcurrency) var orchestraConcurrency = 3
    @AppStorage(SettingsKey.orchestraMaxChildren) var orchestraMaxChildren = 10
    @AppStorage(SettingsKey.orchestraMaxMinutes) var orchestraMaxMinutes = 30
    @AppStorage(SettingsKey.orchestraAskBeforePlan) var askBeforePlan = false
    @State var planApprovedSessions: Set<Int64> = []

    var currentSessionMode: SessionMode {
        get {
            guard engine.sessionID >= 0 else {
                return draftSessionMode ?? SessionMode(rawValue: defaultSessionMode) ?? .autoRoute
            }
            if let m = sessionModes[engine.sessionID] { return m }
            return SessionMode(rawValue: defaultSessionMode) ?? .autoRoute
        }
        nonmutating set {
            if engine.sessionID >= 0 {
                sessionModes[engine.sessionID] = newValue
            } else {
                draftSessionMode = newValue
            }
        }
    }
    @AppStorage("chorus.routerModel") var routerModel = "chorus-openthai-systemone:0.8b"
    @AppStorage("chorus.routerModelAuto") var routerModelAuto = true
    @AppStorage("chorus.autoCatalogUpdates") var autoCatalogUpdates = true
    @AppStorage("chorus.dismissedSetupCards") var dismissedSetupCards = ""
    @AppStorage(SettingsKey.thbRate) var thbRate = ""
    @AppStorage(DisplayKey.currency) var displayCurrency = "auto"
    @State var routingInProgress = false
    @State var routerAutoInstallAttempted = false
    @State var pendingRoute: PendingRoute?
    /// Auto router progress / last pick, shown above the composer.
    @State var routeStatus: RouteStatus?
    @State var reviewModel = ""
    @State var showSettings = false
    @State var showProvidersPopover = false
    @State var showSystemMonitorPopover = false
    /// A tool's own page shown in place of the thread (nil = the thread).
    @State var toolPage: String?
    /// Thread follows new output only while the reader is at the bottom.
    /// Cleared when they scroll up on purpose; set again by scrolling back
    /// down, sending, or opening a session.
    @State var chatPinned = true
    @State var chatSnapshots = ChatSnapshotCache()
    @State var chatLastMinY: CGFloat = 0
    @State var settingsSection = SettingsSheet.Section.accounts
    @State var compareOutput: String?
    @State var inspected: (text: String, tokens: Int64)?
    @State var lastPrompt = ""
    @State var comparing = false
    @State var confirmCompare = false
    @State var compareJudge = false
    @State var compareOrchestra = false

    @State var rightTab = RightTab.context
    @State var showPalette = false
    @State var showInspector = false
    @AppStorage("chorus.failoverAccounts") var failoverAccounts = true
    @AppStorage("chorus.failoverProviders") var failoverProviders = true

    @AppStorage("chorus.showSidebar") var showSidebar = true
    // v2 key: the file tree starts hidden (open it from the ⋯ menu or ⌘B).
    @AppStorage("chorus.showExplorer.v2") var showExplorer = false
    @AppStorage("chorus.showToolsSection") var showToolsSection = false
    @State var selectedFile: String? = nil
    @State var diffReviewMode = true
    @State var showSearchSheet = false

    // Panel Customization & Layout
    @AppStorage("chorus.scrollbarStyle") var scrollbarStyle = "hidden"
    @AppStorage("chorus.inspectorPosition") var inspectorPosition = "trailing"
    @AppStorage("chorus.explorerPosition") var explorerPosition = "leading"
    @AppStorage("chorus.sidebarWidth") var sidebarWidth: Double = 260
    @AppStorage("chorus.explorerWidth") var explorerWidth: Double = 260
    @AppStorage("chorus.inspectorWidth") var inspectorWidth: Double = 330
    /// The viewer's share of the width the sidebar, tree and inspector leave
    /// (the rest is chat). A share, not pixels, so resizing the window keeps
    /// the 1.2 : 1 split the layout decision asks for.
    @AppStorage("chorus.codeViewerShare") var codeViewerShare: Double = WorkbenchLayout.defaultViewerShare
    @State var maximizedPanel: MaximizedPanel = .none

    enum CenterStageMode: String, Equatable {
        case thread
        case livePreview
    }
    @State var centerStageMode: CenterStageMode = .thread
    @State var previewURLString: String = "http://localhost:3000"
    @State var showTerminalDrawer: Bool = false

    var maximizedPanelTitle: String {
        switch maximizedPanel {
        case .codeViewer: return "แผงดูโค้ด (เต็มจอ)"
        case .diffReviewer: return "แผงตรวจสอบ Git Diff (เต็มจอ)"
        case .thread: return "บทสนทนาเต็มจอ"
        case .explorer: return "แผงโครงสร้างไฟล์ (เต็มจอ)"
        case .livePreview: return "แผงพรีวิวเว็บ Live Preview (เต็มจอ)"
        case .none, .orchestrator: return ""
        }
    }
    @State var expanded: Set<String> = []
    @State var expandedSubParents: Set<Int64> = []
    @State var notifiedSubagents: Set<String> = []
    @State var messagingSubagent: Engine.SubagentInfo?
    @State var subagentMessageText = ""
    @StateObject var projects = ProjectStore()
    @StateObject var fileIndex = FileIndex()
    @State var editing: ChorusProject?
    @State var editingIsNew = false
    @State var creatingIn: UUID?
    /// "ลบ session ที่ว่าง" waiting for confirmation, with the count shown.
    @State var emptyCleanup: EmptyCleanup?
    @State var pickingFolderFor: UUID?
    @State var sessionQuery = ""
    @State var renaming: (folder: String, s: Engine.SessionSummary)?
    @State var renameText = ""
    @State var deleting: (folder: String, s: Engine.SessionSummary)?
    @State var queued: [PendingSend] = []
    @State var hint: String?
    @State var importingFiles = 0
    /// Bumped to hand keyboard focus to the composer (session opened, turn sent).
    @State var composerFocus = 0
    @State var confirmRevert = false
    @State var rejected: Set<String> = []
    @State var collapsedFiles: Set<String> = []
    @State var suggestionsPaused = false
    @ObservedObject var powerManager = PowerManager.shared
    @State var inspectedRepoMap: (text: String, count: Int)?
    @State var showingRepoMapSheet = false
    @State var isGeneratingRepoMap = false

    struct PendingSend {
        let model: String
        let text: String
        let attachments: [DraftAttachment]
        let explicitAgent: Bool
    }

    struct PendingRoute: Identifiable {
        let id = UUID()
        let send: PendingSend
        let reason: String
    }

    struct RouterEnvelope: Decodable {
        struct Route: Decodable {
            struct Result: Decodable {
                let candidate_id: String?
                let reason: String?
                let message: String?
            }
            let status: String
            let result: Result?
        }
        let ok: Bool
        let route: Route?
        let error: String?
    }

    enum SetupNeed {
        case agent(Engine.AgentInfo)
        case localRouterModel(Engine.LocalModelStatus.Spec)
        case catalogUnavailable
        case integration(CatalogIntegration)

        var id: String {
            switch self {
            case .agent(let agent): "agent:\(agent.id)"
            case .localRouterModel(let spec): "router:local:\(spec.id)"
            case .catalogUnavailable: "catalog:unavailable"
            case .integration(let entry): "integration:\(entry.id)"
            }
        }
    }

    // One chain this long made the type-checker give up ("unable to
    // type-check this expression in reasonable time") on CI's macos-26
    // runner. Each `some View` property below is checked on its own.
    var body: some View {
        bodyModals
    }

    var bodyFrame: some View {
        GeometryReader { proxy in
            Group {
                if maximizedPanel != .none {
                    maximizedLayout(containerWidth: proxy.size.width)
                } else {
                    multiPanelLayout(containerWidth: proxy.size.width)
                }
            }
            .frame(width: proxy.size.width, height: proxy.size.height)
        }
        .padding(10)
        .background(Backdrop())
        .overlay(alignment: .top) {
            VStack(spacing: 6) {
                InstallStatusChip(onOpen: { settingsSection = .integrations; showSettings = true })
                if maximizedPanel != .none {
                    maximizedFloatingBanner
                }
            }
            .padding(.top, 8)
        }
        .chorusScrollIndicators()
        .onChange(of: engine.sessionID) { _, _ in toolPage = nil }
        .animation(.spring(response: 0.34, dampingFraction: 0.85), value: showSidebar)
        .animation(.spring(response: 0.34, dampingFraction: 0.85), value: showExplorer)
        .animation(.spring(response: 0.34, dampingFraction: 0.85), value: showInspector)
        .animation(.spring(response: 0.34, dampingFraction: 0.85), value: selectedFile != nil)
        .animation(.spring(response: 0.28, dampingFraction: 0.85), value: rightTab)
        .animation(.spring(response: 0.35, dampingFraction: 0.85), value: maximizedPanel)
    }

    var bodyLayout: some View {
        bodyFrame
        .toolbar { mainToolbar }
        .onAppear {
            engine.loadProjectTrust(ws: workspace)
            migrateRouterModelPreference()
            if workspace.isEmpty {
                if let p = projects.projects.first(where: { !$0.folders.isEmpty }),
                   let f = p.folders.first, f != NSHomeDirectory() {
                    workspace = f
                } else if let r = recents.first, r != NSHomeDirectory() {
                    workspace = r
                }
            }
            if !workspace.isEmpty {
                engine.loadSessions(db: dbPath)
                if engine.sessionID < 0 {
                    if let first = engine.sessionsByDB[dbPath]?.first {
                        engine.reopen(db: dbPath, sessionID: first.id)
                    } else {
                        startSession()
                    }
                }
                if engine.sessionID >= 0 {
                    engine.updateSessionCost(db: dbPath, id: engine.sessionID)
                }
            }
            engine.loadAgents(); engine.registerProgress(); engine.loadUsage()
            engine.loadCatalog(project: workspace, automaticUpdates: autoCatalogUpdates)
            engine.loadLocalModels()
            if autoRoute { engine.loadRouterModels() }
            engine.loadLearnedRoutes(workspace: workspace)
            if let raw = UserDefaults.standard.object(forKey: "chorus.defaultPermission") as? Int,
               let p = PermissionLevel(rawValue: Int32(raw)) { permission = p }

            // Auto-expand active project folder in sidebar so user immediately sees sessions
            for p in projects.projects {
                if p.folders.contains(workspace) {
                    expanded.insert(p.id.uuidString)
                    loadSessions(of: p)
                }
            }
            if expanded.isEmpty, let firstProj = projects.projects.first {
                expanded.insert(firstProj.id.uuidString)
                loadSessions(of: firstProj)
            }
        }
        .onChange(of: workspace) { _, w in
            engine.loadProjectTrust(ws: w)
            suggestionsPaused = false
            fileIndex.load(w)
            engine.loadSkills(cwd: w)
            engine.loadCatalog(project: w, automaticUpdates: autoCatalogUpdates)
            engine.loadLearnedRoutes(workspace: w)
            for p in projects.projects {
                if p.folders.contains(w) {
                    expanded.insert(p.id.uuidString)
                    loadSessions(of: p)
                }
            }
        }
        // The last choice is the next launch's default (Settings › สิทธิ์เริ่มต้น
        // reads the same key) — picking "ให้ทำเลย" once should not revert to
        // asking every time the app opens.
        // A turn just ended: ask the local model what the user is likely to
        // type next. Off by setting, or when no local model is configured.
        .onChange(of: engine.busy) { _, busy in
            if busy { engine.nextHint = nil }
            else if suggestNext { engine.loadNextHint(model: routerModel) }
        }
        .onChange(of: permission) { _, p in
            UserDefaults.standard.set(Int(p.rawValue), forKey: "chorus.defaultPermission")
        }
        .onChange(of: autoRoute) { _, enabled in
            if enabled {
                routerAutoInstallAttempted = false
                engine.loadLocalModels()
                engine.loadRouterModels()
            }
        }
        .onChange(of: routerModel) { _, _ in
            if autoRoute { engine.loadRouterModels() }
        }
        .onChange(of: routerModelAuto) { _, enabled in
            if enabled {
                routerAutoInstallAttempted = false
                reconcileRouterModel()
            }
        }
        .onChange(of: engine.routerModels) { _, _ in
            reconcileRouterModel()
        }
    }

    var bodyObservers: some View {
        bodyLayout
        .onChange(of: dismissedSetupCards) { _, value in
            if value.isEmpty { suggestionsPaused = false }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            engine.loadAgents()
            engine.loadCatalog(project: workspace)
            if autoRoute { engine.loadRouterModels() }
        }
        .onChange(of: engine.sessionID) { _, id in
            // The draft's first message just made it a session: keep its mode.
            if id >= 0, let m = draftSessionMode {
                if sessionModes[id] == nil { sessionModes[id] = m }
                draftSessionMode = nil
            }
            engine.metrics = nil
            if !workspace.isEmpty { engine.refreshMetrics(db: dbPath) }
            guard !workspace.isEmpty else { return }
            engine.loadSessions(db: dbPath)
            if id >= 0, let p = projects.project(containing: workspace) {
                engine.setMemoryDB(ProjectStore.memoryPath(p), projectName: p.name)
                engine.setProjectFolders(p.folders)
            }
            if id >= 0 {
                engine.updateSessionCost(db: dbPath, id: id)
                composerFocus &+= 1
            }
        }
        .onChange(of: engine.busy) { _, busy in
            if !busy { engine.refreshMetrics(db: dbPath) }
            flushQueue()
            if !busy && engine.sessionID >= 0 && !NSApp.isActive {
                let reply = engine.events.last(where: { $0.actor != "user" && $0.kind == "msg" })
                let n = TurnNotice.content(lastError: engine.lastError, lastReply: reply?.body, agent: reply?.actor)
                TurnNotice.post(id: "turn-done-\(engine.sessionID)", title: n.title, body: n.body)
            }
            if !busy && engine.sessionID >= 0 {
                engine.updateSessionCost(db: dbPath, id: engine.sessionID)
                // Runs the project's own code — only after "trust this project".
                if !workspace.isEmpty && engine.projectTrusted {
                    engine.verifyWorkspace(ws: workspace)
                }
            }
        }
        .onChange(of: engine.agents) { _, _ in syncModelSelection(); engine.refreshModels() }
        .onChange(of: model) { _, m in
            applyModel(m)
            prepareHandoffIfSwitched(to: m)
        }
        .onChange(of: engine.sessionID) { _, _ in engine.handoffPrep = .none }
        .sheet(isPresented: $showPalette) { palette }
        .alert("เปลี่ยนชื่อ session", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField("ชื่อ", text: $renameText)
            let isRenamingAlert = renaming.map { engine.renamingSessionIDs.contains($0.s.id) } ?? false
            Button(isRenamingAlert ? "กำลังสร้างชื่อด้วย AI…" : "สร้างชื่อด้วย AI ✨") {
                if let r = renaming {
                    engine.renameSessionWithAI(db: dbPath(for: r.folder), id: r.s.id, sampleText: renameText) { title in
                        if let title = title { renameText = title }
                    }
                }
            }
            .disabled(isRenamingAlert)
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
            "ลบ session นี้?",
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
                              // The open session sees a folder added or removed from its next turn.
                              if engine.sessionID >= 0, saved.folders.contains(workspace) {
                                  engine.setProjectFolders(saved.folders)
                              }
                          },
                          onDelete: editingIsNew ? nil : { projects.remove(p) })
        }
        .sheet(item: $messagingSubagent) { sub in
            VStack(alignment: .leading, spacing: 14) {
                Text("ส่งข้อความถึงงานย่อย (\(sub.role))").font(.headline)
                Text(sub.task).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                TextEditor(text: $subagentMessageText)
                    .font(.callout)
                    .frame(height: 100)
                    .padding(6)
                    .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 6))
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.primary.opacity(0.12)))
                HStack {
                    Button("ยกเลิก") {
                        messagingSubagent = nil
                        subagentMessageText = ""
                    }
                    Spacer()
                    Button("ส่งข้อความ") {
                        let text = subagentMessageText.trimmingCharacters(in: .whitespacesAndNewlines)
                        if !text.isEmpty {
                            engine.messageSubagent(db: dbPath(for: workspace), id: sub.id, message: text)
                        }
                        messagingSubagent = nil
                        subagentMessageText = ""
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(subagentMessageText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
            .padding(20)
            .frame(width: 400)
        }
        .onChange(of: engine.subagentsByDB) { _, dbSubs in
            checkSubagentNotifications(in: dbSubs)
        }
    }

    var bodyModals: some View {
        bodyObservers
        .onReceive(AppCommandCenter.shared.fire) { run($0) }
        .onAppear { syncCommandState() }
        .onChange(of: engine.sessionID) { _, _ in syncCommandState() }
        .onChange(of: engine.draft != nil) { _, _ in syncCommandState() }
        .onChange(of: engine.busy) { _, _ in syncCommandState() }
        .onChange(of: selectedFile) { _, _ in syncCommandState() }
        .onChange(of: rightTab) { _, _ in syncCommandState() }
        .onChange(of: showInspector) { _, _ in syncCommandState() }
        .onChange(of: maximizedPanel) { _, _ in syncCommandState() }
        .sheet(isPresented: $showSettings) {
            SettingsSheet(workspace: workspace, section: settingsSection).environmentObject(engine)
        }
        .sheet(item: $pendingRoute) { proposal in
            VStack(alignment: .leading, spacing: 14) {
                Text("เลือกผู้ทำงาน").font(.title3.bold())
                Text(proposal.reason).font(.callout).foregroundStyle(.secondary)
                ModelPicker(model: $reviewModel, onNeedsLogin: { pendingRoute = nil; showSettings = true },
                            respondsToMenu: false)
                    .environmentObject(engine)
                HStack {
                    Button("ยกเลิก", action: cancelReviewedRoute)
                    Spacer()
                    Button("ส่งงาน", action: sendReviewedRoute)
                        .buttonStyle(.borderedProminent)
                        .disabled(agentFor(reviewModel)?.ready != true)
                }
            }
            .padding(20)
            .frame(width: 390)
        }
        .sheet(isPresented: $confirmCompare) {
            compareConfirmSheet
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
                Text("\(askingAgentLabel(ask.agent)) wants to: **\(ask.tool)**")
                if !ask.kind.isEmpty {
                    Text("kind: \(ask.kind)").font(.caption).foregroundStyle(.secondary)
                }
                if !ask.detail.isEmpty {
                    Text(ask.detail).font(.callout).textSelection(.enabled)
                }
                if let why = ask.tainted {
                    Label("\(why) — ข้อความที่อ่านมาอาจแอบสั่ง agent", systemImage: "exclamationmark.shield")
                        .font(.caption).foregroundStyle(.orange)
                }
                HStack {
                    Button("ไม่อนุญาต", role: .destructive) { engine.answerPermission(.deny) }
                        .keyboardShortcut(.cancelAction)
                    Spacer()
                    Button("อนุญาตตลอด session นี้") { engine.answerPermission(.session) }
                        .help("ไม่ถามซ้ำกับคำขอเดียวกันจนกว่าจะปิด session — คำสั่งใหม่ยังถาม")
                    Button("อนุญาตครั้งนี้") { engine.answerPermission(.once) }
                        .keyboardShortcut(.defaultAction)
                }
            }
            .padding(20)
            .frame(width: 480)
            .interactiveDismissDisabled()
        }
        .sheet(item: $engine.pendingSecret) { req in
            SecretRequestSheet(request: req, agentLabel: askingAgentLabel(req.agent)) { engine.answerSecret($0) }
        }
        .sheet(isPresented: $showSearchSheet) {
            ChatSearchView(
                engine: engine,
                dbPath: dbPath,
                onSelectSession: { sessId in
                    engine.reopen(db: dbPath, sessionID: sessId)
                    showSearchSheet = false
                },
                onClose: {
                    showSearchSheet = false
                }
            )
        }
    }

}
