import AppKit
import SwiftUI

struct DashboardMetric: Decodable, Hashable { let label: String; let value: String }

struct DashboardAction: Decodable, Hashable, Identifiable {
    let key: String
    let title: String
    let kind: String
    let target: String?
    let hint: String?
    var id: String { key }
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
    let actions: [DashboardAction]
    let history: [String]
}

/// One page per installed integration: what it is doing for the current
/// project, what it costs on disk, and a way into the tool's own dashboard.
struct DashboardsSettings: View {
    @EnvironmentObject var engine: Engine
    let workspace: String
    @State private var selection: String?
    @State private var board: ToolDashboard?
    @State private var error: String?
    @State private var loading = false

    private var installed: [CatalogIntegration] { engine.catalog.filter(\.isInstalled) }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            VStack(alignment: .leading, spacing: 4) {
                Text("แดชบอร์ด").font(.title2.bold())
                Text("ภาพรวมของเครื่องมือเสริมแต่ละตัว").font(.callout).foregroundStyle(.secondary)
            }

            if installed.isEmpty {
                ContentUnavailableView("ยังไม่มีเครื่องมือเสริม", systemImage: "chart.bar.xaxis",
                                       description: Text("ติดตั้งจากหมวดเครื่องมือเสริม แล้วแดชบอร์ดของแต่ละตัวจะมาอยู่ที่นี่"))
                    .frame(maxWidth: .infinity, minHeight: 220)
            } else {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(installed) { tool in
                            Button { select(tool.id) } label: {
                                HStack(spacing: 7) {
                                    IntegrationIcon(id: tool.id, size: 20)
                                    Text(tool.title)
                                }
                            }
                            .buttonStyle(InteractiveStyle(cornerRadius: 8, selected: selection == tool.id,
                                                          padding: .init(top: 5, leading: 9, bottom: 5, trailing: 11)))
                        }
                    }
                }
                if let board { boardView(board) }
                else if let error { Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(.orange).font(.callout) }
                else if loading { ProgressView().controlSize(.small) }
            }
        }
        .onAppear { if selection == nil { select(installed.first?.id) } }
        .onChange(of: engine.catalog.map(\.id)) { _, _ in
            if selection == nil || !installed.contains(where: { $0.id == selection }) { select(installed.first?.id) }
        }
    }

    private func select(_ id: String?) {
        selection = id
        board = nil
        error = nil
        guard let id else { return }
        loading = true
        engine.loadDashboard(project: workspace, id: id) { result, failure in
            guard selection == id else { return }
            loading = false
            board = result
            error = failure
        }
    }

    @ViewBuilder
    private func boardView(_ d: ToolDashboard) -> some View {
        HStack(spacing: 14) {
            IntegrationIcon(id: d.id, size: 46)
            VStack(alignment: .leading, spacing: 3) {
                Text(d.title).font(.title3.weight(.semibold))
                HStack(spacing: 6) {
                    Circle().fill(d.healthy ? Color.green : Color.orange).frame(width: 7, height: 7)
                    Text("\(d.healthy ? "พร้อมใช้" : "ใช้งานไม่ได้") · v\(d.version)")
                        .font(.callout).foregroundStyle(.secondary)
                }
            }
            Spacer()
            ForEach(d.actions) { action in
                Button(action.title) { perform(action, tool: d.id) }
                    .buttonStyle(.borderedProminent).controlSize(.small)
                    .help(action.hint ?? "")
            }
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

        SettingsGroup(title: "การติดตั้ง") {
            SettingsRow(title: "ที่อยู่") {
                Text(d.location).font(.caption.monospaced()).foregroundStyle(.secondary)
                    .lineLimit(1).truncationMode(.middle).textSelection(.enabled)
            }
            if let bytes = d.disk_bytes {
                Divider()
                SettingsRow(title: "พื้นที่ที่ใช้") {
                    Text(ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file))
                        .foregroundStyle(.secondary)
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

    private func perform(_ action: DashboardAction, tool: String) {
        switch action.kind {
        case "url":
            if let target = action.target, let url = URL(string: target) { NSWorkspace.shared.open(url) }
        case "reveal":
            if let target = action.target { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: target)]) }
        default:
            engine.runCatalogAction(id: tool, key: action.key) { failure in error = failure }
        }
    }
}
