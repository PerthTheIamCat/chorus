import AppKit
import SwiftUI

/// Settings, laid out like System Settings: a list of sections on the left,
/// the section on the right.
struct SettingsSheet: View {
    @EnvironmentObject var engine: Engine
    @Environment(\.dismiss) private var dismiss
    let workspace: String
    @State var section: Section = .accounts
    @AppStorage("chorus.autoCatalogUpdates") private var autoCatalogUpdates = true

    enum Section: String, CaseIterable, Identifiable {
        case general = "ทั่วไป"
        case accounts = "บัญชีและการใช้งาน"
        case agents = "เอเจนต์และโมเดล"
        case orchestration = "ออร์เคสตรา (Multi-Agent)"
        case companion = "Mobile Companion"
        case skills = "Skills"
        case integrations = "เครื่องมือเสริม"
        var id: String { rawValue }
        var icon: String {
            switch self {
            case .general: "gearshape"
            case .accounts: "person.2.badge.gearshape"
            case .agents: "cpu"
            case .orchestration: "point.3.connected.trianglepath.dotted"
            case .companion: "iphone.and.arrow.forward"
            case .skills: "sparkles"
            case .integrations: "puzzlepiece.extension"
            }
        }
    }

    init(workspace: String, section: Section = .accounts) {
        self.workspace = workspace
        _section = State(initialValue: section)
    }

    var body: some View {
        HStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 2) {
                Text("ตั้งค่า").font(.title3.bold()).padding(.horizontal, 10).padding(.bottom, 8)
                ForEach(Section.allCases) { s in
                    Button { section = s } label: {
                        Label(s.rawValue, systemImage: s.icon)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 8, selected: section == s,
                                                  padding: .init(top: 6, leading: 10, bottom: 6, trailing: 10)))
                }
                Spacer()
                Button("เสร็จ") { dismiss() }
                    .keyboardShortcut(.defaultAction)
                    .padding(.horizontal, 10)
            }
            .padding(14)
            .frame(width: 210)
            .background(Color.primary.opacity(0.03))

            Divider()

            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    switch section {
                    case .general: GeneralSettings(workspace: workspace)
                    case .accounts: AccountsSettings()
                    case .agents: AgentsSettings()
                    case .orchestration: OrchestrationSettings(workspace: workspace)
                    case .companion: RemotePairingView()
                    case .skills: SkillsSettings(workspace: workspace)
                    case .integrations:
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
                .padding(22)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .chorusScrollIndicators()
        }
        .frame(width: 760, height: 560)
        .onAppear {
            engine.loadAgents()
            engine.loadUsage()
            engine.loadSkills(cwd: workspace)
            engine.loadCatalog(project: workspace, automaticUpdates: autoCatalogUpdates)
        }
    }
}

// MARK: shared bits

struct SettingsGroup<Content: View>: View {
    let title: String
    var footer: String?
    @ViewBuilder var content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).font(.headline)
            VStack(alignment: .leading, spacing: 0) { content() }
                .padding(.vertical, 4)
                .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            if let footer {
                Text(footer).font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

struct SettingsRow<Trailing: View>: View {
    let title: String
    var detail: String?
    @ViewBuilder var trailing: () -> Trailing

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                if let detail {
                    Text(detail).font(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 8)
            trailing()
        }
        .padding(.horizontal, 12).padding(.vertical, 9)
    }
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

// MARK: general

struct GeneralSettings: View {
    @EnvironmentObject private var engine: Engine
    var workspace: String = ""
    @AppStorage("chorus.defaultPermission") private var defaultPermission = Int(PermissionLevel.ask.rawValue)
    @AppStorage("chorus.enterSends") private var enterSends = true
    @AppStorage(SpotlightIndexer.settingKey) private var spotlightIndexing = false
    @AppStorage("chorus.thinkingOpen") private var thinkingOpen = true
    @AppStorage("chorus.failoverAccounts") private var failoverAccounts = true
    @AppStorage("chorus.failoverProviders") private var failoverProviders = true
    @AppStorage("chorus.autoRoute") private var autoRoute = true
    @AppStorage("chorus.conserveBatteryOnInference") private var conserveBatteryOnInference = true
    @AppStorage("chorus.routerModel") private var routerModel = "chorus-openthai-systemone:0.8b"
    @AppStorage("chorus.routerModelAuto") private var routerModelAuto = true
    @AppStorage("chorus.dismissedSetupCards") private var dismissedSetupCards = ""
    @AppStorage("chorus.scrollbarStyle") private var scrollbarStyle = "hidden"
    @AppStorage("chorus.inspectorPosition") private var inspectorPosition = "trailing"
    @AppStorage("chorus.explorerPosition") private var explorerPosition = "leading"
    @AppStorage("chorus.sidebarWidth") private var sidebarWidth: Double = 260
    @AppStorage("chorus.explorerWidth") private var explorerWidth: Double = 260
    @AppStorage("chorus.inspectorWidth") private var inspectorWidth: Double = 330
    @AppStorage("chorus.codeViewerWidth") private var codeViewerWidth: Double = 520
    @State private var showLearnedRoutes = false
    @State private var isCheckingUpdate = false
    @State private var updateStatusText = "ตรวจหาเวอร์ชันล่าสุดจาก GitHub Releases"

    var body: some View {
        Group {
            SettingsGroup(title: "การทำงาน") {
                SettingsRow(title: "สิทธิ์เริ่มต้นของเซสชันใหม่", detail: PermissionLevel(rawValue: Int32(defaultPermission))?.hint) {
                    Picker("", selection: $defaultPermission) {
                        ForEach(PermissionLevel.allCases, id: \.self) { Text($0.label).tag(Int($0.rawValue)) }
                    }
                    .labelsHidden().fixedSize()
                }
                Divider().padding(.leading, 12)
                SettingsRow(title: "กด Enter เพื่อส่ง", detail: enterSends ? "Shift+Enter ขึ้นบรรทัดใหม่" : "⌘Enter ส่ง, Enter ขึ้นบรรทัดใหม่") {
                    Toggle("", isOn: $enterSends).labelsHidden().toggleStyle(.switch)
                }
                Divider().padding(.leading, 12)
                SettingsRow(title: "เปิดกล่องความคิดระหว่างเอเจนต์ทำงาน") {
                    Toggle("", isOn: $thinkingOpen).labelsHidden().toggleStyle(.switch)
                }
                Divider().padding(.leading, 12)
                SettingsRow(title: "ค้นหาเซสชันผ่าน Spotlight", detail: "ส่งชื่อและเป้าหมายของเซสชันเข้าดัชนี Spotlight ของระบบ (อยู่นอกข้อมูลของ Chorus)") {
                    Toggle("", isOn: $spotlightIndexing).labelsHidden().toggleStyle(.switch)
                        .onChange(of: spotlightIndexing) { _, on in
                            if !on { SpotlightIndexer.shared.clearAll() }
                        }
                }
            }
            SettingsGroup(title: "รูปแบบหน้าต่างและแถบเลื่อน",
                          footer: "ปรับแต่งการจัดวางและแถบเลื่อนให้เหมาะกับการทำงานของคุณ สามารถลากเส้นแบ่งขอบเพื่อยืด/ขยายแต่ละแผงได้ตามต้องการ") {
                SettingsRow(title: "รูปแบบแถบเลื่อน (Scrollbar)",
                            detail: ScrollbarStyle(rawValue: scrollbarStyle)?.hint) {
                    Picker("", selection: $scrollbarStyle) {
                        ForEach(ScrollbarStyle.allCases) { style in
                            Text(style.label).tag(style.rawValue)
                        }
                    }
                    .labelsHidden().fixedSize()
                }
                Divider().padding(.leading, 12)
                SettingsRow(title: "ตำแหน่งแผงข้อมูล (Inspector)", detail: "เลือกแสดงแผงบริบท/สิ่งที่แก้ไข/ความจำ ด้านขวาหรือซ้ายของหน้าจอ") {
                    Picker("", selection: $inspectorPosition) {
                        Text("ด้านขวา (Right)").tag("trailing")
                        Text("ด้านซ้าย (Left)").tag("leading")
                    }
                    .pickerStyle(.segmented)
                    .frame(width: 170)
                }
                Divider().padding(.leading, 12)
                SettingsRow(title: "ตำแหน่งแผงไฟล์ (Explorer)", detail: "เลือกแสดงโครงสร้างไฟล์และ Git Worktrees ด้านซ้ายหรือขวา") {
                    Picker("", selection: $explorerPosition) {
                        Text("ด้านซ้าย (Left)").tag("leading")
                        Text("ด้านขวา (Right)").tag("trailing")
                    }
                    .pickerStyle(.segmented)
                    .frame(width: 170)
                }
                Divider().padding(.leading, 12)
                SettingsRow(title: "ขนาดแผงที่ปรับแต่งไว้", detail: "ความกว้างของแถบโปรเจกต์, ไฟล์, แผงข้อมูล และโค้ด") {
                    Button("คืนค่าขนาดเริ่มต้น") {
                        sidebarWidth = 260
                        explorerWidth = 260
                        inspectorWidth = 330
                        codeViewerWidth = 520
                    }
                }
            }
            SettingsGroup(title: "เมื่อชนลิมิต",
                          footer: "Chorus ส่ง brief (เป้าหมาย สิ่งที่ทำแล้ว diff) ให้ตัวถัดไปทำต่อจากจุดเดิม ความจำและเซสชันเป็นชุดเดียวกัน") {
                SettingsRow(title: "สลับไปบัญชีอื่นของเจ้าเดียวกันก่อน", detail: "เช่น Claude งาน → Claude ส่วนตัว") {
                    Toggle("", isOn: $failoverAccounts).labelsHidden().toggleStyle(.switch)
                }
                Divider().padding(.leading, 12)
                SettingsRow(title: "แล้วค่อยสลับไปเอเจนต์เจ้าอื่น", detail: "เช่น Claude → Codex") {
                    Toggle("", isOn: $failoverProviders).labelsHidden().toggleStyle(.switch)
                }
            }
            SettingsGroup(title: "ระบบเลือกโมเดลอัตโนมัติ (Zero-Ollama Auto Router)",
                          footer: "Chorus รันโมเดลวิเคราะห์งานในเครื่องแบบ In-Process ผ่าน Apple Silicon Metal ออฟไลน์ 100% โดยไม่ต้องติดตั้งหรือเปิดโปรแกรม Ollama หากยังไม่ได้ดาวน์โหลดโมเดล ระบบจะใช้ Fast In-Process Classifier (3–8 ms) ทำงานให้อัตโนมัติ") {
                SettingsRow(title: "ใช้ Auto Router ก่อนส่งงานใหม่") {
                    Toggle("", isOn: $autoRoute).labelsHidden().toggleStyle(.switch)
                }
                Divider().padding(.leading, 12)

                SettingsRow(title: "ประหยัดพลังงานเมื่อใช้แบตเตอรี่",
                            detail: "สลับไปใช้ Fast In-Process Classifier (0ms) อัตโนมัติเมื่ออยู่ในโหมดประหยัดพลังงาน (Low Power Mode) หรืออุณหภูมิเครื่องสูง") {
                    Toggle("", isOn: $conserveBatteryOnInference).labelsHidden().toggleStyle(.switch)
                }
                Divider().padding(.leading, 12)

                // Smart Unified Memory Ceiling
                let totalRamBytes = ProcessInfo.processInfo.physicalMemory
                let totalRamGB = Int(round(Double(totalRamBytes) / (1024 * 1024 * 1024)))
                let isCompactRam = totalRamGB <= 16

                SettingsRow(
                    title: "หน่วยความจำ Unified Memory (\(totalRamGB) GB)",
                    detail: isCompactRam
                        ? "Mac รุ่น \(totalRamGB) GB: เหมาะสำหรับ OpenThai 0.8B (529 MB) และ Extractive Summarizer (0 MB) เพื่อความเร็วสูงสุดและหลีกเลี่ยง Memory Paging"
                        : "Mac รุ่น \(totalRamGB) GB: มีหน่วยความจำกว้างขวางสำหรับรันทั้ง Local Router และ AI Compactor พร้อมกันได้อย่างเต็มประสิทธิภาพ"
                ) {
                    Label(isCompactRam ? "\(totalRamGB) GB (แนะนำโมเดล 0.8B)" : "\(totalRamGB) GB (พร้อมทุกโมเดล)", systemImage: "memorychip")
                        .font(.caption.weight(.medium))
                        .foregroundStyle(isCompactRam ? .orange : .green)
                }
                Divider().padding(.leading, 12)

                // 1-Click Local Router Model Card
                let routerModelStatus = engine.localModels.first { $0.id.contains("openthai") }
                if let rm = routerModelStatus {
                    if engine.isDownloadingLocalModel {
                        SettingsRow(
                            title: "กำลังดาวน์โหลด OpenThai System One",
                            detail: engine.localModelDownloadStatusText
                        ) {
                            HStack(spacing: 8) {
                                ProgressView(value: engine.localModelDownloadProgress)
                                    .progressViewStyle(.linear)
                                    .frame(width: 140)
                                Button("ยกเลิก") {
                                    engine.cancelLocalModelDownload()
                                }
                                .buttonStyle(.bordered)
                                .disabled(!engine.canCancelLocalModelDownload)
                            }
                        }
                    } else if rm.isInstalled {
                        SettingsRow(
                            title: "OpenThai System One 0.8B",
                            detail: "พร้อมใช้งานในเครื่อง (ขนาด \(rm.size_bytes.map { String(format: "%.1f MB", Double($0) / (1024 * 1024)) } ?? "529 MB")) · Metal Acceleration"
                        ) {
                            HStack(spacing: 8) {
                                Label("ติดตั้งแล้ว", systemImage: "checkmark.circle.fill")
                                    .foregroundStyle(.green)
                                    .font(.caption.weight(.medium))
                                Button("ลบโมเดล") {
                                    engine.deleteLocalModel(modelId: rm.id)
                                }
                                .buttonStyle(.bordered)
                            }
                        }
                    } else {
                        SettingsRow(
                            title: "OpenThai System One 0.8B (แนะนำ)",
                            detail: "โมเดล Router สำหรับงานภาษาไทยและอังกฤษ · ดาวน์โหลดคลิกเดียว (~529 MB) ติดตั้งใน Application Support"
                        ) {
                            Button {
                                engine.downloadLocalModel(rm.spec)
                            } label: {
                                Label("ดาวน์โหลดคลิกเดียว (529 MB)", systemImage: "arrow.down.circle.fill")
                            }
                            .buttonStyle(.borderedProminent)
                        }
                    }
                } else {
                    SettingsRow(
                        title: "โมเดล Local Router ในเครื่อง",
                        detail: "กำลังตรวจสอบสถานะโมเดลในเครื่อง..."
                    ) {
                        Button("ตรวจใหม่") {
                            engine.loadLocalModels()
                        }
                    }
                }

                if let err = engine.localModelError {
                    Divider().padding(.leading, 12)
                    SettingsRow(title: "ข้อผิดพลาด", detail: err) {
                        EmptyView()
                    }
                }

                Divider().padding(.leading, 12)

                // Optional AI Compactor Model Card
                let compactorStatus = engine.localModels.first { $0.id.contains("qwen2.5-coder") }
                if let cm = compactorStatus {
                    if cm.isInstalled {
                        SettingsRow(
                            title: "AI Conversation Compactor",
                            detail: "Qwen 2.5 Coder 1.5B ติดตั้งแล้ว · ย่อบริบทประวัติแชทด้วย AI"
                        ) {
                            Button("ลบโมเดล") {
                                engine.deleteLocalModel(modelId: cm.id)
                            }
                            .buttonStyle(.bordered)
                        }
                    } else {
                        SettingsRow(
                            title: "AI Conversation Compactor (อุปกรณ์เสริม)",
                            detail: isCompactRam
                                ? "ปัจจุบันใช้ Extractive สรุปประวัติแชทรวดเร็ว (0 MB) · สำหรับ Mac \(totalRamGB) GB แนะนำใช้ Extractive หรือโหลด Qwen 2.5 Coder (~893 MB) หากต้องการสรุปเชิงลึก"
                                : "ปัจจุบันใช้ Extractive สรุปประวัติแชทรวดเร็วเป็นค่าเริ่มต้น (0 MB) · สามารถโหลด Qwen 2.5 Coder (~893 MB) เพื่อเพิ่มประสิทธิภาพการสรุปข้อความ"
                        ) {
                            Button("ดาวน์โหลด (893 MB)") {
                                engine.downloadLocalModel(cm.spec)
                            }
                            .buttonStyle(.bordered)
                            .disabled(engine.isDownloadingLocalModel)
                        }
                    }
                }
                Divider().padding(.leading, 12)
                SettingsRow(
                    title: "สิ่งที่ Auto Route เรียนรู้แล้ว",
                    detail: engine.learnedRoutes.isEmpty
                        ? "ยังไม่มีประวัติการเรียนรู้ (ระบบจะเรียนรู้อัตโนมัติจากการเลือกหรือยืนยันโมเดลของคุณ)"
                        : "จดจำความต้องการไว้แล้ว \(engine.learnedRoutes.count) รายการ เพื่อเลือกโมเดลให้ตรงใจคุณยิ่งขึ้น"
                ) {
                    Button {
                        showLearnedRoutes = true
                    } label: {
                        HStack(spacing: 5) {
                            Image(systemName: "brain.head.profile")
                            Text("ดูสิ่งที่เรียนรู้ (\(engine.learnedRoutes.count))")
                        }
                    }
                }
                Divider().padding(.leading, 12)
                SettingsRow(title: "คำแนะนำการตั้งค่าที่ปิดไว้", detail: "แสดงการ์ดสำหรับ agent, Auto และเครื่องมือเสริมอีกครั้ง") {
                    Button("แสดงอีกครั้ง") { dismissedSetupCards = "" }
                        .disabled(dismissedSetupCards.isEmpty)
                }
            }
            SettingsGroup(title: "ข้อมูล") {
                SettingsRow(title: "โฟลเดอร์ข้อมูลของ Chorus", detail: "~/.chorus — บัญชีเพิ่มเติม, สถานะลิมิต") {
                    Button("เปิด") {
                        let u = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".chorus")
                        try? FileManager.default.createDirectory(at: u, withIntermediateDirectories: true)
                        NSWorkspace.shared.open(u)
                    }
                }
            }
            SettingsGroup(title: "เวอร์ชันและการอัปเดต") {
                SettingsRow(title: "เวอร์ชันปัจจุบัน", detail: "Chorus v0.1.0 (Desktop Agent Harness)") {
                    Text("v0.1.0")
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                }
                Divider().padding(.leading, 12)
                SettingsRow(title: "ตรวจสอบการอัปเดต", detail: updateStatusText) {
                    Button(isCheckingUpdate ? "กำลังตรวจ…" : "ตรวจหาเวอร์ชันใหม่") {
                        checkForUpdates()
                    }
                    .disabled(isCheckingUpdate)
                }
            }
        }
        .onAppear {
            engine.loadLocalModels()
            engine.loadLearnedRoutes(workspace: workspace)
        }
        .sheet(isPresented: $showLearnedRoutes) {
            LearnedRoutesSheet(workspace: workspace)
                .environmentObject(engine)
        }
    }

    private func checkForUpdates() {
        isCheckingUpdate = false
        updateStatusText = "ยังไม่ได้ตั้งค่าแหล่งตรวจสอบการอัปเดต"
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
                    .help("ถามรายการโมเดลล่าสุดจากเอเจนต์")
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
            "chorus": "Chorus — ส่งให้ทุกเอเจนต์",
            "project": "โปรเจกต์นี้ (.claude/skills, .agents/skills)",
            "agents": "Global ใช้ร่วมกัน (~/.agents/skills)",
            "user": "Claude (~/.claude/skills)",
            "codex": "Codex (~/.codex/skills)",
            "gemini": "Gemini / Antigravity (~/.gemini/skills)",
        ]
        TextField("ค้นหา skill…", text: $query).textFieldStyle(.roundedBorder)
        Text("\(engine.skills.count) skill ที่เอเจนต์ในโฟลเดอร์นี้มองเห็น")
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
            Text("ไม่พบ skill — วางไว้ที่ .chorus/skills/<ชื่อ>/SKILL.md เพื่อให้ทุกเอเจนต์ใช้ร่วมกัน")
                .font(.caption).foregroundStyle(.secondary)
        }
    }
}

// MARK: - Orchestration Settings
struct OrchestrationSettings: View {
    let workspace: String
    @EnvironmentObject var engine: Engine

    @AppStorage("chorus.orchestrationEnabled") private var orchestrationEnabled = true
    @AppStorage("chorus.orchestrationLimit") private var orchestrationLimit = 3
    @AppStorage("chorus.orchestrationBasePort") private var orchestrationBasePort = 3000
    @AppStorage("chorus.autoMergeWorktrees") private var autoMergeWorktrees = true
    @AppStorage("chorus.orchestratorLeadModel") private var orchestratorLeadModel = ""

    @State private var worktrees: [Engine.WorktreeInfo] = []

    private var availableModels: [String] {
        var list = [
            "claude-3-7-sonnet-20250219",
            "gpt-5.1-codex",
            "gemini-2.5-flash",
            "gemini-2.5-pro",
            "o3-mini"
        ]
        for a in engine.agents {
            for m in a.baseModels {
                if !list.contains(m.id) { list.append(m.id) }
            }
        }
        return list
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            SettingsGroup(title: "โมเดลหลักสำหรับ Orchestrator (Lead Conductor)") {
                SettingsRow(title: "โมเดลผู้คุมงาน (Lead Orchestrator)",
                            detail: "โมเดลสำหรับวางแผน แตกงาน และสรุปผลรวม (แนะนำโมเดลเก่งที่สุด เช่น Claude 3.7 Sonnet, o3-mini)") {
                    Picker("", selection: $orchestratorLeadModel) {
                        Text("ตามที่เลือกใน Composer (ค่าเริ่มต้น)").tag("")
                        ForEach(availableModels, id: \.self) { m in
                            Text(m).tag(m)
                        }
                    }
                    .labelsHidden()
                    .frame(maxWidth: 240)
                }
            }

            SettingsGroup(title: "การทำงานขนาน (Parallel Multi-Agent)") {
                SettingsRow(title: "เปิดใช้งาน Multi-Agent Worktrees",
                            detail: "อนุญาตให้เอเจนต์แตกงานขนานไปยัง Git Worktree แยกอัตโนมัติ") {
                    Toggle("", isOn: $orchestrationEnabled)
                        .labelsHidden()
                        .toggleStyle(.switch)
                }

                Divider().padding(.leading, 12)

                SettingsRow(title: "จำนวนเอเจนต์ขนานสูงสุด",
                            detail: "จำกัดจำนวนเอเจนต์ที่รันพร้อมกันเพื่อป้องกัน Token หมดไว") {
                    Stepper("\(orchestrationLimit) ตัว", value: $orchestrationLimit, in: 1...8)
                }

                Divider().padding(.leading, 12)

                SettingsRow(title: "ผสานโค้ดกลับอัตโนมัติ (Auto-Merge)",
                            detail: "เมื่อเอเจนต์ใน Worktree ทำงานสำเร็จ ให้รวมเข้ากับกิ่งหลักทันที") {
                    Toggle("", isOn: $autoMergeWorktrees)
                        .labelsHidden()
                        .toggleStyle(.switch)
                }
            }

            SettingsGroup(title: "สภาพแวดล้อม Worktrees (.chorus/worktrees/)") {
                SettingsRow(title: "พอร์ตเริ่มต้นสำหรับ Dev Server",
                            detail: "กำหนด dynamic port เริ่มต้น (เช่น 3000, 3001, ...) ป้องกันพอร์ตชนกัน") {
                    Stepper(":\(orchestrationBasePort)", value: $orchestrationBasePort, in: 1000...9000, step: 100)
                }

                Divider().padding(.leading, 12)

                SettingsRow(title: "จัดการโฟลเดอร์ Worktrees",
                            detail: workspace.isEmpty ? "เลือกโปรเจกต์ก่อนเพื่อดู Worktrees" : "ตำแหน่ง: .chorus/worktrees/") {
                    HStack(spacing: 8) {
                        Button("เปิดใน Finder") {
                            openWorktreesFolder()
                        }
                        .disabled(workspace.isEmpty)

                        Button("ล้าง Worktree ทั้งหมด") {
                            cleanWorktrees()
                        }
                        .disabled(workspace.isEmpty)
                    }
                }
            }

            Text("การทำงานขนานจะเกิดขึ้นเมื่อเอเจนต์เรียกคำสั่ง `chorus_spawn_agent` หรือเมื่อกดปุ่มสลับโหมดขนานข้างกล่องข้อความ โดยโค้ดจะถูกแยกใน Git Worktree ต่างหากไม่รบกวนโค้ดหลัก")
                .font(.caption)
                .foregroundStyle(.secondary)

            Divider().padding(.vertical, 4)

            SettingsGroup(title: "ภาพรวมการทำงานคู่ขนานและบทบาทเอเจนต์") {
                OrchestratorView(
                    engine: engine,
                    workspace: workspace,
                    onSelectWorktree: { _ in },
                    onDispatchParallelPrompt: { _ in }
                )
                .frame(minHeight: 400)
            }
        }
        .onAppear {
            refreshWorktrees()
        }
    }

    private func refreshWorktrees() {
        guard !workspace.isEmpty else { return }
        engine.fetchWorktrees(cwd: workspace) { list in
            self.worktrees = list
        }
    }

    private func openWorktreesFolder() {
        guard !workspace.isEmpty else { return }
        let path = (workspace as NSString).appendingPathComponent(".chorus/worktrees")
        let url = URL(fileURLWithPath: path)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        NSWorkspace.shared.open(url)
    }

    private func cleanWorktrees() {
        guard !workspace.isEmpty else { return }
        let path = (workspace as NSString).appendingPathComponent(".chorus/worktrees")
        if let contents = try? FileManager.default.contentsOfDirectory(atPath: path) {
            for item in contents {
                try? FileManager.default.removeItem(atPath: (path as NSString).appendingPathComponent(item))
            }
        }
        refreshWorktrees()
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
                    Text("ความจำการเลือกโมเดลและเอเจนต์ที่ระบบบันทึกไว้ ช่วยให้ Chorus แนะนำโมเดลสำหรับงานลักษณะเดียวกันได้ทันที (<5ms)")
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
                    Text("เอเจนต์ทั้งหมด").tag("all")
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
                         : "ลองเปลี่ยนคำค้นหาหรือเลือกดูเอเจนต์ทั้งหมด")
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

    private func seedExampleRoutes() {
        let examples: [(prompt: String, model: String, agent: String, reason: String)] = [
            ("ช่วยรีแฟกเตอร์ฟังก์ชันใน Rust ให้เป็น async หน่อย", "claude-3-7-sonnet", "claude", "งานเขียนโค้ด Rust ลึกซึ้ง เหมาะกับ Claude 3.7 Sonnet"),
            ("ออกแบบ UI Settings ด้วย SwiftUI บน macOS", "o3-mini", "codex", "งานออกแบบ Swift/macOS เหมาะกับ Codex"),
            ("อธิบายความหมายของ FFI สั้นๆ ให้หน่อย", "gemini-2.0-flash", "antigravity", "คำถามสั้นต้องการคำตอบเร็ว เหมาะกับ Gemini Flash")
        ]
        for ex in examples {
            engine.recordLearnedRoute(workspace: workspace, prompt: ex.prompt, model: ex.model, agent: ex.agent, reason: ex.reason)
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

