import SwiftUI

// Settings, v0.4: the standard macOS Settings window. A source list of
// categories on the left, a grouped `Form` on the right. Each page is a set of
// rows backed by `@AppStorage` or by Rust through `Engine`; nothing here
// decides behaviour (CLAUDE.md 2.3).

enum SettingsSection: String, CaseIterable, Identifiable {
    case general, display, accounts, routing, orchestration, asking
    case agents, safety, integrations, skills
    case review, cost, storage, languages, learn
    case companion, about, changelog
    var id: String { rawValue }

    var title: String {
        switch self {
        case .general: "ทั่วไป"
        case .display: "การแสดงผล"
        case .accounts: "บัญชี"
        case .routing: "การส่งงาน"
        case .orchestration: "Orchestra"
        case .asking: "การถามและอัตโนมัติ"
        case .agents: "โมเดล"
        case .safety: "สิทธิ์และความปลอดภัย"
        case .integrations: "เครื่องมือเสริม (MCP)"
        case .skills: "Skills"
        case .review: "Review และ checkpoint"
        case .cost: "การใช้งานและค่าใช้จ่าย"
        case .storage: "พื้นที่และประสิทธิภาพ"
        case .languages: "ภาษา (Syntax)"
        case .learn: "เรียนรู้"
        case .companion: "Mobile Companion"
        case .about: "เกี่ยวกับ"
        case .changelog: "มีอะไรใหม่"
        }
    }

    var icon: String {
        switch self {
        case .general: "gearshape"
        case .display: "paintpalette"
        case .accounts: "person.2"
        case .routing: "arrow.triangle.branch"
        case .orchestration: "point.3.connected.trianglepath.dotted"
        case .asking: "bubble.left.and.exclamationmark.bubble.right"
        case .agents: "cpu"
        case .safety: "lock.shield"
        case .integrations: "puzzlepiece.extension"
        case .skills: "sparkles"
        case .review: "plusminus"
        case .cost: "bahtsign.circle"
        case .storage: "internaldrive"
        case .languages: "chevron.left.forwardslash.chevron.right"
        case .learn: "graduationcap"
        case .companion: "iphone"
        case .about: "info.circle"
        case .changelog: "clock.arrow.circlepath"
        }
    }

    static let groups: [(title: String, items: [SettingsSection])] = [
        ("ทั่วไป", [.general, .display, .languages]),
        ("เอเจนต์", [.accounts, .agents, .routing, .orchestration, .asking]),
        ("ความจำและเครื่องมือ", [.learn, .skills, .integrations]),
        ("ความปลอดภัย", [.safety, .review]),
        ("ระบบ", [.cost, .storage, .companion, .about, .changelog]),
    ]
}

struct SettingsRoot: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @AppStorage("chorus.autoCatalogUpdates") private var autoCatalogUpdates = true
    @State private var section: SettingsSection? = .accounts

    var body: some View {
        NavigationSplitView {
            List(selection: $section) {
                ForEach(SettingsSection.groups, id: \.title) { group in
                    Section(group.title) {
                        ForEach(group.items) { s in
                            Label(s.title, systemImage: s.icon).tag(Optional(s))
                        }
                    }
                }
            }
            .navigationSplitViewColumnWidth(min: 200, ideal: 220, max: 280)
        } detail: {
            // Not a grouped `Form`: its NestedSections section indexer traps
            // (IndexPath out of range, EXC_BREAKPOINT) on these panes, which
            // build their sections from ForEach/conditionals. Plain scroll +
            // `SettingsGroup` cards have no such indexer.
            ScrollView {
                VStack(alignment: .leading, spacing: 20) { pane(section ?? .accounts) }
                    .frame(maxWidth: 720, alignment: .leading)
                    .padding(20)
                    .frame(maxWidth: .infinity)
            }
                .navigationTitle((section ?? .accounts).title)
        }
        .frame(minWidth: 760, minHeight: 520)
        .onAppear {
            engine.loadAgents()
            engine.loadUsage()
            engine.loadSkills(cwd: shell.workspace)
            engine.loadCatalog(project: shell.workspace, automaticUpdates: autoCatalogUpdates)
        }
    }

    @ViewBuilder
    private func pane(_ s: SettingsSection) -> some View {
        let workspace = shell.workspace
        switch s {
        case .general: GeneralSettings()
        case .display: DisplaySettingsView()
        case .accounts: AccountsSettings()
        case .routing: RoutingSettings(workspace: workspace)
        case .orchestration: OrchestrationSettings(workspace: workspace)
        case .asking: AskingSettings(workspace: workspace)
        case .agents: AgentsSettings()
        case .safety: SafetySettings(workspace: workspace)
        case .integrations: integrations(workspace)
        case .skills: SkillsSettings(workspace: workspace)
        case .review: ReviewSettings()
        case .cost: CostSettings()
        case .storage: StorageSettings()
        case .languages: LanguagesSettings()
        case .learn: LearnSettings(workspace: workspace)
        case .companion: RemotePairingView()
        case .about: AboutSettings(showChangelog: { section = .changelog })
        case .changelog: ChangelogSettings()
        }
    }

    @ViewBuilder
    private func integrations(_ workspace: String) -> some View {
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
