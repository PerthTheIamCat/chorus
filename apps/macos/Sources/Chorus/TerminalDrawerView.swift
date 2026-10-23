import SwiftUI
import AppKit
import CChorus
import SwiftTerm

/// One shell. The PTY, environment, resize and kill live in Rust
/// (`chorus_term_*`, CLAUDE.md 2.3); this class only owns the SwiftTerm grid
/// that draws the bytes and hands keystrokes back.
final class TerminalTab: NSObject, ObservableObject, Identifiable, TerminalViewDelegate {
    let id = UUID()
    let cwd: String
    let sandboxed: Bool
    @Published var title: String
    @Published var exitCode: Int32?
    @Published var sandboxNote: String?
    let view: TerminalView
    private var ptyId: UInt64 = 0

    init(cwd: String, title: String, sandboxed: Bool) {
        self.cwd = cwd
        self.title = title
        self.sandboxed = sandboxed
        self.view = TerminalView(frame: NSRect(x: 0, y: 0, width: 640, height: 240))
        super.init()
        view.terminalDelegate = self
        view.font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
        applyTheme()
        start()
    }

    /// Follow the app's light/dark appearance instead of a fixed terminal skin.
    func applyTheme() {
        let dark = NSApp.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        view.nativeBackgroundColor = dark ? NSColor(white: 0.10, alpha: 1) : NSColor(white: 0.98, alpha: 1)
        view.nativeForegroundColor = dark ? NSColor(white: 0.90, alpha: 1) : NSColor(white: 0.12, alpha: 1)
    }

    func start() {
        exitCode = nil
        let me = Unmanaged.passRetained(self).toOpaque()
        let json = cwd.withCString { c in
            chorus_term_open(c, 80, 24, sandboxed, me, { ctx, _, data, len in
                guard let ctx, let data else { return }
                let bytes = Array(UnsafeBufferPointer(start: data, count: len))
                let tab = Unmanaged<TerminalTab>.fromOpaque(ctx).takeUnretainedValue()
                DispatchQueue.main.async { tab.view.feed(byteArray: bytes[...]) }
            }, { ctx, _, code in
                guard let ctx else { return }
                let tab = Unmanaged<TerminalTab>.fromOpaque(ctx).takeRetainedValue()
                DispatchQueue.main.async {
                    tab.exitCode = code
                    tab.ptyId = 0
                    tab.view.feed(text: "\r\n\u{1B}[2m[เชลล์จบการทำงาน · รหัส \(code)]\u{1B}[0m\r\n")
                }
            })
        }
        struct Res: Decodable { let ok: Bool; let id: UInt64?; let error: String?; let sandbox_note: String? }
        let text = json.map { String(cString: $0) } ?? ""
        if let json { chorus_string_free(json) }
        guard let res = try? JSONDecoder().decode(Res.self, from: Data(text.utf8)), res.ok, let id = res.id else {
            Unmanaged<TerminalTab>.fromOpaque(me).release()
            let msg = (try? JSONDecoder().decode(Res.self, from: Data(text.utf8)))?.error ?? "เปิดเทอร์มินัลไม่ได้"
            exitCode = -1
            view.feed(text: "\u{1B}[31m\(msg)\u{1B}[0m\r\n")
            return
        }
        ptyId = id
        sandboxNote = res.sandbox_note
        if let note = res.sandbox_note {
            view.feed(text: "\u{1B}[33m[ไม่ได้ครอบ sandbox: \(note)]\u{1B}[0m\r\n")
        }
    }

    func close() {
        if ptyId != 0 { chorus_term_close(ptyId); ptyId = 0 }
    }

    // MARK: TerminalViewDelegate
    func send(source: TerminalView, data: ArraySlice<UInt8>) {
        guard ptyId != 0 else { return }
        let bytes = Array(data)
        _ = bytes.withUnsafeBufferPointer { chorus_term_write(ptyId, $0.baseAddress, $0.count) }
    }
    func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
        guard ptyId != 0, newCols > 0, newRows > 0 else { return }
        chorus_term_resize(ptyId, UInt16(clamping: newCols), UInt16(clamping: newRows))
    }
    func setTerminalTitle(source: TerminalView, title: String) {
        DispatchQueue.main.async { if !title.isEmpty { self.title = title } }
    }
    func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}
    func scrolled(source: TerminalView, position: Double) {}
    func requestOpenLink(source: TerminalView, link: String, params: [String: String]) {
        if let url = URL(string: link), ["http", "https"].contains(url.scheme) { NSWorkspace.shared.open(url) }
    }
    func clipboardCopy(source: TerminalView, content: Data) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setData(content, forType: .string)
    }
    func rangeChanged(source: TerminalView, startY: Int, endY: Int) {}
}

/// Tabs outlive the drawer: closing the drawer must not kill a running build.
@MainActor
final class TerminalStore: ObservableObject {
    static let shared = TerminalStore()
    @Published var tabs: [TerminalTab] = []
    @Published var selected: UUID?

    var current: TerminalTab? { tabs.first { $0.id == selected } ?? tabs.first }

    func open(cwd: String, title: String? = nil, sandboxed: Bool = false) {
        guard !cwd.isEmpty else { return }
        let name = title ?? (cwd as NSString).lastPathComponent
        let tab = TerminalTab(cwd: cwd, title: name, sandboxed: sandboxed)
        tabs.append(tab)
        selected = tab.id
    }

    func close(_ tab: TerminalTab) {
        tab.close()
        tabs.removeAll { $0.id == tab.id }
        if selected == tab.id { selected = tabs.last?.id }
    }
}

private struct TerminalHost: NSViewRepresentable {
    let tab: TerminalTab
    func makeNSView(context: Context) -> TerminalView {
        DispatchQueue.main.async { tab.view.window?.makeFirstResponder(tab.view) }
        return tab.view
    }
    func updateNSView(_ v: TerminalView, context: Context) {
        tab.applyTheme()
    }
}

/// Integrated terminal drawer (⌃`): real PTY shells, one tab per folder or
/// worktree, plus a read-only feed of the commands agents ran.
struct TerminalDrawerView: View {
    let workspace: String
    var onClose: () -> Void

    @EnvironmentObject var engine: Engine
    @ObservedObject private var store = TerminalStore.shared
    @State private var worktrees: [Engine.WorktreeInfo] = []
    @State private var showAgentFeed = false
    @State private var sandboxed = false

    init(workspace: String, onClose: @escaping () -> Void) {
        self.workspace = workspace
        self.onClose = onClose
    }

    private var agentCommands: [LedgerEvent] {
        engine.events.filter { $0.kind == "tool_call" }.suffix(200).reversed()
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            if showAgentFeed {
                agentFeed
            } else if let tab = store.current {
                TerminalHost(tab: tab).id(tab.id)
                    .padding(.horizontal, 6)
                    .background(Color(nsColor: tab.view.nativeBackgroundColor))
            } else {
                VStack(spacing: 8) {
                    Text("ยังไม่มีเทอร์มินัล").foregroundStyle(.secondary)
                    Button("เปิดเทอร์มินัลใหม่") { store.open(cwd: workspace, sandboxed: sandboxed) }
                        .disabled(workspace.isEmpty)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(minHeight: 180, idealHeight: 280, maxHeight: 520)
        .onAppear {
            if store.tabs.isEmpty { store.open(cwd: workspace) }
            engine.fetchWorktrees(cwd: workspace) { worktrees = $0 }
        }
    }

    private var header: some View {
        HStack(spacing: 6) {
            Image(systemName: "terminal.fill").font(.caption).foregroundStyle(.teal)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 4) {
                    ForEach(store.tabs) { tab in
                        TabChip(tab: tab, selected: !showAgentFeed && store.current?.id == tab.id,
                                onSelect: { showAgentFeed = false; store.selected = tab.id },
                                onClose: { store.close(tab) })
                    }
                }
            }
            Menu {
                Button("โฟลเดอร์โปรเจกต์") { showAgentFeed = false; store.open(cwd: workspace, sandboxed: sandboxed) }
                ForEach(worktrees) { wt in
                    Button("worktree · \(wt.branch)") {
                        showAgentFeed = false
                        store.open(cwd: wt.path, title: wt.branch, sandboxed: sandboxed)
                    }
                }
                Divider()
                Toggle("ครอบ sandbox (เขียนได้เฉพาะ workspace)", isOn: $sandboxed)
            } label: { Image(systemName: "plus") }
                .menuStyle(.borderlessButton).frame(width: 28)
                .help("เทอร์มินัลใหม่")
            Spacer()
            Button { showAgentFeed.toggle() } label: {
                Label("คำสั่งของ agent", systemImage: "cpu")
                    .font(.caption)
                    .foregroundStyle(showAgentFeed ? Color.accentColor : .secondary)
            }
            .buttonStyle(.plain)
            .help("ดูคำสั่งที่ agent รันใน session นี้ (อ่านอย่างเดียว)")
            if let tab = store.current, tab.exitCode != nil, !showAgentFeed {
                Button("เปิดใหม่") { tab.start() }.font(.caption)
            }
            Button(action: onClose) { Image(systemName: "xmark").font(.caption2) }
                .buttonStyle(.plain)
        }
        .padding(.horizontal, 10).padding(.vertical, 5)
        .background(Color(NSColor.windowBackgroundColor).opacity(0.95))
    }

    private var agentFeed: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 6) {
                if agentCommands.isEmpty {
                    Text("agent ยังไม่ได้รันคำสั่งใน session นี้").foregroundStyle(.secondary).font(.caption)
                }
                ForEach(agentCommands) { ev in
                    HStack(alignment: .top, spacing: 8) {
                        Text(ev.actor).font(.caption2.weight(.semibold)).foregroundStyle(.teal)
                            .frame(width: 70, alignment: .leading)
                        Text(ev.body.prefix(600)).font(.system(size: 11, design: .monospaced))
                            .textSelection(.enabled)
                    }
                }
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(Color(NSColor.textBackgroundColor).opacity(0.4))
    }
}

private struct TabChip: View {
    @ObservedObject var tab: TerminalTab
    let selected: Bool
    let onSelect: () -> Void
    let onClose: () -> Void

    var body: some View {
        HStack(spacing: 4) {
            Circle().fill(tab.exitCode == nil ? Color.green : Color.secondary).frame(width: 6, height: 6)
            Text(tab.title).font(.caption).lineLimit(1)
            if tab.sandboxed { Image(systemName: "lock.shield").font(.system(size: 9)) }
            Button(action: onClose) { Image(systemName: "xmark").font(.system(size: 8)) }.buttonStyle(.plain)
        }
        .padding(.horizontal, 8).padding(.vertical, 3)
        .background(selected ? Color.accentColor.opacity(0.18) : Color.primary.opacity(0.05), in: Capsule())
        .contentShape(Capsule())
        .onTapGesture(perform: onSelect)
    }
}
