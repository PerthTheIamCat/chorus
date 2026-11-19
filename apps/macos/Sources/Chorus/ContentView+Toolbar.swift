import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

// ContentView, split out of ContentView.swift: window toolbar.
// Moved verbatim; only `private` was dropped so the pieces can see each other.

extension ContentView {
    @ToolbarContentBuilder
    var mainToolbar: some ToolbarContent {
        // ToolbarContentBuilder takes at most 10 items per block; there are 12.
        navigationToolbar
        actionToolbar
    }

    @ToolbarContentBuilder
    var navigationToolbar: some ToolbarContent {
        ToolbarItem(placement: .navigation) {
            Button { showSidebar.toggle() } label: {
                Label("แถบโปรเจกต์", systemImage: "sidebar.left")
            }
            .help("เปิด/ปิดแถบโปรเจกต์ (⌃⌘S)")
        }
        ToolbarItem(placement: .navigation) {
            HStack(spacing: 8) {
                let title = engine.sessionID < 0 ? (engine.draft != nil ? "session ใหม่" : "ยังไม่ได้เปิด session")
                     : (engine.sessionsByDB[dbPath]?.first { $0.id == engine.sessionID }).map(displayTitle)
                        ?? "session \(engine.sessionID)"
                // Capped: a long AI-written title pushed every toolbar item
                // off to the overflow menu. The full title is on hover.
                Text(title)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .font(.headline)
                    .frame(maxWidth: 280, alignment: .leading)
                    .help(title)

                if engine.sessionID >= 0 {
                    let isRenaming = engine.renamingSessionIDs.contains(engine.sessionID)
                    if isRenaming {
                        ProgressView()
                            .controlSize(.mini)
                            .help("กำลังตั้งชื่อ session ด้วย AI…")
                    } else {
                        Button {
                            if let s = engine.sessionsByDB[dbPath]?.first(where: { $0.id == engine.sessionID }) {
                                engine.renameSessionWithAI(db: dbPath, id: s.id, sampleText: displayTitle(s))
                            }
                        } label: {
                            Image(systemName: "sparkles")
                                .font(.system(size: 11))
                                .foregroundColor(.accentColor)
                        }
                        .buttonStyle(.chorusPlain)
                        .help("ให้ AI ตั้งชื่อ session")
                    }
                }
            }
        }
        // Push everything after this to the window's trailing edge.
        if #available(macOS 26, *) {
            ToolbarSpacer(.flexible)
        }
    }

    /// Four things at the trailing edge, nothing else: hand over, search,
    /// "more" (everything occasional) and the side panel. Files, terminal,
    /// preview, focus mode and Orchestra used to be one button each.
    @ToolbarContentBuilder
    var actionToolbar: some ToolbarContent {
        ToolbarItemGroup(placement: .primaryAction) {
            // Handing over is one click per agent. Everything the next
            // agent needs is already in the ledger, so asking the user to
            // fill in a form would be asking them to retype what we know.
            let targets = engine.agents.filter { $0.ready && $0.id != currentAgentID }
            if engine.sessionID >= 0 && !targets.isEmpty {
                Menu {
                    ForEach(targets) { a in
                        Button {
                            engine.handoffAuto(to: a.id, level: max(permission.rawValue, 0))
                        } label: {
                            Label("ส่งต่อให้ \(a.label)", systemImage: "arrowshape.turn.up.right")
                        }
                    }
                } label: {
                    Label("ส่งต่อ", systemImage: "arrowshape.turn.up.right")
                }
                .disabled(engine.busy)
                .help("ส่งงานต่อให้ agent อื่น พร้อมเป้าหมาย สิ่งที่ทำไปแล้ว และ diff (ไม่ต้องพิมพ์อะไร)")
            }
        }
        ToolbarItem(placement: .primaryAction) {
            Button { showSearchSheet = true } label: {
                Label("ค้นหา", systemImage: "magnifyingglass")
            }
            .help("ค้นหาบทสนทนาข้าม session (⇧⌘F)")
        }
        ToolbarItem(placement: .primaryAction) {
            Menu {
                Button {
                    showExplorer.toggle()
                } label: {
                    Label(showExplorer ? "ซ่อนไฟล์" : "แสดงไฟล์", systemImage: "folder")
                }
                Button {
                    withAnimation(.spring(response: 0.28, dampingFraction: 0.85)) { showTerminalDrawer.toggle() }
                } label: {
                    Label(showTerminalDrawer ? "ปิดเทอร์มินัล" : "เทอร์มินัล", systemImage: "terminal")
                }
                Button {
                    withAnimation(.spring(response: 0.28, dampingFraction: 0.85)) {
                        centerStageMode = (centerStageMode == .livePreview) ? .thread : .livePreview
                    }
                } label: {
                    Label(centerStageMode == .livePreview ? "กลับสู่บทสนทนา" : "พรีวิวเว็บ", systemImage: "globe")
                }
                Divider()
                Button {
                    withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                        maximizedPanel = (maximizedPanel == .thread) ? .none : .thread
                    }
                } label: {
                    Label(maximizedPanel == .thread ? "ออกจากโหมดโฟกัส" : "โหมดโฟกัส",
                          systemImage: "viewfinder")
                }
                Button {
                    settingsSection = .orchestration
                    showSettings = true
                } label: {
                    Label("Orchestra และบทบาท", systemImage: "point.3.connected.trianglepath.dotted")
                }
                if powerManager.shouldThrottleConcurrency {
                    Divider()
                    Text("เครื่องร้อน (\(powerManager.thermalStateName)) — ระบบลดงานขนานให้อัตโนมัติ")
                }
            } label: {
                Label("เพิ่มเติม", systemImage: "ellipsis.circle")
            }
            .menuIndicator(.hidden)
            .help("ไฟล์ เทอร์มินัล พรีวิว โหมดโฟกัส Orchestra")
        }
        // Declared last so it sits at the far right, over the panel it opens.
        // A dot says something is waiting inside (changed files, proposals).
        ToolbarItem(placement: .primaryAction) {
            Button { showInspector.toggle() } label: {
                Label("แผงข้อมูล", systemImage: "sidebar.right")
                    .overlay(alignment: .topTrailing) {
                        if !showInspector && (engine.diffFiles.count + engine.pending.count) > 0 {
                            Circle().fill(Color.accentColor).frame(width: 7, height: 7).offset(x: 3, y: -2)
                        }
                    }
            }
            .help("เปิด/ปิดแผงคอนเท็กซ์ ที่แก้ไป ความจำ (⌘⌥I)")
        }
    }
}
