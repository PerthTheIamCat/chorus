import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

// ContentView, split out of ContentView.swift: panel layout: sidebar, explorer, viewer, chat and inspector.
// Moved verbatim; only `private` was dropped so the pieces can see each other.

extension ContentView {
    enum RightTab: String, CaseIterable {
        case context = "คอนเท็กซ์", changes = "ที่แก้ไป", memory = "ความจำ", checkpoints = "จุดบันทึก"

        var icon: String {
            switch self {
            case .context: "gauge.with.dots.needle.50percent"
            case .changes: "plusminus"
            case .memory: "brain.head.profile"
            case .checkpoints: "clock.arrow.circlepath"
            }
        }

        var tooltip: String {
            switch self {
            case .context: "ตรวจสอบบริบท Token และ Repo Map"
            case .changes: "ตรวจสิ่งที่แก้"
            case .memory: "ความจำและข้อเท็จจริงของโปรเจกต์"
            case .checkpoints: "Time-Travel Checkpoints & ย้อนเวลา"
            }
        }
    }

    func badge(_ t: RightTab) -> Int {
        switch t {
        case .context: return 0
        case .changes: return engine.diffFiles.count
        case .memory: return engine.pending.count
        case .checkpoints: return 0
        }
    }

    /// Icon tabs with counts, so the panel says where something needs you
    /// (files changed, proposals waiting) before you open the tab.
    var inspectorTabs: some View {
        HStack(spacing: 2) {
            ForEach(Array(RightTab.allCases.enumerated()), id: \.element) { index, t in
                let n = badge(t)
                Button { rightTab = t } label: {
                    VStack(spacing: 3) {
                        ZStack(alignment: .topTrailing) {
                            Image(systemName: t.icon).font(.system(size: 14, weight: .medium))
                                .frame(width: 26, height: 18)
                            if n > 0 {
                                Text(n > 99 ? "99+" : "\(n)").font(.system(size: 8, weight: .bold)).foregroundStyle(.white)
                                    .padding(.horizontal, 3).padding(.vertical, 1)
                                    .background(t == .memory ? Color.orange : Color.accentColor, in: Capsule())
                                    .offset(x: 10, y: -6)
                            }
                        }
                        Text(t.rawValue).font(.caption2)
                    }
                    .frame(maxWidth: .infinity)
                    .foregroundStyle(rightTab == t ? Color.accentColor : .secondary)
                }
                .buttonStyle(InteractiveStyle(cornerRadius: 9, selected: rightTab == t,
                                              padding: .init(top: 6, leading: 2, bottom: 5, trailing: 2)))
                .help("\(t.tooltip) (⌘⌥\(index + 1))")
                .accessibilityLabel(t.rawValue)
            }
        }
        .padding(8)
    }

    // MARK: - Component Views for Panels

    @ViewBuilder
    var sidebarPanel: some View {
        sidebar
            .frame(width: sidebarWidth)
            .floatingPanel(cornerRadius: 20)
            .transition(.asymmetric(
                insertion: .move(edge: .leading).combined(with: .opacity).combined(with: .scale(scale: 0.98)),
                removal: .move(edge: .leading).combined(with: .opacity)
            ))
    }

    @ViewBuilder
    func explorerPanel(isRail: Bool = false) -> some View {
        WorkspaceExplorerView(
            engine: engine,
            workspace: workspace,
            isMaximized: maximizedPanel == .explorer,
            isRailMode: isRail,
            onToggleMaximize: {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                    maximizedPanel = (maximizedPanel == .explorer) ? .none : .explorer
                }
            },
            onSelectFile: { file in
                selectedFile = file
            },
            onSelectWorktree: { _ in }
        )
        .frame(width: maximizedPanel == .explorer ? nil : (isRail ? 44 : explorerWidth))
        .floatingPanel(cornerRadius: 20)
        .transition(.asymmetric(
            insertion: .move(edge: .leading).combined(with: .opacity).combined(with: .scale(scale: 0.98)),
            removal: .move(edge: .leading).combined(with: .opacity)
        ))
    }

    var inspectorShownWidth: Double {
        rightTab == .changes ? max(inspectorWidth, 420) : inspectorWidth
    }

    @ViewBuilder
    var inspectorPanel: some View {
        VStack(spacing: 0) {
            inspectorTabs
            Divider().opacity(0.5)
            switch rightTab {
            case .context: contextPane
            case .changes:
                if diffReviewMode {
                    DiffReviewerView(
                        engine: engine,
                        workspace: workspace,
                        isMaximized: maximizedPanel == .diffReviewer,
                        onToggleMaximize: {
                            withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                                maximizedPanel = (maximizedPanel == .diffReviewer) ? .none : .diffReviewer
                            }
                        },
                        onSendSteeringPrompt: { prompt in
                            draftBox.text = prompt
                        }
                    )
                } else {
                    changesPane
                }
            case .memory: memoryPane
            case .checkpoints:
                CheckpointTimelineView(
                    engine: engine,
                    dbPath: dbPath,
                    workspace: workspace
                )
            }
        }
        .frame(width: maximizedPanel == .diffReviewer ? nil : inspectorShownWidth)
        .floatingPanel(cornerRadius: 20)
        .transition(.asymmetric(
            insertion: .move(edge: inspectorPosition == "leading" ? .leading : .trailing).combined(with: .opacity).combined(with: .scale(scale: 0.98)),
            removal: .move(edge: inspectorPosition == "leading" ? .leading : .trailing).combined(with: .opacity)
        ))
    }

    @ViewBuilder
    func codeViewerPanel(for file: String, isCenterStage: Bool = false, width: CGFloat? = nil) -> some View {
        CodeViewerView(
            filePath: file,
            cwd: workspace,
            isMaximized: maximizedPanel == .codeViewer,
            onToggleMaximize: {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                    maximizedPanel = (maximizedPanel == .codeViewer) ? .none : .codeViewer
                }
            },
            onClose: {
                if maximizedPanel == .codeViewer { maximizedPanel = .none }
                selectedFile = nil
            }
        )
        .frame(width: (maximizedPanel == .codeViewer || isCenterStage) ? nil : width)
        .frame(maxWidth: isCenterStage ? .infinity : nil)
        .floatingPanel(cornerRadius: 20)
        .transition(.asymmetric(
            insertion: .opacity.combined(with: .scale(scale: 0.98)),
            removal: .opacity
        ))
    }

    @ViewBuilder
    var maximizedFloatingBanner: some View {
        HStack(spacing: 8) {
            Image(systemName: "arrow.down.right.and.arrow.up.left")
                .font(.system(size: 11, weight: .bold))
                .foregroundColor(.accentColor)
            Text(maximizedPanelTitle)
                .font(.system(size: 12, weight: .semibold))
            Button("ย่อขนาดกลับ (⇧⌘M)") {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                    maximizedPanel = .none
                }
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.small)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 7)
        .background(.ultraThinMaterial, in: Capsule())
        .overlay(Capsule().stroke(Color.primary.opacity(0.12), lineWidth: 1))
        .shadow(color: .black.opacity(0.14), radius: 10, y: 3)
        .transition(.move(edge: .top).combined(with: .opacity))
    }

    @ViewBuilder
    var breadcrumbHeader: some View {
        let currentSub = engine.subagentsByDB[dbPath(for: workspace)]?.first { $0.child_sess == engine.sessionID }
        let currentSess = (engine.sessionsByDB[dbPath(for: workspace)] ?? []).first { $0.id == engine.sessionID }
        let parentId = currentSub?.parent_sess ?? currentSess?.forked_from
        if let parentId, let parent = (engine.sessionsByDB[dbPath(for: workspace)] ?? []).first(where: { $0.id == parentId }) {
            HStack(spacing: 6) {
                Button {
                    workspace = parent.cwd.isEmpty ? workspace : parent.cwd
                    engine.reopen(db: dbPath(for: workspace), sessionID: parent.id)
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "chevron.left")
                            .font(.system(size: 10, weight: .bold))
                        Text(displayTitle(parent))
                            .lineLimit(1)
                    }
                }
                .buttonStyle(.chorusPlain)
                .foregroundStyle(Color.accentColor)

                Image(systemName: "chevron.right")
                    .font(.system(size: 9, weight: .bold))
                    .foregroundStyle(.tertiary)

                Text(currentSub.map { "\($0.role) · \($0.task)" } ?? displayTitle(currentSess ?? parent))
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)

                Spacer()

                if let sub = currentSub {
                    if sub.kind == "provider" {
                        Text("Provider Subagent (อ่านอย่างเดียว)")
                            .font(.system(size: 9.5, weight: .semibold))
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 6).padding(.vertical, 2)
                            .background(Color.primary.opacity(0.06), in: Capsule())
                    } else if sub.status == "waiting" {
                        Text("รอคำยืนยัน")
                            .font(.system(size: 9.5, weight: .bold))
                            .foregroundStyle(.white)
                            .padding(.horizontal, 6).padding(.vertical, 2)
                            .background(Color.orange, in: Capsule())
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 6)
            .background(Color.primary.opacity(0.03))
            Divider().opacity(0.3)
        }
    }

    @ViewBuilder
    var centerStageView: some View {
        VStack(spacing: 0) {
            breadcrumbHeader
            switch centerStageMode {
            case .thread:
                thread
            case .livePreview:
                LivePreviewView(
                    currentURLString: $previewURLString,
                    onClose: {
                        withAnimation(.spring(response: 0.28, dampingFraction: 0.85)) {
                            centerStageMode = .thread
                        }
                    }
                )
                .floatingPanel(cornerRadius: 20)
            }

            if showTerminalDrawer {
                TerminalDrawerView(
                    workspace: workspace,
                    onClose: {
                        withAnimation(.spring(response: 0.28, dampingFraction: 0.85)) {
                            showTerminalDrawer = false
                        }
                    }
                )
                .floatingPanel(cornerRadius: 20)
                .padding(.top, WorkbenchLayout.gripWidth)
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
    }

    /// Viewer (opens next to the tree, only while a file is open) and chat
    /// (min 380 px), sharing `space` about 1.2 : 1. When both can't keep
    /// their minimum, the file takes chat's place as in narrow windows.
    @ViewBuilder
    func viewerAndChat(file: String, space: CGFloat) -> some View {
        let gap = WorkbenchLayout.columnSpacing
        let shared = space - WorkbenchLayout.gripWidth - gap * 2
        let chatMin = WorkbenchLayout.chatMinWidth
        let viewerMin = WorkbenchLayout.viewerMinWidth
        if shared >= chatMin + viewerMin {
            let viewerWidth = min(max(shared * codeViewerShare, viewerMin), shared - chatMin)
            HStack(spacing: gap) {
                codeViewerPanel(for: file, isCenterStage: false, width: viewerWidth)
                PanelResizeGrip(
                    width: Binding(
                        get: { Double(viewerWidth) },
                        set: { codeViewerShare = $0 / Double(shared) }
                    ),
                    minWidth: Double(viewerMin), maxWidth: Double(shared - chatMin), direction: .forward
                ) {
                    codeViewerShare = WorkbenchLayout.defaultViewerShare
                }
                centerStageView
                    .frame(minWidth: chatMin)
            }
        } else {
            codeViewerPanel(for: file, isCenterStage: true)
        }
    }

    @ViewBuilder
    func multiPanelLayout(containerWidth: CGFloat) -> some View {
        let isNarrow = containerWidth < 1200
        HStack(spacing: WorkbenchLayout.columnSpacing) {
            // 1. Sidebar (Sessions: 230–260 px)
            if showSidebar {
                sidebarPanel
                PanelResizeGrip(width: $sidebarWidth, minWidth: 180, maxWidth: 420, direction: .forward) {
                    sidebarWidth = 260
                }
            }

            // 2. File tree / Explorer (210–240 px, or 44 px icon rail when < 1200 px)
            if showExplorer {
                explorerPanel(isRail: isNarrow)
                if !isNarrow {
                    PanelResizeGrip(width: $explorerWidth, minWidth: 200, maxWidth: 480, direction: .forward) {
                        explorerWidth = 240
                    }
                } else {
                    Color.clear.frame(width: WorkbenchLayout.gripWidth)   // rail has no grip, keep the gutter
                }
            }

            // 3–4. File viewer + chat share whatever the fixed columns leave,
            // measured rather than summed so panel padding can't skew it.
            if !isNarrow, let file = selectedFile {
                GeometryReader { g in
                    viewerAndChat(file: file, space: g.size.width)
                        .frame(width: g.size.width, height: g.size.height)
                }
                .frame(minWidth: WorkbenchLayout.chatMinWidth)
            } else if let file = selectedFile {
                // Narrow window: an open file temporarily replaces chat.
                codeViewerPanel(for: file, isCenterStage: true)
            } else {
                centerStageView
                    .frame(minWidth: WorkbenchLayout.chatMinWidth)
            }

            // 5. Inspector (stays a toggled right panel ⌥⌘I)
            if showInspector {
                // The Changes tab renders at least 420 pt wide; the grip must start
                // from what is on screen or the first drag does nothing.
                PanelResizeGrip(
                    width: Binding(get: { inspectorShownWidth }, set: { inspectorWidth = $0 }),
                    minWidth: rightTab == .changes ? 420 : 260, maxWidth: 700, direction: .reversed
                ) {
                    inspectorWidth = 330
                }
                inspectorPanel
            }
        }
    }

    @ViewBuilder
    func maximizedLayout(containerWidth: CGFloat) -> some View {
        switch maximizedPanel {
        case .codeViewer:
            if let file = selectedFile {
                codeViewerPanel(for: file, isCenterStage: true)
            } else {
                centerStageView
            }
        case .diffReviewer:
            inspectorPanel
        case .explorer:
            explorerPanel(isRail: false)
        case .thread:
            thread
        case .orchestrator:
            centerStageView
        case .livePreview:
            LivePreviewView(
                currentURLString: $previewURLString,
                onClose: {
                    withAnimation(.spring(response: 0.28, dampingFraction: 0.85)) {
                        maximizedPanel = .none
                    }
                }
            )
            .floatingPanel(cornerRadius: 20)
        case .none:
            multiPanelLayout(containerWidth: containerWidth)
        }
    }
}
