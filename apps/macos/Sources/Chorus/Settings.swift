import AppKit
import SwiftUI

/// Settings, laid out like System Settings: a list of sections on the left,
/// the section on the right.
struct SettingsSheet: View {
    @EnvironmentObject var engine: Engine
    @Environment(\.dismiss) private var dismiss
    let workspace: String
    @State var section: Section = .accounts

    enum Section: String, CaseIterable, Identifiable {
        case general = "ทั่วไป"
        case accounts = "บัญชีและการใช้งาน"
        case agents = "เอเจนต์และโมเดล"
        case skills = "Skills"
        var id: String { rawValue }
        var icon: String {
            switch self {
            case .general: "gearshape"
            case .accounts: "person.2.badge.gearshape"
            case .agents: "cpu"
            case .skills: "sparkles"
            }
        }
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
                    case .general: GeneralSettings()
                    case .accounts: AccountsSettings()
                    case .agents: AgentsSettings()
                    case .skills: SkillsSettings(workspace: workspace)
                    }
                }
                .padding(22)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .frame(width: 760, height: 560)
        .onAppear {
            engine.loadAgents()
            engine.loadUsage()
            engine.loadSkills(cwd: workspace)
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
    @AppStorage("chorus.defaultPermission") private var defaultPermission = Int(PermissionLevel.ask.rawValue)
    @AppStorage("chorus.enterSends") private var enterSends = true
    @AppStorage("chorus.thinkingOpen") private var thinkingOpen = true
    @AppStorage("chorus.failoverAccounts") private var failoverAccounts = true
    @AppStorage("chorus.failoverProviders") private var failoverProviders = true

    var body: some View {
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
        SettingsGroup(title: "ข้อมูล") {
            SettingsRow(title: "โฟลเดอร์ข้อมูลของ Chorus", detail: "~/.chorus — บัญชีเพิ่มเติม, สถานะลิมิต") {
                Button("เปิด") {
                    let u = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".chorus")
                    try? FileManager.default.createDirectory(at: u, withIntermediateDirectories: true)
                    NSWorkspace.shared.open(u)
                }
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
                if let p = rl.primary { window("5 ชั่วโมง", p) }
                if let s = rl.secondary { window("สัปดาห์", s) }
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
                Text(resetText(w.resets_at)).font(.caption2).foregroundStyle(.secondary)
            }
            UsageBar(fraction: w.used_percent / 100)
        }
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
                        Button("แก้ให้ใน Terminal") { engine.runLoginCommand(a.hint) }
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
        let titles = ["chorus": "Chorus — ส่งให้ทุกเอเจนต์", "project": "โปรเจกต์นี้ (.claude/skills)", "user": "ของคุณ (~/.claude/skills)"]
        TextField("ค้นหา skill…", text: $query).textFieldStyle(.roundedBorder)
        Text("\(engine.skills.count) skill ที่เอเจนต์ในโฟลเดอร์นี้มองเห็น")
            .font(.caption).foregroundStyle(.secondary)
        ForEach(["chorus", "project", "user"], id: \.self) { src in
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
