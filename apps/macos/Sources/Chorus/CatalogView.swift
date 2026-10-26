import SwiftUI

/// Presentation model for one trusted catalog entry. The parent owns persistence
/// and FFI; this view only renders state and reports user intent through closures.
struct CatalogIntegration: Identifiable, Hashable {
    let id: String
    let title: String
    let maintainer: String
    let license: String
    let sourceURL: URL
    let reviewedVersion: String
    let modes: [String]
    let requiredRuntimes: [String]
    let actions: [String]
    let projectAccess: [String]
    let networkBehavior: String
    let requiredSecretNames: [String]

    var installedVersion: String?
    var isHealthy: Bool?
    var healthMessage: String?
    var integrity: String?
    var isEnabledForProject: Bool
    var isWorking: Bool = false
    var canRemove: Bool = true
    var missingInstaller: String?
    var brewFormula: String?
    var installedMethod: String?

    var isInstalled: Bool { installedVersion != nil }
    var updateAvailable: Bool {
        guard let installedVersion else { return false }
        if brewFormula != nil {
            return installedVersion.compare(reviewedVersion, options: .numeric) == .orderedAscending
        }
        return installedMethod == "bundled" && installedVersion != reviewedVersion
    }

    var installerURL: URL? {
        switch missingInstaller {
        case "brew": URL(string: "https://brew.sh/")
        case "npm": URL(string: "https://nodejs.org/en/download")
        case "uv": URL(string: "https://docs.astral.sh/uv/getting-started/installation/")
        case "cargo": URL(string: "https://rustup.rs/")
        default: nil
        }
    }
}

struct PlanItem: Decodable, Identifiable {
    let kind: String
    let phase: String
    let title: String
    let detail: String?
    var id: String { "\(phase)-\(title)-\(detail ?? "")" }
}

struct InstallPlan: Decodable {
    let id: String
    let action: String
    let items: [PlanItem]
}

struct CatalogProgress: Decodable, Equatable {
    let id: String
    let current: String
    let done: [String]
    var detail: String = ""

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        current = try c.decode(String.self, forKey: .current)
        done = try c.decode([String].self, forKey: .done)
        detail = (try? c.decode(String.self, forKey: .detail)) ?? ""
    }
    private enum CodingKeys: String, CodingKey { case id, current, done, detail }

    /// One short word for the phase the card and sheet show while it runs.
    var phaseLabel: String {
        switch current {
        case "fetch": "กำลังดาวน์โหลดและติดตั้ง"
        case "verify": "กำลังตรวจสอบ"
        case "enable": "กำลังเปิดใช้"
        case "disable": "กำลังเลิกใช้"
        case "remove": "กำลังลบ"
        default: "กำลังเริ่ม"
        }
    }
}

struct CatalogActionResult: Equatable {
    let id: String
    let action: String
    let error: String?
    var succeeded: Bool { error == nil }
}

enum CatalogActionKind: String { case install = "install_enable", update, remove }

private struct CatalogActionRequest: Identifiable {
    let entryID: String
    let kind: CatalogActionKind
    var id: String { "\(entryID)-\(kind.rawValue)" }
}

/// Curated integration browser. State changes are emitted to the owner so that
/// persistence and managed tool installation remain outside the presentation layer.
struct CatalogView: View {
    let projectName: String
    let entries: [CatalogIntegration]
    let automaticUpdates: Bool
    let backgroundStatus: String?
    let actionResult: CatalogActionResult?
    let progress: CatalogProgress?
    let requestedInstall: String?
    let onRequestHandled: () -> Void
    let onLoadPlan: (String, String, @escaping (InstallPlan?, String?) -> Void) -> Void
    let onAutomaticUpdatesChanged: (Bool) -> Void
    let onInstall: (String) -> Void
    let onUpdate: (String) -> Void
    let onRemove: (String) -> Void
    let onProjectEnableChanged: (String, Bool) -> Void

    @State private var query = ""
    @State private var requestedAction: CatalogActionRequest?
    @State private var dismissedResultID: String?

    private var filteredEntries: [CatalogIntegration] {
        let search = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !search.isEmpty else { return entries }
        return entries.filter { entry in
            [entry.title, entry.maintainer, entry.actions.joined(separator: " ")]
                .contains { $0.localizedCaseInsensitiveContains(search) }
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            VStack(alignment: .leading, spacing: 4) {
                Text("เครื่องมือเสริม").font(.title2.bold())
                Text("ติดตั้งครั้งเดียว เปิดใช้แยกตามโปรเจกต์ · ลบได้ในคลิกเดียว")
                    .font(.callout).foregroundStyle(.secondary)
            }

            if let backgroundStatus {
                HStack(spacing: 9) {
                    ProgressView().controlSize(.small)
                    Text(backgroundStatus).font(.callout)
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.accentColor.opacity(0.08), in: RoundedRectangle(cornerRadius: 10))
                .accessibilityElement(children: .combine)
            }

            if requestedAction == nil, let result = actionResult, result.id != dismissedResultID {
                ResultBanner(result: result, title: entries.first { $0.id == result.id }?.title ?? result.id,
                             onDismiss: { dismissedResultID = result.id })
            }

            SettingsGroup(title: "การอัปเดต") {
                SettingsRow(title: "อัปเดตผ่าน Homebrew อัตโนมัติ",
                            detail: "เฉพาะเวอร์ชันที่ Chorus ตรวจแล้ว") {
                    Toggle("", isOn: Binding(
                        get: { automaticUpdates },
                        set: { onAutomaticUpdatesChanged($0) }
                    ))
                    .labelsHidden()
                    .toggleStyle(.switch)
                    .accessibilityLabel("อัปเดตเครื่องมืออัตโนมัติ")
                }
            }

            TextField("ค้นหาเครื่องมือ…", text: $query)
                .textFieldStyle(.roundedBorder)
                .accessibilityLabel("ค้นหาเครื่องมือ")

            if filteredEntries.isEmpty {
                ContentUnavailableView.search(text: query)
                    .frame(maxWidth: .infinity, minHeight: 150)
            } else {
                ForEach(filteredEntries) { entry in
                    IntegrationCard(
                        entry: entry,
                        projectName: projectName,
                        progress: progress?.id == entry.id ? progress : nil,
                        onShowProgress: {
                            requestedAction = .init(entryID: entry.id,
                                                    kind: !entry.isInstalled ? .install : entry.updateAvailable ? .update : .remove)
                        },
                        onInstall: { requestedAction = .init(entryID: entry.id, kind: .install) },
                        onUpdate: { requestedAction = .init(entryID: entry.id, kind: .update) },
                        onRemove: { requestedAction = .init(entryID: entry.id, kind: .remove) },
                        onProjectEnableChanged: { onProjectEnableChanged(entry.id, $0) }
                    )
                }
            }
        }
        .animation(.easeOut(duration: 0.16), value: filteredEntries.map(\.id))
        .onAppear(perform: openRequestedInstall)
        .onChange(of: requestedInstall) { _, _ in openRequestedInstall() }
        .sheet(item: $requestedAction) { request in
            if let entry = entries.first(where: { $0.id == request.entryID }) {
                InstallSheet(
                    entry: entry,
                    projectName: projectName,
                    kind: request.kind,
                    result: actionResult,
                    progress: progress,
                    loadPlan: onLoadPlan,
                    onDismiss: { requestedAction = nil },
                    onConfirm: {
                        dismissedResultID = nil
                        switch request.kind {
                        case .install: onInstall(entry.id)
                        case .update: onUpdate(entry.id)
                        case .remove: onRemove(entry.id)
                        }
                    }
                )
                .padding(20)
                .frame(minWidth: 420, maxWidth: 500)
            }
        }
    }

    private func openRequestedInstall() {
        guard let id = requestedInstall,
              let entry = entries.first(where: { $0.id == id }), !entry.isInstalled else { return }
        requestedAction = .init(entryID: id, kind: .install)
        onRequestHandled()
    }
}

private struct IntegrationCard: View {
    let entry: CatalogIntegration
    let projectName: String
    let progress: CatalogProgress?
    let onShowProgress: () -> Void
    let onInstall: () -> Void
    let onUpdate: () -> Void
    let onRemove: () -> Void
    let onProjectEnableChanged: (Bool) -> Void

    private var statusColor: Color {
        guard entry.isInstalled else { return .secondary }
        return entry.isHealthy == true ? .green : (entry.isHealthy == false ? .orange : .secondary)
    }

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            IntegrationIcon(id: entry.id, size: 38)

            VStack(alignment: .leading, spacing: 3) {
                Text(entry.title).font(.headline)
                Text(entry.actions.first ?? entry.maintainer)
                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                if entry.isWorking {
                    VStack(alignment: .leading, spacing: 4) {
                        ProgressView().progressViewStyle(.linear).controlSize(.small)
                        Text([progress?.phaseLabel ?? "รอคิว", progress?.detail].compactMap { $0 }
                            .filter { !$0.isEmpty }.joined(separator: " · "))
                            .font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                    }
                    .frame(maxWidth: 320, alignment: .leading)
                } else if entry.isInstalled {
                    HStack(spacing: 6) {
                        Circle().fill(statusColor).frame(width: 6, height: 6)
                        Text(installStatus).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                } else if let missing = entry.missingInstaller {
                    Text("ต้องมี \(missing) ก่อน")
                        .font(.caption).foregroundStyle(.orange)
                }
            }
            Spacer(minLength: 8)

            if entry.isInstalled && !entry.isWorking {
                VStack(alignment: .trailing, spacing: 3) {
                    Toggle("", isOn: Binding(get: { entry.isEnabledForProject }, set: { onProjectEnableChanged($0) }))
                        .labelsHidden().toggleStyle(.switch).controlSize(.small)
                        .disabled(projectName.isEmpty || entry.isHealthy == false)
                    Text(projectName.isEmpty ? "เลือกโปรเจกต์ก่อน"
                         : entry.isEnabledForProject ? "เปิดใช้กับ \(projectName)" : "ปิดอยู่ใน \(projectName)")
                        .font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                }
                .help("เปิดให้ Claude และ Codex ใช้เครื่องมือนี้ในโปรเจกต์ปัจจุบัน")
            }
            actionButton
        }
        .padding(12)
        .frame(minHeight: 56)
        .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(Color.primary.opacity(0.055), lineWidth: 0.5)
        }
    }

    @ViewBuilder
    private var actionButton: some View {
        if entry.isWorking {
            Button("ดู", action: onShowProgress).buttonStyle(.bordered).controlSize(.small)
        } else if let missing = entry.missingInstaller, !entry.isInstalled,
                  let url = entry.installerURL {
            Link("วิธีติดตั้ง \(missing)", destination: url)
                .buttonStyle(.bordered).controlSize(.small)
        } else if !entry.isInstalled {
            Button("ติดตั้ง…", action: onInstall).buttonStyle(.borderedProminent).controlSize(.small)
        } else if entry.updateAvailable {
            Button("อัปเดต…", action: onUpdate).buttonStyle(.borderedProminent).controlSize(.small)
        } else {
            Button(entry.installedMethod == "external" ? "เลิกใช้…" : "ลบ…", role: .destructive, action: onRemove)
                .buttonStyle(.bordered).controlSize(.small)
        }
    }

    private var installStatus: String {
        if entry.updateAvailable { return "มีเวอร์ชันใหม่ · \(entry.installedVersion ?? "")" }
        if entry.isHealthy == false { return "ใช้งานไม่ได้ · \(entry.healthMessage ?? "ตรวจรายละเอียด")" }
        return "พร้อมใช้ · \(entry.installedVersion ?? entry.reviewedVersion)"
    }
}

private struct ResultBanner: View {
    let result: CatalogActionResult
    let title: String
    let onDismiss: () -> Void

    private var verb: String {
        switch result.action {
        case "remove": "ลบ"
        case "update": "อัปเดต"
        case "enable": "เปิดใช้"
        case "disable": "เลิกใช้"
        default: "ติดตั้ง"
        }
    }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 9) {
            Image(systemName: result.succeeded ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                .foregroundStyle(result.succeeded ? .green : .red)
            Text(result.succeeded ? "\(verb) \(title) เรียบร้อย" : "\(verb) \(title) ไม่สำเร็จ: \(result.error ?? "")")
                .font(.callout).textSelection(.enabled)
            Spacer(minLength: 6)
            Button(action: onDismiss) { Image(systemName: "xmark").font(.caption.weight(.semibold)) }
                .buttonStyle(.chorusPlain).foregroundStyle(.secondary)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background((result.succeeded ? Color.green : Color.red).opacity(0.08), in: RoundedRectangle(cornerRadius: 10))
    }
}

struct IntegrationIcon: View {
    let id: String
    var size: CGFloat = 38

    private var symbol: String {
        switch id {
        case "graft": "point.3.connected.trianglepath.dotted"
        case "serena": "sparkle.magnifyingglass"
        case "rtk": "terminal"
        case "headroom": "arrow.up.to.line.compact"
        default: "puzzlepiece.extension"
        }
    }

    var body: some View {
        Image(systemName: symbol)
            .font(.system(size: size * 0.46, weight: .medium))
            .foregroundStyle(.white)
            .frame(width: size, height: size)
            .background(LinearGradient(colors: [Color.accentColor, Color.accentColor.opacity(0.72)],
                                       startPoint: .top, endPoint: .bottom),
                        in: RoundedRectangle(cornerRadius: size * 0.23, style: .continuous))
    }
}

/// Apple-style install sheet: shows exactly what will be created or removed,
/// then the same list ticks off live while it runs. The list comes from the
/// core, so the preview and the actual change cannot disagree.
struct InstallSheet: View {
    let entry: CatalogIntegration
    let projectName: String
    let kind: CatalogActionKind
    let result: CatalogActionResult?
    let progress: CatalogProgress?
    let loadPlan: (String, String, @escaping (InstallPlan?, String?) -> Void) -> Void
    let onDismiss: () -> Void
    let onConfirm: () -> Void

    @State private var plan: InstallPlan?
    @State private var planError: String?
    @State private var started = false
    @State private var startedAt = Date()

    private var removing: Bool { kind == .remove }
    private var external: Bool { entry.installedMethod == "external" }
    private var finished: CatalogActionResult? {
        guard started, !entry.isWorking, result?.id == entry.id else { return nil }
        return result
    }
    private var planAction: String {
        kind == .install && projectName.isEmpty ? "install" : kind.rawValue
    }

    private var title: String {
        switch kind {
        case .install: "ติดตั้ง \(entry.title)"
        case .update: "อัปเดต \(entry.title)"
        case .remove: "\(external ? "เลิกใช้" : "ลบ") \(entry.title)"
        }
    }

    private var subtitle: String {
        if let finished {
            return finished.succeeded ? (removing ? "ลบเรียบร้อย ไม่เหลืออะไรค้างไว้" : "พร้อมใช้งาน") : "ไม่สำเร็จ"
        }
        if entry.isWorking { return removing ? "กำลังลบ…" : "กำลังติดตั้ง…" }
        if external { return "ติดตั้งไว้ก่อนแล้ว · จะเลิกใช้ใน Chorus แต่ไม่ลบตัวโปรแกรม" }
        return removing ? "จะลบเฉพาะสิ่งที่ Chorus เพิ่มไว้" : "สำหรับ Claude และ Codex"
    }

    var body: some View {
        VStack(spacing: 18) {
            VStack(spacing: 10) {
                ZStack {
                    IntegrationIcon(id: entry.id, size: 64)
                        .saturation(removing ? 0.2 : 1)
                    if let finished {
                        Image(systemName: finished.succeeded ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                            .font(.system(size: 22))
                            .symbolRenderingMode(.palette)
                            .foregroundStyle(.white, finished.succeeded ? Color.green : Color.red)
                            .offset(x: 26, y: 26)
                            .transition(.scale.combined(with: .opacity))
                    }
                }
                Text(title).font(.title2.weight(.semibold))
                Text(subtitle).font(.callout).foregroundStyle(.secondary)
            }
            .animation(.spring(duration: 0.3), value: finished?.succeeded)

            VStack(spacing: 0) {
                if let plan {
                    ForEach(Array(plan.items.enumerated()), id: \.element.id) { index, item in
                        if index > 0 { Divider().padding(.leading, 46) }
                        PlanRow(item: item, state: rowState(item))
                    }
                } else if let planError {
                    Label(planError, systemImage: "exclamationmark.triangle")
                        .font(.callout).foregroundStyle(.orange).padding(14)
                } else {
                    ProgressView().controlSize(.small).padding(18)
                }
            }
            .frame(maxWidth: .infinity)
            .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 12, style: .continuous))

            if started, entry.isWorking {
                TimelineView(.periodic(from: startedAt, by: 1)) { context in
                    HStack(spacing: 8) {
                        Text([progress?.detail ?? ""].filter { !$0.isEmpty }.first ?? progress?.phaseLabel ?? "กำลังเริ่ม")
                            .lineLimit(1).truncationMode(.middle)
                        Spacer(minLength: 8)
                        Text("\(Int(context.date.timeIntervalSince(startedAt))) วิ").monospacedDigit()
                    }
                    .font(.caption).foregroundStyle(.secondary)
                }
            } else if let message = finished?.error {
                Text(message).font(.caption).foregroundStyle(.red)
                    .textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
            } else if !started && !removing && !entry.projectAccess.isEmpty {
                Label(entry.projectAccess[0], systemImage: "lock.shield")
                    .font(.caption).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }

            HStack {
                Spacer()
                if finished != nil {
                    Button("เสร็จสิ้น", action: onDismiss).buttonStyle(.borderedProminent)
                        .keyboardShortcut(.defaultAction)
                } else if entry.isWorking {
                    Button("ทำต่อในพื้นหลัง", action: onDismiss)
                } else {
                    Button("ยกเลิก", action: onDismiss)
                        .keyboardShortcut(.cancelAction)
                    Button(removing ? (external ? "เลิกใช้" : "ลบ") : kind == .update ? "อัปเดต" : "ติดตั้ง") {
                        started = true
                        startedAt = Date()
                        onConfirm()
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(removing ? .red : .accentColor)
                    .keyboardShortcut(.defaultAction)
                    .disabled(plan == nil || entry.isWorking || started)
                }
            }
        }
        .padding(24)
        .frame(width: 440)
        .onAppear {
            if entry.isWorking { started = true }
            loadPlan(entry.id, planAction) { plan, error in
                self.plan = plan
                self.planError = error
            }
        }
    }

    private func rowState(_ item: PlanItem) -> PlanRow.Phase {
        guard started else { return .pending }
        if let finished { return finished.succeeded ? .done : (progress?.current == item.phase ? .failed : .pending) }
        if progress?.done.contains(item.phase) == true { return .done }
        if progress?.current == item.phase { return .running }
        return .pending
    }
}

private struct PlanRow: View {
    enum Phase { case pending, running, done, failed }
    let item: PlanItem
    let state: Phase

    private var symbol: String {
        switch item.kind {
        case "package": "shippingbox"
        case "file": "doc"
        case "state": "list.bullet.rectangle"
        case "check": "stethoscope"
        case "project": "folder"
        case "keep": "hand.raised"
        default: "circle"
        }
    }

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: symbol)
                .font(.system(size: 14))
                .foregroundStyle(item.kind == "keep" ? Color.orange : Color.secondary)
                .frame(width: 22)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.title).font(.callout)
                if let detail = item.detail {
                    Text(detail).font(.caption.monospaced()).foregroundStyle(.secondary)
                        .lineLimit(2).truncationMode(.middle).textSelection(.enabled)
                }
            }
            Spacer(minLength: 6)
            switch state {
            case .pending: EmptyView()
            case .running: ProgressView().controlSize(.small)
            case .done: Image(systemName: "checkmark").foregroundStyle(.green).transition(.scale)
            case .failed: Image(systemName: "xmark").foregroundStyle(.red)
            }
        }
        .padding(.horizontal, 12).padding(.vertical, 10)
        .animation(.easeOut(duration: 0.15), value: state)
        .accessibilityElement(children: .combine)
    }
}
