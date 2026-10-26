import AppKit
import SwiftUI

struct DashboardMetric: Decodable, Hashable { let label: String; let value: String }

struct ToolAction: Decodable, Hashable, Identifiable {
    let key: String
    let title: String
    let kind: String
    let target: String?
    let summary: String
    let effects: [String]
    let danger: Bool
    var id: String { key }

    var symbol: String {
        switch kind {
        case "url": "safari"
        case "reveal": "folder"
        default: danger ? "exclamationmark.triangle" : "play.circle"
        }
    }
}

struct ToolDashboard: Decodable {
    let id: String
    let title: String
    let version: String
    let healthy: Bool
    let location: String
    let disk_bytes: Int?
    let projects: [String]
    let metrics: [DashboardMetric]
    let actions: [ToolAction]
    let history: [String]
}

struct ToolActionOutput: Decodable { let ok: Bool; let output: String }

/// A tool's own page in the main window: what it is doing, what it can do,
/// and every action as a button that says what will happen before it runs.
struct ToolPage: View {
    @EnvironmentObject var engine: Engine
    let toolID: String
    let workspace: String
    let onClose: () -> Void

    @State private var board: ToolDashboard?
    @State private var error: String?
    @State private var pending: ToolAction?
    @State private var manage: CatalogActionKind?

    private var entry: CatalogIntegration? { engine.catalog.first { $0.id == toolID } }
    private var projectName: String { workspace.isEmpty ? "" : URL(fileURLWithPath: workspace).lastPathComponent }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                HStack {
                    Button(action: onClose) { Label("กลับไปที่ session", systemImage: "chevron.left") }
                        .buttonStyle(.chorusPlain).foregroundStyle(.secondary)
                    Spacer()
                }
                if let board { content(board) }
                else if let error { Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(.orange) }
                else { ProgressView().controlSize(.small) }
            }
            .frame(maxWidth: 760, alignment: .leading)
            .frame(maxWidth: .infinity)
            .padding(24)
        }
        .onAppear(perform: reload)
        .onChange(of: toolID) { _, _ in board = nil; error = nil; reload() }
        .onChange(of: engine.catalogWorking) { _, _ in reload() }
        .sheet(item: $pending) { action in
            ToolActionSheet(action: action, tool: board?.title ?? toolID, toolID: toolID, workspace: workspace)
        }
        .sheet(item: Binding(get: { manage.map { ManageRequest(kind: $0) } }, set: { manage = $0?.kind })) { request in
            if let entry {
                InstallSheet(entry: entry, projectName: projectName, kind: request.kind,
                             result: engine.catalogActionResult, progress: engine.catalogProgress,
                             loadPlan: { id, action, done in
                                 engine.loadCatalogPlan(project: workspace, id: id, action: action, completion: done)
                             },
                             onDismiss: { manage = nil; if entry.isInstalled == false { onClose() } },
                             onConfirm: {
                                 engine.catalogAction(project: workspace, id: toolID,
                                                      action: request.kind == .remove ? "remove" : "update")
                             })
            }
        }
    }

    private struct ManageRequest: Identifiable { let kind: CatalogActionKind; var id: String { kind.rawValue } }

    private func reload() {
        engine.loadDashboard(project: workspace, id: toolID) { result, failure in
            board = result
            error = failure
        }
    }

    @ViewBuilder
    private func content(_ d: ToolDashboard) -> some View {
        HStack(spacing: 14) {
            IntegrationIcon(id: d.id, size: 52)
            VStack(alignment: .leading, spacing: 3) {
                Text(d.title).font(.title2.weight(.semibold))
                HStack(spacing: 6) {
                    Circle().fill(d.healthy ? Color.green : Color.orange).frame(width: 7, height: 7)
                    Text("\(d.healthy ? "พร้อมใช้" : "ใช้งานไม่ได้") · v\(d.version)")
                        .font(.callout).foregroundStyle(.secondary)
                }
            }
            Spacer()
        }

        if !d.metrics.isEmpty {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 150), spacing: 10)], spacing: 10) {
                ForEach(d.metrics, id: \.self) { m in
                    VStack(alignment: .leading, spacing: 4) {
                        Text(m.value).font(.title2.weight(.semibold)).monospacedDigit()
                        Text(m.label).font(.caption).foregroundStyle(.secondary)
                    }
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                }
            }
        }

        if !d.actions.isEmpty {
            SettingsGroup(title: "คำสั่ง", footer: "ทุกปุ่มจะบอกก่อนว่าจะเกิดอะไรขึ้น และรอให้ยืนยัน") {
                ForEach(Array(d.actions.enumerated()), id: \.element.id) { index, action in
                    if index > 0 { Divider() }
                    Button { pending = action } label: {
                        HStack(spacing: 12) {
                            Image(systemName: action.symbol)
                                .foregroundStyle(action.danger ? Color.red : Color.accentColor)
                                .frame(width: 22)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(action.title).foregroundStyle(action.danger ? Color.red : Color.primary)
                                Text(action.summary).font(.caption).foregroundStyle(.secondary)
                            }
                            Spacer()
                            Image(systemName: "chevron.right").font(.caption).foregroundStyle(.tertiary)
                        }
                        .padding(.horizontal, 12).padding(.vertical, 9)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.chorusPlain)
                }
            }
        }

        SettingsGroup(title: "การตั้งค่า") {
            SettingsRow(title: projectName.isEmpty ? "เปิดใช้กับโปรเจกต์" : "เปิดใช้กับ \(projectName)",
                        detail: "เปิดให้ Claude และ Codex ใช้เครื่องมือนี้ในโปรเจกต์ปัจจุบัน") {
                Toggle("", isOn: Binding(get: { entry?.isEnabledForProject ?? false },
                                         set: { engine.catalogAction(project: workspace, id: toolID, action: $0 ? "enable" : "disable") }))
                    .labelsHidden().toggleStyle(.switch)
                    .disabled(projectName.isEmpty || entry?.isHealthy == false)
            }
            if entry?.updateAvailable == true {
                Divider()
                SettingsRow(title: "มีเวอร์ชันใหม่", detail: "ดูรายการก่อนอัปเดต") {
                    Button("อัปเดต…") { manage = .update }.buttonStyle(.borderedProminent).controlSize(.small)
                }
            }
            Divider()
            SettingsRow(title: entry?.installedMethod == "external" ? "เลิกใช้ใน Chorus" : "ลบเครื่องมือนี้",
                        detail: entry?.installedMethod == "external" ? "ตัวโปรแกรมจะไม่ถูกลบ" : "ลบเฉพาะที่ Chorus ติดตั้ง") {
                Button(entry?.installedMethod == "external" ? "เลิกใช้…" : "ลบ…", role: .destructive) { manage = .remove }
                    .buttonStyle(.bordered).controlSize(.small)
            }
        }

        SettingsGroup(title: "การติดตั้ง") {
            SettingsRow(title: "ที่อยู่") {
                Text(d.location).font(.caption.monospaced()).foregroundStyle(.secondary)
                    .lineLimit(1).truncationMode(.middle).textSelection(.enabled)
            }
            if let bytes = d.disk_bytes {
                Divider()
                SettingsRow(title: "พื้นที่ที่ใช้") {
                    Text(ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)).foregroundStyle(.secondary)
                }
            }
            Divider()
            SettingsRow(title: "เปิดใช้กับ") {
                Text(d.projects.isEmpty ? "ยังไม่มีโปรเจกต์" : d.projects.joined(separator: ", "))
                    .foregroundStyle(.secondary).lineLimit(1)
            }
        }

        if !d.history.isEmpty {
            SettingsGroup(title: "ประวัติ") {
                ForEach(Array(d.history.enumerated()), id: \.offset) { index, line in
                    if index > 0 { Divider() }
                    SettingsRow(title: line) { EmptyView() }
                }
            }
        }
    }
}

/// Always shown before a tool command runs: what will happen, then the choice.
private struct ToolActionSheet: View {
    @EnvironmentObject var engine: Engine
    @Environment(\.dismiss) private var dismiss
    let action: ToolAction
    let tool: String
    let toolID: String
    let workspace: String

    @State private var running = false
    @State private var result: ToolActionOutput?
    @State private var failure: String?

    var body: some View {
        VStack(spacing: 18) {
            VStack(spacing: 10) {
                Image(systemName: action.symbol)
                    .font(.system(size: 26, weight: .medium))
                    .foregroundStyle(.white)
                    .frame(width: 60, height: 60)
                    .background(action.danger ? Color.red : Color.accentColor,
                                in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                Text(action.title).font(.title3.weight(.semibold))
                Text(tool).font(.callout).foregroundStyle(.secondary)
            }

            VStack(alignment: .leading, spacing: 0) {
                Text("สิ่งที่จะเกิดขึ้น").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                    .padding(.horizontal, 12).padding(.top, 10).padding(.bottom, 4)
                ForEach(Array(action.effects.enumerated()), id: \.offset) { index, effect in
                    if index > 0 { Divider().padding(.leading, 38) }
                    HStack(alignment: .firstTextBaseline, spacing: 10) {
                        Image(systemName: action.danger && index == 0 ? "exclamationmark.triangle.fill" : "checkmark.circle")
                            .foregroundStyle(action.danger && index == 0 ? Color.red : Color.secondary)
                            .frame(width: 18)
                        Text(effect).font(.callout).fixedSize(horizontal: false, vertical: true)
                    }
                    .padding(.horizontal, 12).padding(.vertical, 8)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 12, style: .continuous))

            if running {
                HStack(spacing: 8) { ProgressView().controlSize(.small); Text("กำลังทำงาน…").font(.callout) }
            } else if let result {
                ScrollView {
                    Text(result.output).font(.caption.monospaced()).textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 180).padding(10)
                .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 10))
                if !result.ok {
                    Label("คำสั่งจบด้วยข้อผิดพลาด", systemImage: "exclamationmark.triangle.fill")
                        .font(.caption).foregroundStyle(.red)
                }
            } else if let failure {
                Label(failure, systemImage: "exclamationmark.triangle.fill").font(.callout).foregroundStyle(.red)
            }

            HStack {
                Spacer()
                if result != nil || failure != nil {
                    Button("เสร็จสิ้น") { dismiss() }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                } else {
                    Button("ยกเลิก") { dismiss() }.keyboardShortcut(.cancelAction).disabled(running)
                    Button(action.title, role: action.danger ? .destructive : nil, action: go)
                        .buttonStyle(.borderedProminent).tint(action.danger ? .red : .accentColor)
                        .keyboardShortcut(.defaultAction).disabled(running)
                }
            }
        }
        .padding(24)
        .frame(width: 460)
    }

    private func go() {
        switch action.kind {
        case "url":
            if let target = action.target, let url = URL(string: target) { NSWorkspace.shared.open(url) }
            dismiss()
        case "reveal":
            if let target = action.target { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: target)]) }
            dismiss()
        default:
            running = true
            engine.runToolAction(project: workspace, id: toolID, key: action.key) { output, error in
                running = false
                result = output
                failure = error
            }
        }
    }
}

/// Install/update/remove progress that stays visible outside Settings.
struct InstallStatusChip: View {
    @EnvironmentObject var engine: Engine
    let onOpen: () -> Void
    @State private var dismissed: String?

    private var workingID: String? { engine.catalogWorking.sorted().first }
    private var title: (String) -> String { { id in engine.catalog.first { $0.id == id }?.title ?? id } }

    var body: some View {
        Group {
            if let id = workingID {
                chip {
                    ProgressView().controlSize(.small)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(title(id)).font(.callout.weight(.medium))
                        Text([engine.catalogProgress?.phaseLabel ?? "รอคิว", engine.catalogProgress?.detail]
                                .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "))
                            .font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                    }
                }
            } else if let result = engine.catalogActionResult, result.id != dismissed {
                chip {
                    Image(systemName: result.succeeded ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                        .foregroundStyle(result.succeeded ? .green : .red)
                    Text(result.succeeded ? "\(title(result.id)) เรียบร้อย" : "\(title(result.id)) ไม่สำเร็จ")
                        .font(.callout.weight(.medium))
                    Button { dismissed = result.id } label: { Image(systemName: "xmark").font(.caption.weight(.semibold)) }
                        .buttonStyle(.chorusPlain).foregroundStyle(.secondary)
                }
                .task(id: result) {
                    try? await Task.sleep(for: .seconds(8))
                    dismissed = result.id
                }
            }
        }
        .animation(.spring(response: 0.3, dampingFraction: 0.85), value: workingID)
    }

    private func chip<C: View>(@ViewBuilder _ content: () -> C) -> some View {
        Button(action: onOpen) {
            HStack(spacing: 10) { content() }
                .padding(.horizontal, 14).padding(.vertical, 9)
                .floatingPanel(cornerRadius: 16)
        }
        .buttonStyle(.chorusPlain)
        .transition(.move(edge: .top).combined(with: .opacity))
    }
}
