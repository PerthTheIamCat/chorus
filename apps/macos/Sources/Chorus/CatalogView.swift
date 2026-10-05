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
        default: nil
        }
    }
}

struct CatalogActionResult: Equatable {
    let id: String
    let action: String
    let error: String?
    var succeeded: Bool { error == nil }
}

private enum CatalogActionKind { case install, update, remove }

private struct CatalogActionRequest: Identifiable {
    let entryID: String
    let kind: CatalogActionKind
    var id: String { "\(entryID)-\(kind)" }
}

/// Curated integration browser. State changes are emitted to the owner so that
/// persistence and managed tool installation remain outside the presentation layer.
struct CatalogView: View {
    let projectName: String
    let entries: [CatalogIntegration]
    let automaticUpdates: Bool
    let backgroundStatus: String?
    let actionResult: CatalogActionResult?
    let onAutomaticUpdatesChanged: (Bool) -> Void
    let onInstall: (String) -> Void
    let onUpdate: (String) -> Void
    let onRemove: (String) -> Void
    let onProjectEnableChanged: (String, Bool) -> Void

    @State private var query = ""
    @State private var requestedAction: CatalogActionRequest?


    private var filteredEntries: [CatalogIntegration] {
        let search = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !search.isEmpty else { return entries }
        return entries.filter { entry in
            [entry.title, entry.maintainer, entry.license, entry.actions.joined(separator: " ")]
                .contains { $0.localizedCaseInsensitiveContains(search) }
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            VStack(alignment: .leading, spacing: 4) {
                Text("เครื่องมือเสริม").font(.title2.bold())
                Text("ตรวจสถานะผ่าน Homebrew แล้วเลือกเปิดใช้แยกตามโปรเจกต์")
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

            SettingsGroup(title: "การอัปเดต") {
                SettingsRow(title: "อัปเดตผ่าน Homebrew อัตโนมัติ",
                            detail: "เมื่อเวอร์ชันเก่ากว่าที่ Chorus ตรวจไว้ Homebrew จะใช้สูตรล่าสุดและ Chorus จะตรวจสุขภาพอีกครั้ง") {
                    Toggle("", isOn: Binding(
                        get: { automaticUpdates },
                        set: { onAutomaticUpdatesChanged($0) }
                    ))
                    .labelsHidden()
                    .toggleStyle(.switch)
                    .accessibilityLabel("ติดตั้งเวอร์ชันที่ Chorus อนุมัติอัตโนมัติ")
                }
            }

            TextField("ค้นหา integration…", text: $query)
                .textFieldStyle(.roundedBorder)
                .accessibilityLabel("ค้นหา integration")

            if filteredEntries.isEmpty {
                ContentUnavailableView.search(text: query)
                    .frame(maxWidth: .infinity, minHeight: 150)
            } else {
                ForEach(filteredEntries) { entry in
                    IntegrationCard(
                        entry: entry,
                        projectName: projectName,
                        onInstall: { requestedAction = .init(entryID: entry.id, kind: .install) },
                        onUpdate: { requestedAction = .init(entryID: entry.id, kind: .update) },
                        onRemove: { requestedAction = .init(entryID: entry.id, kind: .remove) },
                        onProjectEnableChanged: { onProjectEnableChanged(entry.id, $0) }
                    )
                }
            }
        }
        .animation(.easeOut(duration: 0.16), value: filteredEntries.map(\.id))
        .sheet(item: $requestedAction) { request in
            if let entry = entries.first(where: { $0.id == request.entryID }) {
                CatalogActionSheet(entry: entry, projectName: projectName, kind: request.kind,
                                   result: actionResult,
                                   onDismiss: { requestedAction = nil },
                                   onConfirm: {
                                       switch request.kind {
                                       case .install: onInstall(entry.id)
                                       case .update: onUpdate(entry.id)
                                       case .remove: onRemove(entry.id)
                                       }
                                   })
            }
        }
    }
}

private struct IntegrationCard: View {
    let entry: CatalogIntegration
    let projectName: String
    let onInstall: () -> Void
    let onUpdate: () -> Void
    let onRemove: () -> Void
    let onProjectEnableChanged: (Bool) -> Void

    private var statusColor: Color {
        guard entry.isInstalled else { return .secondary }
        return entry.isHealthy == true ? .green : (entry.isHealthy == false ? .orange : .secondary)
    }


    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: icon)
                    .font(.system(size: 17, weight: .medium))
                    .foregroundStyle(.tint)
                    .frame(width: 34, height: 34)
                    .background(Color.accentColor.opacity(0.1), in: RoundedRectangle(cornerRadius: 9))

                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 8) {
                        Text(entry.title).font(.headline)
                        Text(entry.modes.joined(separator: " · ").uppercased())
                            .font(.system(size: 9, weight: .semibold, design: .rounded))
                            .foregroundStyle(.secondary)
                    }
                    Text("\(entry.maintainer) · \(entry.license) · reviewed \(entry.reviewedVersion)")
                        .font(.caption).foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
                Spacer(minLength: 8)
                actionButton
            }

            if entry.isInstalled {
                HStack(spacing: 6) {
                    Circle().fill(statusColor).frame(width: 6, height: 6)
                    Text(installStatus).font(.caption).foregroundStyle(.secondary)
                    if let integrity = entry.integrity, !integrity.isEmpty {
                        Text("·").foregroundStyle(.tertiary)
                        Text("Integrity: \(integrity)").font(.caption2.monospaced()).foregroundStyle(.tertiary)
                            .lineLimit(1).truncationMode(.middle)
                            .textSelection(.enabled)
                    }
                }
            }

            Divider()

            detailSection("ทำอะไรได้", values: entry.actions)
            detailSection("สิทธิ์และการเข้าถึง", values: entry.projectAccess)
            HStack(alignment: .top, spacing: 5) {
                Text("เครือข่าย").font(.caption.weight(.medium)).foregroundStyle(.secondary)
                Text(entry.networkBehavior).font(.caption).foregroundStyle(.secondary)
            }
            if !entry.requiredRuntimes.isEmpty {
                HStack(alignment: .top, spacing: 5) {
                    Text("ต้องมี").font(.caption.weight(.medium)).foregroundStyle(.secondary)
                    Text(entry.requiredRuntimes.joined(separator: ", ")).font(.caption).foregroundStyle(.secondary)
                }
            }
            if let missing = entry.missingInstaller, !entry.isInstalled {
                Text(missing == "Homebrew formula"
                     ? "ยังไม่มีสูตร Homebrew ที่ Chorus ตรวจสอบแล้วสำหรับเครื่องมือนี้"
                     : "ยังไม่มี \(missing) สำหรับติดตั้งเครื่องมือนี้")
                    .font(.caption).foregroundStyle(.orange)
            }
            if !entry.requiredSecretNames.isEmpty {
                HStack(alignment: .top, spacing: 5) {
                    Text("ชื่อตัวแปรลับ").font(.caption.weight(.medium)).foregroundStyle(.secondary)
                    Text(entry.requiredSecretNames.joined(separator: ", ")).font(.caption.monospaced()).foregroundStyle(.secondary)
                }
            }

            HStack(spacing: 6) {
                Link(destination: entry.sourceURL) {
                    Label("ซอร์สและรายละเอียด", systemImage: "arrow.up.right.square")
                        .font(.caption)
                }
                Spacer()
                if entry.isInstalled {
                    Toggle(isOn: Binding(
                        get: { entry.isEnabledForProject },
                        set: { onProjectEnableChanged($0) }
                    )) {
                        Text(projectName.isEmpty ? "เลือกโปรเจกต์ก่อนเปิดใช้" : "เปิดใช้กับ \(projectName)")
                            .font(.caption)
                    }
                    .toggleStyle(.switch)
                    .controlSize(.small)
                    .disabled(projectName.isEmpty || entry.isHealthy != true || entry.isWorking)
                }
            }
        }
        .padding(14)
        .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(Color.primary.opacity(0.055), lineWidth: 0.5)
        }
    }

    @ViewBuilder
    private var actionButton: some View {
        if entry.isWorking {
            ProgressView().controlSize(.small).frame(width: 74)
        } else if let missing = entry.missingInstaller, !entry.isInstalled,
                  let url = entry.installerURL {
            Link("วิธีติดตั้ง \(missing)", destination: url)
                .buttonStyle(.borderedProminent).controlSize(.small)
        } else if entry.brewFormula == nil && entry.id != "chorus-review" && !entry.isInstalled {
            Button("รอสูตร Homebrew") {}
                .buttonStyle(.bordered).controlSize(.small).disabled(true)
        } else if !entry.isInstalled {
            Button(projectName.isEmpty ? "ติดตั้ง" : "ติดตั้งและเปิดใช้", action: onInstall)
                .buttonStyle(.borderedProminent).controlSize(.small)
        } else if entry.updateAvailable {
            Button("อัปเดต", action: onUpdate).buttonStyle(.borderedProminent).controlSize(.small)
        } else {
            Button("ลบ…", role: .destructive, action: onRemove)
                .buttonStyle(.bordered).controlSize(.small)
                .help("ตรวจรายละเอียดก่อนลบเครื่องมือนี้จาก Mac")
        }
    }

    private var installStatus: String {
        if entry.installedMethod != "homebrew" && entry.installedMethod != "bundled" {
            return "ติดตั้งแบบเดิม · ลบได้จากปุ่มนี้ แต่ยังย้ายไป Homebrew ไม่ได้"
        }
        if entry.updateAvailable { return "ติดตั้ง \(entry.installedVersion ?? "") · มีเวอร์ชันที่อนุมัติใหม่" }
        if entry.isHealthy == false { return "ตรวจสุขภาพไม่ผ่าน · \(entry.healthMessage ?? "ตรวจสอบรายละเอียด")" }
        if entry.isHealthy == true { return "พร้อมใช้ · \(entry.installedVersion ?? entry.reviewedVersion)" }
        return "ยังไม่ได้ยืนยันสุขภาพการติดตั้ง · \(entry.installedVersion ?? entry.reviewedVersion)"
    }

    private var icon: String {
        switch entry.id {
        case "graft": "point.3.connected.trianglepath.dotted"
        case "serena": "sparkle.magnifyingglass"
        case "rtk": "terminal"
        case "headroom": "arrow.up.to.line.compact"
        default: "puzzlepiece.extension"
        }
    }

    @ViewBuilder
    private func detailSection(_ title: String, values: [String]) -> some View {
        if !values.isEmpty {
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.caption.weight(.medium)).foregroundStyle(.secondary)
                ForEach(values, id: \.self) { value in
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Image(systemName: "circle.fill").font(.system(size: 3)).foregroundStyle(.tertiary)
                        Text(value).font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
        }
    }
}

private struct CatalogActionSheet: View {
    let entry: CatalogIntegration
    let projectName: String
    let kind: CatalogActionKind
    let result: CatalogActionResult?
    let onDismiss: () -> Void
    let onConfirm: () -> Void

    @State private var started = false

    private var matchingResult: CatalogActionResult? {
        guard started, result?.id == entry.id else { return nil }
        return result
    }

    private var isRemoving: Bool { kind == .remove }
    private var verb: String {
        switch kind {
        case .install: "ติดตั้ง"
        case .update: "อัปเดต"
        case .remove: "ลบ"
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack(alignment: .top, spacing: 14) {
                Image(systemName: isRemoving ? "trash" : "shippingbox")
                    .font(.system(size: 24, weight: .medium))
                    .foregroundStyle(isRemoving ? Color.red : Color.accentColor)
                    .frame(width: 48, height: 48)
                    .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 12))
                VStack(alignment: .leading, spacing: 4) {
                    Text("\(verb) \(entry.title)").font(.title2.bold())
                    Text(isRemoving ? "ตรวจรายการที่จะลบจาก Mac ก่อนดำเนินการ" : "ตรวจรายการที่จะติดตั้งก่อนดำเนินการ")
                        .font(.callout).foregroundStyle(.secondary)
                }
            }

            VStack(alignment: .leading, spacing: 11) {
                Text("Chorus จะทำสิ่งต่อไปนี้").font(.headline)
                planRow("1", commandDescription)
                planRow("2", isRemoving ? "ปิดการใช้งานในทุกโปรเจกต์ของ Chorus" : "ตรวจว่าเครื่องมือเปิดใช้งานได้")
                if !isRemoving && !projectName.isEmpty {
                    planRow("3", "เปิดใช้กับโปรเจกต์ \(projectName)")
                }
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 12))

            if !started {
                if isRemoving, entry.brewFormula != nil {
                    Text("การลบผ่าน Homebrew อาจกระทบแอปอื่นที่ใช้เครื่องมือนี้ด้วย")
                        .font(.callout).foregroundStyle(.orange)
                } else {
                    VStack(alignment: .leading, spacing: 5) {
                        Text("การเข้าถึงหลังเปิดใช้").font(.callout.weight(.semibold))
                        ForEach(entry.projectAccess, id: \.self) { access in
                            Label(access, systemImage: "checkmark.shield")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        Text(entry.networkBehavior).font(.caption).foregroundStyle(.secondary)
                    }
                }
            } else if entry.isWorking {
                HStack(spacing: 10) {
                    ProgressView().controlSize(.small)
                    Text(progressDescription)
                        .font(.callout)
                }
                .accessibilityElement(children: .combine)
            } else if let matchingResult {
                Label(matchingResult.error ?? "\(verb) \(entry.title) สำเร็จ", systemImage: matchingResult.succeeded ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                    .foregroundStyle(matchingResult.succeeded ? .green : .red)
                    .font(.callout)
                    .textSelection(.enabled)
            }

            HStack {
                Spacer()
                Button(started ? "ปิด" : "ยกเลิก", action: onDismiss)
                    .disabled(entry.isWorking)
                if !started {
                    Button(isRemoving ? "ลบจาก Mac" : entry.brewFormula == nil ? verb : "\(verb)ด้วย Homebrew") {
                        started = true
                        onConfirm()
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(isRemoving ? .red : .accentColor)
                    .keyboardShortcut(.defaultAction)
                }
            }
        }
        .padding(24)
        .frame(width: 520)
    }

    private var commandDescription: String {
        if let formula = entry.brewFormula {
            return "\(isRemoving ? "brew uninstall" : kind == .update ? "brew upgrade" : "brew install") \(formula)"
        }
        if isRemoving, entry.installedMethod != "bundled" {
            return "ลบการติดตั้งเดิมที่ Chorus จัดเก็บไว้"
        }
        return isRemoving ? "ลบ skill ที่ Chorus จัดเก็บไว้" : "เพิ่ม skill ที่มากับ Chorus"
    }

    private var progressDescription: String {
        if entry.brewFormula != nil {
            return isRemoving ? "Homebrew กำลังลบและตรวจสอบสถานะ…" : "Homebrew กำลังติดตั้งและตรวจสอบสถานะ…"
        }
        return isRemoving ? "กำลังลบและตรวจสอบสถานะ…" : "กำลังติดตั้งและตรวจสอบสถานะ…"
    }

    private func planRow(_ number: String, _ title: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(number).font(.caption.bold()).foregroundStyle(.secondary)
                .frame(width: 19, height: 19)
                .background(Color.primary.opacity(0.07), in: Circle())
            Text(title).font(.callout).fixedSize(horizontal: false, vertical: true)
        }
    }
}
