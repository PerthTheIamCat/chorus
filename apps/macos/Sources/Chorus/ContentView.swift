import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

private struct SubagentLedgerPayload: Decodable {
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
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

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
    /// Plan mode: the next messages get a read-only plan first; the agent
    /// only changes anything after "ทำตามแผน".
    @AppStorage("chorus.planFirst") private var planFirst = false
    @State private var model = ""
    @AppStorage("chorus.autoRoute") private var autoRoute = true
    @State private var sessionModes: [Int64: SessionMode] = [:]
    @AppStorage(SettingsKey.defaultMode) private var defaultSessionMode = "auto"
    @AppStorage(SettingsKey.orchestratorModel) private var configuredOrchestrator = ""
    @AppStorage(SettingsKey.orchestratorBackup) private var configuredOrchestratorBackup = ""
    @AppStorage(SettingsKey.orchestraConcurrency) private var orchestraConcurrency = 3
    @AppStorage(SettingsKey.orchestraMaxChildren) private var orchestraMaxChildren = 10
    @AppStorage(SettingsKey.orchestraMaxMinutes) private var orchestraMaxMinutes = 30
    @AppStorage(SettingsKey.orchestraAskBeforePlan) private var askBeforePlan = false
    @State private var planApprovedSessions: Set<Int64> = []

    private var currentSessionMode: SessionMode {
        get {
            guard engine.sessionID >= 0 else {
                return SessionMode(rawValue: defaultSessionMode) ?? .autoRoute
            }
            if let m = sessionModes[engine.sessionID] { return m }
            return SessionMode(rawValue: defaultSessionMode) ?? .autoRoute
        }
        nonmutating set {
            if engine.sessionID >= 0 {
                sessionModes[engine.sessionID] = newValue
            }
        }
    }
    @AppStorage("chorus.routerModel") private var routerModel = "chorus-openthai-systemone:0.8b"
    @AppStorage("chorus.routerModelAuto") private var routerModelAuto = true
    @AppStorage("chorus.autoCatalogUpdates") private var autoCatalogUpdates = true
    @AppStorage("chorus.dismissedSetupCards") private var dismissedSetupCards = ""
    @AppStorage(SettingsKey.thbRate) private var thbRate = ""
    @State private var routingInProgress = false
    @State private var routerAutoInstallAttempted = false
    @State private var pendingRoute: PendingRoute?
    /// Auto router progress / last pick, shown above the composer.
    @State private var routeStatus: RouteStatus?
    @State private var reviewModel = ""
    @State private var showSettings = false
    @State private var showProvidersPopover = false
    @State private var showSystemMonitorPopover = false
    /// A tool's own page shown in place of the thread (nil = the thread).
    @State private var toolPage: String?
    /// Thread follows new output only while the reader is at the bottom.
    /// Cleared when they scroll up on purpose; set again by scrolling back
    /// down, sending, or opening a session.
    @State private var chatPinned = true
    @State private var chatLastMinY: CGFloat = 0
    @State private var settingsSection = SettingsSheet.Section.accounts
    @State private var compareOutput: String?
    @State private var inspected: (text: String, tokens: Int64)?
    @State private var lastPrompt = ""

    @State private var rightTab = RightTab.context
    @State private var showPalette = false
    @State private var showInspector = true
    @AppStorage("chorus.failoverAccounts") private var failoverAccounts = true
    @AppStorage("chorus.failoverProviders") private var failoverProviders = true

    @AppStorage("chorus.showSidebar") private var showSidebar = true
    @AppStorage("chorus.showExplorer") private var showExplorer = true
    @State private var selectedFile: String? = nil
    @State private var diffReviewMode = true
    @State private var showSearchSheet = false

    // Panel Customization & Layout
    @AppStorage("chorus.scrollbarStyle") private var scrollbarStyle = "hidden"
    @AppStorage("chorus.inspectorPosition") private var inspectorPosition = "trailing"
    @AppStorage("chorus.explorerPosition") private var explorerPosition = "leading"
    @AppStorage("chorus.sidebarWidth") private var sidebarWidth: Double = 260
    @AppStorage("chorus.explorerWidth") private var explorerWidth: Double = 260
    @AppStorage("chorus.inspectorWidth") private var inspectorWidth: Double = 330
    /// The viewer's share of the width the sidebar, tree and inspector leave
    /// (the rest is chat). A share, not pixels, so resizing the window keeps
    /// the 1.2 : 1 split the layout decision asks for.
    @AppStorage("chorus.codeViewerShare") private var codeViewerShare: Double = WorkbenchLayout.defaultViewerShare
    @State private var maximizedPanel: MaximizedPanel = .none

    enum CenterStageMode: String, Equatable {
        case thread
        case livePreview
    }
    @State private var centerStageMode: CenterStageMode = .thread
    @State private var previewURLString: String = "http://localhost:3000"
    @State private var showTerminalDrawer: Bool = false

    private var maximizedPanelTitle: String {
        switch maximizedPanel {
        case .codeViewer: return "แผงดูโค้ด (เต็มจอ)"
        case .diffReviewer: return "แผงตรวจสอบ Git Diff (เต็มจอ)"
        case .thread: return "บทสนทนาเต็มจอ"
        case .explorer: return "แผงโครงสร้างไฟล์ (เต็มจอ)"
        case .livePreview: return "แผงพรีวิวเว็บ Live Preview (เต็มจอ)"
        case .none, .orchestrator: return ""
        }
    }
    @State private var expanded: Set<String> = []
    @State private var expandedSubParents: Set<Int64> = []
    @State private var notifiedSubagents: Set<String> = []
    @State private var messagingSubagent: Engine.SubagentInfo?
    @State private var subagentMessageText = ""
    @StateObject private var projects = ProjectStore()
    @StateObject private var fileIndex = FileIndex()
    @State private var editing: ChorusProject?
    @State private var editingIsNew = false
    @State private var creatingIn: UUID?
    /// "ลบ session ที่ว่าง" waiting for confirmation, with the count shown.
    @State private var emptyCleanup: EmptyCleanup?
    @State private var pickingFolderFor: UUID?
    @State private var sessionQuery = ""
    @State private var renaming: (folder: String, s: Engine.SessionSummary)?
    @State private var renameText = ""
    @State private var deleting: (folder: String, s: Engine.SessionSummary)?
    @State private var queued: [PendingSend] = []
    @State private var hint: String?
    @State private var importingFiles = 0
    /// Bumped to hand keyboard focus to the composer (session opened, turn sent).
    @State private var composerFocus = 0
    @State private var confirmRevert = false
    @State private var rejected: Set<String> = []
    @State private var collapsedFiles: Set<String> = []
    @State private var suggestionsPaused = false
    @ObservedObject private var powerManager = PowerManager.shared
    @State private var inspectedRepoMap: (text: String, count: Int)?
    @State private var showingRepoMapSheet = false
    @State private var isGeneratingRepoMap = false

    private struct PendingSend {
        let model: String
        let text: String
        let attachments: [DraftAttachment]
        let explicitAgent: Bool
    }

    private struct PendingRoute: Identifiable {
        let id = UUID()
        let send: PendingSend
        let reason: String
    }

    private struct RouterEnvelope: Decodable {
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

    private enum SetupNeed {
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

    /// Keep the evaluated candidates in quality order. Unknown local models
    /// remain available for manual selection, but are not chosen automatically.
    private static let preferredRouterModels = [
        "chorus-openthai-systemone:0.8b", "tev1:0.8b", "qwen3:1.7b", "qwen2.5:3b"
    ]

    private func reconcileRouterModel() {
        guard autoRoute, routerModelAuto, let installed = engine.routerModels else { return }
        if let best = Self.preferredRouterModels.first(where: { installed.contains($0) }) {
            if routerModel != best { routerModel = best }
        } else if !routerAutoInstallAttempted && !engine.routerDownloading {
            routerAutoInstallAttempted = true
            engine.pullRouterModel(Self.preferredRouterModels[0])
        }
    }

    private func migrateRouterModelPreference() {
        let defaults = UserDefaults.standard
        guard defaults.object(forKey: "chorus.routerModelAuto") == nil else { return }
        // Before the automatic option existed, these were the two shipped
        // defaults. Preserve a different value that the user typed manually.
        if let saved = defaults.string(forKey: "chorus.routerModel"),
           saved != "qwen2.5:3b", saved != Self.preferredRouterModels[0] {
            routerModelAuto = false
        } else {
            routerModelAuto = true
        }
    }

    private var setupNeed: SetupNeed? {
        let dismissed = Set(dismissedSetupCards.split(separator: ",").map(String.init))
        func visible(_ need: SetupNeed) -> Bool { !dismissed.contains(need.id) }

        if !engine.agents.isEmpty && !engine.agents.contains(where: \.ready),
           let missing = engine.agents.first(where: { !$0.ready }) {
            let need = SetupNeed.agent(missing)
            if visible(need) { return need }
        }
        if autoRoute {
            if let ot = engine.localModels.first(where: { $0.id.contains("openthai") }),
               !ot.isInstalled {
                let need = SetupNeed.localRouterModel(ot.spec)
                if visible(need) { return need }
            }
        }
        if !workspace.isEmpty && !suggestionsPaused {
            if engine.catalog.isEmpty, engine.catalogError != nil {
                let need = SetupNeed.catalogUnavailable
                if visible(need) { return need }
            }
            let order = ["serena", "graft", "rtk", "headroom"]
            for id in order {
                if let entry = engine.catalog.first(where: { $0.id == id && (!$0.isInstalled || ($0.isHealthy == true && !$0.isEnabledForProject)) }) {
                    let need = SetupNeed.integration(entry)
                    if visible(need) { return need }
                }
            }
        }
        if let missing = engine.agents.first(where: { !$0.ready }) {
            let need = SetupNeed.agent(missing)
            if visible(need) { return need }
        }
        return nil
    }

    private func dismissSetup(_ need: SetupNeed) {
        var ids = Set(dismissedSetupCards.split(separator: ",").map(String.init))
        ids.insert(need.id)
        dismissedSetupCards = ids.sorted().joined(separator: ",")
        if case .integration = need { suggestionsPaused = true }
    }

    private func integrationDescription(_ id: String) -> String {
        switch id {
        case "serena": "ช่วย agent ค้นหาและแก้โค้ดในโปรเจกต์"
        case "graft": "ช่วยหาโค้ดและดูความเชื่อมโยงของส่วนต่าง ๆ"
        case "rtk": "ย่อผลลัพธ์จากคำสั่งให้ agent อ่านง่ายขึ้น"
        case "headroom": "ช่วยจัดการข้อมูลที่ส่งให้ agent"
        default: "เพิ่มความสามารถให้ agent ในโปรเจกต์นี้"
        }
    }

    private var ollamaAppURL: URL? {
        let roots = [URL(fileURLWithPath: "/Applications/Ollama.app"),
                     FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Applications/Ollama.app")]
        return roots.first { FileManager.default.fileExists(atPath: $0.path) }
    }

    private var ollamaCLIURL: URL? {
        let inherited = (ProcessInfo.processInfo.environment["PATH"] ?? "")
            .split(separator: ":").map(String.init)
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let folders = inherited + ["/opt/homebrew/bin", "/usr/local/bin", "\(home)/.local/bin"]
        return folders.lazy.map { URL(fileURLWithPath: $0).appendingPathComponent("ollama") }
            .first { FileManager.default.isExecutableFile(atPath: $0.path) }
    }

    @ViewBuilder
    private var setupCard: some View {
        if let need = setupNeed {
            switch need {
            case .agent(let agent):
                SetupPromptCard(
                    title: "\(agent.label) ยังไม่พร้อมใช้",
                    detail: agent.auth == "needs_login"
                        ? "เปิด Terminal เพื่อเข้าสู่ระบบ แล้ว Chorus จะตรวจสถานะให้อีกครั้ง"
                        : "เริ่มคำสั่งติดตั้งใน Terminal แล้วกลับมาที่ Chorus เพื่อตรวจสถานะ",
                    symbol: "person.crop.circle.badge.exclamationmark",
                    actionTitle: agent.auth == "needs_login" ? "เริ่มเข้าสู่ระบบ" : "เริ่มติดตั้ง",
                    onAction: { engine.runLoginCommand(agent.hint) },
                    onDismiss: { dismissSetup(need) },
                    secondaryTitle: "ดูสถานะ",
                    onSecondary: { settingsSection = .agents; showSettings = true })
            case .localRouterModel(let spec):
                SetupPromptCard(
                    title: "ติดตั้ง AI Auto Router (คลิกเดียว)",
                    detail: engine.localModelError ?? (engine.isDownloadingLocalModel
                        ? (engine.localModelDownloadStatusText.isEmpty ? "กำลังดาวน์โหลด \(spec.display_name)..." : engine.localModelDownloadStatusText)
                        : "ติดตั้ง \(spec.display_name) (\(ByteCountFormatter.string(fromByteCount: spec.expected_size_bytes, countStyle: .file))) สำหรับเลือกโมเดลในเครื่องอัตโนมัติ ไม่ต้องพึ่งพา Ollama"),
                    symbol: "sparkles",
                    actionTitle: engine.isDownloadingLocalModel ? "กำลังดาวน์โหลด..." : "ติดตั้ง \(spec.display_name)",
                    working: engine.isDownloadingLocalModel,
                    onAction: {
                        if !engine.isDownloadingLocalModel {
                            engine.downloadLocalModel(spec)
                        }
                    },
                    onDismiss: { dismissSetup(need) },
                    secondaryTitle: "ตั้งค่าโมเดล",
                    onSecondary: { settingsSection = .storage; showSettings = true })
            case .catalogUnavailable:
                SetupPromptCard(
                    title: "ยังตรวจเครื่องมือเสริมไม่ได้",
                    detail: engine.catalogError ?? "ลองโหลดรายการเครื่องมืออีกครั้ง",
                    symbol: "puzzlepiece.extension",
                    actionTitle: "ตรวจใหม่",
                    onAction: { engine.loadCatalog(project: workspace) },
                    onDismiss: { dismissSetup(need) },
                    secondaryTitle: "เปิดเครื่องมือเสริม",
                    onSecondary: { settingsSection = .integrations; showSettings = true })
            case .integration(let entry):
                let missing = entry.isInstalled ? nil : entry.missingInstaller
                SetupPromptCard(
                    title: "แนะนำ \(entry.title) สำหรับโปรเจกต์นี้",
                    detail: engine.catalogActionError ?? (missing.map {
                        "ต้องติดตั้ง \($0) ก่อนเพิ่มเครื่องมือนี้"
                    } ?? (entry.isInstalled
                        ? "ติดตั้งแล้ว แต่ยังไม่ได้เปิดใช้กับโปรเจกต์นี้"
                        : "ยังไม่ได้เพิ่มผ่าน Chorus · \(integrationDescription(entry.id))")),
                    symbol: "puzzlepiece.extension",
                    actionTitle: missing.map { "วิธีติดตั้ง \($0)" }
                        ?? (entry.isInstalled ? "เปิดใช้" : "ติดตั้งและเปิดใช้"),
                    working: entry.isWorking,
                    onAction: {
                        if missing != nil, let url = entry.installerURL { NSWorkspace.shared.open(url) }
                        else if entry.isInstalled {
                            engine.catalogAction(project: workspace, id: entry.id, action: "enable")
                        } else {
                            // Show what will be created before anything is installed.
                            engine.requestedInstall = entry.id
                            settingsSection = .integrations
                            showSettings = true
                        }
                    },
                    onDismiss: { dismissSetup(need) },
                    secondaryTitle: "ดูรายละเอียด",
                    onSecondary: { settingsSection = .integrations; showSettings = true })
            }
        }
    }

    enum RightTab: String, CaseIterable {
        case context = "คอนเท็กซ์", changes = "ที่แก้ไป", memory = "ความจำ", checkpoints = "จุดบันทึก"

        var icon: String {
            switch self {
            case .context: "gauge.with.dots.needle.50percent"
            case .changes: "plusminus"
            case .memory: "brain.head.profile"
            case .checkpoints: "clock.arrow.circlepath"
            }
        }

        var tooltip: String {
            switch self {
            case .context: "ตรวจสอบบริบท Token และ Repo Map"
            case .changes: "ตรวจสิ่งที่แก้"
            case .memory: "ความจำและข้อเท็จจริงของโปรเจกต์"
            case .checkpoints: "Time-Travel Checkpoints & ย้อนเวลา"
            }
        }
    }

    private func badge(_ t: RightTab) -> Int {
        switch t {
        case .context: return 0
        case .changes: return engine.diffFiles.count
        case .memory: return engine.pending.count
        case .checkpoints: return 0
        }
    }

    /// Icon tabs with counts, so the panel says where something needs you
    /// (files changed, proposals waiting) before you open the tab.
    private var inspectorTabs: some View {
        HStack(spacing: 2) {
            ForEach(Array(RightTab.allCases.enumerated()), id: \.element) { index, t in
                let n = badge(t)
                Button { rightTab = t } label: {
                    VStack(spacing: 3) {
                        ZStack(alignment: .topTrailing) {
                            Image(systemName: t.icon).font(.system(size: 14, weight: .medium))
                                .frame(width: 26, height: 18)
                            if n > 0 {
                                Text(n > 99 ? "99+" : "\(n)").font(.system(size: 8, weight: .bold)).foregroundStyle(.white)
                                    .padding(.horizontal, 3).padding(.vertical, 1)
                                    .background(t == .memory ? Color.orange : Color.accentColor, in: Capsule())
                                    .offset(x: 10, y: -6)
                            }
                        }
                        Text(t.rawValue).font(.caption2)
                    }
                    .frame(maxWidth: .infinity)
                    .foregroundStyle(rightTab == t ? Color.accentColor : .secondary)
                }
                .buttonStyle(InteractiveStyle(cornerRadius: 9, selected: rightTab == t,
                                              padding: .init(top: 6, leading: 2, bottom: 5, trailing: 2)))
                .help("\(t.tooltip) (⌘⌥\(index + 1))")
                .accessibilityLabel(t.rawValue)
            }
        }
        .padding(8)
    }

    // MARK: - Component Views for Panels

    @ViewBuilder
    private var sidebarPanel: some View {
        sidebar
            .frame(width: sidebarWidth)
            .floatingPanel(cornerRadius: 20)
            .transition(.asymmetric(
                insertion: .move(edge: .leading).combined(with: .opacity).combined(with: .scale(scale: 0.98)),
                removal: .move(edge: .leading).combined(with: .opacity)
            ))
    }

    @ViewBuilder
    private func explorerPanel(isRail: Bool = false) -> some View {
        WorkspaceExplorerView(
            engine: engine,
            workspace: workspace,
            isMaximized: maximizedPanel == .explorer,
            isRailMode: isRail,
            onToggleMaximize: {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                    maximizedPanel = (maximizedPanel == .explorer) ? .none : .explorer
                }
            },
            onSelectFile: { file in
                selectedFile = file
            },
            onSelectWorktree: { _ in }
        )
        .frame(width: maximizedPanel == .explorer ? nil : (isRail ? 44 : explorerWidth))
        .floatingPanel(cornerRadius: 20)
        .transition(.asymmetric(
            insertion: .move(edge: .leading).combined(with: .opacity).combined(with: .scale(scale: 0.98)),
            removal: .move(edge: .leading).combined(with: .opacity)
        ))
    }

    @ViewBuilder
    private var inspectorPanel: some View {
        VStack(spacing: 0) {
            inspectorTabs
            Divider().opacity(0.5)
            switch rightTab {
            case .context: contextPane
            case .changes:
                if diffReviewMode {
                    DiffReviewerView(
                        engine: engine,
                        workspace: workspace,
                        isMaximized: maximizedPanel == .diffReviewer,
                        onToggleMaximize: {
                            withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                                maximizedPanel = (maximizedPanel == .diffReviewer) ? .none : .diffReviewer
                            }
                        },
                        onSendSteeringPrompt: { prompt in
                            draftBox.text = prompt
                        }
                    )
                } else {
                    changesPane
                }
            case .memory: memoryPane
            case .checkpoints:
                CheckpointTimelineView(
                    engine: engine,
                    dbPath: dbPath,
                    workspace: workspace
                )
            }
        }
        .frame(width: maximizedPanel == .diffReviewer ? nil : (rightTab == .changes ? max(inspectorWidth, 420) : inspectorWidth))
        .floatingPanel(cornerRadius: 20)
        .transition(.asymmetric(
            insertion: .move(edge: inspectorPosition == "leading" ? .leading : .trailing).combined(with: .opacity).combined(with: .scale(scale: 0.98)),
            removal: .move(edge: inspectorPosition == "leading" ? .leading : .trailing).combined(with: .opacity)
        ))
    }

    @ViewBuilder
    private func codeViewerPanel(for file: String, isCenterStage: Bool = false, width: CGFloat? = nil) -> some View {
        CodeViewerView(
            filePath: file,
            cwd: workspace,
            isMaximized: maximizedPanel == .codeViewer,
            onToggleMaximize: {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                    maximizedPanel = (maximizedPanel == .codeViewer) ? .none : .codeViewer
                }
            },
            onClose: {
                if maximizedPanel == .codeViewer { maximizedPanel = .none }
                selectedFile = nil
            }
        )
        .frame(width: (maximizedPanel == .codeViewer || isCenterStage) ? nil : width)
        .frame(maxWidth: isCenterStage ? .infinity : nil)
        .floatingPanel(cornerRadius: 20)
        .transition(.asymmetric(
            insertion: .opacity.combined(with: .scale(scale: 0.98)),
            removal: .opacity
        ))
    }

    @ViewBuilder
    private var maximizedFloatingBanner: some View {
        HStack(spacing: 8) {
            Image(systemName: "arrow.down.right.and.arrow.up.left")
                .font(.system(size: 11, weight: .bold))
                .foregroundColor(.accentColor)
            Text(maximizedPanelTitle)
                .font(.system(size: 12, weight: .semibold))
            Button("ย่อขนาดกลับ (⇧⌘M)") {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                    maximizedPanel = .none
                }
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.small)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 7)
        .background(.ultraThinMaterial, in: Capsule())
        .overlay(Capsule().stroke(Color.primary.opacity(0.12), lineWidth: 1))
        .shadow(color: .black.opacity(0.14), radius: 10, y: 3)
        .transition(.move(edge: .top).combined(with: .opacity))
    }

    @ViewBuilder
    private var breadcrumbHeader: some View {
        let currentSub = engine.subagentsByDB[dbPath(for: workspace)]?.first { $0.child_sess == engine.sessionID }
        let currentSess = (engine.sessionsByDB[dbPath(for: workspace)] ?? []).first { $0.id == engine.sessionID }
        let parentId = currentSub?.parent_sess ?? currentSess?.forked_from
        if let parentId, let parent = (engine.sessionsByDB[dbPath(for: workspace)] ?? []).first(where: { $0.id == parentId }) {
            HStack(spacing: 6) {
                Button {
                    workspace = parent.cwd.isEmpty ? workspace : parent.cwd
                    engine.reopen(db: dbPath(for: workspace), sessionID: parent.id)
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "chevron.left")
                            .font(.system(size: 10, weight: .bold))
                        Text(displayTitle(parent))
                            .lineLimit(1)
                    }
                }
                .buttonStyle(.plain)
                .foregroundStyle(Color.accentColor)

                Image(systemName: "chevron.right")
                    .font(.system(size: 9, weight: .bold))
                    .foregroundStyle(.tertiary)

                Text(currentSub.map { "\($0.role) · \($0.task)" } ?? displayTitle(currentSess ?? parent))
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)

                Spacer()

                if let sub = currentSub {
                    if sub.kind == "provider" {
                        Text("Provider Subagent (อ่านอย่างเดียว)")
                            .font(.system(size: 9.5, weight: .semibold))
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 6).padding(.vertical, 2)
                            .background(Color.primary.opacity(0.06), in: Capsule())
                    } else if sub.status == "waiting" {
                        Text("รอคำยืนยัน")
                            .font(.system(size: 9.5, weight: .bold))
                            .foregroundStyle(.white)
                            .padding(.horizontal, 6).padding(.vertical, 2)
                            .background(Color.orange, in: Capsule())
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 6)
            .background(Color.primary.opacity(0.03))
            Divider().opacity(0.3)
        }
    }

    @ViewBuilder
    private var centerStageView: some View {
        VStack(spacing: 0) {
            breadcrumbHeader
            switch centerStageMode {
            case .thread:
                thread
            case .livePreview:
                LivePreviewView(
                    currentURLString: $previewURLString,
                    onClose: {
                        withAnimation(.spring(response: 0.28, dampingFraction: 0.85)) {
                            centerStageMode = .thread
                        }
                    }
                )
                .floatingPanel(cornerRadius: 20)
            }

            if showTerminalDrawer {
                Divider()
                TerminalDrawerView(
                    workspace: workspace,
                    onClose: {
                        withAnimation(.spring(response: 0.28, dampingFraction: 0.85)) {
                            showTerminalDrawer = false
                        }
                    }
                )
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
    }

    /// Viewer (opens next to the tree, only while a file is open) and chat
    /// (min 380 px), sharing `space` about 1.2 : 1. When both can't keep
    /// their minimum, the file takes chat's place as in narrow windows.
    @ViewBuilder
    private func viewerAndChat(file: String, space: CGFloat) -> some View {
        let gap = WorkbenchLayout.columnSpacing
        let shared = space - WorkbenchLayout.gripWidth - gap * 2
        let chatMin = WorkbenchLayout.chatMinWidth
        let viewerMin = WorkbenchLayout.viewerMinWidth
        if shared >= chatMin + viewerMin {
            let viewerWidth = min(max(shared * codeViewerShare, viewerMin), shared - chatMin)
            HStack(spacing: gap) {
                codeViewerPanel(for: file, isCenterStage: false, width: viewerWidth)
                PanelResizeGrip(
                    width: Binding(
                        get: { Double(viewerWidth) },
                        set: { codeViewerShare = $0 / Double(shared) }
                    ),
                    minWidth: Double(viewerMin), maxWidth: Double(shared - chatMin), direction: .forward
                ) {
                    codeViewerShare = WorkbenchLayout.defaultViewerShare
                }
                centerStageView
                    .frame(minWidth: chatMin)
            }
        } else {
            codeViewerPanel(for: file, isCenterStage: true)
        }
    }

    @ViewBuilder
    private func multiPanelLayout(containerWidth: CGFloat) -> some View {
        let isNarrow = containerWidth < 1200
        HStack(spacing: WorkbenchLayout.columnSpacing) {
            // 1. Sidebar (Sessions: 230–260 px)
            if showSidebar {
                sidebarPanel
                PanelResizeGrip(width: $sidebarWidth, minWidth: 180, maxWidth: 420, direction: .forward) {
                    sidebarWidth = 260
                }
            }

            // 2. File tree / Explorer (210–240 px, or 44 px icon rail when < 1200 px)
            if showExplorer {
                explorerPanel(isRail: isNarrow)
                if !isNarrow {
                    PanelResizeGrip(width: $explorerWidth, minWidth: 200, maxWidth: 480, direction: .forward) {
                        explorerWidth = 240
                    }
                }
            }

            // 3–4. File viewer + chat share whatever the fixed columns leave,
            // measured rather than summed so panel padding can't skew it.
            if !isNarrow, let file = selectedFile {
                GeometryReader { g in
                    viewerAndChat(file: file, space: g.size.width)
                        .frame(width: g.size.width, height: g.size.height)
                }
                .frame(minWidth: WorkbenchLayout.chatMinWidth)
            } else if let file = selectedFile {
                // Narrow window: an open file temporarily replaces chat.
                codeViewerPanel(for: file, isCenterStage: true)
            } else {
                centerStageView
                    .frame(minWidth: WorkbenchLayout.chatMinWidth)
            }

            // 5. Inspector (stays a toggled right panel ⌥⌘I)
            if showInspector {
                PanelResizeGrip(width: $inspectorWidth, minWidth: 260, maxWidth: 700, direction: .reversed) {
                    inspectorWidth = 330
                }
                inspectorPanel
            }
        }
    }

    @ViewBuilder
    private func maximizedLayout(containerWidth: CGFloat) -> some View {
        switch maximizedPanel {
        case .codeViewer:
            if let file = selectedFile {
                codeViewerPanel(for: file, isCenterStage: true)
            } else {
                centerStageView
            }
        case .diffReviewer:
            inspectorPanel
        case .explorer:
            explorerPanel(isRail: false)
        case .thread:
            thread
        case .orchestrator:
            centerStageView
        case .livePreview:
            LivePreviewView(
                currentURLString: $previewURLString,
                onClose: {
                    withAnimation(.spring(response: 0.28, dampingFraction: 0.85)) {
                        maximizedPanel = .none
                    }
                }
            )
            .floatingPanel(cornerRadius: 20)
        case .none:
            multiPanelLayout(containerWidth: containerWidth)
        }
    }

    // One chain this long made the type-checker give up ("unable to
    // type-check this expression in reasonable time") on CI's macos-26
    // runner. Each `some View` property below is checked on its own.
    var body: some View {
        bodyModals
    }

    private var bodyFrame: some View {
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

    @ToolbarContentBuilder
    private var mainToolbar: some ToolbarContent {
        // ToolbarContentBuilder takes at most 10 items per block; there are 12.
        navigationToolbar
        actionToolbar
    }

    @ToolbarContentBuilder
    private var navigationToolbar: some ToolbarContent {
        ToolbarItem(placement: .navigation) {
            Button { showSidebar.toggle() } label: {
                Label("แถบโปรเจกต์", systemImage: "sidebar.left")
            }
            .help("เปิด/ปิดแถบโปรเจกต์ (⌃⌘S)")
        }
        ToolbarItem(placement: .navigation) {
            Button { showExplorer.toggle() } label: {
                Label("ไฟล์", systemImage: "folder")
            }
            .help("เปิด/ปิดแถบไฟล์ (⌘B)")
        }
        ToolbarItem(placement: .navigation) {
            HStack(spacing: 6) {
                Text(engine.sessionID < 0 ? (engine.draft != nil ? "session ใหม่" : "ยังไม่ได้เปิด session")
                     : (engine.sessionsByDB[dbPath]?.first { $0.id == engine.sessionID }).map(displayTitle)
                        ?? "session \(engine.sessionID)")
                    .lineLimit(1)
                    .font(.headline)

                if engine.sessionID >= 0 {
                    let isRenaming = engine.renamingSessionIDs.contains(engine.sessionID)
                    if isRenaming {
                        ProgressView()
                            .controlSize(.mini)
                            .help("กำลังตั้งชื่อ session ด้วย AI…")
                    } else {
                        Button {
                            if let s = engine.sessionsByDB[dbPath]?.first(where: { $0.id == engine.sessionID }) {
                                engine.renameSessionWithAI(db: dbPath, id: s.id, sampleText: displayTitle(s))
                            }
                        } label: {
                            Image(systemName: "sparkles")
                                .font(.system(size: 11))
                                .foregroundColor(.accentColor)
                        }
                        .buttonStyle(.plain)
                        .help("ให้ AI ตั้งชื่อ session")
                    }
                }
            }
        }
        // Push everything after this to the window's trailing edge.
        if #available(macOS 26, *) {
            ToolbarSpacer(.flexible)
        }
    }

    @ToolbarContentBuilder
    private var actionToolbar: some ToolbarContent {
        ToolbarItemGroup(placement: .primaryAction) {
            // Handing over is one click per agent. Everything the next
            // agent needs is already in the ledger, so asking the user to
            // fill in a form would be asking them to retype what we know.
            if engine.sessionID >= 0 {
                ForEach(engine.agents.filter { $0.ready && $0.id != currentAgentID }) { a in
                    Button {
                        engine.handoffAuto(to: a.id, level: max(permission.rawValue, 0))
                    } label: {
                        HStack(spacing: 5) {
                            Image(systemName: "arrowshape.turn.up.right.fill")
                                .font(.caption2)
                                .foregroundStyle(Color.accentColor)
                            ProviderIcon(provider: a.id, size: 13)
                            Text("ส่งต่อ \(a.label)")
                                .font(.caption.weight(.medium))
                        }
                    }
                    .disabled(engine.busy)
                    .help("ส่งงานต่อให้ \(a.label) พร้อมเป้าหมาย สิ่งที่ทำไปแล้ว และ diff (ไม่ต้องพิมพ์อะไร)")
                }
            }
        }
        ToolbarItem(placement: .primaryAction) {
            Button { showSearchSheet = true } label: {
                Label("ค้นหา", systemImage: "magnifyingglass")
            }
            .help("ค้นหาบทสนทนาข้าม session (⇧⌘F)")
        }
        ToolbarItem(placement: .primaryAction) {
            Button {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                    maximizedPanel = (maximizedPanel == .thread) ? .none : .thread
                }
            } label: {
                Label(maximizedPanel == .thread ? "ออกจากโหมดโฟกัส" : "โหมดโฟกัส",
                      systemImage: maximizedPanel == .thread ? "arrow.down.right.and.arrow.up.left" : "viewfinder")
            }
            .help(maximizedPanel == .thread ? "ออกจากโหมดโฟกัส (⇧⌘M)" : "ขยายบทสนทนาเต็มจอ / โหมดโฟกัส (⇧⌘M)")
        }
        ToolbarItem(placement: .primaryAction) {
            if powerManager.shouldThrottleConcurrency {
                HStack(spacing: 4) {
                    Image(systemName: "flame.fill")
                        .font(.system(size: 11))
                        .foregroundColor(.orange)
                    Text(powerManager.thermalStateName)
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(.secondary)
                }
                .padding(.horizontal, 6)
                .padding(.vertical, 3)
                .background(Color.orange.opacity(0.12), in: Capsule())
                .help("Apple Silicon Thermal Governor: ระบบควบคุมความร้อนกำลังปรับการทำงานให้เหมาะสม")
            }
        }
        ToolbarItem(placement: .primaryAction) {
            Button {
                settingsSection = .orchestration
                showSettings = true
            } label: {
                Label("Orchestra & Roles", systemImage: "point.3.connected.trianglepath.dotted")
            }
            .help("เปิดการตั้งค่า Orchestra และ Role Library (⌘⌥O)")
        }
        ToolbarItem(placement: .primaryAction) {
            Button {
                withAnimation(.spring(response: 0.28, dampingFraction: 0.85)) {
                    centerStageMode = (centerStageMode == .livePreview) ? .thread : .livePreview
                }
            } label: {
                Label(centerStageMode == .livePreview ? "กลับสู่บทสนทนา" : "Live Preview",
                      systemImage: centerStageMode == .livePreview ? "globe.badge.chevron.backward" : "globe")
                    .foregroundStyle(centerStageMode == .livePreview ? Color.accentColor : Color.primary)
            }
            .help(centerStageMode == .livePreview ? "กลับสู่บทสนทนา (⌘⌥P)" : "เปิด Web Live Preview (⌘⌥P)")
        }
        ToolbarItem(placement: .primaryAction) {
            Button {
                withAnimation(.spring(response: 0.28, dampingFraction: 0.85)) {
                    showTerminalDrawer.toggle()
                }
            } label: {
                Label(showTerminalDrawer ? "ปิดเทอร์มินัล" : "เทอร์มินัล",
                      systemImage: showTerminalDrawer ? "terminal.fill" : "terminal")
                    .foregroundStyle(showTerminalDrawer ? Color.accentColor : Color.primary)
            }
            .help("เปิด/ปิดเทอร์มินัลในตัว (⌃`)")
        }
        // Declared last so it sits at the far right, over the panel it opens.
        ToolbarItem(placement: .primaryAction) {
            Button { showInspector.toggle() } label: {
                Label("แผงข้อมูล", systemImage: "sidebar.right")
            }
            .help("เปิด/ปิดแผงคอนเท็กซ์ ที่แก้ไป ความจำ (⌘⌥I)")
        }
    }

    private var bodyLayout: some View {
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

    private var bodyObservers: some View {
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
        .onChange(of: model) { _, m in applyModel(m) }
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

    private var bodyModals: some View {
        bodyObservers
        .onReceive(AppCommandCenter.shared.fire) { run($0) }
        .onAppear { syncCommandState() }
        .onChange(of: engine.sessionID) { _, _ in syncCommandState() }
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

    // MARK: sidebar — open / start a session

    // A plain leading-aligned VStack instead of `Form`: Form's two-column
    // label|control layout was the cause of the earlier truncated labels —
    // there just isn't enough width in a sidebar for a label column plus a
    // control column plus a text field. Every label sits above its control
    // instead, and gets the sidebar's full width to wrap in.
    /// Hand-built rows, each one a single full-width button: what lights up
    /// on hover is exactly what takes the click. (A `List` here drew its
    /// selection pill and our row content at different sizes.)
    private var isScratchChat: Bool {
        workspace.contains("chorus_chats")
    }

    private var chatsSection: some View {
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
                .buttonStyle(.plain)
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

    private func startScratchChat() {
        let scratchDir = FileManager.default.temporaryDirectory.appendingPathComponent("chorus_chats").path
        try? FileManager.default.createDirectory(atPath: scratchDir, withIntermediateDirectories: true)
        workspace = scratchDir
        startSession()
    }

    private var sidebar: some View {
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

    private var sidebarContent: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                HStack(spacing: 5) {
                    Image(systemName: "magnifyingglass").font(.caption).foregroundStyle(.secondary)
                    TextField("ค้นหา session", text: $sessionQuery).textFieldStyle(.plain)
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
    private var toolsSection: some View {
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
    private var agentFooter: some View {
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

                            Text("\(readyAgentsCount) พร้อมใช้")
                                .font(.system(size: 11, weight: .medium))
                                .lineLimit(1)
                                .fixedSize()

                            Image(systemName: "chevron.up")
                                .font(.system(size: 8, weight: .bold))
                                .foregroundStyle(.tertiary)
                        }
                        .padding(.horizontal, 7)
                        .padding(.vertical, 4.5)
                        .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 7, style: .continuous))
                    }
                    .buttonStyle(.plain)
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
                .buttonStyle(.plain)
                .help("ตั้งค่า (⌘,)")
            }
            .padding(.horizontal, 10).padding(.vertical, 8)
        }
    }

    private var readyAgentsCount: Int {
        engine.agents.filter(\.ready).count
    }

    private var overallAgentsColor: Color {
        if engine.agents.isEmpty { return .secondary }
        let anyLimited = engine.accounts.filter(\.ready).contains(where: \.limited)
        if anyLimited { return .orange }
        return engine.agents.allSatisfy(\.ready) ? .green : .orange
    }

    private var providersPopoverContent: some View {
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
                    .buttonStyle(.plain)
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
            .buttonStyle(.plain)
        }
        .padding(12)
        .frame(width: 270)
    }

    private func shortAgentLabel(_ a: Engine.AgentInfo) -> String {
        switch a.id.lowercased() {
        case "antigravity", "agy": return "Agy"
        case "claude": return "Claude"
        case "codex": return "Codex"
        default: return a.label
        }
    }

    private func agentHelpTooltip(_ a: Engine.AgentInfo, accs: [Engine.Account], limited: Bool, worst: Double?) -> String {
        var desc = a.label
        if accs.count > 1 {
            desc += " (\(accs.count) บัญชี)"
        }
        if limited {
            desc += ": ทุกบัญชีติดลิมิต"
        } else if let worst {
            desc += ": บัญชีที่ว่างที่สุดใช้ไป \(Int(worst * 100))%"
        } else if a.ready {
            desc += ": พร้อมใช้งาน"
        } else {
            desc += ": \(a.detail)"
        }
        return desc
    }

    /// The agent whose model id is currently selected.
    private func agentFor(_ modelID: String) -> Engine.AgentInfo? {
        let base = ModelID(modelID).base
        return engine.agents.first { a in
            a.id == modelID || a.id == base || a.models.contains { m in
                m.id == modelID || ModelID(m.id).base == base
            }
        }
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
        // A draft, not a row: the session exists once the first message is sent.
        engine.openDraft(db: db, cwd: workspace)
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

    @ViewBuilder
    private func rowForEvent(_ ev: LedgerEvent, atts: [SessionAttachment], lastAgentMsg: Int64?) -> some View {
        let isUser = ev.actor == "user"
        let isLastAgentMsg = !engine.busy
            && !isUser
            && ev.kind == "msg"
            && ev.id == lastAgentMsg
        let th = isLastAgentMsg ? engine.lastThoughts : nil
        EventRow(event: ev,
                 attachments: atts,
                 agents: engine.agents,
                 thoughts: th,
                 onDeleteAttachment: engine.deleteAttachment,
                 // Echoed rows (negative seq) aren't in the ledger yet.
                 onRewind: isUser && ev.kind == "msg" && ev.seq > 0
                     ? { code in rewindChat(before: ev.seq, code: code) } : nil)
            .id(ev.seq)
    }

    private var thread: some View {
        Group {
            if let toolPage {
                ToolPage(toolID: toolPage, workspace: workspace, onClose: { self.toolPage = nil })
                    .id(toolPage)
            } else if engine.sessionID < 0 && engine.draft == nil {
                emptyState
            } else if engine.events.isEmpty && !engine.busy {
                sessionWelcomeView
            } else {
                GeometryReader { viewport in
                    ScrollViewReader { proxy in
                        ScrollView {
                            LazyVStack(alignment: .leading, spacing: 14) {
                                // Looked up once per render, not once per row.
                                let attsBySeq = Dictionary(grouping: engine.attachments.filter { $0.event_seq != nil },
                                                           by: { $0.event_seq! })
                                let lastAgentMsg = engine.events.last(where: { $0.actor != "user" && $0.kind == "msg" })?.id
                                ForEach(engine.events) { ev in
                                    rowForEvent(ev, atts: attsBySeq[ev.seq] ?? [], lastAgentMsg: lastAgentMsg)
                                }
                                if engine.busy {
                                    WorkingRow(stream: engine.liveStream,
                                               agentID: engine.liveAgent ?? currentAgentID,
                                               agentLabel: engine.agents.first(where: { $0.id == (engine.liveAgent ?? currentAgentID) })?.label ?? "Agent",
                                               startedAt: engine.turnStartedAt,
                                               onStop: { engine.cancel(); queued.removeAll() },
                                               onGrow: {
                                                   if engine.busy && chatPinned {
                                                       proxy.scrollTo("chatBottom", anchor: .bottom)
                                                   }
                                               })
                                        .id("working")
                                }
                                orchestraStatusSection
                                Color.clear.frame(height: 1).id("chatBottom")
                            }
                            .frame(maxWidth: 820)
                            .frame(maxWidth: .infinity)
                            .padding(.horizontal, 20).padding(.vertical, 16)
                            .background(GeometryReader { g in
                                let f = g.frame(in: .named("chatScroll"))
                                Color.clear.preference(key: ChatContentFrameKey.self,
                                                       value: ChatContentFrame(minY: f.minY, maxY: f.maxY))
                            })
                        }
                        .coordinateSpace(name: "chatScroll")
                        .modifier(OpenAtBottom())
                        // A fresh scroll view per session, so it opens on the
                        // latest message instead of keeping the last one's offset.
                        .id(engine.sessionID)
                        .onPreferenceChange(ChatContentFrameKey.self) { f in
                            // Only an offset change (the reader or us scrolling)
                            // decides pinning; growth at the bottom moves maxY
                            // alone and must not unpin a reader who is following.
                            guard abs(f.minY - chatLastMinY) > 0.5 else { return }
                            chatLastMinY = f.minY
                            let atBottom = f.maxY - viewport.size.height < 80
                            if atBottom != chatPinned { chatPinned = atBottom }
                        }
                        .onChange(of: engine.sessionID) { _, _ in chatPinned = true }
                        .onChange(of: engine.events.count) { _, _ in
                            guard chatPinned else { return }
                            if reduceMotion {
                                proxy.scrollTo("chatBottom", anchor: .bottom)
                            } else {
                                withAnimation(.spring(response: 0.28, dampingFraction: 0.86)) {
                                    proxy.scrollTo("chatBottom", anchor: .bottom)
                                }
                            }
                        }
                        .onChange(of: engine.busy) { _, b in
                            // Sending is a request to see the reply: re-pin.
                            if b {
                                chatPinned = true
                                withAnimation(.spring(response: 0.3, dampingFraction: 0.85)) {
                                    proxy.scrollTo("chatBottom", anchor: .bottom)
                                }
                            }
                        }
                        .chorusScrollIndicators()
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        // Floating composer: the thread scrolls underneath it instead of
        // stopping at a divider, the way Claude/Codex/ChatGPT do.
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if toolPage == nil {
                composer
                    .frame(maxWidth: 820)
                    .padding(.horizontal, 16).padding(.bottom, 14).padding(.top, 4)
            }
        }
        .overlay(alignment: .top) {
            VStack(spacing: 8) {
                if isScratchChat {
                    HStack(spacing: 8) {
                        Image(systemName: "bubble.left.and.bubble.right.fill")
                            .foregroundStyle(Color.accentColor)
                        Text("แชทลอย")
                            .font(.caption.weight(.medium))
                        Spacer()
                        if let p = projects.projects.first, let firstFolder = p.folders.first {
                            Button("ผูกเข้ากับโปรเจกต์ \(p.name)") {
                                workspace = firstFolder
                                startSession()
                            }
                            .buttonStyle(.borderedProminent)
                            .controlSize(.small)
                            .help("ย้ายบทสนทนานี้เข้าไปในโปรเจกต์ \(p.name)")
                        }
                    }
                    .padding(.horizontal, 12).padding(.vertical, 7)
                    .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(Color.accentColor.opacity(0.2), lineWidth: 1))
                }
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
                }
                setupCard
            }
            .frame(maxWidth: 820)
            .padding(.horizontal, 16).padding(.top, 8)
            .animation(.spring(response: 0.35, dampingFraction: 0.85), value: setupNeed != nil)
            .animation(.spring(response: 0.35, dampingFraction: 0.85), value: engine.lastError != nil)
        }
        .frame(minWidth: 420)
        .clipShape(RoundedRectangle(cornerRadius: 20))
        .onDrop(of: [UTType.fileURL, UTType.url, UTType.image], isTargeted: nil, perform: importDroppedProviders)
    }

    @ViewBuilder
    private var orchestraStatusSection: some View {
        if !engine.checklist.isEmpty {
            PlanChecklistView(
                items: engine.checklist,
                askBeforePlan: askBeforePlan,
                isPlanApproved: !askBeforePlan || (engine.sessionID >= 0 && planApprovedSessions.contains(engine.sessionID)),
                onStartPlan: {
                    if engine.sessionID >= 0 {
                        planApprovedSessions.insert(engine.sessionID)
                        let currentSubs = engine.subagentsByParent[engine.sessionID] ?? (engine.subagentsByDB[dbPath] ?? [])
                        let waitingSubs = currentSubs.filter { $0.status == "waiting" && $0.kind == "chorus" }
                        for sub in waitingSubs {
                            engine.resumeSubagent(db: dbPath, id: sub.id)
                        }
                    }
                },
                onStop: {
                    if engine.sessionID >= 0 {
                        engine.cancelOrchestration(db: dbPath, parentSess: engine.sessionID)
                    }
                    engine.cancel()
                    queued.removeAll()
                }
            )
            .id("planChecklist")
        }
        let currentSubs = (engine.sessionID >= 0 ? engine.subagentsByParent[engine.sessionID] : nil) ?? (engine.subagentsByDB[dbPath] ?? [])
        let waitingSubs = currentSubs.filter { $0.status == "waiting" && $0.kind == "chorus" }
        if !waitingSubs.isEmpty {
            CapPausePromptCard(
                subagents: waitingSubs,
                onContinue: { subId in
                    engine.resumeSubagent(db: dbPath, id: subId)
                },
                onStop: { subId in
                    engine.cancelSubagent(db: dbPath, id: subId)
                },
                onStopAll: {
                    if engine.sessionID >= 0 {
                        engine.cancelOrchestration(db: dbPath, parentSess: engine.sessionID)
                    }
                    engine.cancel()
                    queued.removeAll()
                }
            )
            .id("capPause")
        }
    }

    private var emptyState: some View {
        VStack(spacing: 10) {
            Spacer()
            Image(systemName: "bubble.left.and.bubble.right")
                .font(.system(size: 36)).foregroundStyle(.tertiary)
            Text("ยังไม่ได้เปิด session").font(.title3.weight(.semibold))
            Text("เลือกโฟลเดอร์โปรเจกต์ทางซ้าย แล้วกดเริ่ม session")
                .font(.callout).foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var sessionWelcomeView: some View {
        VStack(spacing: 16) {
            Spacer()
            VStack(spacing: 8) {
                ZStack {
                    Circle()
                        .fill(Color.accentColor.opacity(0.12))
                        .frame(width: 58, height: 58)
                    Image(systemName: "sparkles")
                        .font(.system(size: 26, weight: .medium))
                        .foregroundStyle(Color.accentColor)
                }
                Text("พร้อมเริ่มงานใน session นี้")
                    .font(.title3.weight(.semibold))
                Text("สั่งงาน ถามคำถามเกี่ยวกับโค้ด หรือเลือกแนวทางเริ่มต้นด้านล่าง")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }

            HStack(spacing: 8) {
                welcomePromptPill(title: "สรุปการเปลี่ยนแปลงล่าสุด", icon: "clock.arrow.circlepath") {
                    draftBox.text = "สรุปการเปลี่ยนแปลงล่าสุดในโปรเจกต์ให้หน่อยว่ามีอะไรบ้าง"
                }
                welcomePromptPill(title: "อธิบายโครงสร้างโค้ด", icon: "folder.badge.gearshape") {
                    draftBox.text = "ช่วยอธิบายโครงสร้างและการทำงานหลักของโปรเจกต์นี้ให้หน่อย"
                }
                welcomePromptPill(title: "ตรวจสอบโค้ดหาจุดบกพร่อง", icon: "checkmark.shield") {
                    draftBox.text = "ช่วยตรวจสอบโค้ดในโปรเจกต์ว่ามีจุดไหนที่ควรปรับปรุงหรือมีบั๊กซ่อนอยู่ไหม"
                }
            }
            .padding(.top, 6)
            Spacer()
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .transition(.opacity)
    }

    private func welcomePromptPill(title: String, icon: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 6) {
                Image(systemName: icon).font(.caption).foregroundStyle(Color.accentColor)
                Text(title).font(.caption.weight(.medium))
            }
            .padding(.horizontal, 12).padding(.vertical, 8)
            .background(Color(nsColor: .controlBackgroundColor).opacity(0.6), in: Capsule())
            .overlay(Capsule().stroke(Color.primary.opacity(0.08), lineWidth: 1))
        }
        .buttonStyle(.plain)
    }

    @ViewBuilder
    private var composer: some View {
        let currentSub = engine.subagentsByDB[dbPath(for: workspace)]?.first { $0.child_sess == engine.sessionID }
        if currentSub?.kind == "provider" {
            HStack(spacing: 12) {
                Image(systemName: "eye.fill")
                    .foregroundStyle(.secondary)
                    .font(.system(size: 14))
                VStack(alignment: .leading, spacing: 2) {
                    Text("Provider subagent (อ่านอย่างเดียว)")
                        .font(.caption.weight(.semibold))
                    Text("การสนทนาของ subagent ฝั่ง Provider บันทึกเพื่อให้อ่านย้อนหลัง ไม่สามารถส่งข้อความโดยตรงได้")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                if engine.busy {
                    Button(role: .destructive) {
                        engine.cancel()
                        queued.removeAll()
                    } label: {
                        Label("หยุดเทิร์นของ agent หลัก", systemImage: "stop.circle.fill")
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
                }
            }
            .padding(12)
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).stroke(Color.primary.opacity(0.1), lineWidth: 1))
            .shadow(color: .black.opacity(0.06), radius: 8, y: 3)
        } else {
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
            if let routeStatus {
                RouteStatusCard(status: routeStatus) { self.routeStatus = nil }
                    .transition(.opacity)
            }
            if let hint { Text(hint).font(.caption).foregroundStyle(.orange).frame(maxWidth: .infinity, alignment: .leading) }
            if let planAgents = engine.pendingPlanAgents, !engine.busy {
                planApprovalBar(agents: planAgents)
            }
            ComposerField(box: draftBox, suggestions: suggest, onSubmit: submit,
                          onImportURLs: importFiles,
                          onPasteImage: { data, mime in importPastedImage(data, mime: mime) },
                          onRemoveAttachment: { id in
                              draftBox.attachments.removeAll { $0.id == id }
                              AttachmentThumbnails.shared.forget(id)
                              engine.deleteAttachment(id)
                          },
                          focusToken: composerFocus,
                          isBusy: engine.busy,
                          canSend: engine.sessionID >= 0 || engine.draft != nil,
                          onStop: { engine.cancel(); queued.removeAll() },
                          routeNoteEnabled: currentSessionMode == .autoRoute) {
                ComposerModeMenu(mode: Binding(
                    get: { currentSessionMode },
                    set: { currentSessionMode = $0 }
                )) {
                    settingsSection = .orchestration
                    showSettings = true
                }
                if currentSessionMode == .autoRoute, !engine.routePresets.isEmpty {
                    ChipPicker(
                        icon: "gauge.with.dots.needle.50percent",
                        value: engine.routePreset,
                        options: engine.routePresets.map(\.id),
                        labels: Dictionary(uniqueKeysWithValues: engine.routePresets.map { ($0.id, $0.label) }),
                        selected: engine.routePreset
                    ) { engine.setRoutePreset($0) }
                    .help(engine.routePresets.first { $0.id == engine.routePreset }.map { "\($0.label) — \($0.hint)" } ?? "")
                }
                ChipPicker(
                    icon: permission == .ask ? "hand.raised" : "bolt.fill",
                    value: "\(permission.rawValue)",
                    options: PermissionLevel.allCases.map { "\($0.rawValue)" },
                    labels: Dictionary(uniqueKeysWithValues: PermissionLevel.allCases.map { ("\($0.rawValue)", $0.label) }),
                    selected: "\(permission.rawValue)"
                ) { v in if let p = PermissionLevel(rawValue: Int32(v) ?? 0) { permission = p } }
                .help("\(permission.label) — \(permission.hint)")
                if currentSessionMode != .orchestra {
                    Button { planFirst.toggle() } label: {
                        CompactLabel(text: "วางแผนก่อน", spacing: 3) {
                            Image(systemName: planFirst ? "list.bullet.clipboard.fill" : "list.bullet.clipboard")
                        }
                        .font(.caption.weight(.medium))
                        .foregroundStyle(planFirst ? Color.accentColor : .secondary)
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 7, padding: .init(top: 4, leading: 8, bottom: 4, trailing: 8)))
                    .help(planFirst
                          ? "วางแผนก่อน: เปิดอยู่ — agent อ่านอย่างเดียวแล้วตอบเป็นแผน ยังไม่แก้อะไรจนกว่าจะกด ทำตามแผน"
                          : "วางแผนก่อน: ให้ agent ตอบเป็นแผน (อ่านอย่างเดียว) ก่อนลงมือ")
                }
            } trailing: {
                if currentSessionMode == .orchestra {
                    let orchLabel = configuredOrchestrator.isEmpty ? "Claude Opus 5.5" : ModelID(configuredOrchestrator).base
                    Button {
                        settingsSection = .orchestration
                        showSettings = true
                    } label: {
                        CompactLabel(text: "นำโดย \(orchLabel)") {
                            Image(systemName: "crown.fill")
                                .font(.system(size: 9))
                                .foregroundStyle(.yellow)
                        }
                        .font(.caption.weight(.medium))
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 7, padding: .init(top: 4, leading: 8, bottom: 4, trailing: 8)))
                    .help("นำโดย \(orchLabel) — Orchestra: วางแผนและแจกงานให้ agent ย่อย (คลิกเพื่อตั้งค่าโมเดล)")
                } else if currentSessionMode == .autoRoute {
                    Button { currentSessionMode = .manual } label: {
                        CompactLabel(text: "router เลือก", spacing: 3) {
                            Image(systemName: "arrow.triangle.branch")
                        }
                        .font(.caption.weight(.medium))
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 7, padding: .init(top: 4, leading: 8, bottom: 4, trailing: 8)))
                    .help("Auto route: router เสนอเอเจนต์และโมเดลตอนส่ง — คลิกเพื่อเลือกโมเดลเอง")
                } else {
                    ModelPicker(model: $model, onNeedsLogin: { settingsSection = .agents; showSettings = true })
                }
            }
        }
        .padding(.horizontal, 14).padding(.top, 12).padding(.bottom, 10)
        .floatingPanel(cornerRadius: 22)
        }
    }

    /// `@codex fix this` → ("codex's default model", "fix this"). Matches agent
    /// id or label, case-insensitively; anything else is sent untouched.
    private func route(_ text: String) -> (model: String, text: String) {
        guard text.hasPrefix("@") else { return (model, text) }
        let parts = text.dropFirst().split(separator: " ", maxSplits: 1, omittingEmptySubsequences: true)
        guard let tag = parts.first?.lowercased(),
              let a = engine.agents.first(where: {
                  $0.ready && ($0.id.lowercased() == tag
                               || $0.label.lowercased() == tag
                               || ($0.id == "antigravity" && tag == "agy"))
              })
        else { return (model, text) }
        return (a.defaultModel, parts.count > 1 ? String(parts[1]) : "")
    }

    private func ensureSessionReady() -> Bool {
        if engine.sessionID >= 0 || engine.draft != nil { return true }
        if workspace.isEmpty {
            if let p = projects.projects.first(where: { !$0.folders.isEmpty }),
               let f = p.folders.first {
                workspace = f
            }
        }
        if !workspace.isEmpty && !engine.busy {
            startSession()
            return true
        }
        return false
    }

    /// Menu bar → window. The menu (`ChorusCommands`) owns every shortcut;
    /// this only maps a command onto the same state the toolbar buttons flip.
    private func run(_ command: AppCommand) {
        let spring = Animation.spring(response: 0.28, dampingFraction: 0.85)
        switch command {
        case .newSession:
            if workspace.isEmpty || engine.busy {
                editing = ChorusProject(name: "", folders: []); editingIsNew = true
            } else { startSession() }
        case .settings: showSettings = true
        case .commandPalette: showPalette = true
        case .searchSessions: showSearchSheet = true
        case .toggleSidebar: showSidebar.toggle()
        case .toggleExplorer: showExplorer.toggle()
        case .toggleInspector: showInspector.toggle()
        case .inspectorTab(let i):
            guard RightTab.allCases.indices.contains(i) else { return }
            rightTab = RightTab.allCases[i]
            showInspector = true
        case .toggleOrchestrator:
            settingsSection = .orchestration
            showSettings = true
        case .toggleLivePreview:
            withAnimation(spring) { centerStageMode = centerStageMode == .livePreview ? .thread : .livePreview }
        case .toggleTerminal:
            withAnimation(spring) { showTerminalDrawer.toggle() }
        case .toggleFocusMode:
            // One meaning: leave whatever panel is maximized, else focus the thread.
            withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                maximizedPanel = maximizedPanel == .none ? .thread : .none
            }
        case .send:
            // A sheet (project editor, settings) owns ⌘↩ while it is up.
            guard NSApp.mainWindow?.attachedSheet == nil else { return }
            submit()
        case .stop:
            guard engine.busy else { return }
            engine.cancel(); queued.removeAll()
        case .compareAgents:
            runCompare()
        case .chooseModel, .nextEffort, .sendReview, .openInEditor:
            break // handled by ModelPicker / DiffReviewerView / CodeViewerView
        case .systemMonitor:
            if !showSidebar || maximizedPanel != .none {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                    maximizedPanel = .none
                    showSidebar = true
                }
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) {
                    showSystemMonitorPopover.toggle()
                }
            } else {
                showSystemMonitorPopover.toggle()
            }
        }
    }

    /// "claude@work" → "Claude (work)"; falls back to the raw name.
    private func askingAgentLabel(_ name: String) -> String {
        let parts = name.split(separator: "@", maxSplits: 1).map(String.init)
        let id = parts.first ?? name
        let label = engine.agents.first { $0.id == id }?.label ?? id
        return parts.count > 1 ? "\(label) (\(parts[1]))" : label
    }

    private func syncCommandState() {
        let c = AppCommandCenter.shared
        c.hasSession = engine.sessionID >= 0
        c.busy = engine.busy
        c.fileOpen = selectedFile != nil
        c.reviewVisible = (showInspector && rightTab == .changes) || maximizedPanel == .diffReviewer
        c.focusMode = maximizedPanel != .none
    }

    private func importFiles(_ urls: [URL]) {
        // Attachments are stored per session, so attaching ends the draft.
        if engine.sessionID < 0 && engine.draft != nil {
            engine.commitDraft { applyModel(model); importFiles(urls) }
            return
        }
        guard ensureSessionReady() && engine.sessionID >= 0 else {
            hint = workspace.isEmpty ? "สร้างหรือเลือกโปรเจกต์ก่อนแนบไฟล์" : "เปิด session ก่อนแนบไฟล์"
            return
        }
        let room = max(0, 5 - draftBox.attachments.count - importingFiles)
        guard urls.count <= room else { hint = "แนบได้สูงสุด 5 ไฟล์ต่อข้อความ"; return }
        importingFiles += urls.count
        for url in urls {
            engine.importFile(url) { attachment, error in
                importingFiles -= 1
                if let attachment {
                    draftBox.attachments.append(attachment)
                    AttachmentThumbnails.shared.load(id: attachment.id, url: url)
                } else if let error {
                    let size = (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize).map(Int64.init)
                    draftBox.rejected.append(RejectedAttachment(name: url.lastPathComponent, size: size, reason: error))
                }
            }
        }
    }

    private func importPastedImage(_ data: Data, mime: String) {
        if engine.sessionID < 0 && engine.draft != nil {
            engine.commitDraft { applyModel(model); importPastedImage(data, mime: mime) }
            return
        }
        guard ensureSessionReady() && engine.sessionID >= 0 else {
            hint = workspace.isEmpty ? "สร้างหรือเลือกโปรเจกต์ก่อนแนบรูป" : "เปิด session ก่อนแนบรูป"
            return
        }
        guard draftBox.attachments.count + importingFiles < 5 else { hint = "แนบได้สูงสุด 5 ไฟล์ต่อข้อความ"; return }
        importingFiles += 1
        engine.importImage(data, mime: mime) { attachment, error in
            importingFiles -= 1
            if let attachment {
                draftBox.attachments.append(attachment)
                AttachmentThumbnails.shared.load(id: attachment.id, data: data)
            }
            if let error { hint = error }
        }
    }

    private func importDroppedProviders(_ providers: [NSItemProvider]) -> Bool {
        var handled = false
        for provider in providers {
            if provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier)
                || provider.hasItemConformingToTypeIdentifier(UTType.url.identifier) {
                handled = true
                let type = provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier)
                    ? UTType.fileURL.identifier : UTType.url.identifier
                provider.loadItem(forTypeIdentifier: type, options: nil) { item, _ in
                    let url = (item as? URL) ?? (item as? NSURL).map { $0 as URL } ?? (item as? Data).flatMap { URL(dataRepresentation: $0, relativeTo: nil) }
                    if let url { DispatchQueue.main.async { self.importFiles([url]) } }
                }
            } else if let imageType = provider.registeredTypeIdentifiers.first(where: { UTType($0)?.conforms(to: .image) == true }) {
                handled = true
                provider.loadDataRepresentation(forTypeIdentifier: imageType) { data, _ in
                    guard let data else { return }
                    let mime = UTType(imageType)?.preferredMIMEType ?? "image/png"
                    DispatchQueue.main.async { self.importPastedImage(data, mime: mime) }
                }
            }
        }
        return handled
    }

    private func submit() {
        let raw = draftBox.text.trimmingCharacters(in: .whitespacesAndNewlines)
        let (target, text) = route(raw)
        guard engine.sessionID >= 0 || engine.draft != nil,
              !raw.isEmpty || !draftBox.attachments.isEmpty else { return }
        guard importingFiles == 0 else { hint = "รอแนบไฟล์ให้เสร็จก่อนส่ง"; return }
        if text.isEmpty && draftBox.attachments.isEmpty {
            hint = "พิมพ์ข้อความต่อจาก \(raw) ด้วย"
            return
        }
        guard draftBox.attachments.allSatisfy(\.agentReadable) else {
            hint = "มีไฟล์ที่ agent ยังอ่านไม่ได้ กรุณาลบไฟล์นั้นก่อนส่ง"
            return
        }
        guard draftBox.attachments.reduce(Int64(0), { $0 + $1.size }) <= 50 * 1024 * 1024 else {
            hint = "ไฟล์แนบรวมต้องไม่เกิน 50 MB"
            return
        }
        hint = nil
        draftBox.text = ""
        composerFocus &+= 1
        let explicitTag = raw.hasPrefix("@")
            && engine.agents.contains { candidate in
                let tag = raw.dropFirst().split(separator: " ", maxSplits: 1).first.map(String.init) ?? ""
                return candidate.ready && (candidate.id.caseInsensitiveCompare(tag) == .orderedSame
                    || candidate.label.caseInsensitiveCompare(tag) == .orderedSame
                    || (candidate.id == "antigravity" && tag.caseInsensitiveCompare("agy") == .orderedSame))
            }
        let send = PendingSend(model: target,
                               text: text.isEmpty ? "ช่วยดูไฟล์ที่แนบมา" : text,
                               attachments: draftBox.attachments,
                               explicitAgent: explicitTag)
        draftBox.attachments = []
        draftBox.rejected = []
        if engine.sessionID < 0 && engine.draft != nil {
            // First message: the draft becomes a session now. The model was
            // picked with no engine to tell, so pin it before sending.
            engine.commitDraft { applyModel(model); routeOrDispatch(send) }
            return
        }
        // Busy: line it up behind the running turn (Cursor-style queue)
        // instead of dropping the keystroke on a disabled button.
        if engine.busy || routingInProgress || pendingRoute != nil { queued.append(send); return }
        routeOrDispatch(send)
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

    private func routeOrDispatch(_ send: PendingSend) {
        if currentSessionMode == .orchestra && !send.explicitAgent {
            // Rust picks the lead and its backup, and runs it read-only.
            lastPrompt = send.text
            engine.sendOrchestra(send.text, primary: configuredOrchestrator,
                                 backup: configuredOrchestratorBackup,
                                 concurrency: orchestraConcurrency,
                                 maxChildren: orchestraMaxChildren,
                                 maxMinutes: orchestraMaxMinutes,
                                 level: permission.rawValue,
                                 attachments: send.attachments.map(\.id))
            return
        }
        guard currentSessionMode == .autoRoute, !send.explicitAgent else {
            engine.recordLearnedRoute(
                workspace: workspace,
                prompt: send.text,
                model: send.model,
                agent: agentFor(send.model)?.id ?? "agent",
                reason: send.explicitAgent ? "ผู้ใช้ระบุ agent เจาะจง" : "ผู้ใช้เลือกโมเดลเองโดยตรง"
            )
            dispatch(send.model, send.text, attachments: send.attachments.map(\.id))
            return
        }
        let integrations = engine.catalog.filter(\.isEnabledForProject).map(\.id)
        let candidates = engine.agents.filter(\.ready).flatMap { agent in
            agent.models.map { choice -> [String: Any] in
                ["id": "\(agent.id):\(choice.id)", "agent": agent.id,
                 "model": ModelID(choice.id).base,
                 "effort": ModelID(choice.id).effort ?? "default",
                 "ready": true, "media_types": ["text/*", "image/*"],
                 "integrations": integrations]
            }
        }
        let task: [String: Any] = [
            "text": send.text,
            "attachments": send.attachments.map { file -> [String: Any] in
                ["media_type": file.mediaType.hasPrefix("image/") ? file.mediaType : "text/plain",
                 "size_bytes": file.size]
            },
            "enabled_integrations": integrations, "required_integrations": []
        ]
        guard let taskData = try? JSONSerialization.data(withJSONObject: task),
              let candidatesData = try? JSONSerialization.data(withJSONObject: ["candidates": candidates]),
              let taskJSON = String(data: taskData, encoding: .utf8),
              let candidatesJSON = String(data: candidatesData, encoding: .utf8),
              !routerModel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            reviewModel = send.model
            pendingRoute = PendingRoute(send: send, reason: "ตั้งค่าโมเดล router ในตั้งค่าก่อน หรือเลือกโมเดลเอง")
            return
        }
        let conserveEnergy = PowerManager.shared.shouldConserveEnergyForInference
        let effectiveRouterModel = conserveEnergy ? "in-process-heuristic" : routerModel
        routingInProgress = true
        routeStatus = .thinking(routerModel: effectiveRouterModel, candidates: candidates.count)
        let session = engine.sessionID
        engine.proposeRoute(taskJSON: taskJSON, candidatesJSON: candidatesJSON, routerModel: effectiveRouterModel) { json in
            routingInProgress = false
            guard session == engine.sessionID else { routeStatus = nil; return }
            let response = try? JSONDecoder().decode(RouterEnvelope.self, from: Data(json.utf8))
            if response?.ok == true, response?.route?.status == "proposal",
               let chosenID = response?.route?.result?.candidate_id,
               let chosen = engine.agents.flatMap({ agent in
                   agent.models.map { choice in ("\(agent.id):\(choice.id)", choice.id, agent.id) }
               }).first(where: { $0.0 == chosenID }) {
                var chosenReason = response?.route?.result?.reason ?? "เหมาะกับงานนี้"
                if conserveEnergy {
                    chosenReason += " [โหมดประหยัดแบตเตอรี่]"
                }
                let id = ModelID(chosen.1)
                routeStatus = .chosen(
                    agent: engine.agents.first(where: { $0.id == chosen.2 })?.label ?? chosen.2,
                    model: id.base,
                    effort: id.effort ?? "ค่าเริ่มต้น",
                    preset: engine.routePresetLabel,
                    reason: chosenReason
                )
                // The router's own pick is not learned: only user overrides are.
                dispatch(chosen.1, send.text,
                         attachments: send.attachments.map(\.id),
                         agentOverride: chosen.2)
            } else {
                routeStatus = nil
                reviewModel = send.model
                pendingRoute = PendingRoute(send: send,
                    reason: response?.route?.result?.message ?? response?.error
                        ?? "Router ในเครื่องยังเลือกไม่ได้ กรุณาเลือกโมเดลและยืนยันส่งเอง")
            }
        }
    }

    private func sendReviewedRoute() {
        guard let pendingRoute else { return }
        self.pendingRoute = nil
        routeStatus = nil
        engine.recordLearnedRoute(
            workspace: workspace,
            prompt: pendingRoute.send.text,
            model: reviewModel,
            agent: agentFor(reviewModel)?.id ?? "agent",
            reason: "ผู้ใช้ยืนยันและเลือกโมเดลนี้เอง"
        )
        dispatch(reviewModel, pendingRoute.send.text,
                 attachments: pendingRoute.send.attachments.map(\.id),
                 agentOverride: nil)
    }

    private func cancelReviewedRoute() {
        guard let pendingRoute else { return }
        routeStatus = nil
        draftBox.text = pendingRoute.send.text + (draftBox.text.isEmpty ? "" : "\n" + draftBox.text)
        draftBox.attachments.insert(contentsOf: pendingRoute.send.attachments, at: 0)
        self.pendingRoute = nil
        flushQueue()
    }

    private func dispatch(_ target: String, _ text: String, attachments: [Int64] = [],
                          agentOverride: String? = nil) {
        lastPrompt = text
        let agentID = agentOverride ?? agentFor(target)?.id ?? target
        // `@codex …` picked another agent's default model; pin it for the turn.
        if target != model {
            model = target
            engine.setModel(agent: agentID, model: target)
        }
        engine.liveAgent = agentID
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
        if planFirst {
            // Read-only whatever `permission` says; enforced in Rust.
            engine.sendPlan(text, agents: names.joined(separator: ","), attachments: attachments)
        } else if permission == .ask {
            engine.sendInteractive(text, agent: names.first ?? agentID, attachments: attachments)
        } else {
            // Always the relay path, even for one agent: a chain of one
            // behaves exactly like a single turn, and the user never has
            // to know the word "relay" to get failover.
            engine.sendRelay(text, agents: names.joined(separator: ","), level: permission.rawValue,
                             attachments: attachments)
        }
    }

    /// Shown under a finished plan: approve it with the composer's own
    /// permission, or just type to change it (any message clears the bar).
    private func planApprovalBar(agents: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "list.bullet.clipboard").foregroundStyle(Color.accentColor)
            Text("แผนพร้อมแล้ว — กดทำตามแผน หรือพิมพ์บอกว่าจะแก้ตรงไหน")
                .font(.caption)
                .lineLimit(2)
            Spacer()
            Button("ทำตามแผน") { approvePlan(agents: agents) }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
                .help("ส่งให้ agent ลงมือตามแผน ด้วยสิทธิ์ \(permission.label)")
            Button { engine.pendingPlanAgents = nil } label: { Image(systemName: "xmark") }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .help("ไม่ทำตามแผนนี้")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(Color.accentColor.opacity(0.08), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }

    private func approvePlan(agents: String) {
        let text = engine.planApprovedPrompt
        guard !text.isEmpty else { return }
        engine.pendingPlanAgents = nil
        if permission == .ask {
            let first = agents.split(separator: ",").first.map(String.init) ?? agents
            engine.sendInteractive(text, agent: first)
        } else {
            engine.sendRelay(text, agents: agents, level: permission.rawValue)
        }
    }

    /// Right-click › ย้อน on one of the user's messages: continue from just
    /// before it in a fork, with the message back in the composer to edit.
    private func rewindChat(before seq: Int64, code: Bool) {
        engine.rewind(beforeSeq: seq, code: code, conversation: true, db: dbPath) { text, error in
            if let error {
                engine.lastError = error
                return
            }
            if let text {
                draftBox.text = text
                composerFocus &+= 1
            }
        }
    }

    private func flushQueue() {
        guard !engine.busy, !routingInProgress, pendingRoute == nil,
              !queued.isEmpty, engine.lastError == nil else { return }
        let next = queued.removeFirst()
        routeOrDispatch(next)
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
            if engine.agents.contains(where: { $0.ready && $0.id == "antigravity" }) && hit("agy") {
                out.append(Suggestion(title: "@agy", detail: "ส่งข้อความนี้ให้ Antigravity", icon: "person.crop.circle",
                                      insert: "@agy "))
            }
            out += fileIndex.match(query, limit: 8 - min(out.count, 3)).map { f in
                Suggestion(title: (f as NSString).lastPathComponent, detail: f, icon: "doc.text", insert: "@\(f) ")
            }
            return out
        }
        var out: [Suggestion] = []
        for a in engine.agents where a.ready && a.id != currentAgentID && engine.sessionID >= 0 {
            out.append(.init(title: "/handoff \(a.id)", detail: "ส่งงานต่อให้ \(a.label) พร้อม brief", icon: "arrowshape.turn.up.right.fill") {
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
        out.append(.init(title: "/new", detail: "session ใหม่ในโฟลเดอร์นี้", icon: "square.and.pencil") {
            if !workspace.isEmpty { startSession() }
        })
        out.append(.init(title: "/compare", detail: "ถามทุก agent แล้วเทียบ", icon: "square.split.2x1") { runCompare() })
        out.append(.init(title: "/snapshot", detail: "บันทึกจุดย้อนกลับตอนนี้", icon: "camera") { engine.snapshot() })
        out.append(.init(title: "/ask", detail: "ถามก่อนเขียนไฟล์", icon: "hand.raised") { permission = .ask })
        out.append(.init(title: "/auto", detail: "ให้ทำเลยไม่ต้องถาม", icon: "bolt.fill") { permission = .allowAll })
        out.append(.init(title: "/plan", detail: planFirst ? "ปิดวางแผนก่อน" : "วางแผนก่อนลงมือ (อ่านอย่างเดียว)",
                         icon: "list.bullet.clipboard") { planFirst.toggle() })
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
                items.append(.init(title: "ส่งต่องานให้ \(a.label)", icon: "arrowshape.turn.up.right.fill") {
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
        let hunks = engine.diffHunks
        let files = Dictionary(grouping: hunks, by: \.file)
        let order = engine.diffFiles
        return VStack(spacing: 0) {
            PaneHeader(title: "ที่แก้ไปใน session นี้",
                       subtitle: hunks.isEmpty ? nil
                           : "\(order.count) ไฟล์ · +\(hunks.map(\.addCount).reduce(0, +)) −\(hunks.map(\.delCount).reduce(0, +))") {
                Button { engine.snapshot() } label: { Image(systemName: "camera") }
                    .buttonStyle(InteractiveStyle(cornerRadius: 6, padding: .init(top: 4, leading: 6, bottom: 4, trailing: 6)))
                    .disabled(engine.sessionID < 0)
                    .help("บันทึกจุดย้อนกลับตอนนี้")
            }
            if hunks.isEmpty {
                EmptyPane(icon: "checkmark.seal", title: "ยังไม่มีไฟล์ถูกแก้",
                          hint: "เมื่อ agent แก้ไฟล์ จะเห็นทีละก้อนที่นี่ เก็บหรือตีกลับได้ทีละก้อน")
            } else {
                ScrollView {
                    // Lazy and flat: one row per file header and per hunk,
                    // so only what is on screen is built.
                    LazyVStack(alignment: .leading, spacing: 6) {
                        ForEach(order, id: \.self) { f in
                            let fileHunks = files[f] ?? []
                            FileChangesHeader(file: f, hunks: fileHunks, open: !collapsedFiles.contains(f)) {
                                if collapsedFiles.contains(f) { collapsedFiles.remove(f) } else { collapsedFiles.insert(f) }
                            }
                            if !collapsedFiles.contains(f) {
                                ForEach(fileHunks) { h in
                                    HunkView(hunk: h, rejected: rejected.contains(h.id)) {
                                        if rejected.contains(h.id) { rejected.remove(h.id) } else { rejected.insert(h.id) }
                                    }
                                }
                            }
                        }
                    }
                    .padding(10)
                }
                .chorusScrollIndicators()
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
                        Label("ย้อนทุกอย่างกลับไปตอนเริ่ม session", systemImage: "clock.arrow.circlepath")
                            .frame(maxWidth: .infinity)
                            .foregroundStyle(.red)
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 8))
                    .confirmationDialog(
                        "ทิ้งทุกอย่างที่ agent แก้ใน session นี้?",
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
                       subtitle: "ทุก agent ใน session นี้อ่านชุดเดียวกัน") { EmptyView() }
            if engine.facts.isEmpty && engine.pending.isEmpty {
                EmptyPane(icon: "brain", title: "ยังไม่มีความจำ",
                          hint: "agent เสนอเรื่องที่ควรจำผ่าน memory_propose แล้วจะมารอให้คุณอนุมัติที่นี่")
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
                .chorusScrollIndicators()
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

    private func checkSubagentNotifications(in dbSubs: [String: [Engine.SubagentInfo]]) {
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

    private func postNotification(id: String, title: String, body: String) {
        TurnNotice.post(id: id, title: title, body: body)
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

    private func selectProject(_ p: ChorusProject) {
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

    private func projectRow(_ p: ChorusProject) -> some View {
        let open = expanded.contains(p.id.uuidString) || !sessionQuery.isEmpty
        let rows = sessions(of: p).filter { matches($0.s) }
        let isCurrent = p.folders.contains(workspace)
        return VStack(alignment: .leading, spacing: 1) {
            HStack(spacing: 2) {
                Button { selectProject(p) } label: {
                    HStack(spacing: 7) {
                        Button { toggle(p) } label: {
                            Image(systemName: "chevron.right")
                                .font(.system(size: 9, weight: .bold)).foregroundStyle(.secondary)
                                .rotationEffect(.degrees(open ? 90 : 0))
                                .frame(width: 14, height: 14)
                        }
                        .buttonStyle(.plain)

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
    private func draftFolder(_ d: Engine.DraftInfo) -> String {
        ((d.db as NSString).deletingLastPathComponent as NSString).deletingLastPathComponent
    }

    /// The unsent new session: not in the store yet, so not in `rows`.
    private var draftRow: some View {
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
    private func confirmEmptyCleanup(_ p: ChorusProject) {
        let dbs = p.folders.map { dbPath(for: $0) }
        engine.emptySessionCount(dbs: dbs) { n in
            emptyCleanup = EmptyCleanup(project: p.name, dbs: dbs, count: n)
        }
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
    private func displayTitle(_ s: Engine.SessionSummary) -> String {
        let t = s.title.trimmingCharacters(in: .whitespaces)
        if !t.isEmpty && t != "session" { return t }
        return s.goal.isEmpty ? "session #\(s.id)" : s.goal
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
                            .buttonStyle(.plain)
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
                            let costText: String = {
                                if let rate = Double(thbRate.trimmingCharacters(in: .whitespaces)), rate > 0 {
                                    return String(format: "฿%.1f", cost * rate)
                                }
                                return String(format: "$%.3f", cost)
                            }()
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

    private var dbPath: String {
        (workspace as NSString).appendingPathComponent(".chorus/store.db")
    }

    private var contextPane: some View {
        VStack(spacing: 0) {
            PaneHeader(title: "คอนเท็กซ์", subtitle: "สิ่งที่ agent ถัดไปจะได้รับ") { EmptyView() }
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    let frac: Double? = {
                        guard let u = engine.contextUsed, let w = engine.contextWindow, w > 0 else { return nil }
                        return min(max(Double(u) / Double(w), 0.0), 1.0)
                    }()
                    // No reading is "ไม่ทราบ", never 0%: a session that has
                    // talked has used context even if the agent never said how much.
                    let hasTurns = engine.events.contains { $0.actor != "user" }
                    let percentText = frac.map { "\(Int($0 * 100))%" } ?? (hasTurns ? "ไม่ทราบ" : "0%")
                    let barColor: Color = {
                        guard let f = frac else { return .accentColor }
                        if f > 0.9 { return .red }
                        if f > 0.75 { return .orange }
                        return .accentColor
                    }()

                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            Label("ขนาดหน้าต่างบริบท", systemImage: "cpu")
                                .font(.subheadline.weight(.medium))
                            Spacer()
                            Text(percentText)
                                .font(.subheadline.weight(.semibold).monospacedDigit())
                                .foregroundStyle(barColor)
                        }

                        // Horizontal progress bar (Codex / Claude style)
                        GeometryReader { geo in
                            ZStack(alignment: .leading) {
                                Capsule()
                                    .fill(Color.primary.opacity(0.08))
                                    .frame(height: 7)
                                Capsule()
                                    .fill(barColor)
                                    .frame(width: max(geo.size.width * CGFloat(frac ?? 0), (frac != nil && frac! > 0) ? 6 : 0), height: 7)
                                    .animation(.easeOut(duration: 0.25), value: frac)
                            }
                        }
                        .frame(height: 7)

                        HStack {
                            if let u = engine.contextUsed, let w = engine.contextWindow {
                                Text("\(u.formatted()) / \(w.formatted()) tokens")
                                    .font(.caption.monospacedDigit())
                                    .foregroundStyle(.secondary)
                            } else if let u = engine.contextUsed {
                                Text("\(u.formatted()) tokens · ไม่ทราบขนาดหน้าต่าง")
                                    .font(.caption.monospacedDigit())
                                    .foregroundStyle(.secondary)
                            } else if hasTurns {
                                Text("agent นี้ไม่ได้รายงานจำนวน token")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            } else {
                                Text("ยังไม่มีเทิร์น (ขึ้นหลัง agent ตอบ)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            if let f = frac, f > 0.75 {
                                Label("ใกล้เต็ม — ส่งต่องานได้", systemImage: "exclamationmark.triangle.fill")
                                    .font(.caption2.weight(.medium))
                                    .foregroundStyle(.orange)
                            }
                        }
                    }
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                    // 2. 5-hour and Weekly Quota Bars (from limits.rs)
                    if let accountWithLimits = engine.accounts.first(where: { $0.rate_limits?.primary != nil || $0.rate_limits?.secondary != nil }),
                       let rl = accountWithLimits.rate_limits {
                        VStack(alignment: .leading, spacing: 8) {
                            HStack {
                                Label("โควตาการใช้งาน (\(accountWithLimits.agent.isEmpty ? accountWithLimits.name : accountWithLimits.agent))", systemImage: "chart.bar.xaxis")
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(.secondary)
                                Spacer()
                            }

                            if let p = rl.primary {
                                let pct = min(max(p.used_percent / 100.0, 0.0), 1.0)
                                VStack(alignment: .leading, spacing: 3) {
                                    HStack {
                                        Text("5 ชั่วโมง")
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)
                                        Spacer()
                                        Text("\(Int(p.used_percent))%")
                                            .font(.caption2.weight(.semibold).monospacedDigit())
                                            .foregroundStyle(p.used_percent >= 90 ? Color.red : (p.used_percent >= 75 ? Color.orange : Color.primary))
                                    }
                                    GeometryReader { g in
                                        ZStack(alignment: .leading) {
                                            Capsule().fill(Color.primary.opacity(0.08)).frame(height: 5)
                                            Capsule().fill(p.used_percent >= 90 ? Color.red : (p.used_percent >= 75 ? Color.orange : Color.accentColor))
                                                .frame(width: max(g.size.width * CGFloat(pct), pct > 0 ? 5 : 0), height: 5)
                                        }
                                    }
                                    .frame(height: 5)
                                    if let reset = p.resets_label {
                                        Text("รีเซ็ตใน \(reset)")
                                            .font(.system(size: 9.5))
                                            .foregroundStyle(.tertiary)
                                    }
                                }
                            }

                            if let s = rl.secondary {
                                let pct = min(max(s.used_percent / 100.0, 0.0), 1.0)
                                VStack(alignment: .leading, spacing: 3) {
                                    HStack {
                                        Text("รายสัปดาห์")
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)
                                        Spacer()
                                        Text("\(Int(s.used_percent))%")
                                            .font(.caption2.weight(.semibold).monospacedDigit())
                                            .foregroundStyle(s.used_percent >= 90 ? Color.red : (s.used_percent >= 75 ? Color.orange : Color.primary))
                                    }
                                    GeometryReader { g in
                                        ZStack(alignment: .leading) {
                                            Capsule().fill(Color.primary.opacity(0.08)).frame(height: 5)
                                            Capsule().fill(s.used_percent >= 90 ? Color.red : (s.used_percent >= 75 ? Color.orange : Color.accentColor))
                                                .frame(width: max(g.size.width * CGFloat(pct), pct > 0 ? 5 : 0), height: 5)
                                        }
                                    }
                                    .frame(height: 5)
                                    if let reset = s.resets_label {
                                        Text("รีเซ็ตใน \(reset)")
                                            .font(.system(size: 9.5))
                                            .foregroundStyle(.tertiary)
                                    }
                                }
                            }
                        }
                        .padding(10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Color.primary.opacity(0.03), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }

                    // 3. Session Tokens In / Out
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Label("Token ของ session", systemImage: "arrow.triangle.swap")
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                            Spacer()
                        }
                        HStack(spacing: 14) {
                            HStack(spacing: 4) {
                                Image(systemName: "arrow.down.right")
                                    .font(.system(size: 10))
                                    .foregroundStyle(.blue)
                                Text("In: \(engine.sessionTokensIn.formatted())")
                                    .font(.caption.monospacedDigit())
                            }
                            HStack(spacing: 4) {
                                Image(systemName: "arrow.up.right")
                                    .font(.system(size: 10))
                                    .foregroundStyle(.purple)
                                Text("Out: \(engine.sessionTokensOut.formatted())")
                                    .font(.caption.monospacedDigit())
                            }
                            Spacer()
                        }
                    }
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.primary.opacity(0.03), in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                    // 4. Monetary Cost Accounting Card (มูลค่าเทียบ API)
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Label("มูลค่าเทียบ API", systemImage: "dollarsign.circle.fill")
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.green)
                            Spacer()
                            Button {
                                if engine.sessionID >= 0 {
                                    engine.updateSessionCost(db: dbPath, id: engine.sessionID)
                                }
                            } label: {
                                Image(systemName: "arrow.clockwise")
                                    .font(.system(size: 10))
                                    .foregroundStyle(.secondary)
                            }
                            .buttonStyle(.plain)
                            .help("รีเฟรชค่าใช้จ่าย")
                        }
                        if let cost = engine.sessionCostUSD {
                            HStack(alignment: .firstTextBaseline, spacing: 6) {
                                Text(String(format: "$%.4f USD", cost))
                                    .font(.system(.body, design: .monospaced).weight(.bold))
                                    .foregroundStyle(cost > 0 ? Color.primary : Color.secondary)

                                if let rate = Double(thbRate.trimmingCharacters(in: .whitespaces)), rate > 0 {
                                    Text(String(format: "(≈ ฿%.2f)", cost * rate))
                                        .font(.caption.monospacedDigit())
                                        .foregroundStyle(.secondary)
                                } else {
                                    Button("ตั้งอัตราบาท") {
                                        settingsSection = .cost
                                        showSettings = true
                                    }
                                    .font(.caption)
                                    .buttonStyle(.link)
                                }
                                Spacer()
                            }
                            Text("รายงานจาก usage_update.cost ของ bridge")
                                .font(.caption2)
                                .foregroundStyle(.tertiary)
                        } else {
                            HStack {
                                Text("ไม่มีข้อมูลราคา")
                                    .font(.system(.body).weight(.medium))
                                    .foregroundStyle(.secondary)
                                Spacer()
                            }
                            Text("โมเดล/bridge นี้ไม่รายงานราคา API (เช่น บัญชี subscription หรือ Codex)")
                                .font(.caption2)
                                .foregroundStyle(.tertiary)
                        }
                    }
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.green.opacity(0.06), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(Color.green.opacity(0.18), lineWidth: 1))

                    // Compiler & Linter Verification Health Card (Autonomous Self-Healing Loop)
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            if engine.isVerifying {
                                ProgressView()
                                    .controlSize(.mini)
                                Text("กำลังตรวจสอบโค้ด...")
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(.secondary)
                            } else if let v = engine.lastVerification {
                                Image(systemName: v.passed ? "checkmark.seal.fill" : "exclamationmark.triangle.fill")
                                    .foregroundStyle(v.passed ? .green : .red)
                                    .font(.caption)
                                Text(v.passed ? "คอมไพล์ผ่าน" : "พบข้อผิดพลาด \(v.diagnostics.count) จุด")
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(v.passed ? .green : .red)
                            } else {
                                Image(systemName: "wrench.and.screwdriver")
                                    .foregroundStyle(.secondary)
                                    .font(.caption)
                                Text("การตรวจสอบโค้ดอัตโนมัติ")
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            Button {
                                engine.verifyWorkspace(ws: workspace)
                            } label: {
                                Image(systemName: "arrow.clockwise")
                                    .font(.system(size: 10))
                                    .foregroundStyle(.secondary)
                            }
                            .buttonStyle(.plain)
                            .disabled(engine.isVerifying || workspace.isEmpty || !engine.projectTrusted)
                            .help("รัน cargo check, biome, ruff หรือ swiftlint ในโปรเจกต์ทันที (ใน sandbox)")
                        }

                        Toggle(isOn: Binding(
                            get: { engine.projectTrusted },
                            set: { engine.setProjectTrusted(ws: workspace, $0) }
                        )) {
                            Text("เชื่อถือโปรเจกต์นี้ — รันตรวจโค้ด/hooks หลังทุกเทิร์น")
                                .font(.caption2)
                        }
                        .toggleStyle(.switch)
                        .controlSize(.mini)
                        .disabled(workspace.isEmpty)
                        .help("build.rs, hooks และ linter คือโค้ดของโปรเจกต์ เปิดเฉพาะโปรเจกต์ที่ไว้ใจ — ถึงเปิดก็รันใน sandbox เดียวกับเทิร์น")

                        if let v = engine.lastVerification, !v.passed {
                            VStack(alignment: .leading, spacing: 4) {
                                ForEach(v.diagnostics.prefix(3)) { d in
                                    HStack(alignment: .top, spacing: 4) {
                                        Text("[\(d.tool)]")
                                            .font(.caption2.monospaced())
                                            .foregroundStyle(.red)
                                        Text("\((d.file as NSString).lastPathComponent):\(d.line) \(d.message)")
                                            .font(.caption2)
                                            .lineLimit(1)
                                            .truncationMode(.tail)
                                    }
                                }
                                if v.diagnostics.count > 3 {
                                    let rem = v.diagnostics.count - 3
                                    Text("...และอีก \(rem) รายการ")
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                }
                            }
                            .padding(6)
                            .background(Color.red.opacity(0.08), in: RoundedRectangle(cornerRadius: 6))

                            // One-click Self-Heal button
                            Button {
                                var prompt = "กรุณาแก้ข้อผิดพลาดจากการคอมไพล์/ตรวจโค้ดต่อไปนี้ให้ผ่าน:\n\n"
                                for d in v.diagnostics {
                                    prompt += "- [\(d.tool)] \(d.file):\(d.line) - \(d.message)\n"
                                }
                                draftBox.text = prompt
                                submit()
                            } label: {
                                HStack(spacing: 4) {
                                    Image(systemName: "bolt.fill")
                                    Text("ให้ agent แก้เลย")
                                }
                                .font(.caption.weight(.semibold))
                                .frame(maxWidth: .infinity)
                            }
                            .buttonStyle(InteractiveStyle(cornerRadius: 6))
                            .tint(.red)
                            .disabled(engine.busy)
                        } else {
                            Text("รันคอมไพเลอร์อัตโนมัติหลัง agent ตอบ เพื่อป้องกันโค้ดพัง")
                                .font(.caption2)
                                .foregroundStyle(.tertiary)
                        }
                    }
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                    Button {
                        engine.assembledContext { text, tokens in inspected = (text, tokens) }
                    } label: {
                        Label("ดูสิ่งที่ agent เห็นจริง ๆ", systemImage: "doc.text.magnifyingglass")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 8))
                    .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 8))
                    .disabled(engine.sessionID < 0)

                    // High-Density Repo Map (Personalized PageRank)
                    Button {
                        isGeneratingRepoMap = true
                        engine.generateRepoMap(workspace: workspace) { map, count, _ in
                            isGeneratingRepoMap = false
                            if let map = map {
                                inspectedRepoMap = (map, count ?? 0)
                                showingRepoMapSheet = true
                            }
                        }
                    } label: {
                        HStack {
                            Label("High-Density Repo Map", systemImage: "point.3.filled.connected.trianglepath.dotted")
                            Spacer()
                            if isGeneratingRepoMap {
                                ProgressView().controlSize(.mini)
                            }
                        }
                        .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 8))
                    .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 8))
                    .disabled(workspace.isEmpty || isGeneratingRepoMap)
                    .sheet(isPresented: $showingRepoMapSheet) {
                        if let rm = inspectedRepoMap {
                            VStack(alignment: .leading, spacing: 10) {
                                HStack {
                                    Label("Repo Map (\(rm.count) Symbols Ranked)", systemImage: "point.3.filled.connected.trianglepath.dotted")
                                        .font(.headline)
                                    Spacer()
                                    Button("ปิด") { showingRepoMapSheet = false }
                                        .buttonStyle(.borderedProminent)
                                        .controlSize(.small)
                                }
                                Divider()
                                ScrollView {
                                    Text(rm.text)
                                        .font(.caption.monospaced())
                                        .textSelection(.enabled)
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                }
                            }
                            .padding(16)
                            .frame(minWidth: 540, minHeight: 400)
                        }
                    }

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
            .chorusScrollIndicators()
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

/// A file's row in the Changes pane: name, totals, collapse toggle. Its
/// hunks are sibling rows of the parent's lazy stack, not children here.
struct FileChangesHeader: View {
    let file: String
    let hunks: [DiffHunk]
    let open: Bool
    let toggle: () -> Void

    var body: some View {
        Button { withAnimation(.easeOut(duration: 0.15)) { toggle() } } label: {
            HStack(spacing: 6) {
                Image(systemName: "chevron.right").font(.system(size: 9, weight: .bold))
                    .rotationEffect(.degrees(open ? 90 : 0)).foregroundStyle(.secondary)
                Image(systemName: "doc.text").foregroundStyle(.secondary)
                Text((file as NSString).lastPathComponent).font(.callout.weight(.medium)).lineLimit(1)
                Spacer(minLength: 4)
                Text("+\(hunks.reduce(0) { $0 + $1.addCount })").foregroundStyle(.green)
                Text("−\(hunks.reduce(0) { $0 + $1.delCount })").foregroundStyle(.red)
            }
            .font(.caption.monospacedDigit())
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .buttonStyle(InteractiveStyle(cornerRadius: 7, padding: .init(top: 5, leading: 6, bottom: 5, trailing: 6)))
        .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .help(file)
        .padding(.top, 4)
    }
}

/// Right-click menu on a user message: edit-and-resend (chat only), or chat
/// and files, which asks first because it changes the workspace.
private struct RewindMenu: ViewModifier {
    let onRewind: ((Bool) -> Void)?
    @Binding var confirmFiles: Bool

    func body(content: Content) -> some View {
        if let onRewind {
            content
                .contextMenu {
                    Button {
                        onRewind(false)
                    } label: {
                        Label("แก้ข้อความนี้แล้วส่งใหม่", systemImage: "arrow.uturn.backward")
                    }
                    Button {
                        confirmFiles = true
                    } label: {
                        Label("ย้อนแชทและไฟล์กลับไปก่อนข้อความนี้…", systemImage: "clock.arrow.circlepath")
                    }
                }
                .confirmationDialog("ย้อนไฟล์กลับไปก่อนข้อความนี้?", isPresented: $confirmFiles) {
                    Button("ย้อนแชทและไฟล์", role: .destructive) { onRewind(true) }
                } message: {
                    Text("ไฟล์ใน workspace จะกลับไปเหมือนตอนส่งข้อความนี้ Chorus เก็บสภาพปัจจุบันไว้ใน shadow repo ก่อน และแชทเดิมยังอยู่ครบ — คุยต่อใน session ที่แตกออกมา")
                }
        } else {
            content
        }
    }
}

struct EventRow: View {
    let event: LedgerEvent
    var attachments: [SessionAttachment] = []
    var agents: [Engine.AgentInfo] = []
    var thoughts: String? = nil
    var onDeleteAttachment: (Int64) -> Void = { _ in }
    /// Set on the user's own messages: rewind to just before it
    /// (`true` = files too). Rust does the work; this only asks.
    var onRewind: ((Bool) -> Void)? = nil

    @State private var isHovered = false
    @State private var confirmFileRewind = false
    @State private var copied = false

    private var isUser: Bool { event.actor == "user" }
    private var cleanActor: String {
        let base = String(event.actor.split(separator: ":", maxSplits: 1).first ?? Substring(event.actor))
        return String(base.split(separator: "@", maxSplits: 1).first ?? Substring(base))
    }
    private var agent: Engine.AgentInfo? {
        agents.first { cleanActor == $0.id }
    }
    private var providerID: String {
        agent?.id ?? cleanActor
    }
    private var modelLabel: String? {
        guard let modelID = event.model_id else { return nil }
        let model = ModelID(modelID)
        let name = agent?.baseModels.first(where: { $0.id == model.base })?.label ?? model.base
        return [name, model.effort].compactMap { $0 }.joined(separator: " · ")
    }

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

    // Keep role and provider identity visible for every turn, even when an
    // agent changes during a handoff.
    private var message: some View {
        VStack(alignment: isUser ? .trailing : .leading, spacing: 5) {
            HStack(spacing: 6) {
                if isUser {
                    if isHovered {
                        copyButton
                            .transition(.opacity)
                    }
                    Text("คุณ").foregroundStyle(.secondary)
                    Image(systemName: "person.crop.circle.fill")
                        .foregroundStyle(Color.accentColor)
                } else {
                    ProviderIcon(provider: providerID, size: 16)
                    Text(agent?.label ?? event.actor).foregroundStyle(.primary.opacity(0.88))
                    if let modelLabel {
                        Text(modelLabel)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 6).padding(.vertical, 1.5)
                            .background(Color.primary.opacity(0.05), in: Capsule())
                    }
                    if let cost = event.cost_usd {
                        Text(String(format: "$%.4f", cost))
                            .font(.caption2.monospacedDigit())
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 5).padding(.vertical, 1)
                            .background(Color.primary.opacity(0.05), in: Capsule())
                            .help("มูลค่าเทียบ API ของเทิร์นนี้: $\(String(format: "%.4f", cost)) USD")
                    }
                    Spacer(minLength: 0)
                    if isHovered {
                        copyButton
                            .transition(.opacity)
                    }
                }
            }
            .font(.caption.weight(.semibold))
            .frame(maxWidth: isUser ? 680 : .infinity, alignment: isUser ? .trailing : .leading)

            if !isUser, let thoughts, !thoughts.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                ThinkingBlock(text: thoughts, running: false)
            }

            MarkdownText(text: event.body)
                .padding(.horizontal, 14).padding(.vertical, 10)
                .background(
                    isUser
                        ? AnyShapeStyle(LinearGradient(colors: [Color.accentColor.opacity(0.18), Color.accentColor.opacity(0.12)],
                                                      startPoint: .topLeading, endPoint: .bottomTrailing))
                        : AnyShapeStyle(Color(nsColor: .controlBackgroundColor).opacity(0.55)),
                    in: RoundedRectangle(cornerRadius: 16, style: .continuous)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 16, style: .continuous)
                        .stroke(isUser ? Color.accentColor.opacity(0.24) : Color.primary.opacity(0.07), lineWidth: 1)
                )
                .shadow(color: .black.opacity(isUser ? 0.04 : 0.03), radius: 6, y: 2)
                .frame(maxWidth: isUser ? 680 : .infinity, alignment: isUser ? .trailing : .leading)
                .modifier(RewindMenu(onRewind: onRewind, confirmFiles: $confirmFileRewind))

            if !attachments.isEmpty {
                ForEach(attachments) { file in
                    HStack(spacing: 6) {
                        Image(systemName: file.status == "deleted" ? "doc.badge.xmark" : "paperclip")
                        Text(file.name).lineLimit(1)
                        if file.status == "deleted" {
                            Text("ลบแล้ว").foregroundStyle(.secondary)
                        } else {
                            Button("ลบ", role: .destructive) { onDeleteAttachment(file.id) }
                                .buttonStyle(.link)
                                .accessibilityLabel("ลบไฟล์ \(file.name) จาก session")
                        }
                    }
                    .font(.caption)
                    .padding(.horizontal, 8).padding(.vertical, 4)
                    .background(Color.primary.opacity(0.06), in: Capsule())
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: isUser ? .trailing : .leading)
        .onHover { h in withAnimation(.spring(response: 0.2, dampingFraction: 0.8)) { isHovered = h } }
    }

    private var copyButton: some View {
        Button {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(event.body, forType: .string)
            withAnimation(.spring(response: 0.2, dampingFraction: 0.7)) { copied = true }
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.8) {
                withAnimation { copied = false }
            }
        } label: {
            HStack(spacing: 3) {
                Image(systemName: copied ? "checkmark" : "doc.on.doc")
                    .font(.caption2)
                Text(copied ? "คัดลอกแล้ว" : "คัดลอก")
                    .font(.caption2)
            }
            .foregroundStyle(copied ? .green : .secondary)
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(Color.primary.opacity(0.06), in: Capsule())
        }
        .buttonStyle(.plain)
    }

    /// One collapsed line per tool call; click to see the payload.
    private var toolRow: some View {
        DisclosureGroup {
            if let payload = subagentPayload {
                VStack(alignment: .leading, spacing: 4) {
                    if let task = payload.task, !task.isEmpty { Text(task) }
                    if let message = payload.message, !message.isEmpty { Text(message) }
                    if let state = payload.state { Text(Self.subagentStateLabel(state)).foregroundStyle(.secondary) }
                }
                .font(.caption2)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, 4)
            } else {
                Text(event.body)
                    .font(.caption2.monospaced())
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.top, 4)
            }
        } label: {
            HStack(spacing: 6) {
                Image(systemName: subagentPayload == nil ? (event.kind == "diff" ? "plusminus" : "wrench.and.screwdriver") : "point.3.connected.trianglepath.dotted")
                    .font(.caption2).foregroundStyle(.secondary)
                Text(toolLabel).font(.caption).lineLimit(1).foregroundStyle(.secondary)
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

    private var subagentPayload: SubagentLedgerPayload? {
        guard let data = event.body.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(SubagentLedgerPayload.self, from: data)
    }

    private var toolLabel: String {
        guard let payload = subagentPayload else { return firstLine }
        if payload.event == "subagent_spawned" { return "งานย่อย · \(payload.name ?? "Agent")" }
        return "สถานะงานย่อย · \(payload.state.map(Self.subagentStateLabel) ?? "อัปเดต")"
    }

    private static func subagentStateLabel(_ state: String) -> String {
        switch state {
        case "completed": "เสร็จแล้ว"
        case "failed": "ล้มเหลว"
        case "cancelled": "ยกเลิกแล้ว"
        case "disconnected": "ขาดการเชื่อมต่อ"
        default: state
        }
    }

    private func note(_ t: String) -> some View {
        Text(t).font(.caption).foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func card<C: View>(tint: Color, @ViewBuilder _ c: () -> C) -> some View {
        c().padding(10)
            .background(tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 10))
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

    /// Counted once at parse time: views read these on every render.
    let addCount: Int
    let delCount: Int

    init(file: String, preamble: String, header: String, lines: [String]) {
        self.file = file
        self.preamble = preamble
        self.header = header
        self.lines = lines
        addCount = lines.reduce(0) { $0 + ($1.hasPrefix("+") ? 1 : 0) }
        delCount = lines.reduce(0) { $0 + ($1.hasPrefix("-") ? 1 : 0) }
    }

    var patch: String { preamble + header + "\n" + lines.joined(separator: "\n") + "\n" }

    /// Compute line number within the file for a given line index in the hunk (1-based).
    func fileLineNumber(for lineIndex: Int) -> Int {
        let parts = header.components(separatedBy: "@@")
        guard parts.count >= 2 else { return lineIndex + 1 }
        let spec = parts[1].trimmingCharacters(in: .whitespaces)
        guard let plusRange = spec.range(of: "+") else { return lineIndex + 1 }
        let afterPlus = spec[plusRange.upperBound...]
        let countPart = afterPlus.split(separator: " ").first?.split(separator: ",").first
        guard let startLine = countPart.flatMap({ Int($0) }) else { return lineIndex + 1 }

        var currentNewLine = startLine
        for i in 0..<min(lineIndex, lines.count) {
            let l = lines[i]
            if !l.hasPrefix("-") {
                currentNewLine += 1
            }
        }
        return currentNewLine
    }
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


/// An isolated timer so TimelineView does not invalidate the entire working row every second.
struct TurnTimerView: View {
    let startedAt: Date
    let isEmpty: Bool

    var body: some View {
        TimelineView(.periodic(from: startedAt, by: 1)) { ctx in
            let sec = max(0, Int(ctx.date.timeIntervalSince(startedAt)))
            Text(isEmpty ? "กำลังเริ่ม agent… \(sec) วิ" : "กำลังตอบ… \(sec) วิ")
        }
    }
}

/// A macOS-native code block with language title bar, interactive Canvas preview, and one-click copy feedback.
struct CodeBlockView: View {
    let language: String
    let code: String
    @State private var copied = false
    @State private var showCanvas: Bool = true

    private var isCanvasSupported: Bool {
        let lang = language.lowercased()
        return lang == "mermaid" || lang == "svg" || lang == "html"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Image(systemName: isCanvasSupported ? "paintpalette.fill" : "curlybraces")
                    .font(.caption2)
                    .foregroundStyle(isCanvasSupported ? .purple : .secondary)
                Text(language.isEmpty ? "code" : language.lowercased())
                    .font(.caption2.weight(.semibold).monospaced())
                    .foregroundStyle(.secondary)

                if isCanvasSupported {
                    HStack(spacing: 2) {
                        Button {
                            showCanvas = true
                        } label: {
                            Text("แคนวาส")
                                .font(.system(size: 10, weight: showCanvas ? .bold : .regular))
                                .foregroundStyle(showCanvas ? .purple : .secondary)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(showCanvas ? Color.purple.opacity(0.14) : Color.clear, in: RoundedRectangle(cornerRadius: 4))
                        }
                        .buttonStyle(.plain)

                        Button {
                            showCanvas = false
                        } label: {
                            Text("โค้ด")
                                .font(.system(size: 10, weight: !showCanvas ? .bold : .regular))
                                .foregroundStyle(!showCanvas ? Color.primary : .secondary)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 2)
                                .background(!showCanvas ? Color.primary.opacity(0.08) : Color.clear, in: RoundedRectangle(cornerRadius: 4))
                        }
                        .buttonStyle(.plain)
                    }
                    .padding(2)
                    .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 6))
                }

                Spacer()
                Button {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(code, forType: .string)
                    withAnimation(.spring(response: 0.2, dampingFraction: 0.7)) {
                        copied = true
                    }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                        withAnimation { copied = false }
                    }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: copied ? "checkmark" : "doc.on.doc")
                            .font(.caption2)
                        Text(copied ? "คัดลอกแล้ว" : "คัดลอกโค้ด")
                            .font(.caption2)
                    }
                    .foregroundStyle(copied ? .green : .secondary)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 3)
                    .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 5, style: .continuous))
                }
                .buttonStyle(.plain)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(Color.primary.opacity(0.04))

            Divider().opacity(0.4)

            if isCanvasSupported && showCanvas {
                CanvasPreviewView(language: language, code: code)
            } else {
                ScrollView(.horizontal, showsIndicators: true) {
                    Text(code)
                        .font(.system(.callout, design: .monospaced))
                        .textSelection(.enabled)
                        .padding(10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
        .background(Color(nsColor: .textBackgroundColor).opacity(0.65), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(Color.primary.opacity(0.09), lineWidth: 1))
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}

/// The running turn, live: reasoning (collapsible, like the thinking
/// block in Claude/Codex), tool calls as they start and finish, and the
/// answer as it streams — then Stop.
struct WorkingRow: View {
    @ObservedObject var stream: LiveStream
    let agentID: String
    let agentLabel: String
    let startedAt: Date?
    let onStop: () -> Void
    /// Called when the streamed content grows by a visible step, so the
    /// thread can keep following the bottom.
    let onGrow: () -> Void

    private var growth: Int {
        stream.turn.message.count / 80 + stream.turn.thoughts.count / 150 + stream.turn.tools.count
    }

    var body: some View {
        let live = stream.turn
        return VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 7) {
                ProviderIcon(provider: agentID, size: 16)
                Text(agentLabel)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                ProgressView().controlSize(.mini)
                Spacer()
                if let t = startedAt {
                    TurnTimerView(startedAt: t, isEmpty: live.isEmpty)
                        .font(.caption2.monospacedDigit())
                        .foregroundStyle(.tertiary)
                }
                Button(action: onStop) {
                    HStack(spacing: 4) {
                        Image(systemName: "stop.fill").font(.system(size: 8))
                        Text("หยุด")
                    }
                    .font(.caption2.weight(.medium))
                    .foregroundStyle(.red)
                    .padding(.horizontal, 8).padding(.vertical, 3.5)
                    .background(Color.red.opacity(0.08), in: Capsule())
                }
                .buttonStyle(.plain)
                .help("หยุดเทิร์นนี้ (⌘.) — ล้างคิวด้วย")
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            if !live.thoughts.isEmpty {
                ThinkingBlock(text: live.thoughts, running: live.message.isEmpty)
            }

            ForEach(live.tools) { t in
                HStack(spacing: 7) {
                    switch t.state {
                    case "running": ProgressView().controlSize(.mini)
                    case "failed": Image(systemName: "xmark.circle.fill").foregroundStyle(.red)
                    default: Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                    }
                    Text(t.title).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                .padding(.horizontal, 8).padding(.vertical, 4)
                .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 6, style: .continuous))
            }

            if !live.message.isEmpty {
                MarkdownText(text: live.message)
            } else if live.tools.isEmpty && live.thoughts.isEmpty {
                HStack(spacing: 6) {
                    Circle().frame(width: 6, height: 6).foregroundStyle(Color.accentColor).opacity(0.8)
                    Text("กำลังเตรียมการประมวลผล…")
                        .font(.caption).foregroundStyle(.secondary)
                }
                .padding(.vertical, 4)
            }
        }
        .padding(14)
        .background(Color(nsColor: .controlBackgroundColor).opacity(0.55), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).stroke(Color.primary.opacity(0.07), lineWidth: 1))
        .shadow(color: .black.opacity(0.03), radius: 6, y: 2)
        .onChange(of: growth) { _, _ in onGrow() }
    }
}

/// Markdown for agent output: fenced code becomes a monospaced block with header,
/// the rest goes through `AttributedString` (bold, italic, inline code, links) with
/// whitespace preserved so lists and paragraphs keep their shape.
struct MarkdownText: View {
    let text: String

    enum Part {
        case code(language: String, body: String)
        case prose(AttributedString)
    }

    /// Parsed parts keyed by the exact text. Rows re-render far more often than
    /// their text changes (every scroll, hover and engine publish), and both
    /// the fence split and `AttributedString(markdown:)` used to run each time.
    /// Cost is the text length, so a streamed reply's many short-lived
    /// prefixes are evicted before the stable history is.
    private final class Box { let parts: [Part]; init(_ p: [Part]) { parts = p } }
    private static let cache: NSCache<NSString, Box> = {
        let c = NSCache<NSString, Box>()
        c.totalCostLimit = 8 * 1024 * 1024
        return c
    }()

    static func parts(for text: String) -> [Part] {
        let key = text as NSString
        if let hit = cache.object(forKey: key) { return hit.parts }
        let parsed = parse(text)
        cache.setObject(Box(parsed), forKey: key, cost: key.length * 2)
        return parsed
    }

    static func parse(_ text: String) -> [Part] {
        var out: [Part] = []
        for (i, chunk) in text.components(separatedBy: "```").enumerated() {
            guard !chunk.isEmpty else { continue }
            if i % 2 == 1 {
                let lines = chunk.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: false)
                let lang = lines.first.map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) } ?? ""
                let codeBody = lines.count > 1 ? String(lines[1]).trimmingCharacters(in: .newlines) : chunk.trimmingCharacters(in: .newlines)
                out.append(.code(language: lang, body: codeBody))
            } else {
                let trimmed = chunk.trimmingCharacters(in: .newlines)
                guard !trimmed.isEmpty else { continue }
                let attributed = (try? AttributedString(
                    markdown: trimmed,
                    options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(trimmed)
                out.append(.prose(attributed))
            }
        }
        return out
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(Self.parts(for: text).enumerated()), id: \.offset) { _, p in
                switch p {
                case let .code(language, body):
                    CodeBlockView(language: language, code: body)
                case let .prose(a):
                    Text(a).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
    }
}

/// Collapsible reasoning with smooth spring chevron and pulsing indicator.
struct ThinkingBlock: View {
    let text: String
    let running: Bool
    @State private var open: Bool?
    @AppStorage("chorus.thinkingOpen") private var thinkingOpen = true

    var body: some View {
        let cleanText = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let isOpen = open ?? (running && thinkingOpen)
        VStack(alignment: .leading, spacing: 6) {
            Button {
                withAnimation(.spring(response: 0.28, dampingFraction: 0.82)) {
                    open = !isOpen
                }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "brain.head.profile")
                        .foregroundStyle(.purple)
                        .symbolEffect(.pulse, isActive: running)
                    Text(running ? "กำลังคิด…" : "กระบวนการคิด").font(.caption.weight(.semibold))
                    if !cleanText.isEmpty {
                        Text("\(cleanText.count) ตัวอักษร")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 5).padding(.vertical, 1.5)
                            .background(Color.purple.opacity(0.08), in: Capsule())
                    }
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .rotationEffect(.degrees(isOpen ? 90 : 0))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if isOpen {
                if !cleanText.isEmpty {
                    VStack(alignment: .leading, spacing: 4) {
                        if cleanText.count > 300 {
                            ScrollViewReader { proxy in
                                ScrollView {
                                    MarkdownText(text: cleanText)
                                        .font(.caption).foregroundStyle(.secondary)
                                        .textSelection(.enabled)
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                    Color.clear.frame(height: 1).id("end")
                                }
                                .frame(maxHeight: 180)
                                .onChange(of: cleanText.count) { _, _ in if running { proxy.scrollTo("end") } }
                            }
                        } else {
                            MarkdownText(text: cleanText)
                                .font(.caption).foregroundStyle(.secondary)
                                .textSelection(.enabled)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }
                    .padding(.top, 4)
                } else if running {
                    Text("กำลังใคร่ครวญและวิเคราะห์ข้อมูล…")
                        .font(.caption).foregroundStyle(.tertiary)
                        .padding(.top, 2)
                }
            }
        }
        .padding(9)
        .background(Color.purple.opacity(0.06), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(Color.purple.opacity(0.12), lineWidth: 1))
    }
}


extension View {
    /// A clean panel: crisp continuous rounded corners with subtle native border and translucent material.
    /// Completely eliminates harsh drop-shadow artifacts and corner clipping anomalies.
    @ViewBuilder
    func floatingPanel(cornerRadius: CGFloat, tint: Color? = nil) -> some View {
        self
            .clipShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
            .background(
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .fill(.regularMaterial)
            )
            .overlay(
                Group {
                    if let tint {
                        RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                            .fill(tint.opacity(0.12))
                    }
                }
            )
            .overlay(
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .stroke(Color.primary.opacity(0.08), lineWidth: 1)
            )
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

/// The thread's content frame in its scroll view's coordinate space:
/// `minY` tracks the scroll offset, `maxY` the bottom of the content.
struct ChatContentFrame: Equatable {
    var minY: CGFloat
    var maxY: CGFloat
}

struct ChatContentFrameKey: PreferenceKey {
    static let defaultValue = ChatContentFrame(minY: 0, maxY: 0)
    static func reduce(value: inout ChatContentFrame, nextValue: () -> ChatContentFrame) { value = nextValue() }
}

/// Open on the newest message. macOS 15 can limit the anchor to the first
/// layout; on 14 the plain anchor also keeps the bottom on content growth.
private struct OpenAtBottom: ViewModifier {
    func body(content: Content) -> some View {
        if #available(macOS 15.0, *) {
            content.defaultScrollAnchor(.bottom, for: .initialOffset)
        } else {
            content.defaultScrollAnchor(.bottom)
        }
    }
}

struct PlanChecklistView: View {
    let items: [Engine.ChecklistItem]
    let askBeforePlan: Bool
    let isPlanApproved: Bool
    let onStartPlan: () -> Void
    let onStop: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: "checklist")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(.purple)

                Text("แผนงาน Orchestra")
                    .font(.caption.weight(.semibold))

                Spacer()

                if askBeforePlan && !isPlanApproved {
                    Button(action: onStartPlan) {
                        Label("เริ่มทำตามแผน", systemImage: "play.fill")
                            .font(.caption.weight(.medium))
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
                }

                Button(role: .destructive, action: onStop) {
                    Label("หยุด", systemImage: "stop.fill")
                        .font(.caption.weight(.medium))
                }
                .buttonStyle(.bordered)
                .controlSize(.small)
                .help("หยุดการทำงานของแผนงาน Orchestra ทั้งหมด")
            }

            VStack(alignment: .leading, spacing: 6) {
                ForEach(items) { item in
                    HStack(alignment: .top, spacing: 8) {
                        if item.done {
                            Image(systemName: "checkmark.circle.fill")
                                .font(.system(size: 12))
                                .foregroundStyle(.green)
                        } else if item.in_progress {
                            ProgressView()
                                .controlSize(.mini)
                        } else {
                            Image(systemName: "circle")
                                .font(.system(size: 12))
                                .foregroundStyle(.secondary)
                        }

                        Text(item.text)
                            .font(.caption)
                            .foregroundStyle(item.done ? .secondary : .primary)
                            .strikethrough(item.done)
                    }
                }
            }
            .padding(.top, 2)
        }
        .padding(12)
        .background(Color(nsColor: .controlBackgroundColor).opacity(0.8), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(Color.primary.opacity(0.08), lineWidth: 1))
    }
}

struct CapPausePromptCard: View {
    let subagents: [Engine.SubagentInfo]
    let onContinue: (Int64) -> Void
    let onStop: (Int64) -> Void
    let onStopAll: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Text("รอ")
                    .font(.system(size: 9, weight: .bold))
                    .foregroundStyle(.white)
                    .padding(.horizontal, 5)
                    .padding(.vertical, 2)
                    .background(Color.orange, in: Capsule())

                Text("งานย่อยหยุดรอ (ติด Concurrency Cap หรือรอการยืนยัน)")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.orange)

                Spacer()

                Button(role: .destructive, action: onStopAll) {
                    Text("หยุดทั้งหมด")
                        .font(.caption2)
                }
                .buttonStyle(.bordered)
                .controlSize(.mini)
            }

            VStack(alignment: .leading, spacing: 6) {
                ForEach(subagents) { sub in
                    HStack(spacing: 8) {
                        Text(sub.role)
                            .font(.caption.weight(.semibold))
                        Text(sub.task)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)

                        Spacer()

                        Button {
                            onContinue(sub.id)
                        } label: {
                            Label("ทำต่อ", systemImage: "play.fill")
                                .font(.caption2.weight(.medium))
                        }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.mini)

                        Button(role: .destructive) {
                            onStop(sub.id)
                        } label: {
                            Image(systemName: "xmark")
                                .font(.system(size: 9, weight: .bold))
                        }
                        .buttonStyle(.bordered)
                        .controlSize(.mini)
                        .help("ยกเลิกงานย่อยนี้")
                    }
                }
            }
        }
        .padding(12)
        .background(Color.orange.opacity(0.08), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(Color.orange.opacity(0.25), lineWidth: 1))
    }
}

struct SubagentTreeRow: View {
    let sub: Engine.SubagentInfo
    let siblings: [Engine.SubagentInfo]
    let level: Int
    let folder: String
    let currentWorkspace: String
    let selectedSessionID: Int64?
    let subagentsByParent: [Int64: [Engine.SubagentInfo]]
    let onSelect: (Int64) -> Void
    let onMessage: (Engine.SubagentInfo) -> Void
    let onCancel: (Int64) -> Void
    let onCancelParent: () -> Void
    var onKeepRole: ((String, Engine.SubagentInfo) -> Void)? = nil

    var body: some View {
        let isSelected = folder == currentWorkspace && sub.child_sess != nil && sub.child_sess == selectedSessionID
        let isBuiltin = ["orchestrator", "investigator", "builder", "reviewer"].contains(sub.role.lowercased())
        let roleSiblings = siblings.filter { $0.role == sub.role }.sorted { $0.created_ms < $1.created_ms }
        let repeatIndex = roleSiblings.firstIndex(where: { $0.id == sub.id }).map { $0 + 1 } ?? 1
        let roleTitle = roleSiblings.count > 1 ? "\(sub.role) \(repeatIndex)" : sub.role
        let displayName = sub.kind == "provider" ? (sub.task.isEmpty ? sub.role : sub.task) : "\(roleTitle) · \(sub.task)"
        let nestedSubs = sub.child_sess.flatMap { subagentsByParent[$0] } ?? []

        VStack(alignment: .leading, spacing: 1) {
            Button {
                guard let childId = sub.child_sess, !isSelected else { return }
                onSelect(childId)
            } label: {
                HStack(spacing: 5) {
                    // Tree branch indicator
                    Image(systemName: "arrow.turn.down.right")
                        .font(.system(size: 8, weight: .bold))
                        .foregroundStyle(.tertiary)

                    // Status dot
                    switch sub.status {
                    case "running":
                        ProgressView().controlSize(.mini)
                            .help("กำลังทำ")
                    case "waiting":
                        Text("รอ")
                            .font(.system(size: 8.5, weight: .bold))
                            .foregroundStyle(.white)
                            .padding(.horizontal, 4)
                            .padding(.vertical, 1)
                            .background(Color.orange, in: Capsule())
                            .help("รอดำเนินการหรือติด Cap")
                    case "completed":
                        Image(systemName: "checkmark.circle.fill")
                            .font(.system(size: 10))
                            .foregroundStyle(.green)
                            .help("เสร็จแล้ว")
                    case "failed":
                        Image(systemName: "xmark.circle.fill")
                            .font(.system(size: 10))
                            .foregroundStyle(.red)
                            .help("ล้มเหลว: \(sub.summary ?? "")")
                    case "cancelled":
                        Image(systemName: "nosign")
                            .font(.system(size: 10))
                            .foregroundStyle(.secondary)
                            .help("ยกเลิก")
                    default:
                        Circle().fill(.secondary).frame(width: 5, height: 5)
                    }

                    Text(displayName)
                        .font(.caption2)
                        .fontWeight(isSelected ? .semibold : .regular)
                        .lineLimit(1)

                    Spacer(minLength: 4)

                    // New role badge & Keep action
                    if !isBuiltin && sub.kind == "chorus" {
                        HStack(spacing: 3) {
                            Text("new role")
                                .font(.system(size: 8, weight: .semibold))
                                .foregroundStyle(.purple)
                                .padding(.horizontal, 4)
                                .padding(.vertical, 0.5)
                                .background(Color.purple.opacity(0.12), in: Capsule())

                            if let onKeepRole {
                                Button {
                                    onKeepRole(sub.role, sub)
                                } label: {
                                    Text("Keep")
                                        .font(.system(size: 8, weight: .bold))
                                        .foregroundStyle(.white)
                                        .padding(.horizontal, 4)
                                        .padding(.vertical, 0.5)
                                        .background(Color.purple, in: Capsule())
                                }
                                .buttonStyle(.plain)
                                .help("บันทึก Role '\(sub.role)' ลง .chorus/agents/\(sub.role).md")
                            }
                        }
                    }

                    // Model chip
                    if let m = sub.model, !m.isEmpty {
                        Text(ModelID(m).base)
                            .font(.system(size: 8.5, weight: .medium, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 3)
                            .padding(.vertical, 0.5)
                            .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 3))
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(InteractiveStyle(cornerRadius: 6, selected: isSelected,
                                          padding: .init(top: 3, leading: 6, bottom: 3, trailing: 6)))
            .padding(.leading, CGFloat(24 + level * 10))
            .help(displayName)
            .contextMenu {
                if sub.kind == "chorus" {
                    if !isBuiltin {
                        Button("Keep Role '\(sub.role)' (.chorus/agents/)") {
                            onKeepRole?(sub.role, sub)
                        }
                    }
                    if sub.status == "running" || sub.status == "waiting" {
                        Button("ส่งข้อความถึงงานย่อย…") {
                            onMessage(sub)
                        }
                        Button("หยุดงานย่อยนี้", role: .destructive) {
                            onCancel(sub.id)
                        }
                    }
                    if let wt = sub.worktree, !wt.isEmpty {
                        Button("เปิด Git Worktree ใน Finder") {
                            NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: wt)])
                        }
                    }
                } else {
                    Text("Provider subagent (อ่านอย่างเดียว)").font(.caption).foregroundStyle(.secondary)
                    Button("หยุดเทิร์นของ agent หลัก", role: .destructive) {
                        onCancelParent()
                    }
                }
            }

            // Nesting up to 2 levels; deeper levels shown as …+N
            if !nestedSubs.isEmpty {
                if level < 2 {
                    ForEach(nestedSubs) { nested in
                        SubagentTreeRow(
                            sub: nested,
                            siblings: nestedSubs,
                            level: level + 1,
                            folder: folder,
                            currentWorkspace: currentWorkspace,
                            selectedSessionID: selectedSessionID,
                            subagentsByParent: subagentsByParent,
                            onSelect: onSelect,
                            onMessage: onMessage,
                            onCancel: onCancel,
                            onCancelParent: onCancelParent,
                            onKeepRole: onKeepRole
                        )
                    }
                } else {
                    Text("…+\(nestedSubs.count)")
                        .font(.system(size: 9, weight: .medium))
                        .foregroundStyle(.tertiary)
                        .padding(.leading, CGFloat(24 + (level + 1) * 10))
                        .help("มีงานย่อยระดับลึกอีก \(nestedSubs.count) งาน")
                }
            }
        }
    }
}


/// A pending "ลบ session ที่ว่าง" confirmation.
struct EmptyCleanup: Identifiable {
    let project: String
    let dbs: [String]
    let count: Int
    var id: String { project }
}
