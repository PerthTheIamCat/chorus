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
    /// The project (workspace root) whose drawer this tab belongs to; `cwd`
    /// may be a worktree of it. Switching project must not show — or keep
    /// typing into — another project's shells.
    let project: String
    let sandboxed: Bool
    @Published var title: String
    @Published var exitCode: Int32?
    @Published var sandboxNote: String?
    let view: TerminalView
    private var ptyId: UInt64 = 0
    private var appliedDark: Bool?

    init(cwd: String, project: String, title: String, sandboxed: Bool) {
        self.cwd = cwd
        self.project = project
        self.title = title
        self.sandboxed = sandboxed
        self.view = TerminalView(frame: NSRect(x: 0, y: 0, width: 640, height: 240))
        super.init()
        view.terminalDelegate = self
        view.font = Self.font(Self.savedFontSize)
        applyTheme()
        start()
    }

    /// A Nerd Font when one is installed — prompts like powerlevel10k draw
    /// their icons from the private-use area, which the system mono font
    /// lacks (they show as boxes). Otherwise the system monospaced font.
    static func font(_ size: CGFloat) -> NSFont {
        if let name = nerdFontName, let f = NSFont(name: name, size: size) { return f }
        return NSFont.monospacedSystemFont(ofSize: size, weight: .regular)
    }

    private static let nerdFontName: String? = {
        let all = NSFontManager.shared.availableFonts
        let mono = all.filter { $0.contains("NerdFontMono") || $0.contains("Nerd Font Mono") || $0.contains("NFM") }
        let any = all.filter { $0.contains("NerdFont") || $0.contains("Nerd Font") || $0.hasPrefix("MesloLGS-NF") }
        return (mono.first(where: { $0.lowercased().contains("regular") }) ?? mono.first
            ?? any.first(where: { $0.lowercased().contains("regular") }) ?? any.first)
    }()

    static var savedFontSize: CGFloat {
        let v = UserDefaults.standard.double(forKey: "terminalFontSize")
        return v >= 9 ? CGFloat(v) : 12
    }

    func setFontSize(_ size: CGFloat) {
        let clamped = min(24, max(9, size))
        UserDefaults.standard.set(Double(clamped), forKey: "terminalFontSize")
        view.font = Self.font(clamped)
    }

    func zoom(_ delta: CGFloat) { setFontSize(view.font.pointSize + delta) }

    /// Clear the screen and scrollback, then ask the shell to redraw its prompt.
    func clear() {
        view.feed(text: "\u{1B}[2J\u{1B}[3J\u{1B}[H")
        send(source: view, data: [0x0c][...])
    }

    /// Follow the app's light/dark appearance instead of a fixed terminal skin.
    /// The background is transparent so the drawer shows the same panel
    /// material as every other panel.
    func applyTheme() {
        let dark = NSApp.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        // Runs on every SwiftUI update of the host; each colour set repaints
        // the whole grid, so only touch the view when something changed.
        guard appliedDark != dark else { return }
        appliedDark = dark
        view.nativeBackgroundColor = .clear
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

    func tabs(for project: String) -> [TerminalTab] { tabs.filter { $0.project == project } }

    func current(for project: String) -> TerminalTab? {
        let mine = tabs(for: project)
        return mine.first { $0.id == selected } ?? mine.last
    }

    func open(cwd: String, project: String? = nil, title: String? = nil, sandboxed: Bool = false) {
        guard !cwd.isEmpty else { return }
        let name = title ?? (cwd as NSString).lastPathComponent
        let tab = TerminalTab(cwd: cwd, project: project ?? cwd, title: name, sandboxed: sandboxed)
        tabs.append(tab)
        selected = tab.id
    }

    func close(_ tab: TerminalTab) {
        tab.close()
        tabs.removeAll { $0.id == tab.id }
        if selected == tab.id { selected = tabs(for: tab.project).last?.id }
    }
}

struct TerminalHost: NSViewRepresentable {
    let tab: TerminalTab
    func makeNSView(context: Context) -> TerminalView {
        DispatchQueue.main.async { tab.view.window?.makeFirstResponder(tab.view) }
        return tab.view
    }
    func updateNSView(_ v: TerminalView, context: Context) {
        tab.applyTheme()
    }
}

