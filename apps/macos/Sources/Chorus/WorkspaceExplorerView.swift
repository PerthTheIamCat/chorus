import AppKit
import SwiftUI

/// Workspace Explorer (Column 2, toggled via ⌘B).
/// Virtualized file tree with Git status badges, live agent focus ring,
/// and active multi-agent worktrees.
struct WorkspaceExplorerView: View {
    @ObservedObject var engine: Engine
    let workspace: String
    var isMaximized: Bool = false
    var isRailMode: Bool = false
    var onToggleMaximize: (() -> Void)? = nil
    var onSelectFile: (String) -> Void
    var onSelectWorktree: ((Engine.WorktreeInfo) -> Void)? = nil

    @State private var filterText = ""
    @State private var fileTree: Engine.FileTreeResult?
    @State private var expandedPaths: Set<String> = []
    /// Workspace whose top level has already been auto-expanded. Seeding runs
    /// once per folder, so a refresh (every 15 s, on diff, on activation)
    /// never re-opens a folder the user collapsed.
    @State private var seededRoot: String?
    @State private var isLoading = false

    var body: some View {
        VStack(spacing: 0) {
            if isRailMode {
                // Icon rail view when window is < 1200 px
                VStack(spacing: 12) {
                    Button(action: refreshTree) {
                        Image(systemName: "folder.fill")
                            .font(.system(size: 14))
                            .foregroundColor(.secondary)
                    }
                    .buttonStyle(.plain)
                    .help("โครงสร้างไฟล์ (Rail Mode)")
                    .padding(.top, 10)

                    Divider()

                    ScrollView {
                        VStack(spacing: 8) {
                            if let tree = fileTree {
                                ForEach(tree.nodes.prefix(30), id: \.id) { node in
                                    Button(action: {
                                        if !node.is_dir { onSelectFile(node.rel_path) }
                                    }) {
                                        ZStack(alignment: .topTrailing) {
                                            FileTypeIcon(key: node.icon, size: 16)
                                            if node.is_focus {
                                                Circle()
                                                    .fill(Color.blue)
                                                    .frame(width: 5, height: 5)
                                                    .offset(x: 3, y: -2)
                                            }
                                        }
                                        .frame(width: 28, height: 28)
                                        .background(node.is_focus ? Color.blue.opacity(0.12) : Color.clear)
                                        .cornerRadius(6)
                                    }
                                    .buttonStyle(.plain)
                                    .help(node.name)
                                }
                            }
                        }
                        .padding(.vertical, 4)
                    }
                    .chorusScrollIndicators()
                }
                .frame(width: 44)
            } else {
                // Header & Filter field
                VStack(spacing: 8) {
                    HStack {
                        Image(systemName: "folder.fill")
                            .foregroundColor(.secondary)
                        Text("FILES")
                            .font(.system(size: 11, weight: .bold))
                            .foregroundColor(.secondary)
                        Spacer()
                        Button(action: refreshTree) {
                            Image(systemName: "arrow.clockwise")
                                .font(.system(size: 11))
                        }
                        .buttonStyle(.plain)
                        .help("Refresh explorer")

                        if let onToggleMaximize {
                            PanelMaximizeButton(isMaximized: isMaximized, onToggle: onToggleMaximize)
                        }
                    }
                    .padding(.horizontal, 12)
                    .padding(.top, 10)

                    HStack(spacing: 6) {
                        Image(systemName: "magnifyingglass")
                            .foregroundColor(.secondary)
                            .font(.system(size: 11))
                        TextField("Filter files... (⌘P)", text: $filterText)
                            .textFieldStyle(.plain)
                            .font(.system(size: 12))
                        if !filterText.isEmpty {
                            Button(action: { filterText = "" }) {
                                Image(systemName: "xmark.circle.fill")
                                    .foregroundColor(.secondary)
                                    .font(.system(size: 11))
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 5)
                    .background(Color(NSColor.controlBackgroundColor).opacity(0.6))
                    .cornerRadius(6)
                    .padding(.horizontal, 10)
                }
                .padding(.bottom, 8)

                Divider()

                if workspace.isEmpty {
                    VStack(spacing: 8) {
                        Spacer()
                        Image(systemName: "folder.badge.questionmark")
                            .font(.system(size: 28))
                            .foregroundColor(.secondary.opacity(0.6))
                        Text("ยังไม่ได้เลือกโปรเจกต์")
                            .font(.system(size: 12, weight: .medium))
                        Text("เลือกโฟลเดอร์จากแถบด้านซ้าย\nเพื่อดูโครงสร้างไฟล์")
                            .font(.caption2)
                            .foregroundColor(.secondary)
                            .multilineTextAlignment(.center)
                            .padding(.horizontal, 16)
                        Spacer()
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    // Tree Content
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 1) {
                            if let tree = fileTree {
                                ForEach(filteredNodes(tree.nodes), id: \.id) { node in
                                    FileTreeNodeRow(
                                        node: node,
                                        depth: 0,
                                        expandedPaths: $expandedPaths,
                                        onSelectFile: onSelectFile
                                    )
                                }
                            } else if isLoading {
                                HStack {
                                    Spacer()
                                    ProgressView()
                                        .scaleEffect(0.7)
                                    Spacer()
                                }
                                .padding(.top, 20)
                            } else {
                                Text("ไม่มีไฟล์ในโฟลเดอร์นี้")
                                    .font(.system(size: 11))
                                    .foregroundColor(.secondary)
                                    .padding(.horizontal, 12)
                                    .padding(.top, 10)
                            }
                        }
                        .padding(.vertical, 4)
                    }
                    .chorusScrollIndicators()
                }
            }
        }
        .onAppear {
            refreshTree()
        }
        .onChange(of: workspace) { _, _ in
            refreshTree()
        }
        .onChange(of: engine.agentFocusFile) { _, _ in
            refreshTree()
        }
        .onChange(of: engine.workingDiff) { _, _ in
            debouncedRefreshTree()
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            debouncedRefreshTree()
        }
        // A safety net only: turns (workingDiff) and coming back to the app
        // already refresh. Each pass is `git status -uall` plus a stat of every
        // file to depth 6, so a 15 s poll that also ran in the background kept
        // a big monorepo's machine warm doing nothing.
        .onReceive(Timer.publish(every: 60, on: .main, in: .common).autoconnect()) { _ in
            if NSApp.isActive && !workspace.isEmpty && !isLoading {
                refreshTree()
            }
        }
    }

    @State private var refreshTask: Task<Void, Never>? = nil

    private func debouncedRefreshTree() {
        refreshTask?.cancel()
        refreshTask = Task {
            try? await Task.sleep(nanoseconds: 1_200_000_000)
            if !Task.isCancelled {
                await MainActor.run { refreshTree() }
            }
        }
    }

    private func refreshTree() {
        guard !workspace.isEmpty else { return }
        isLoading = true
        let root = workspace
        engine.fetchFileTree(cwd: root, focusFile: engine.agentFocusFile) { tree in
            self.fileTree = tree
            self.isLoading = false
            if let rootNodes = tree?.nodes,
               let seeded = Self.seedExpansion(rootNodes, root: root, seededRoot: self.seededRoot) {
                self.seededRoot = root
                self.expandedPaths = seeded
            }
        }
    }

    /// Top-level folders open on the first load of a folder, and never again:
    /// returns `nil` (leave the user's expansion alone) once `root` is seeded.
    static func seedExpansion(_ nodes: [Engine.FileNode], root: String, seededRoot: String?) -> Set<String>? {
        guard seededRoot != root else { return nil }
        return Set(nodes.filter(\.is_dir).map(\.rel_path))
    }

    private func filteredNodes(_ nodes: [Engine.FileNode]) -> [Engine.FileNode] {
        if filterText.isEmpty { return nodes }
        let q = filterText.lowercased()
        return nodes.compactMap { filterNode($0, query: q) }
    }

    private func filterNode(_ node: Engine.FileNode, query: String) -> Engine.FileNode? {
        if node.name.lowercased().contains(query) {
            return node
        }
        if node.is_dir {
            let filteredChildren = node.children.compactMap { filterNode($0, query: query) }
            if !filteredChildren.isEmpty {
                return Engine.FileNode(
                    name: node.name,
                    rel_path: node.rel_path,
                    is_dir: true,
                    icon: nil,
                    size: node.size,
                    git_status: node.git_status,
                    is_focus: node.is_focus,
                    children: filteredChildren
                )
            }
        }
        return nil
    }
}

private struct FileTreeNodeRow: View {
    let node: Engine.FileNode
    let depth: Int
    @Binding var expandedPaths: Set<String>
    var onSelectFile: (String) -> Void

    var isExpanded: Bool {
        expandedPaths.contains(node.rel_path)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button(action: handleClick) {
                HStack(spacing: 4) {
                    // Indentation
                    Spacer()
                        .frame(width: CGFloat(depth * 14))

                    if node.is_dir {
                        Image(systemName: isExpanded ? "chevron.down" : "chevron.right")
                            .font(.system(size: 9))
                            .foregroundColor(.secondary)
                            .frame(width: 12)
                        Image(systemName: isExpanded ? "folder.fill" : "folder")
                            .font(.system(size: 12))
                            .foregroundColor(.secondary)
                    } else {
                        Spacer().frame(width: 12)
                        FileTypeIcon(key: node.icon)
                    }

                    Text(node.name)
                        .font(.system(size: 11, design: .monospaced))
                        .lineLimit(1)
                        .foregroundColor(node.is_focus ? .blue : .primary)

                    Spacer()

                    // Agent Focus Ring
                    if node.is_focus {
                        Circle()
                            .fill(Color.blue)
                            .frame(width: 6, height: 6)
                            .help("Agent is actively reading or modifying this file")
                    }

                    // Git Status Badge
                    if let status = node.git_status, !status.isEmpty {
                        Text(status)
                            .font(.system(size: 9, weight: .bold, design: .monospaced))
                            .foregroundColor(gitColor(status))
                            .padding(.horizontal, 3)
                            .padding(.vertical, 1)
                            .background(gitColor(status).opacity(0.15))
                            .cornerRadius(3)
                    }
                }
                .padding(.vertical, 3)
                .padding(.horizontal, 8)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if node.is_dir && isExpanded {
                ForEach(node.children, id: \.id) { child in
                    FileTreeNodeRow(
                        node: child,
                        depth: depth + 1,
                        expandedPaths: $expandedPaths,
                        onSelectFile: onSelectFile
                    )
                }
            }
        }
    }

    private func handleClick() {
        if node.is_dir {
            if isExpanded {
                expandedPaths.remove(node.rel_path)
            } else {
                expandedPaths.insert(node.rel_path)
            }
        } else {
            onSelectFile(node.rel_path)
        }
    }

    private func gitColor(_ status: String) -> Color {
        if status.contains("M") { return .orange }
        if status.contains("A") || status.contains("?") { return .green }
        if status.contains("D") { return .red }
        return .secondary
    }
}
