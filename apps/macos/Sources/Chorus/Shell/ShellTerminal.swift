import SwiftUI

// The terminal tool: real PTY shells (the PTY is Rust's), one tab per folder
// or worktree. Tabs outlive the sheet so closing it never kills a build.

struct TerminalTool: View {
    @EnvironmentObject private var engine: Engine
    @ObservedObject private var store = TerminalStore.shared
    let workspace: String

    private var tabs: [TerminalTab] { store.tabs(for: workspace) }
    private var current: TerminalTab? { store.current(for: workspace) }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Picker("เทอร์มินัล", selection: Binding(
                    get: { current?.id },
                    set: { store.selected = $0 }
                )) {
                    ForEach(tabs) { Text($0.title).tag(Optional($0.id)) }
                }
                .labelsHidden()
                .frame(maxWidth: 260)
                Button { store.open(cwd: workspace, project: workspace, title: nil, sandboxed: false) } label: {
                    Label("เทอร์มินัลใหม่", systemImage: "plus")
                }
                Button { store.open(cwd: workspace, project: workspace, title: "ใน sandbox", sandboxed: true) } label: {
                    Label("ใน sandbox", systemImage: "lock.shield")
                }
                .help("เปิดเชลล์ที่เขียนได้เฉพาะในโฟลเดอร์โปรเจกต์ เหมือนที่ agent โดนจำกัด")
                Spacer()
                if let t = current {
                    Button { t.zoom(-1) } label: { Image(systemName: "textformat.size.smaller") }.help("ลดขนาดตัวอักษร")
                    Button { t.zoom(1) } label: { Image(systemName: "textformat.size.larger") }.help("เพิ่มขนาดตัวอักษร")
                    Button { t.clear() } label: { Image(systemName: "eraser") }.help("ล้างหน้าจอ")
                    Button { store.close(t) } label: { Image(systemName: "xmark.circle") }.help("ปิดเทอร์มินัลนี้")
                }
            }
            .buttonStyle(.borderless)
            .padding(10)
            Divider()
            if let t = current {
                if let n = t.sandboxNote {
                    Label(n, systemImage: "exclamationmark.triangle").font(.caption).foregroundStyle(.orange)
                        .padding(6).frame(maxWidth: .infinity, alignment: .leading)
                }
                TerminalHost(tab: t).id(t.id)
            } else {
                ContentUnavailableView {
                    Label("ยังไม่มีเทอร์มินัล", systemImage: "terminal")
                } actions: {
                    Button("เปิดเทอร์มินัล") { store.open(cwd: workspace, project: workspace, title: nil, sandboxed: false) }
                }
            }
        }
        .onAppear { if tabs.isEmpty { store.open(cwd: workspace, project: workspace, title: nil, sandboxed: false) } }
    }
}
