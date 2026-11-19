import AppKit
import SwiftUI

// The files tool: the project tree on the left (git state beside each file),
// the selected file on the right. Read-only; editing is the agent's or the
// person's editor's job.

extension Engine.FileNode {
    /// `List(_:children:)` wants nil for leaves.
    var kids: [Engine.FileNode]? { is_dir ? children : nil }
}

struct FilesTool: View {
    @EnvironmentObject private var engine: Engine
    let workspace: String
    @State private var tree: Engine.FileTreeResult?
    @State private var selected: String?
    @State private var query = ""
    @State private var loading = false

    var body: some View {
        HSplitView {
            VStack(spacing: 0) {
                TextField("ค้นหาไฟล์", text: $query)
                    .textFieldStyle(.roundedBorder)
                    .padding(8)
                list
            }
            .frame(minWidth: 240, idealWidth: 300, maxWidth: 420)
            Group {
                if let rel = selected, let node = find(rel), !node.is_dir {
                    FileViewer(root: workspace, node: node).id(rel)
                } else {
                    ContentUnavailableView("เลือกไฟล์ทางซ้าย", systemImage: "doc.text")
                }
            }
            .frame(minWidth: 320)
        }
        .onAppear(perform: load)
        .onChange(of: engine.workingDiff) { _, _ in load() }
    }

    @ViewBuilder
    private var list: some View {
        if let tree {
            if query.trimmingCharacters(in: .whitespaces).isEmpty {
                List(tree.nodes, children: \.kids, selection: $selected) { row($0) }
                    .listStyle(.sidebar)
            } else {
                let q = query.lowercased()
                List(flat(tree.nodes).filter { $0.rel_path.lowercased().contains(q) }.prefix(300).map { $0 },
                     selection: $selected) { row($0) }
                    .listStyle(.sidebar)
            }
        } else {
            VStack { Spacer(); ProgressView(); Spacer() }.frame(maxWidth: .infinity)
        }
    }

    private func row(_ n: Engine.FileNode) -> some View {
        HStack(spacing: 6) {
            Image(systemName: n.is_dir ? "folder" : "doc.text").foregroundStyle(.secondary)
            Text(n.name).lineLimit(1)
            Spacer(minLength: 0)
            if let g = n.git_status, !g.isEmpty {
                Text(g).font(.caption2.monospaced()).foregroundStyle(color(g))
            }
            if n.is_focus { Circle().fill(Color.accentColor).frame(width: 6, height: 6).help("agent กำลังดูไฟล์นี้") }
        }
    }

    private func color(_ g: String) -> Color {
        switch g.trimmingCharacters(in: .whitespaces).first {
        case "A", "?": .green
        case "D": .red
        default: .orange
        }
    }

    private func flat(_ nodes: [Engine.FileNode]) -> [Engine.FileNode] {
        nodes.flatMap { $0.is_dir ? flat($0.children) : [$0] }
    }

    private func find(_ rel: String) -> Engine.FileNode? {
        func go(_ nodes: [Engine.FileNode]) -> Engine.FileNode? {
            for n in nodes {
                if n.rel_path == rel { return n }
                if let hit = go(n.children) { return hit }
            }
            return nil
        }
        return tree.flatMap { go($0.nodes) }
    }

    private func load() {
        loading = true
        engine.fetchFileTree(cwd: workspace, focusFile: engine.agentFocusFile) { t in
            tree = t
            loading = false
        }
    }
}

private struct FileViewer: View {
    let root: String
    let node: Engine.FileNode
    @State private var text: String?
    @State private var note: String?
    private static let limit = 400_000

    private var url: URL { URL(fileURLWithPath: root).appendingPathComponent(node.rel_path) }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Label(node.rel_path, systemImage: "doc.text").font(.callout).lineLimit(1).truncationMode(.head)
                Spacer()
                Button("แสดงใน Finder") { NSWorkspace.shared.activateFileViewerSelecting([url]) }
                Button("เปิดด้วยแอปเริ่มต้น") { NSWorkspace.shared.open(url) }
            }
            .buttonStyle(.borderless)
            .padding(10)
            Divider()
            if let text {
                ScrollView([.vertical, .horizontal]) {
                    Text(text)
                        .font(.system(size: 12, design: .monospaced))
                        .textSelection(.enabled)
                        .padding(10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                if let note { Text(note).font(.caption).foregroundStyle(.secondary).padding(6) }
            } else if let note {
                ContentUnavailableView(note, systemImage: "doc.questionmark")
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .task { await read() }
    }

    private func read() async {
        let target = url
        let result: (String?, String?) = await Task.detached(priority: .userInitiated) {
            guard let handle = try? FileHandle(forReadingFrom: target) else { return (nil, "เปิดไฟล์ไม่ได้") }
            defer { try? handle.close() }
            let data = (try? handle.read(upToCount: FileViewer.limit + 1)) ?? Data()
            guard let s = String(data: data.prefix(FileViewer.limit), encoding: .utf8) else {
                return (nil, "ไฟล์นี้ไม่ใช่ข้อความ เปิดด้วยแอปเริ่มต้นแทน")
            }
            return (s, data.count > FileViewer.limit ? "แสดงแค่ส่วนต้นของไฟล์" : nil)
        }.value
        text = result.0
        note = result.1
    }
}
