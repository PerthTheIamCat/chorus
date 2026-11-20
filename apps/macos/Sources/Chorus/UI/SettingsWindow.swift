import SwiftUI

// Design board "Settings — 4 กลุ่ม": a searchable list of sections in four
// groups (วง · การทำงาน · ความปลอดภัย · ระบบ) on the left, a page with a title
// and one-line lead on the right. "บัญชีและโควตา" opens first and leads with
// the relay order and every account's quota; the rows of each page are the
// grouped Forms in Settings.swift (backed by @AppStorage or Rust via Engine).

enum SettingsSection: String, CaseIterable, Identifiable {
    case accounts, agents, routing, orchestration
    case general, display, asking, skills, integrations, learn, languages
    case safety, review
    case cost, storage, companion, about, changelog
    var id: String { rawValue }

    var title: String {
        switch self {
        case .accounts: "บัญชีและโควตา"
        case .agents: "Agent และโมเดล"
        case .routing: "Auto route"
        case .orchestration: "Orchestra และ Roles"
        case .general: "ทั่วไป"
        case .display: "การแสดงผล"
        case .asking: "การถามและอัตโนมัติ"
        case .skills: "Skills"
        case .integrations: "เครื่องมือเสริม (MCP)"
        case .learn: "ความจำที่เรียนรู้"
        case .languages: "ภาษาโค้ด"
        case .safety: "สิทธิ์ Sandbox และ Secrets"
        case .review: "รีวิวอัตโนมัติและ checkpoint"
        case .cost: "ค่าใช้จ่าย"
        case .storage: "พื้นที่เก็บข้อมูล"
        case .companion: "มือถือ"
        case .about: "เกี่ยวกับ"
        case .changelog: "มีอะไรใหม่"
        }
    }

    var lead: String {
        switch self {
        case .accounts: "หลายบัญชีต่อเจ้าได้ ชนลิมิตแล้วข้ามบัญชีก่อน แล้วค่อยส่งไม้ข้ามเจ้า"
        case .agents: "ตัวที่ Chorus เจอบนเครื่องนี้ · รายชื่อโมเดลถามจาก bridge จริง"
        case .routing: "ให้ router เลือกโมเดลต่อเทิร์นตามงานและโควตาที่เหลือ"
        case .orchestration: "หัวหน้าวงวางแผนและแจกงานย่อย"
        case .general: "หน้าตา ภาษา และการแจ้งเตือน"
        case .display: "ตัวเลขที่โชว์ใต้ช่องพิมพ์และการแสดงความคิดของ agent"
        case .asking: "agent ถามคุณเมื่อไหร่ และอะไรที่รับเองได้"
        case .skills: "เขียนที่เดียว ฉายให้ทุก agent ตามที่มันอ่านได้"
        case .integrations: "สิ่งที่อยู่ในนี้คือทั้งหมดที่ agent เอื้อมถึง"
        case .learn: "บทเรียนจากงานที่ผ่านมา ผ่านคิวอนุมัติเสมอ"
        case .languages: "syntax highlight ที่ติดตั้งเพิ่มได้"
        case .safety: "กันตอนเขียน ไม่ใช่ย้อนทีหลัง · ค่าจริงอยู่ใน Keychain"
        case .review: "ให้ agent อีกเจ้าตรวจงานก่อนคุณดู"
        case .cost: "มูลค่าเทียบราคา API จาก usage ที่ bridge รายงาน"
        case .storage: "ledger · blob · shadow repo"
        case .companion: "ดูและอนุญาตงานจาก iPhone"
        case .about: ""
        case .changelog: ""
        }
    }

    var icon: String {
        switch self {
        case .accounts: "person.2"
        case .agents: "cpu"
        case .routing: "arrow.triangle.branch"
        case .orchestration: "point.3.connected.trianglepath.dotted"
        case .general: "gearshape"
        case .display: "textformat.size"
        case .asking: "bubble.left.and.exclamationmark.bubble.right"
        case .skills: "book.closed"
        case .integrations: "puzzlepiece.extension"
        case .learn: "graduationcap"
        case .languages: "chevron.left.forwardslash.chevron.right"
        case .safety: "lock.shield"
        case .review: "eye"
        case .cost: "bahtsign.circle"
        case .storage: "internaldrive"
        case .companion: "iphone"
        case .about: "info.circle"
        case .changelog: "sparkles"
        }
    }

    static let groups: [(title: String, items: [SettingsSection])] = [
        ("วง", [.accounts, .agents, .routing, .orchestration]),
        ("การทำงาน", [.general, .display, .asking, .skills, .integrations, .learn, .languages]),
        ("ความปลอดภัย", [.safety, .review]),
        ("ระบบ", [.cost, .storage, .companion, .about, .changelog]),
    ]
}

struct SettingsRoot: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @AppStorage("chorus.autoCatalogUpdates") private var autoCatalogUpdates = true
    @State private var section: SettingsSection = .accounts
    @State private var query = ""

    var body: some View {
        HStack(spacing: 0) {
            nav.frame(width: 236)
            Hairline(vertical: true)
            VStack(alignment: .leading, spacing: 0) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(section.title).font(Typo.title)
                    if !section.lead.isEmpty {
                        Text(section.lead).font(.system(size: 13.5)).foregroundStyle(Palette.text2)
                    }
                }
                .padding(.horizontal, 40).padding(.top, 32).padding(.bottom, 4)
                .frame(maxWidth: .infinity, alignment: .leading)
                // The grouped Form owns scrolling: nesting it in a ScrollView
                // crashes SwiftUI's section layout (NestedSections, macOS 14+).
                if section == .accounts {
                    QuotaOverview().padding(.horizontal, 40).padding(.top, 16)
                }
                Form { pane(section) }
                    .formStyle(.grouped)
                    .scrollContentBackground(.hidden)
            }
            .background(Palette.window)
        }
        .frame(minWidth: 960, minHeight: 780)
        .tint(Palette.accent)
        .onAppear {
            engine.loadAgents()
            engine.loadUsage()
            engine.loadSkills(cwd: shell.workspace)
            engine.loadCatalog(project: shell.workspace, automaticUpdates: autoCatalogUpdates)
        }
    }

    private var nav: some View {
        VStack(alignment: .leading, spacing: 2) {
            TextField("ค้นตั้งค่า", text: $query).textFieldStyle(.roundedBorder).padding(.bottom, 6)
            ScrollView {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(SettingsSection.groups, id: \.title) { g in
                        let items = g.items.filter { query.isEmpty || $0.title.localizedCaseInsensitiveContains(query) }
                        if !items.isEmpty {
                            GroupHeader(g.title).padding(.leading, 8).padding(.top, 12).padding(.bottom, 2)
                            ForEach(items) { s in
                                Button { section = s } label: {
                                    Label(s.title, systemImage: s.icon).labelStyle(SettingsNavLabel())
                                }
                                .buttonStyle(RowStyle(selected: section == s))
                            }
                        }
                    }
                }
            }
        }
        .padding(12)
        .background(Palette.sidebar)
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
                    onLoadPlan: { id, action, done in engine.loadCatalogPlan(project: workspace, id: id, action: action, completion: done) },
                    onAutomaticUpdatesChanged: { value in
                        autoCatalogUpdates = value
                        if value { engine.loadCatalog(project: workspace, automaticUpdates: true) }
                    },
                    onInstall: { engine.catalogAction(project: workspace, id: $0, action: workspace.isEmpty ? "install" : "install_enable") },
                    onUpdate: { engine.catalogAction(project: workspace, id: $0, action: "update") },
                    onRemove: { engine.catalogAction(project: workspace, id: $0, action: "remove") },
                    onProjectEnableChanged: { id, enabled in
                        engine.catalogAction(project: workspace, id: id, action: enabled ? "enable" : "disable")
                    })
        if let error = engine.catalogActionError ?? engine.catalogError {
            Text(error).font(Typo.caption).foregroundStyle(Palette.bad)
        }
    }
}

private struct SettingsNavLabel: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 8) {
            configuration.icon.font(.system(size: 12)).frame(width: 16)
            configuration.title
        }
    }
}

/// The top of "บัญชีและโควตา": relay order, then each provider's accounts with
/// their 5-hour and weekly windows.
private struct QuotaOverview: View {
    @EnvironmentObject private var engine: Engine

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            VStack(alignment: .leading, spacing: 10) {
                Text("ลำดับส่งไม้").font(Typo.headline)
                HStack(spacing: 8) {
                    ForEach(Array(engine.agents.filter(\.ready).enumerated()), id: \.element.id) { i, a in
                        if i > 0 { Image(systemName: "arrow.right").font(.system(size: 10)).foregroundStyle(Palette.text3) }
                        HStack(spacing: 8) {
                            ProviderIcon(provider: a.id, size: 13)
                            Text(a.label)
                        }
                        .font(Typo.small).padding(.horizontal, 12).frame(height: 34)
                        .background(Palette.surface, in: RoundedRectangle(cornerRadius: 9))
                        .overlay(RoundedRectangle(cornerRadius: 9).strokeBorder(Palette.line2))
                    }
                }
            }
            ForEach(engine.agents) { a in
                let accs = engine.accounts.filter { $0.agent == a.id }
                if !accs.isEmpty {
                    VStack(spacing: 0) {
                        HStack(spacing: 10) {
                            ProviderAvatar(provider: a.id, size: 26)
                            Text(a.label).font(.system(size: 14, weight: .semibold))
                            Text("\(accs.count) บัญชี").font(Typo.caption).foregroundStyle(Palette.text3)
                            Spacer()
                        }
                        .padding(14)
                        ForEach(accs) { acc in
                            Hairline()
                            HStack(spacing: 16) {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(acc.title).font(Typo.small)
                                    Text(acc.limited ? "ติดลิมิต" : (acc.ready ? "พร้อม" : "ยังไม่ login"))
                                        .font(Typo.tiny).foregroundStyle(acc.limited ? Palette.warn : Palette.text3)
                                }
                                .frame(width: 150, alignment: .leading)
                                VStack(spacing: 6) {
                                    window("5 ชม.", acc.rate_limits?.primary, limited: acc.limited)
                                    window("สัปดาห์", acc.rate_limits?.secondary, limited: false)
                                }
                            }
                            .padding(.horizontal, 14).padding(.vertical, 12)
                        }
                    }
                    .background(Palette.surface, in: RoundedRectangle(cornerRadius: 12))
                    .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Palette.line))
                }
            }
            Text("Chorus ตรวจแค่ว่ามีไฟล์ login อยู่ไหม ไม่อ่านหรือเก็บ token · ตัวเลขโควตามาจาก bridge ของแต่ละเจ้า")
                .font(Typo.caption).foregroundStyle(Palette.text3)
        }
    }

    private func window(_ name: String, _ w: Engine.UsageWindow?, limited: Bool) -> some View {
        HStack(spacing: 8) {
            Text(name).font(Typo.tiny).foregroundStyle(Palette.text3).frame(width: 50, alignment: .leading)
            Meter(value: (w?.used_percent ?? 0) / 100, tint: limited ? Palette.warn : Palette.accent)
            Text(w.map { limited ? ($0.resets_label ?? "\(Int($0.used_percent))%") : "\(Int($0.used_percent))%" } ?? "—")
                .font(Typo.monoSmall).foregroundStyle(Palette.text2).frame(width: 90, alignment: .trailing).lineLimit(1)
        }
    }
}
