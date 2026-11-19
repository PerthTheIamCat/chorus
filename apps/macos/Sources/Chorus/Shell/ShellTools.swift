import SwiftUI

// The tools that are not part of every conversation: commit and PR, terminal,
// files, search, checkpoints, orchestra, system monitor. They open as sheets
// from one toolbar menu so the main window stays three columns. The views
// inside are the existing ones (they take plain parameters); only the way to
// reach them is new.

enum ShellTool: String, Identifiable {
    case ship, terminal, files, search, checkpoints, orchestra, monitor, preview
    var id: String { rawValue }

    var title: String {
        switch self {
        case .ship: "Commit และ PR"
        case .terminal: "เทอร์มินัล"
        case .files: "ไฟล์"
        case .search: "ค้นแชท"
        case .checkpoints: "Checkpoint"
        case .orchestra: "Orchestra"
        case .monitor: "System Monitor"
        case .preview: "ตัวอย่างสด"
        }
    }

    var symbol: String {
        switch self {
        case .ship: "arrow.up.doc"
        case .terminal: "terminal"
        case .files: "folder"
        case .search: "magnifyingglass"
        case .checkpoints: "clock.arrow.circlepath"
        case .orchestra: "point.3.connected.trianglepath.dotted"
        case .monitor: "gauge.with.dots.needle.50percent"
        case .preview: "safari"
        }
    }
}

struct ToolSheet: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @Environment(\.dismiss) private var dismiss
    let tool: ShellTool
    @State private var openFile: String?

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Label(tool.title, systemImage: tool.symbol).font(.headline)
                Spacer()
                Button("ปิด") { dismiss() }.keyboardShortcut(.cancelAction)
            }
            .padding(12)
            Divider()
            content
        }
        .frame(minWidth: 760, idealWidth: 900, minHeight: 520, idealHeight: 640)
    }

    @ViewBuilder
    private var content: some View {
        let ws = shell.workspace
        switch tool {
        case .ship:
            GitShipSheet(workspace: ws, sessionFiles: engine.diffFiles, goal: "") { dismiss() }
        case .terminal:
            TerminalDrawerView(workspace: ws) { dismiss() }
        case .files:
            HSplitView {
                WorkspaceExplorerView(engine: engine, workspace: ws) { openFile = $0 }
                    .frame(minWidth: 240, idealWidth: 300)
                if let f = openFile {
                    CodeViewerView(filePath: f, cwd: ws) { openFile = nil }
                        .id(f)
                } else {
                    ContentUnavailableView("เลือกไฟล์ทางซ้าย", systemImage: "doc.text")
                }
            }
        case .search:
            ChatSearchView(engine: engine, dbPath: shell.currentDB, onSelectSession: { id in
                shell.open(SessionKey(folder: ws, id: id))
                dismiss()
            }, onClose: { dismiss() })
        case .checkpoints:
            CheckpointTimelineView(engine: engine, dbPath: shell.currentDB, workspace: ws) { dismiss() }
        case .orchestra:
            OrchestratorView(engine: engine, workspace: ws)
        case .monitor:
            SystemMonitorView()
        case .preview:
            ShellPreview()
        }
    }
}
