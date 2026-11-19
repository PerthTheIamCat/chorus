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
            Button { showExplorer.toggle() } label: {
                Label("ไฟล์", systemImage: "folder")
            }
            .help("เปิด/ปิดแถบไฟล์ (⌘B)")
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

    @ToolbarContentBuilder
    var actionToolbar: some ToolbarContent {
        ToolbarItemGroup(placement: .primaryAction) {
            // Handing over is one click per agent. Everything the next
            // agent needs is already in the ledger, so asking the user to
            // fill in a form would be asking them to retype what we know.
            // One menu, not a button per agent: with three agents ready the
            // row of "ส่งต่อ X" labels crowded the toolbar and read as text.
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
            Button {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                    maximizedPanel = (maximizedPanel == .thread) ? .none : .thread
                }
            } label: {
                Label(maximizedPanel == .thread ? "ออกจากโหมดโฟกัส" : "โหมดโฟกัส",
                      systemImage: maximizedPanel == .thread ? "arrow.down.right.and.arrow.up.left" : "viewfinder")
            }
            .help(maximizedPanel == .thread ? "ออกจากโหมดโฟกัส (⇧⌘M)" : "ขยายบทสนทนาเต็มจอ / โหมดโฟกัส (⇧⌘M)")
        }
        ToolbarItem(placement: .primaryAction) {
            if powerManager.shouldThrottleConcurrency {
                HStack(spacing: 4) {
                    Image(systemName: "flame.fill")
                        .font(.system(size: 11))
                        .foregroundColor(.orange)
                    Text(powerManager.thermalStateName)
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(.secondary)
                }
                .padding(.horizontal, 6)
                .padding(.vertical, 3)
                .background(Color.orange.opacity(0.12), in: Capsule())
                .help("Apple Silicon Thermal Governor: ระบบควบคุมความร้อนกำลังปรับการทำงานให้เหมาะสม")
            }
        }
        ToolbarItem(placement: .primaryAction) {
            Button {
                settingsSection = .orchestration
                showSettings = true
            } label: {
                Label("Orchestra & Roles", systemImage: "point.3.connected.trianglepath.dotted")
            }
            .help("เปิดการตั้งค่า Orchestra และ Role Library (⌘⌥O)")
        }
        ToolbarItem(placement: .primaryAction) {
            Button {
                withAnimation(.spring(response: 0.28, dampingFraction: 0.85)) {
                    centerStageMode = (centerStageMode == .livePreview) ? .thread : .livePreview
                }
            } label: {
                Label(centerStageMode == .livePreview ? "กลับสู่บทสนทนา" : "Live Preview",
                      systemImage: centerStageMode == .livePreview ? "globe.badge.chevron.backward" : "globe")
                    .foregroundStyle(centerStageMode == .livePreview ? Color.accentColor : Color.primary)
            }
            .help(centerStageMode == .livePreview ? "กลับสู่บทสนทนา (⌘⌥P)" : "เปิด Web Live Preview (⌘⌥P)")
        }
        ToolbarItem(placement: .primaryAction) {
            Button {
                withAnimation(.spring(response: 0.28, dampingFraction: 0.85)) {
                    showTerminalDrawer.toggle()
                }
            } label: {
                Label(showTerminalDrawer ? "ปิดเทอร์มินัล" : "เทอร์มินัล",
                      systemImage: showTerminalDrawer ? "terminal.fill" : "terminal")
                    .foregroundStyle(showTerminalDrawer ? Color.accentColor : Color.primary)
            }
            .help("เปิด/ปิดเทอร์มินัลในตัว (⌃`)")
        }
        // Declared last so it sits at the far right, over the panel it opens.
        ToolbarItem(placement: .primaryAction) {
            Button { showInspector.toggle() } label: {
                Label("แผงข้อมูล", systemImage: "sidebar.right")
            }
            .help("เปิด/ปิดแผงคอนเท็กซ์ ที่แก้ไป ความจำ (⌘⌥I)")
        }
    }
}
