import AppKit
import SwiftUI

/// Settings, laid out like System Settings: 13 categories in 4 groups on the
/// left, the category on the right. Each row is a short label, its control,
/// and an (i) for anything longer than one line.
struct SettingsSheet: View {
    @EnvironmentObject var engine: Engine
    @Environment(\.dismiss) private var dismiss
    let workspace: String
    @State var section: Section = .accounts
    @AppStorage("chorus.autoCatalogUpdates") private var autoCatalogUpdates = true

    enum Section: String, CaseIterable, Identifiable {
        case general, accounts, routing, orchestration
        case agents, safety, integrations, skills
        case review, cost, storage
        case companion, about
        var id: String { rawValue }

        var title: String {
            switch self {
            case .general: "ทั่วไป"
            case .accounts: "บัญชี"
            case .routing: "การส่งงาน"
            case .orchestration: "Orchestra"
            case .agents: "โมเดล"
            case .safety: "สิทธิ์และความปลอดภัย"
            case .integrations: "เครื่องมือเสริม (MCP)"
            case .skills: "Skills"
            case .review: "Review และ checkpoint"
            case .cost: "การใช้งานและค่าใช้จ่าย"
            case .storage: "พื้นที่และประสิทธิภาพ"
            case .companion: "Mobile Companion"
            case .about: "เกี่ยวกับ"
            }
        }

        var subtitle: String {
            switch self {
            case .general: "การพิมพ์ การแสดงผล และหน้าต่าง"
            case .accounts: "บัญชีของแต่ละเจ้าและโควตาที่เหลือ"
            case .routing: "เทิร์นของคุณถูกส่งไปที่ไหน"
            case .orchestration: "โมเดลหลักที่วางแผนและแจกงาน"
            case .agents: "agent ที่ติดตั้งและโมเดลที่มันมี"
            case .safety: "agent ทำอะไรได้บ้าง"
            case .integrations: "MCP server ที่ทุก agent ใช้ร่วมกัน"
            case .skills: "skill ที่ agent ในโฟลเดอร์นี้มองเห็น"
            case .review: "การตรวจสิ่งที่ agent แก้"
            case .cost: "token โควตา และเงิน"
            case .storage: "โมเดลในเครื่อง หน่วยความจำ และข้อมูล"
            case .companion: "สั่งงานจาก iPhone"
            case .about: "เวอร์ชันและสัญญาอนุญาต"
            }
        }

        var icon: String {
            switch self {
            case .general: "gearshape"
            case .accounts: "person.2"
            case .routing: "arrow.triangle.branch"
            case .orchestration: "point.3.connected.trianglepath.dotted"
            case .agents: "cpu"
            case .safety: "lock.shield"
            case .integrations: "puzzlepiece.extension"
            case .skills: "sparkles"
            case .review: "plusminus"
            case .cost: "bahtsign.circle"
            case .storage: "internaldrive"
            case .companion: "iphone"
            case .about: "info.circle"
            }
        }

        var tint: Color {
            switch self {
            case .general, .about: .gray
            case .accounts: .teal
            case .routing: .blue
            case .orchestration: .purple
            case .agents: .indigo
            case .safety: .red
            case .integrations: .orange
            case .skills: .yellow
            case .review: .green
            case .cost: .mint
            case .storage: .brown
            case .companion: .cyan
            }
        }

        static let groups: [(title: String, items: [Section])] = [
            ("ใช้งาน", [.general, .accounts, .routing, .orchestration]),
            ("Agent", [.agents, .safety, .integrations, .skills]),
            ("ข้อมูล", [.review, .cost, .storage]),
            ("ระบบ", [.companion, .about]),
        ]
    }

    init(workspace: String, section: Section = .accounts) {
        self.workspace = workspace
        _section = State(initialValue: section)
    }

    var body: some View {
        HStack(spacing: 0) {
            sidebar
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(section.title).font(.title3.bold())
                        Text(section.subtitle).font(.callout).foregroundStyle(.secondary)
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityAddTraits(.isHeader)
                    pane
                }
                .padding(.horizontal, 22).padding(.vertical, 18)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .chorusScrollIndicators()
        }
        .frame(width: 820, height: 620)
        .onAppear {
            engine.loadAgents()
            engine.loadUsage()
            engine.loadSkills(cwd: workspace)
            engine.loadCatalog(project: workspace, automaticUpdates: autoCatalogUpdates)
        }
    }

    private var sidebar: some View {
        VStack(alignment: .leading, spacing: 1) {
            ForEach(Section.groups, id: \.title) { group in
                Text(group.title)
                    .font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                    .padding(.horizontal, 8).padding(.top, 10).padding(.bottom, 2)
                    .accessibilityAddTraits(.isHeader)
                ForEach(group.items) { s in
                    Button { section = s } label: {
                        HStack(spacing: 8) {
                            Image(systemName: s.icon)
                                .font(.system(size: 11, weight: .semibold))
                                .foregroundStyle(.white)
                                .frame(width: 20, height: 20)
                                .background(s.tint.gradient, in: RoundedRectangle(cornerRadius: 5, style: .continuous))
                            Text(s.title).lineLimit(1)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 6, selected: section == s,
                                                  padding: .init(top: 3, leading: 6, bottom: 3, trailing: 6)))
                    .accessibilityAddTraits(section == s ? .isSelected : [])
                }
            }
            Spacer()
            Button("เสร็จ") { dismiss() }
                .keyboardShortcut(.defaultAction)
                .padding(.horizontal, 8)
        }
        .padding(.horizontal, 8).padding(.vertical, 12)
        .frame(width: 220)
        .background(Color.primary.opacity(0.03))
    }

    @ViewBuilder private var pane: some View {
        switch section {
        case .general: GeneralSettings()
        case .accounts: AccountsSettings()
        case .routing: RoutingSettings(workspace: workspace)
        case .orchestration: OrchestrationSettings(workspace: workspace)
        case .agents: AgentsSettings()
        case .safety: SafetySettings(workspace: workspace)
        case .integrations: integrations
        case .skills: SkillsSettings(workspace: workspace)
        case .review: ReviewSettings()
        case .cost: CostSettings()
        case .storage: StorageSettings()
        case .companion: RemotePairingView()
        case .about: AboutSettings()
        }
    }

    @ViewBuilder private var integrations: some View {
        CatalogView(projectName: workspace.isEmpty ? "" : URL(fileURLWithPath: workspace).lastPathComponent,
                    entries: engine.catalog,
                    automaticUpdates: autoCatalogUpdates,
                    backgroundStatus: engine.catalogBackgroundStatus,
                    actionResult: engine.catalogActionResult,
                    progress: engine.catalogProgress,
                    requestedInstall: engine.requestedInstall,
                    onRequestHandled: { engine.requestedInstall = nil },
                    onLoadPlan: { id, action, done in
                        engine.loadCatalogPlan(project: workspace, id: id, action: action, completion: done)
                    },
                    onAutomaticUpdatesChanged: { value in
                        autoCatalogUpdates = value
                        if value { engine.loadCatalog(project: workspace, automaticUpdates: true) }
                    },
                    onInstall: { engine.catalogAction(project: workspace, id: $0,
                                                      action: workspace.isEmpty ? "install" : "install_enable") },
                    onUpdate: { engine.catalogAction(project: workspace, id: $0, action: "update") },
                    onRemove: { engine.catalogAction(project: workspace, id: $0, action: "remove") },
                    onProjectEnableChanged: { id, enabled in
                        engine.catalogAction(project: workspace, id: id,
                                             action: enabled ? "enable" : "disable")
                    })
        if let error = engine.catalogActionError ?? engine.catalogError {
            Text(error).font(.caption).foregroundStyle(.red)
        }
    }
}

/// Every key Settings owns, with the default decided for it. Several back
/// features that have not shipped yet (orchestra caps, route preset, trusted
/// projects, THB rate): they are stored now so those features can read them.
enum SettingsKey {
    // การส่งงาน
    static let defaultMode = "chorus.defaultMode"                 // "auto" | "orchestra"
    static let defaultRoutePreset = "chorus.defaultRoutePreset"   // "economy" | "balanced" | "max"
    // Orchestra
    static let orchestratorModel = "chorus.orchestratorLeadModel" // "" = Claude Opus 5.5 · medium
    static let orchestratorBackup = "chorus.orchestratorBackupModel" // "" = strongest ready model elsewhere
    static let orchestraConcurrency = "chorus.orchestrationLimit"
    static let orchestraMaxChildren = "chorus.orchestraMaxChildren"
    static let orchestraMaxMinutes = "chorus.orchestraMaxMinutes"
    static let orchestraAskBeforePlan = "chorus.orchestraAskBeforePlan"
    static let orchestraAutoMerge = "chorus.orchestraAutoMerge"
    static let orchestraCopyEnv = "chorus.orchestraCopyEnv"
    // สิทธิ์และความปลอดภัย
    static let trustedProjects = "chorus.trustedProjects"         // newline-separated paths
    static let dictationOnDevice = "chorus.dictationOnDevice"
    // ข้อมูล
    static let showChorusFiles = "chorus.showChorusWrittenFiles"
    static let thbRate = "chorus.thbRate"                         // "" = not set; baht per 1 USD
}

// MARK: shared bits

struct SettingsGroup<Content: View>: View {
    let title: String
    var footer: String?
    @ViewBuilder var content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                .padding(.leading, 4)
                .accessibilityAddTraits(.isHeader)
            VStack(alignment: .leading, spacing: 0) { content() }
                .padding(.vertical, 2)
                .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            if let footer {
                Text(footer).font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.leading, 4)
            }
        }
    }
}

/// One row: a short label, an optional (i) with the long explanation, an
/// optional one-line status under the label, and the control on the right.
struct SettingsRow<Trailing: View>: View {
    let title: String
    var detail: String?
    var info: String?
    @ViewBuilder var trailing: () -> Trailing

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(title)
                    if let info { InfoTip(text: info) }
                }
                if let detail {
                    Text(detail).font(.caption).foregroundStyle(.secondary)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 8)
            trailing()
        }
        .padding(.horizontal, 12).padding(.vertical, 7)
        .frame(minHeight: 38)
    }
}

/// The (i): the native tooltip on hover, a popover on keyboard focus or click,
/// and the full text for VoiceOver.
struct InfoTip: View {
    let text: String
    @State private var shown = false
    @FocusState private var focused: Bool

    var body: some View {
        Image(systemName: "info.circle")
            .font(.system(size: 12))
            .foregroundStyle(focused || shown ? Color.accentColor : .secondary)
            .contentShape(Circle())
            .help(text)
            .focusable()
            .focused($focused)
            // Open on focus but never close on blur: a popover that takes key
            // would blur the icon and snap itself shut.
            .onChange(of: focused) { _, f in if f { shown = true } }
            .onTapGesture { shown.toggle() }
            .onKeyPress(.space) { shown.toggle(); return .handled }
            .onKeyPress(.escape) { shown = false; return .handled }
            .popover(isPresented: $shown, arrowEdge: .trailing) {
                Text(text)
                    .font(.callout)
                    .frame(width: 260, alignment: .leading)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(12)
            }
            .accessibilityElement()
            .accessibilityLabel("ข้อมูลเพิ่มเติม")
            .accessibilityValue(text)
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { shown.toggle() }
    }
}

struct RowDivider: View {
    var body: some View { Divider().padding(.leading, 12) }
}

/// A usage bar: green → orange → red as it fills.
struct UsageBar: View {
    let fraction: Double
    var height: CGFloat = 6

    private var color: Color { fraction >= 0.9 ? .red : fraction >= 0.7 ? .orange : .green }

    var body: some View {
        GeometryReader { g in
            ZStack(alignment: .leading) {
                Capsule().fill(Color.primary.opacity(0.08))
                Capsule().fill(color).frame(width: max(height, g.size.width * min(max(fraction, 0), 1)))
            }
        }
        .frame(height: height)
        .animation(.easeOut, value: fraction)
    }
}

func resetText(_ unix: Int64?) -> String {
    guard let unix else { return "" }
    let d = Date(timeIntervalSince1970: TimeInterval(unix))
    let f = DateFormatter()
    f.locale = Locale(identifier: "th_TH")
    f.dateFormat = Calendar.current.isDateInToday(d) ? "HH:mm" : "EEE d MMM HH:mm"
    return "รีเซ็ต " + f.string(from: d)
}

// MARK: ทั่วไป

struct GeneralSettings: View {
    @AppStorage("chorus.enterSends") private var enterSends = true
    @AppStorage("chorus.thinkingOpen") private var thinkingOpen = true
    @AppStorage("chorus.dismissedSetupCards") private var dismissedSetupCards = ""
    @AppStorage("chorus.scrollbarStyle") private var scrollbarStyle = "hidden"
    @AppStorage("chorus.inspectorPosition") private var inspectorPosition = "trailing"
    @AppStorage("chorus.explorerPosition") private var explorerPosition = "leading"
    @AppStorage("chorus.sidebarWidth") private var sidebarWidth: Double = 260
    @AppStorage("chorus.explorerWidth") private var explorerWidth: Double = 260
    @AppStorage("chorus.inspectorWidth") private var inspectorWidth: Double = 330
    @AppStorage("chorus.codeViewerWidth") private var codeViewerWidth: Double = 520

    var body: some View {
        SettingsGroup(title: "การพิมพ์") {
            SettingsRow(title: "Enter เพื่อส่ง",
                        info: enterSends ? "Shift+Enter ขึ้นบรรทัดใหม่" : "⌘Enter ส่ง · Enter ขึ้นบรรทัดใหม่") {
                Toggle("Enter เพื่อส่ง", isOn: $enterSends).labelsHidden().toggleStyle(.switch)
            }
            RowDivider()
            SettingsRow(title: "เปิดกล่องความคิดไว้",
                        info: "แสดงสิ่งที่ agent คิดระหว่างทำงาน ปิดไว้ก็กดเปิดดูทีหลังได้") {
                Toggle("เปิดกล่องความคิดไว้", isOn: $thinkingOpen).labelsHidden().toggleStyle(.switch)
            }
        }
        SettingsGroup(title: "หน้าต่าง") {
            SettingsRow(title: "แถบเลื่อน", info: ScrollbarStyle(rawValue: scrollbarStyle)?.hint) {
                Picker("แถบเลื่อน", selection: $scrollbarStyle) {
                    ForEach(ScrollbarStyle.allCases) { Text($0.label).tag($0.rawValue) }
                }
                .labelsHidden().fixedSize()
            }
            RowDivider()
            SettingsRow(title: "แผงบริบท", info: "แผงบริบท สิ่งที่แก้ และความจำ") {
                Picker("แผงบริบท", selection: $inspectorPosition) {
                    Text("ขวา").tag("trailing")
                    Text("ซ้าย").tag("leading")
                }
                .pickerStyle(.segmented).labelsHidden().frame(width: 120)
            }
            RowDivider()
            SettingsRow(title: "แผงไฟล์", info: "โครงสร้างไฟล์และ Git worktree") {
                Picker("แผงไฟล์", selection: $explorerPosition) {
                    Text("ซ้าย").tag("leading")
                    Text("ขวา").tag("trailing")
                }
                .pickerStyle(.segmented).labelsHidden().frame(width: 120)
            }
            RowDivider()
            SettingsRow(title: "ขนาดแผง", info: "ลากเส้นแบ่งเพื่อปรับความกว้างของแต่ละแผง ปุ่มนี้คืนค่าเดิมทั้งหมด") {
                Button("คืนค่าเริ่มต้น") {
                    sidebarWidth = 260
                    explorerWidth = 260
                    inspectorWidth = 330
                    codeViewerWidth = 520
                }
            }
        }
        SettingsGroup(title: "คำแนะนำ") {
            SettingsRow(title: "การ์ดแนะนำที่ปิดไว้", info: "การ์ดตั้งค่า agent, Auto route และเครื่องมือเสริม") {
                Button("แสดงอีกครั้ง") { dismissedSetupCards = "" }
                    .disabled(dismissedSetupCards.isEmpty)
            }
        }
    }
}

// MARK: การส่งงาน

struct RoutingSettings: View {
    @EnvironmentObject private var engine: Engine
    let workspace: String
    @AppStorage(SettingsKey.defaultMode) private var defaultMode = "auto"
    @AppStorage(SettingsKey.defaultRoutePreset) private var defaultPreset = "balanced"
    @AppStorage("chorus.autoRoute") private var autoRoute = true
    @AppStorage("chorus.failoverAccounts") private var failoverAccounts = true
    @AppStorage("chorus.failoverProviders") private var failoverProviders = true
    @AppStorage("chorus.conserveBatteryOnInference") private var conserveBattery = true
    @State private var showLearnedRoutes = false

    var body: some View {
        SettingsGroup(title: "ค่าเริ่มต้นของ session ใหม่") {
            SettingsRow(title: "โหมด",
                        info: "Auto route: router เลือกโมเดลให้ทุกเทิร์น · Orchestra: โมเดลหลักวางแผนแล้วแจกงานให้ agent ย่อย เปลี่ยนราย session ได้ที่ช่องพิมพ์") {
                Picker("โหมด", selection: $defaultMode) {
                    Text("Auto route").tag("auto")
                    Text("Orchestra").tag("orchestra")
                }
                .pickerStyle(.segmented).labelsHidden().frame(width: 190)
            }
            RowDivider()
            SettingsRow(title: "Route preset",
                        info: "ประหยัด: โมเดลเล็กลงหนึ่งขั้น · สมดุล: ตามความยากของงาน · เต็มกำลัง: ใหญ่ขึ้นหนึ่งขั้น") {
                Picker("Route preset", selection: $defaultPreset) {
                    Text("ประหยัด").tag("economy")
                    Text("สมดุล").tag("balanced")
                    Text("เต็มกำลัง").tag("max")
                }
                .pickerStyle(.segmented).labelsHidden().frame(width: 230)
            }
            RowDivider()
            SettingsRow(title: "ให้ router เลือกโมเดล",
                        info: "ปิด = ใช้โมเดลที่เลือกเองในช่องพิมพ์ทุกเทิร์น") {
                Toggle("ให้ router เลือกโมเดล", isOn: $autoRoute).labelsHidden().toggleStyle(.switch)
            }
        }
        SettingsGroup(title: "เมื่อชนลิมิต") {
            SettingsRow(title: "สลับบัญชีของเจ้าเดิมก่อน",
                        info: "เช่น Claude งาน → Claude ส่วนตัว Chorus ส่ง brief (เป้าหมาย สิ่งที่ทำแล้ว diff) ให้ทำต่อจากจุดเดิม") {
                Toggle("สลับบัญชีของเจ้าเดิมก่อน", isOn: $failoverAccounts).labelsHidden().toggleStyle(.switch)
            }
            RowDivider()
            SettingsRow(title: "แล้วสลับไปเจ้าอื่น", info: "เช่น Claude → Codex ความจำและ session เป็นชุดเดียวกัน") {
                Toggle("แล้วสลับไปเจ้าอื่น", isOn: $failoverProviders).labelsHidden().toggleStyle(.switch)
            }
        }
        SettingsGroup(title: "Router") {
            SettingsRow(title: "ประหยัดแบตตอนถอดสาย",
                        info: "ใช้ตัวจัดประเภทแบบเร็วแทนโมเดลในเครื่อง เมื่อเปิด Low Power Mode หรือเครื่องร้อน") {
                Toggle("ประหยัดแบตตอนถอดสาย", isOn: $conserveBattery).labelsHidden().toggleStyle(.switch)
            }
            RowDivider()
            SettingsRow(title: "สิ่งที่เรียนรู้แล้ว",
                        detail: engine.learnedRoutes.isEmpty ? "ยังไม่มี" : "\(engine.learnedRoutes.count) รายการ",
                        info: "จำจากตอนที่คุณเปลี่ยนโมเดลเอง เพื่อเลือกให้ตรงใจขึ้นในงานแบบเดียวกัน") {
                Button("ดู") { showLearnedRoutes = true }
            }
        }
        .onAppear { engine.loadLearnedRoutes(workspace: workspace) }
        .sheet(isPresented: $showLearnedRoutes) {
            LearnedRoutesSheet(workspace: workspace).environmentObject(engine)
        }
    }
}

// MARK: สิทธิ์และความปลอดภัย

struct SafetySettings: View {
    let workspace: String
    @AppStorage("chorus.defaultPermission") private var defaultPermission = Int(PermissionLevel.ask.rawValue)
    @AppStorage(SpotlightIndexer.settingKey) private var spotlightIndexing = false
    @AppStorage(SettingsKey.dictationOnDevice) private var dictationOnDevice = true
    @AppStorage(SettingsKey.trustedProjects) private var trustedRaw = ""

    private var projects: [String] {
        var seen = Set<String>()
        let recents = UserDefaults.standard.stringArray(forKey: "chorus.recentWorkspaces") ?? []
        return ([workspace] + recents).filter { !$0.isEmpty && seen.insert($0).inserted }.prefix(4).map { $0 }
    }

    private var trusted: Set<String> { Set(trustedRaw.split(separator: "\n").map(String.init)) }

    private func setTrusted(_ path: String, _ on: Bool) {
        var t = trusted
        if on { t.insert(path) } else { t.remove(path) }
        trustedRaw = t.sorted().joined(separator: "\n")
    }

    var body: some View {
        SettingsGroup(title: "สิทธิ์") {
            SettingsRow(title: "ค่าเริ่มต้น",
                        info: (PermissionLevel(rawValue: Int32(defaultPermission))?.hint ?? "")
                            + " · ทั้งสองโหมดยังอยู่ใน sandbox เขียนได้เฉพาะ workspace") {
                Picker("สิทธิ์เริ่มต้น", selection: $defaultPermission) {
                    ForEach(PermissionLevel.allCases, id: \.self) { Text($0.label).tag(Int($0.rawValue)) }
                }
                .pickerStyle(.segmented).labelsHidden().frame(width: 170)
            }
        }
        SettingsGroup(title: "โปรเจกต์ที่เชื่อถือ",
                      footer: "repo ใหม่จะไม่รันอะไรจนกว่าคุณเปิดเอง") {
            if projects.isEmpty {
                SettingsRow(title: "ยังไม่มีโปรเจกต์") { EmptyView() }
            }
            ForEach(Array(projects.enumerated()), id: \.element) { i, path in
                if i > 0 { RowDivider() }
                SettingsRow(title: URL(fileURLWithPath: path).lastPathComponent,
                            info: i == 0 ? "เปิดแล้ว Chorus จะรัน cargo check / linter / .chorus/hooks หลังทุกเทิร์น ภายใน sandbox" : nil) {
                    Toggle("เชื่อถือ \(URL(fileURLWithPath: path).lastPathComponent)",
                           isOn: Binding(get: { trusted.contains(path) }, set: { setTrusted(path, $0) }))
                        .labelsHidden().toggleStyle(.switch)
                        .help(path)
                }
            }
        }
        SettingsGroup(title: "ความเป็นส่วนตัว") {
            SettingsRow(title: "ให้ Spotlight ค้นหา session",
                        info: "ชื่อและเป้าหมายของ session จะไปอยู่ใน index ของระบบ ซึ่งอยู่นอกข้อมูลของ Chorus ปิดแล้วลบออกทันที") {
                Toggle("ให้ Spotlight ค้นหา session", isOn: $spotlightIndexing).labelsHidden().toggleStyle(.switch)
                    .onChange(of: spotlightIndexing) { _, on in
                        if !on { SpotlightIndexer.shared.clearAll() }
                    }
            }
            RowDivider()
            SettingsRow(title: "พิมพ์ด้วยเสียงในเครื่องเท่านั้น",
                        info: "ถอดเสียงบน Mac ไม่ส่งเสียงออกไปนอกเครื่อง ถ้าภาษานั้นไม่มีโมเดลในเครื่อง การพิมพ์ด้วยเสียงจะใช้ไม่ได้") {
                Toggle("พิมพ์ด้วยเสียงในเครื่องเท่านั้น", isOn: $dictationOnDevice).labelsHidden().toggleStyle(.switch)
            }
        }
    }
}

// MARK: Review และ checkpoint

struct ReviewSettings: View {
    @AppStorage(SettingsKey.showChorusFiles) private var showChorusFiles = false

    var body: some View {
        SettingsGroup(title: "Review") {
            SettingsRow(title: "แสดงไฟล์ที่ Chorus เขียนเอง",
                        info: "CLAUDE.md, AGENTS.md, GEMINI.md และ .claude/skills ที่ Chorus ฉายจากความจำ ปิด = ซ่อนจากรายการที่แก้") {
                Toggle("แสดงไฟล์ที่ Chorus เขียนเอง", isOn: $showChorusFiles).labelsHidden().toggleStyle(.switch)
            }
        }
        SettingsGroup(title: "Checkpoint") {
            SettingsRow(title: "จุดย้อนกลับก่อนทุกเทิร์น",
                        info: "Chorus snapshot โปรเจกต์ลง repo เงาที่ .chorus/ckpt ก่อน agent เริ่ม ตาม .gitignore ของคุณ ย้อนได้จากแผง Changes") {
                Text("เปิดเสมอ").foregroundStyle(.secondary)
            }
        }
    }
}

// MARK: การใช้งานและค่าใช้จ่าย

struct CostSettings: View {
    @AppStorage(SettingsKey.thbRate) private var thbRate = ""

    private var rateIsValid: Bool {
        let t = thbRate.trimmingCharacters(in: .whitespaces)
        return t.isEmpty || (Double(t).map { $0 > 0 } ?? false)
    }

    var body: some View {
        SettingsGroup(title: "สกุลเงิน") {
            SettingsRow(title: "อัตราบาท",
                        detail: rateIsValid ? nil : "ใส่ตัวเลขมากกว่า 0",
                        info: "แสดง USD ก่อนเสมอ ใส่อัตราแล้วจะแปลงเป็นบาทให้ด้วย Chorus ไม่ดึงอัตราจากเน็ต") {
                HStack(spacing: 6) {
                    Text("1 USD =").foregroundStyle(.secondary)
                    TextField("ยังไม่ตั้ง", text: $thbRate)
                        .textFieldStyle(.roundedBorder)
                        .multilineTextAlignment(.trailing)
                        .frame(width: 80)
                        .accessibilityLabel("อัตราบาทต่อ 1 USD")
                    Text("บาท").foregroundStyle(.secondary)
                }
            }
        }
        SettingsGroup(title: "ตัวเลขที่แสดง") {
            SettingsRow(title: "Token", info: "แสดงทุกเจ้า ทั้งต่อเทิร์นและต่อ session") {
                Text("เสมอ").foregroundStyle(.secondary)
            }
            RowDivider()
            SettingsRow(title: "มูลค่าเทียบ API",
                        info: "แสดงเฉพาะเมื่อ bridge รายงานราคามาเอง (ตอนนี้มีแค่ Claude) ผู้ใช้แบบ subscription ไม่ได้จ่ายตาม token จริง") {
                Text("เมื่อมีข้อมูล").foregroundStyle(.secondary)
            }
        }
    }
}

// MARK: พื้นที่และประสิทธิภาพ

struct StorageSettings: View {
    @EnvironmentObject private var engine: Engine

    var body: some View {
        let totalRamGB = Int(round(Double(ProcessInfo.processInfo.physicalMemory) / (1024 * 1024 * 1024)))
        let isCompactRam = totalRamGB <= 16
        SettingsGroup(title: "โมเดลในเครื่อง") {
            SettingsRow(title: "หน่วยความจำ \(totalRamGB) GB",
                        info: isCompactRam
                            ? "เหมาะกับ router 0.8B (529 MB) คู่กับการย่อแบบ Extractive (0 MB) เพื่อไม่ให้เครื่อง swap"
                            : "พอสำหรับรันทั้ง router และตัวย่อบริบทในเครื่องพร้อมกัน") {
                Label(isCompactRam ? "แนะนำ 0.8B" : "พร้อมทุกโมเดล", systemImage: "memorychip")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(isCompactRam ? .orange : .green)
            }
            RowDivider()
            routerRow
            if let err = engine.localModelError {
                RowDivider()
                SettingsRow(title: "ข้อผิดพลาด", detail: err) { EmptyView() }
            }
            RowDivider()
            compactorRow(isCompactRam: isCompactRam)
        }
        SettingsGroup(title: "ข้อมูล") {
            SettingsRow(title: "โฟลเดอร์ข้อมูล", info: "~/.chorus เก็บบัญชีเพิ่มเติมและสถานะลิมิต") {
                Button("เปิด") {
                    let u = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".chorus")
                    try? FileManager.default.createDirectory(at: u, withIntermediateDirectories: true)
                    NSWorkspace.shared.open(u)
                }
            }
        }
        .onAppear { engine.loadLocalModels() }
    }

    @ViewBuilder private var routerRow: some View {
        if let rm = engine.localModels.first(where: { $0.id.contains("openthai") }) {
            if engine.isDownloadingLocalModel {
                SettingsRow(title: "กำลังดาวน์โหลด router", detail: engine.localModelDownloadStatusText) {
                    HStack(spacing: 8) {
                        ProgressView(value: engine.localModelDownloadProgress)
                            .progressViewStyle(.linear).frame(width: 120)
                        Button("ยกเลิก") { engine.cancelLocalModelDownload() }
                            .disabled(!engine.canCancelLocalModelDownload)
                    }
                }
            } else if rm.isInstalled {
                SettingsRow(title: "Router (OpenThai 0.8B)",
                            detail: rm.size_bytes.map { String(format: "%.0f MB · ติดตั้งแล้ว", Double($0) / (1024 * 1024)) } ?? "ติดตั้งแล้ว",
                            info: "โมเดลเล็กที่ใช้เลือก agent และโมเดลให้แต่ละเทิร์น รันบน Metal ไม่ใช้โควตา ไม่ส่งข้อมูลออก") {
                    Button("ลบ") { engine.deleteLocalModel(modelId: rm.id) }
                }
            } else {
                SettingsRow(title: "Router (OpenThai 0.8B)",
                            info: "เลือก agent และโมเดลให้แต่ละเทิร์น ภาษาไทยและอังกฤษ ยังไม่ติดตั้งก็ใช้ตัวจัดประเภทแบบเร็ว (3–8 ms) แทน") {
                    Button("ดาวน์โหลด 529 MB") { engine.downloadLocalModel(rm.spec) }
                        .buttonStyle(.borderedProminent)
                }
            }
        } else {
            SettingsRow(title: "Router", detail: "กำลังตรวจ…") {
                Button("ตรวจใหม่") { engine.loadLocalModels() }
            }
        }
    }

    @ViewBuilder private func compactorRow(isCompactRam: Bool) -> some View {
        if let cm = engine.localModels.first(where: { $0.id.contains("qwen2.5-coder") }) {
            if cm.isInstalled {
                SettingsRow(title: "ตัวย่อบริบท (Qwen 2.5 Coder)", detail: "ติดตั้งแล้ว",
                            info: "ย่อประวัติแชทเก่าด้วย AI ตอนเครื่องว่าง") {
                    Button("ลบ") { engine.deleteLocalModel(modelId: cm.id) }
                }
            } else {
                SettingsRow(title: "ตัวย่อบริบท (ไม่บังคับ)",
                            detail: "ตอนนี้ใช้ Extractive",
                            info: isCompactRam
                                ? "Extractive ย่อได้เร็วและไม่กินแรม เหมาะกับเครื่องนี้ โหลด Qwen 2.5 Coder ถ้าอยากได้สรุปที่ละเอียดกว่า"
                                : "Extractive ย่อได้เร็วและไม่กินแรม โหลด Qwen 2.5 Coder เพื่อสรุปที่ละเอียดกว่า") {
                    Button("ดาวน์โหลด 893 MB") { engine.downloadLocalModel(cm.spec) }
                        .disabled(engine.isDownloadingLocalModel)
                }
            }
        }
    }
}

// MARK: เกี่ยวกับ

struct AboutSettings: View {
    private var version: String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0.1.0"
    }

    var body: some View {
        SettingsGroup(title: "Chorus") {
            SettingsRow(title: "เวอร์ชัน") {
                Text("v\(version)").font(.callout.monospaced()).foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            RowDivider()
            SettingsRow(title: "ตรวจหาอัปเดต", info: "Sparkle ตรวจลายเซ็นของไฟล์อัปเดตก่อนติดตั้งทุกครั้ง") {
                Text("เมนู Chorus › Check for Updates…").font(.caption).foregroundStyle(.secondary)
            }
        }
        SettingsGroup(title: "สัญญาอนุญาต") {
            SettingsRow(title: "ไอคอนไฟล์",
                        detail: "Catppuccin Icons for VS Code v1.26.0 · MIT",
                        info: "© 2023 Catppuccin, © 2023 thang-nm") {
                Link(destination: URL(string: "https://github.com/catppuccin/vscode-icons")!) {
                    Image(systemName: "arrow.up.forward.square")
                }
                .help("github.com/catppuccin/vscode-icons")
                .accessibilityLabel("เปิดหน้า Catppuccin Icons")
            }
        }
    }
}

// MARK: accounts & usage

struct AccountsSettings: View {
    @EnvironmentObject var engine: Engine
    @State private var adding: String?
    @State private var newName = ""
    @State private var removing: Engine.Account?

    var body: some View {
        ForEach(engine.agents) { agent in
            let rows = engine.accounts.filter { $0.agent == agent.id }
            SettingsGroup(title: agent.label,
                          footer: agent.id == "claude"
                              ? "Claude ไม่บอกเปอร์เซ็นต์การใช้งานออกมา Chorus รู้แค่ตอนที่ติดลิมิตและเวลารีเซ็ตที่มันแจ้ง"
                              : agent.id == "antigravity"
                              ? "ดึงโควตาเรียลไทม์แยกตามกลุ่มโมเดลจาก Antigravity Language Server โดยตรง"
                              : "ตัวเลขมาจาก log ของ Codex เอง อัปเดตทุกครั้งที่บัญชีนั้นตอบ") {
                ForEach(Array(rows.enumerated()), id: \.element.id) { i, acc in
                    if i > 0 { Divider().padding(.leading, 12) }
                    AccountRow(account: acc, onRemove: { removing = acc })
                }
                Divider().padding(.leading, 12)
                Button { adding = agent.id; newName = "" } label: {
                    Label("เพิ่มบัญชี \(agent.label)", systemImage: "plus.circle")
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(InteractiveStyle(cornerRadius: 8, padding: .init(top: 8, leading: 12, bottom: 8, trailing: 12)))
            }
        }
        HStack {
            Spacer()
            Button { engine.loadUsage(); engine.loadAgents() } label: { Label("ตรวจใหม่", systemImage: "arrow.clockwise") }
        }
        .alert("เพิ่มบัญชี", isPresented: Binding(get: { adding != nil }, set: { if !$0 { adding = nil } })) {
            TextField("ชื่อ เช่น work, personal", text: $newName)
            Button("เพิ่มแล้ว login") {
                if let a = adding { engine.addAccount(agent: a, name: newName.trimmingCharacters(in: .whitespaces).lowercased()) }
                adding = nil
            }
            Button("ยกเลิก", role: .cancel) { adding = nil }
        } message: {
            Text("แต่ละบัญชีมีโฟลเดอร์ login ของตัวเอง Chorus จะเปิด Terminal ให้ login — ไม่เก็บรหัสหรือ token เอง")
        }
        .confirmationDialog("ลบบัญชี \(removing?.title ?? "")?",
                            isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
                            titleVisibility: .visible) {
            Button("ลบบัญชีและ login ของมัน", role: .destructive) {
                if let r = removing { engine.removeAccount(agent: r.agent, name: r.account) }
                removing = nil
            }
            Button("ยกเลิก", role: .cancel) { removing = nil }
        } message: {
            Text("ลบโฟลเดอร์ \(removing?.dir ?? "") ถ้าจะใช้อีกต้อง login ใหม่")
        }
    }
}

struct AccountRow: View {
    @EnvironmentObject var engine: Engine
    let account: Engine.Account
    let onRemove: () -> Void
    @AppStorage private var preferred: String

    init(account: Engine.Account, onRemove: @escaping () -> Void) {
        self.account = account
        self.onRemove = onRemove
        _preferred = AppStorage(wrappedValue: "", "chorus.preferred.\(account.agent)")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Circle().fill(!account.ready ? Color.orange : account.limited ? .red : .green)
                    .frame(width: 8, height: 8)
                Text(account.title).fontWeight(.medium)
                if preferred == account.name || (preferred.isEmpty && account.account.isEmpty) {
                    Text("ใช้ก่อน").font(.caption2.weight(.semibold)).foregroundStyle(Color.accentColor)
                        .padding(.horizontal, 6).padding(.vertical, 1)
                        .background(Color.accentColor.opacity(0.15), in: Capsule())
                }
                Spacer()
                if !account.ready {
                    Button("Login") { engine.runLoginCommand(account.login) }
                } else if preferred != account.name {
                    Button("ใช้บัญชีนี้ก่อน") { preferred = account.name }
                        .buttonStyle(.link).font(.caption)
                }
                if !account.account.isEmpty {
                    Button(role: .destructive, action: onRemove) { Image(systemName: "trash") }
                        .buttonStyle(InteractiveStyle(cornerRadius: 6, padding: .init(top: 3, leading: 6, bottom: 3, trailing: 6)))
                        .help("ลบบัญชีนี้")
                }
            }
            if let rl = account.rate_limits {
                if let groups = rl.groups, !groups.isEmpty {
                    VStack(alignment: .leading, spacing: 10) {
                        ForEach(groups) { group in
                            VStack(alignment: .leading, spacing: 6) {
                                HStack(spacing: 5) {
                                    Text(group.name)
                                        .font(.caption.weight(.semibold))
                                        .foregroundStyle(.primary)
                                    if let desc = group.description, !desc.isEmpty {
                                        Image(systemName: "info.circle")
                                            .font(.system(size: 10))
                                            .foregroundStyle(.secondary)
                                            .help(desc)
                                    }
                                }
                                if let p = group.five_hour {
                                    window("5 ชั่วโมง", p)
                                }
                                if let s = group.weekly {
                                    window("สัปดาห์", s)
                                }
                            }
                        }
                    }
                } else {
                    if let p = rl.primary { window("5 ชั่วโมง", p) }
                    if let s = rl.secondary { window("สัปดาห์", s) }
                }
            } else if let hit = account.hit, account.limited {
                HStack(spacing: 6) {
                    Image(systemName: "hourglass").foregroundStyle(.red)
                    Text("ติดลิมิต" + (hit.resets.map { " · รีเซ็ต \($0)" } ?? "")).font(.caption)
                }
            } else if account.ready {
                Text("ยังไม่ติดลิมิต").font(.caption).foregroundStyle(.secondary)
            } else {
                Text("ยังไม่ได้ login — กด Login แล้วทำตามใน Terminal").font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, 12).padding(.vertical, 10)
    }

    private func window(_ label: String, _ w: Engine.UsageWindow) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack {
                Text(label).font(.caption)
                Spacer()
                Text("\(Int(w.used_percent))%").font(.caption.monospacedDigit().weight(.semibold))
                Text(resetLabelText(w)).font(.caption2).foregroundStyle(.secondary)
            }
            UsageBar(fraction: w.used_percent / 100)
        }
    }

    private func resetLabelText(_ w: Engine.UsageWindow) -> String {
        if let unix = w.resets_at {
            return resetText(unix)
        }
        if let raw = w.resets_label, !raw.isEmpty {
            if let idx = raw.range(of: "fully refresh in ") {
                return "รีเซ็ตใน " + String(raw[idx.upperBound...])
            }
            return raw.hasPrefix("รีเซ็ต") ? raw : ("รีเซ็ต " + raw)
        }
        return ""
    }
}

// MARK: agents & models

struct AgentsSettings: View {
    @EnvironmentObject var engine: Engine

    var body: some View {
        ForEach(engine.agents) { a in
            SettingsGroup(title: a.label) {
                SettingsRow(title: a.ready ? "พร้อมใช้" : (a.auth == "needs_login" ? "ยังไม่ได้ login" : "ยังไม่ได้ติดตั้ง CLI"),
                            detail: a.detail) {
                    if !a.ready {
                        Button(a.auth == "needs_login" ? "เริ่มเข้าสู่ระบบ" : "เริ่มติดตั้ง") {
                            engine.runLoginCommand(a.hint)
                        }
                        .help("เปิด Terminal พร้อมคำสั่งที่ต้องใช้ — ขั้นตอนเข้าสู่ระบบยังต้องทำด้วยตัวเอง")
                    }
                }
                Divider().padding(.leading, 12)
                SettingsRow(title: "โมเดล", detail: a.baseModels.map(\.label).joined(separator: " · ")) {
                    Button {
                        engine.refreshModels(force: true)
                    } label: {
                        if engine.refreshingModels { ProgressView().controlSize(.small) }
                        else { Image(systemName: "arrow.clockwise") }
                    }
                    .help("ถามรายการโมเดลล่าสุดจาก agent")
                }
            }
        }
        Text("Chorus เรียก CLI ของแต่ละเจ้าผ่าน ACP — login ทำที่ CLI นั้น ไม่ใช่ในแอป")
            .font(.caption).foregroundStyle(.secondary)
    }
}

// MARK: skills

struct SkillsSettings: View {
    @EnvironmentObject var engine: Engine
    let workspace: String
    @State private var query = ""

    var body: some View {
        let q = query.trimmingCharacters(in: .whitespaces)
        let shown = engine.skills.filter {
            q.isEmpty || $0.name.localizedCaseInsensitiveContains(q) || $0.description.localizedCaseInsensitiveContains(q)
        }
        let titles = [
            "chorus": "Chorus — ส่งให้ทุก agent",
            "project": "โปรเจกต์นี้ (.claude/skills, .agents/skills)",
            "agents": "Global ใช้ร่วมกัน (~/.agents/skills)",
            "user": "Claude (~/.claude/skills)",
            "codex": "Codex (~/.codex/skills)",
            "gemini": "Gemini / Antigravity (~/.gemini/skills)",
        ]
        TextField("ค้นหา skill…", text: $query).textFieldStyle(.roundedBorder)
        Text("\(engine.skills.count) skill ที่ agent ในโฟลเดอร์นี้มองเห็น")
            .font(.caption).foregroundStyle(.secondary)
        ForEach(["chorus", "project", "agents", "user", "codex", "gemini"], id: \.self) { src in
            let group = shown.filter { $0.source == src }
            if !group.isEmpty {
                SettingsGroup(title: titles[src] ?? src) {
                    ForEach(Array(group.enumerated()), id: \.element.id) { i, sk in
                        if i > 0 { Divider().padding(.leading, 12) }
                        SettingsRow(title: sk.name, detail: sk.description) {
                            Button { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: sk.path)]) } label: {
                                Image(systemName: "arrow.up.forward.square")
                            }
                            .buttonStyle(InteractiveStyle(cornerRadius: 6, padding: .init(top: 3, leading: 6, bottom: 3, trailing: 6)))
                            .help(sk.path)
                        }
                    }
                }
            }
        }
        if shown.isEmpty {
            Text("ไม่พบ skill — วางไว้ที่ .chorus/skills/<ชื่อ>/SKILL.md เพื่อให้ทุก agent ใช้ร่วมกัน")
                .font(.caption).foregroundStyle(.secondary)
        }
    }
}

// MARK: Orchestra

/// Defaults from the Orchestra mode decision: caps 3 / 10 / 30 min, plan
/// starts without asking, merges wait for Review, `.env` stays put. Rust will
/// enforce the caps; these are only what it reads.
struct OrchestrationSettings: View {
    let workspace: String
    @EnvironmentObject var engine: Engine

    @AppStorage(SettingsKey.orchestratorModel) private var orchestrator = ""
    @AppStorage(SettingsKey.orchestratorBackup) private var backup = ""
    @AppStorage(SettingsKey.orchestraConcurrency) private var concurrency = 3
    @AppStorage(SettingsKey.orchestraMaxChildren) private var maxChildren = 10
    @AppStorage(SettingsKey.orchestraMaxMinutes) private var maxMinutes = 30
    @AppStorage(SettingsKey.orchestraAskBeforePlan) private var askBeforePlan = false
    @AppStorage(SettingsKey.orchestraAutoMerge) private var autoMerge = false
    @AppStorage(SettingsKey.orchestraCopyEnv) private var copyEnv = false
    @State private var showRoles = false

    var body: some View {
        SettingsGroup(title: "โมเดลหลัก") {
            SettingsRow(title: "Orchestrator", info: "วางแผน ตอบคำถามง่าย และแจกงาน แต่ไม่เขียนไฟล์เอง") {
                modelPicker("Orchestrator", selection: $orchestrator, automatic: "Claude Opus 5.5 · medium")
            }
            RowDivider()
            SettingsRow(title: "ตัวสำรอง", info: "ใช้เมื่อตัวหลักชนลิมิต ส่งต่อด้วย handoff brief และบอกใน thread") {
                modelPicker("ตัวสำรอง", selection: $backup, automatic: "อัตโนมัติ (แรงสุดของเจ้าอื่น)")
            }
        }
        SettingsGroup(title: "ขีดจำกัด") {
            SettingsRow(title: "ทำงานพร้อมกัน") {
                Stepper("\(concurrency) ตัว", value: $concurrency, in: 1...8)
            }
            RowDivider()
            SettingsRow(title: "agent ย่อยต่อครั้ง") {
                Stepper("\(maxChildren) ตัว", value: $maxChildren, in: 1...50)
            }
            RowDivider()
            SettingsRow(title: "เวลาสูงสุด", info: "ถึงเพดานแล้วจะหยุดรอถาม ไม่ตัดทิ้ง") {
                Stepper("\(maxMinutes) นาที", value: $maxMinutes, in: 5...240, step: 5)
            }
        }
        SettingsGroup(title: "การรวมงาน") {
            SettingsRow(title: "ถามก่อนเริ่มแผน", info: "ปิด = แผนขึ้นใน thread แล้วเริ่มทันที กด Stop ได้ตลอด") {
                Toggle("ถามก่อนเริ่มแผน", isOn: $askBeforePlan).labelsHidden().toggleStyle(.switch)
            }
            RowDivider()
            SettingsRow(title: "Merge อัตโนมัติเมื่อไม่มี conflict", info: "ปิด = ต้องกดอนุมัติใน Review ทุกครั้ง") {
                Toggle("Merge อัตโนมัติ", isOn: $autoMerge).labelsHidden().toggleStyle(.switch)
            }
            RowDivider()
            SettingsRow(title: "คัดลอก .env ไป worktree", info: "ปิดไว้เพื่อไม่ให้ secret กระจายไปทุก worktree") {
                Toggle("คัดลอก .env ไป worktree", isOn: $copyEnv).labelsHidden().toggleStyle(.switch)
            }
        }
        SettingsGroup(title: "บทบาท") {
            SettingsRow(title: "บทบาทและ worktree", info: "บทบาทใน .chorus/agents และ worktree ของงานย่อย") {
                Button("เปิด") { showRoles = true }
                    .disabled(workspace.isEmpty)
            }
        }
        .sheet(isPresented: $showRoles) {
            VStack(spacing: 0) {
                HStack {
                    Text("บทบาทและ worktree").font(.headline)
                    Spacer()
                    Button("เสร็จ") { showRoles = false }.keyboardShortcut(.defaultAction)
                }
                .padding(14)
                Divider()
                OrchestratorView(engine: engine, workspace: workspace,
                                 onSelectWorktree: { _ in }, onDispatchParallelPrompt: { _ in })
            }
            .frame(minWidth: 760, minHeight: 560)
        }
    }

    private func modelPicker(_ label: String, selection: Binding<String>, automatic: String) -> some View {
        Picker(label, selection: selection) {
            Text(automatic).tag("")
            ForEach(engine.knownBaseModels(keeping: selection.wrappedValue), id: \.id) { m in
                Text(m.label).tag(m.id)
            }
        }
        .labelsHidden()
        .frame(maxWidth: 250)
    }
}

// MARK: - Learned Routes Sheet

struct LearnedRoutesSheet: View {
    @EnvironmentObject var engine: Engine
    @Environment(\.dismiss) private var dismiss
    let workspace: String

    @State private var search = ""
    @State private var filterAgent = "all"
    @State private var showClearConfirm = false
    @State private var copiedId: Int64? = nil

    private var filteredRoutes: [Engine.LearnedRoute] {
        var list = engine.learnedRoutes
        if filterAgent != "all" {
            list = list.filter { $0.chosen_agent.lowercased() == filterAgent.lowercased() }
        }
        let q = search.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if !q.isEmpty {
            list = list.filter {
                $0.prompt.lowercased().contains(q) ||
                $0.chosen_model.lowercased().contains(q) ||
                $0.chosen_agent.lowercased().contains(q) ||
                $0.reason.lowercased().contains(q)
            }
        }
        return list
    }

    private func agentColor(_ agent: String) -> Color {
        switch agent.lowercased() {
        case "claude": return .orange
        case "codex": return .green
        case "antigravity": return .blue
        case "gemini": return .purple
        default: return .secondary
        }
    }

    private func formatDate(_ ms: Int64) -> String {
        guard ms > 0 else { return "เมื่อสักครู่" }
        let d = Date(timeIntervalSince1970: TimeInterval(ms) / 1000.0)
        let f = DateFormatter()
        f.locale = Locale(identifier: "th_TH")
        if Calendar.current.isDateInToday(d) {
            f.dateFormat = "วันนี้ HH:mm"
        } else if Calendar.current.isDateInYesterday(d) {
            f.dateFormat = "เมื่อวาน HH:mm"
        } else {
            f.dateFormat = "d MMM yyyy HH:mm"
        }
        return f.string(from: d)
    }

    var body: some View {
        VStack(spacing: 0) {
            // Header
            HStack(alignment: .top, spacing: 14) {
                ZStack {
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .fill(Color.purple.opacity(0.15))
                        .frame(width: 44, height: 44)
                    Image(systemName: "brain.head.profile")
                        .font(.system(size: 22, weight: .semibold))
                        .foregroundStyle(.purple)
                }
                VStack(alignment: .leading, spacing: 3) {
                    Text("สิ่งที่ Auto Route เรียนรู้แล้ว")
                        .font(.headline)
                    Text("ความจำการเลือกโมเดลและ agent ที่ระบบบันทึกไว้ ช่วยให้ Chorus แนะนำโมเดลสำหรับงานลักษณะเดียวกันได้ทันที (<5ms)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 12)
                Button("เสร็จ") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
            .padding(18)
            .background(Color.primary.opacity(0.02))

            Divider()

            // Filter bar
            HStack(spacing: 12) {
                HStack(spacing: 6) {
                    Image(systemName: "magnifyingglass")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    TextField("ค้นหาจากข้อความงาน, โมเดล, หรือเหตุผล...", text: $search)
                        .textFieldStyle(.plain)
                        .font(.callout)
                    if !search.isEmpty {
                        Button {
                            search = ""
                        } label: {
                            Image(systemName: "xmark.circle.fill")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 8, style: .continuous))

                Picker("", selection: $filterAgent) {
                    Text("agent ทั้งหมด").tag("all")
                    Text("Claude").tag("claude")
                    Text("Codex").tag("codex")
                    Text("Antigravity").tag("antigravity")
                }
                .labelsHidden()
                .frame(width: 140)

                Spacer(minLength: 4)

                if !engine.learnedRoutes.isEmpty {
                    Button {
                        exportLearnedRoutes()
                    } label: {
                        Label("ส่งออก JSON", systemImage: "square.and.arrow.up")
                            .font(.caption)
                    }
                    .buttonStyle(.bordered)

                    Button(role: .destructive) {
                        showClearConfirm = true
                    } label: {
                        Label("ล้างทั้งหมด", systemImage: "trash")
                            .font(.caption)
                    }
                    .buttonStyle(.bordered)
                    .tint(.red)
                }
            }
            .padding(.horizontal, 18)
            .padding(.vertical, 10)
            .background(Color.primary.opacity(0.01))

            Divider()

            // List of items or empty state
            if engine.isLoadingLearnedRoutes {
                VStack(spacing: 12) {
                    ProgressView()
                    Text("กำลังโหลดประวัติการเรียนรู้...").font(.caption).foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if filteredRoutes.isEmpty {
                VStack(spacing: 14) {
                    Image(systemName: search.isEmpty ? "sparkles" : "magnifyingglass")
                        .font(.system(size: 38))
                        .foregroundStyle(.tertiary)
                    Text(search.isEmpty ? "ยังไม่มีประวัติการเรียนรู้" : "ไม่พบรายการที่ตรงกับ \"\(search)\"")
                        .font(.callout.weight(.medium))
                    Text(search.isEmpty
                         ? "เมื่อคุณส่งคำถามหรือเลือกเปลี่ยนโมเดลในกล่องแชท Auto Route จะจดจำไว้เพื่อแนะนำโมเดลที่แม่นยำยิ่งขึ้นในครั้งถัดไป"
                         : "ลองเปลี่ยนคำค้นหาหรือเลือกดู agent ทั้งหมด")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: 380)

                    if search.isEmpty && engine.learnedRoutes.isEmpty {
                        Button {
                            seedExampleRoutes()
                        } label: {
                            Label("สร้างตัวอย่างข้อมูลเพื่อทดสอบ", systemImage: "plus.bubble")
                        }
                        .buttonStyle(.borderedProminent)
                        .padding(.top, 6)
                    }
                }
                .padding(32)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    LazyVStack(spacing: 10) {
                        HStack {
                            Text("พบ \(filteredRoutes.count) รายการ")
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                            Spacer()
                        }
                        .padding(.horizontal, 4)

                        ForEach(filteredRoutes) { route in
                            LearnedRouteCard(route: route,
                                             agentColor: agentColor(route.chosen_agent),
                                             formattedDate: formatDate(route.created_ms),
                                             isCopied: copiedId == route.id) {
                                NSPasteboard.general.clearContents()
                                NSPasteboard.general.setString(route.prompt, forType: .string)
                                copiedId = route.id
                                DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                                    if copiedId == route.id { copiedId = nil }
                                }
                            }
                        }
                    }
                    .padding(18)
                }
            }
        }
        .frame(minWidth: 640, idealWidth: 700, minHeight: 480, idealHeight: 540)
        .confirmationDialog(
            "คุณต้องการล้างประวัติที่ Auto Route เรียนรู้ไว้ทั้งหมดหรือไม่?",
            isPresented: $showClearConfirm,
            titleVisibility: .visible
        ) {
            Button("ล้างประวัติทั้งหมด", role: .destructive) {
                engine.clearLearnedRoutes(workspace: workspace)
            }
            Button("ยกเลิก", role: .cancel) {}
        } message: {
            Text("เมื่อล้างแล้ว Auto Route จะเริ่มเรียนรู้ใหม่จากคำสั่งและตัวเลือกถัดไปของคุณ")
        }
    }

    /// Example routes use whatever the agents actually report (7.19), so the
    /// seeded history never points at a model id that no longer exists.
    private func seedExampleRoutes() {
        let examples: [(prompt: String, agent: String, reason: String)] = [
            ("ช่วยรีแฟกเตอร์ฟังก์ชันใน Rust ให้เป็น async หน่อย", "claude", "งานเขียนโค้ด Rust ลึกซึ้ง เหมาะกับ Claude"),
            ("ออกแบบ UI Settings ด้วย SwiftUI บน macOS", "codex", "งานออกแบบ Swift/macOS เหมาะกับ Codex"),
            ("อธิบายความหมายของ FFI สั้นๆ ให้หน่อย", "antigravity", "คำถามสั้นต้องการคำตอบเร็ว เหมาะกับโมเดลเร็ว")
        ]
        for ex in examples {
            guard let agent = engine.agents.first(where: { $0.id == ex.agent }) else { continue }
            engine.recordLearnedRoute(workspace: workspace, prompt: ex.prompt, model: agent.defaultModel,
                                      agent: ex.agent, reason: ex.reason)
        }
    }

    private func exportLearnedRoutes() {
        let panel = NSSavePanel()
        panel.title = "ส่งออกประวัติการเรียนรู้ Auto Router"
        let dateStr = ISO8601DateFormatter().string(from: Date()).prefix(10)
        panel.nameFieldStringValue = "router-overrides-\(dateStr).json"
        panel.allowedContentTypes = [.json]
        panel.canCreateDirectories = true
        panel.begin { response in
            guard response == .OK, let url = panel.url else { return }
            struct ExportPayload: Encodable {
                struct EvalCase: Encodable {
                    let language: String
                    let text: String
                    let kind: String
                    let level: String
                    let expected: String
                    let reason: String
                }
                let exported_at: String
                let workspace: String
                let total_records: Int
                let records: [Engine.LearnedRoute]
                let eval_cases: [EvalCase]
            }

            let evalCases: [ExportPayload.EvalCase] = engine.learnedRoutes.map { r in
                let isThai = r.prompt.unicodeScalars.contains { $0.value >= 0x0E00 && $0.value <= 0x0E7F }
                return ExportPayload.EvalCase(
                    language: isThai ? "th" : "en",
                    text: r.prompt,
                    kind: r.prompt.count > 200 ? "think" : "do",
                    level: "moderate",
                    expected: "\(r.chosen_agent):\(r.chosen_model)",
                    reason: r.reason
                )
            }

            let payload = ExportPayload(
                exported_at: ISO8601DateFormatter().string(from: Date()),
                workspace: workspace,
                total_records: engine.learnedRoutes.count,
                records: engine.learnedRoutes,
                eval_cases: evalCases
            )

            let encoder = JSONEncoder()
            encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            if let data = try? encoder.encode(payload) {
                try? data.write(to: url, options: .atomic)
            }
        }
    }
}

struct LearnedRouteCard: View {
    let route: Engine.LearnedRoute
    let agentColor: Color
    let formattedDate: String
    let isCopied: Bool
    let onCopy: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                // Agent badge
                HStack(spacing: 4) {
                    Circle().fill(agentColor).frame(width: 7, height: 7)
                    Text(route.chosen_agent.capitalized)
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(agentColor)
                }
                .padding(.horizontal, 7).padding(.vertical, 3)
                .background(agentColor.opacity(0.12), in: Capsule())

                // Model badge
                HStack(spacing: 3) {
                    Image(systemName: "cpu")
                        .font(.system(size: 9))
                    Text(route.chosen_model)
                        .font(.caption.monospaced())
                }
                .foregroundStyle(.secondary)
                .padding(.horizontal, 7).padding(.vertical, 3)
                .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 5, style: .continuous))

                Spacer()

                Text(formattedDate)
                    .font(.caption2)
                    .foregroundStyle(.tertiary)

                Button {
                    onCopy()
                } label: {
                    Image(systemName: isCopied ? "checkmark" : "doc.on.doc")
                        .font(.caption)
                        .foregroundStyle(isCopied ? .green : .secondary)
                }
                .buttonStyle(.plain)
                .help("คัดลอกข้อความ")
            }

            // Prompt preview
            VStack(alignment: .leading, spacing: 3) {
                Text("ข้อความงาน / Prompt:")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
                Text(route.prompt)
                    .font(.callout)
                    .foregroundStyle(.primary)
                    .lineLimit(4)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(10)
            .background(Color.primary.opacity(0.03), in: RoundedRectangle(cornerRadius: 6, style: .continuous))

            // Reason
            if !route.reason.isEmpty {
                HStack(alignment: .top, spacing: 6) {
                    Image(systemName: "lightbulb.fill")
                        .font(.caption)
                        .foregroundStyle(.yellow)
                        .padding(.top, 1)
                    Text(route.reason)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .padding(12)
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .stroke(Color.primary.opacity(0.08), lineWidth: 1)
        )
    }
}

