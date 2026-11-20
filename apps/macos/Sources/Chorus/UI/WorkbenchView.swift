import AppKit
import SwiftUI
import WebKit

// Design boards "โต๊ะงาน › ที่แก้ไป / ไฟล์ / พรีวิว". One stage with three tabs:
// review the session's diff hunk by hunk (keep · send back · comment), browse
// files, or preview a local web server. The review bar at the bottom sends the
// review to the agent or opens Commit & PR.

struct WorkbenchView: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 2) {
                ForEach(WorkbenchTab.allCases) { t in
                    Button { shell.workbenchTab = t } label: {
                        HStack(spacing: 5) {
                            Text(t.title)
                            if t == .changes, !engine.diffHunks.isEmpty {
                                Text("\(engine.diffHunks.count)").foregroundStyle(Palette.text3)
                            }
                        }
                    }
                    .buttonStyle(RowStyle(selected: shell.workbenchTab == t, height: 30))
                    .fixedSize()
                }
                Spacer()
            }
            .padding(.horizontal, 10).frame(height: 44)
            Hairline()
            if shell.workspace.isEmpty {
                ContentUnavailableView("ยังไม่ได้เลือกโปรเจกต์", systemImage: "folder.badge.questionmark")
            } else {
                switch shell.workbenchTab {
                case .changes: DiffReview()
                case .files: FileBrowser(workspace: shell.workspace)
                case .preview: PreviewPane()
                }
            }
        }
        .background(Palette.window)
    }
}

// MARK: ที่แก้ไป

private struct DiffReview: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @State private var file: String?
    @State private var kept: Set<String> = []
    @State private var comments: [String: String] = [:]
    @State private var commenting: String?
    @State private var draft = ""
    @State private var confirmRevert = false
    @State private var showShip = false

    var body: some View {
        if engine.diffFiles.isEmpty {
            ContentUnavailableView {
                Label("ยังไม่มีอะไรให้รีวิว", systemImage: "checkmark.seal")
            } description: {
                Text("เมื่อ agent แก้ไฟล์ ก้อนที่แก้จะขึ้นที่นี่ให้เก็บหรือตีกลับทีละก้อน")
            }
        } else {
            VStack(spacing: 0) {
                HStack(spacing: 0) {
                    fileList.frame(width: 250)
                    Hairline(vertical: true)
                    hunks
                }
                Hairline()
                reviewBar
            }
            .onAppear { if file == nil { file = engine.diffFiles.first } }
            .onChange(of: engine.diffFiles) { _, f in if file == nil || !f.contains(file!) { file = f.first } }
            .confirmationDialog("ย้อนไฟล์ทั้งหมดกลับไปตอนเริ่ม session?", isPresented: $confirmRevert) {
                Button("ย้อนทั้งหมด", role: .destructive) { engine.revertToStart() }
            } message: { Text("ระบบเก็บ snapshot ก่อนย้อนไว้ให้") }
            .sheet(isPresented: $showShip) { ShipSheet() }
        }
    }

    private var fileList: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(engine.diffFiles, id: \.self) { f in
                    let h = engine.diffHunks.filter { $0.file == f }
                    Button { file = f } label: {
                        HStack(spacing: 8) {
                            FileTypeIcon(key: (f as NSString).pathExtension, size: 13)
                            Text((f as NSString).lastPathComponent).font(Typo.monoSmall).lineLimit(1)
                            Spacer(minLength: 2)
                            let add = h.reduce(0) { $0 + $1.addCount }, del = h.reduce(0) { $0 + $1.delCount }
                            if add > 0 { Text("+\(add)").font(Typo.monoSmall).foregroundStyle(Palette.ok) }
                            if del > 0 { Text("−\(del)").font(Typo.monoSmall).foregroundStyle(Palette.bad) }
                        }
                    }
                    .buttonStyle(RowStyle(selected: file == f, height: 32))
                    .help(f)
                }
            }
            .padding(8)
        }
        .background(Palette.sidebar)
        .overlay(alignment: .bottom) {
            Text("ก้อนที่ตีกลับ agent จะเห็นในเทิร์นถัดไป").font(Typo.tiny).foregroundStyle(Palette.text3)
                .padding(10).frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder
    private var hunks: some View {
        let f = file ?? engine.diffFiles.first ?? ""
        let list = engine.diffHunks.filter { $0.file == f }
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 14) {
                HStack(spacing: 10) {
                    Text(f).font(.system(size: 13, weight: .medium, design: .monospaced)).lineLimit(1).truncationMode(.head)
                    if engine.testFiles.contains(f) { Tag(text: "แก้เทสต์", tone: .warn) }
                    Text("\(list.count) ก้อน").font(Typo.caption).foregroundStyle(Palette.text3)
                    Spacer()
                    Button("เปิดใน Finder") {
                        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: shell.workspace).appendingPathComponent(f)])
                    }
                    .buttonStyle(QuietButtonStyle(small: true))
                }
                ForEach(list) { h in hunk(h) }
            }
            .padding(20)
        }
    }

    private func hunk(_ h: DiffHunk) -> some View {
        let isKept = kept.contains(h.id)
        return VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                Text(h.header).font(Typo.monoSmall).foregroundStyle(Palette.text3).lineLimit(1)
                Spacer()
                if isKept {
                    Label("เก็บแล้ว", systemImage: "checkmark").font(Typo.caption).foregroundStyle(Palette.ok)
                    Button("เลิก") { kept.remove(h.id) }.buttonStyle(QuietButtonStyle(small: true))
                } else {
                    Button { commenting = h.id; draft = comments[h.id] ?? "" } label: { Image(systemName: "text.bubble") }
                        .buttonStyle(IconButtonStyle()).help("คอมเมนต์ถึง agent").accessibilityLabel("คอมเมนต์")
                    Button("เก็บ") { kept.insert(h.id) }.buttonStyle(PrimaryButtonStyle(small: true))
                    Button("ตีกลับ") { engine.rejectHunks(h.patch) }.buttonStyle(SecondaryButtonStyle(small: true))
                        .help("ย้อนก้อนนี้ในไฟล์ แล้วบอก agent ว่าถูกตีกลับ")
                }
            }
            .padding(.horizontal, 12).frame(height: 36)
            .background(Palette.surface)
            if !isKept {
                Hairline()
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(h.lines.enumerated()), id: \.offset) { i, l in
                        DiffLine(number: h.fileLineNumber(for: i), text: l)
                    }
                }
                .padding(.vertical, 4)
                .textSelection(.enabled)
                if commenting == h.id {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("คอมเมนต์ถึง agent").font(Typo.caption).foregroundStyle(Palette.text3)
                        TextField("เช่น ใช้ค่าจาก env แทนได้ไหม", text: $draft, axis: .vertical)
                            .textFieldStyle(.plain).font(Typo.ui).lineLimit(1...4)
                        HStack {
                            Spacer()
                            Button("ยกเลิก") { commenting = nil }.buttonStyle(QuietButtonStyle(small: true))
                            Button("เพิ่มในรีวิว") {
                                comments[h.id] = draft.trimmingCharacters(in: .whitespacesAndNewlines)
                                commenting = nil
                            }
                            .buttonStyle(PrimaryButtonStyle(small: true))
                        }
                    }
                    .padding(12)
                    .background(Palette.surface, in: RoundedRectangle(cornerRadius: 8))
                    .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Palette.warn))
                    .padding(.horizontal, 12).padding(.bottom, 10).padding(.leading, 46)
                } else if let c = comments[h.id], !c.isEmpty {
                    Label(c, systemImage: "text.bubble").font(Typo.caption).foregroundStyle(Palette.text2)
                        .padding(.horizontal, 12).padding(.bottom, 10).padding(.leading, 46)
                }
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(isKept ? Palette.line : Palette.line2))
        .opacity(isKept ? 0.75 : 1)
    }

    private var reviewBar: some View {
        let unseen = engine.diffHunks.count - kept.count
        let noted = comments.values.filter { !$0.isEmpty }.count
        return HStack(spacing: 10) {
            Text("เก็บ \(kept.count) · ยังไม่ดู \(max(unseen, 0)) · คอมเมนต์ \(noted)").font(Typo.small).foregroundStyle(Palette.text2)
            Button("ย้อนทั้งหมด") { confirmRevert = true }.buttonStyle(QuietButtonStyle(small: true, danger: true))
            Spacer()
            Button { sendReview() } label: {
                HStack(spacing: 6) {
                    Text("ส่งรีวิวให้")
                    if let a = engine.liveAgent ?? shell.agent(for: shell.model)?.id { ProviderIcon(provider: ProviderTint.provider(of: a), size: 12) }
                    Kbd("⇧⌘↩")
                }
            }
            .buttonStyle(SecondaryButtonStyle())
            .keyboardShortcut(.return, modifiers: [.command, .shift])
            .disabled(noted == 0)
            Button { showShip = true } label: {
                HStack(spacing: 6) { Image(systemName: "arrow.triangle.branch"); Text("Commit & PR") }
            }
            .buttonStyle(PrimaryButtonStyle())
        }
        .padding(.horizontal, 16).frame(height: 56)
        .background(Palette.window)
    }

    /// The comments become the next message, so the agent sees them in the ledger.
    private func sendReview() {
        let notes = engine.diffHunks.compactMap { h -> String? in
            guard let c = comments[h.id], !c.isEmpty else { return nil }
            return "- \(h.file) \(h.header.split(separator: "@").first.map(String.init) ?? ""): \(c)"
        }
        guard !notes.isEmpty else { return }
        shell.text = "รีวิว diff:\n" + notes.joined(separator: "\n")
        comments = [:]
        shell.stage = .conversation
        shell.send()
    }
}

private struct DiffLine: View {
    let number: Int
    let text: String

    var body: some View {
        let add = text.hasPrefix("+"), del = text.hasPrefix("-")
        HStack(spacing: 0) {
            Text(add || !del ? "\(number)" : "")
                .frame(width: 46, alignment: .trailing).padding(.trailing, 12)
                .foregroundStyle(add ? Palette.ok : del ? Palette.bad : Palette.text3)
            Text(text.isEmpty ? " " : text)
                .foregroundStyle(add || del ? Palette.text : Palette.text2)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .font(Typo.mono)
        .frame(minHeight: 21)
        .background(add ? Palette.addWash : del ? Palette.delWash : .clear)
    }
}

// MARK: ไฟล์

extension Engine.FileNode {
    var kids: [Engine.FileNode]? { is_dir ? children : nil }
}

private struct FileBrowser: View {
    @EnvironmentObject private var engine: Engine
    let workspace: String
    @State private var tree: Engine.FileTreeResult?
    @State private var selected: String?
    @State private var query = ""

    var body: some View {
        HStack(spacing: 0) {
            VStack(spacing: 0) {
                TextField("ค้นไฟล์", text: $query).textFieldStyle(.roundedBorder).padding(10)
                if let tree {
                    List(selection: $selected) {
                        if query.isEmpty {
                            OutlineGroup(tree.nodes, children: \.kids) { row($0) }
                        } else {
                            ForEach(flat(tree.nodes).filter { $0.rel_path.localizedCaseInsensitiveContains(query) }.prefix(300)) { row($0) }
                        }
                    }
                    .listStyle(.sidebar)
                    .scrollContentBackground(.hidden)
                } else {
                    Spacer(); ProgressView(); Spacer()
                }
                Text("ยึด .gitignore · ไฟล์ .env ถูกซ่อนจาก agent").font(Typo.tiny).foregroundStyle(Palette.text3)
                    .padding(10).frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(width: 270)
            .background(Palette.sidebar)
            Hairline(vertical: true)
            Group {
                if let rel = selected, let node = find(rel), !node.is_dir {
                    FileViewer(root: workspace, node: node).id(rel)
                } else {
                    ContentUnavailableView("เลือกไฟล์ทางซ้าย", systemImage: "doc.text")
                }
            }
            .frame(maxWidth: .infinity)
        }
        .onAppear(perform: load)
        .onChange(of: engine.workingDiff) { _, _ in load() }
    }

    private func row(_ n: Engine.FileNode) -> some View {
        HStack(spacing: 6) {
            if n.is_dir { Image(systemName: "folder").foregroundStyle(Palette.text3) } else {
                FileTypeIcon(key: n.icon ?? (n.name as NSString).pathExtension, size: 13)
            }
            Text(n.name).font(Typo.small).lineLimit(1)
            Spacer(minLength: 0)
            if let g = n.git_status, !g.isEmpty {
                Text(g).font(Typo.monoSmall).foregroundStyle(g.contains("D") ? Palette.bad : g.contains("A") || g.contains("?") ? Palette.ok : Palette.warn)
            }
            if n.is_focus { Image(systemName: "eye").font(.system(size: 10)).foregroundStyle(Palette.accent).help("agent กำลังดูไฟล์นี้") }
        }
        .tag(n.rel_path)
    }

    private func flat(_ nodes: [Engine.FileNode]) -> [Engine.FileNode] { nodes.flatMap { $0.is_dir ? flat($0.children) : [$0] } }

    private func find(_ rel: String) -> Engine.FileNode? {
        func go(_ nodes: [Engine.FileNode]) -> Engine.FileNode? {
            for n in nodes { if n.rel_path == rel { return n }; if let h = go(n.children) { return h } }
            return nil
        }
        return tree.flatMap { go($0.nodes) }
    }

    private func load() { engine.fetchFileTree(cwd: workspace, focusFile: engine.agentFocusFile) { tree = $0 } }
}

private let fileViewerLimit = 400_000

private struct FileViewer: View {
    let root: String
    let node: Engine.FileNode
    @EnvironmentObject private var shell: ShellModel
    @State private var text: String?
    @State private var note: String?

    private var url: URL { URL(fileURLWithPath: root).appendingPathComponent(node.rel_path) }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Text(node.rel_path.split(separator: "/").joined(separator: "  ›  "))
                    .font(Typo.monoSmall).foregroundStyle(Palette.text3).lineLimit(1).truncationMode(.head)
                Spacer()
                Button("แนบเข้า prompt") { shell.text += (shell.text.isEmpty ? "" : " ") + "@\(node.rel_path) " }
                    .buttonStyle(QuietButtonStyle(small: true))
                Button("แสดงใน Finder") { NSWorkspace.shared.activateFileViewerSelecting([url]) }
                    .buttonStyle(QuietButtonStyle(small: true))
                Button("เปิดใน editor") { NSWorkspace.shared.open(url) }.buttonStyle(SecondaryButtonStyle(small: true))
            }
            .padding(.horizontal, 14).frame(height: 40)
            Hairline()
            if let text {
                ScrollView([.vertical, .horizontal]) {
                    let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(lines.enumerated()), id: \.offset) { i, l in
                            HStack(spacing: 0) {
                                Text("\(i + 1)").foregroundStyle(Palette.text3).frame(width: 46, alignment: .trailing).padding(.trailing, 12)
                                Text(l.isEmpty ? " " : String(l)).foregroundStyle(Palette.text)
                            }
                            .font(Typo.mono).frame(height: 20)
                        }
                    }
                    .padding(.vertical, 8)
                    .textSelection(.enabled)
                }
                if let note { Text(note).font(Typo.caption).foregroundStyle(Palette.text3).padding(6) }
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
        let r: (String?, String?) = await Task.detached(priority: .userInitiated) {
            guard let h = try? FileHandle(forReadingFrom: target) else { return (nil, "เปิดไฟล์ไม่ได้") }
            defer { try? h.close() }
            let data = (try? h.read(upToCount: fileViewerLimit + 1)) ?? Data()
            guard let s = String(data: data.prefix(fileViewerLimit), encoding: .utf8) else {
                return (nil, "ไฟล์นี้ไม่ใช่ข้อความ เปิดด้วยแอปเริ่มต้นแทน")
            }
            return (s, data.count > fileViewerLimit ? "แสดงแค่ส่วนต้นของไฟล์" : nil)
        }.value
        text = r.0; note = r.1
    }
}

// MARK: พรีวิว

@MainActor
final class PreviewModel: NSObject, ObservableObject, WKNavigationDelegate {
    let webView = ArtifactPreviewSecurity.makeWebView()
    @Published var input = "http://localhost:3000"
    @Published var loading = false
    @Published var canGoBack = false
    @Published var canGoForward = false
    @Published var error: String?

    override init() {
        super.init()
        webView.navigationDelegate = self
    }

    func load() {
        guard let url = ArtifactPreviewSecurity.webURL(input) else {
            error = "ใส่ที่อยู่แบบ http:// หรือ localhost:พอร์ต"
            return
        }
        error = nil
        webView.load(URLRequest(url: url))
    }

    private func refresh() {
        loading = webView.isLoading
        canGoBack = webView.canGoBack
        canGoForward = webView.canGoForward
        if let u = webView.url?.absoluteString { input = u }
    }

    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                 decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void) {
        decisionHandler(ArtifactPreviewSecurity.allowsNavigation(action.request.url, artifact: false) ? .allow : .cancel)
    }
    nonisolated func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        Task { @MainActor in self.refresh() }
    }
    nonisolated func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        Task { @MainActor in self.refresh() }
    }
    nonisolated func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        let m = error.localizedDescription
        Task { @MainActor in self.refresh(); self.error = m }
    }
    nonisolated func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        let m = error.localizedDescription
        Task { @MainActor in self.refresh(); self.error = m }
    }
}

private struct WebHost: NSViewRepresentable {
    let webView: WKWebView
    func makeNSView(context: Context) -> WKWebView { webView }
    func updateNSView(_ nsView: WKWebView, context: Context) {}
}

private struct PreviewPane: View {
    @StateObject private var model = PreviewModel()
    @State private var width: Width = .desktop

    enum Width: String, CaseIterable, Identifiable {
        case desktop = "Desktop", tablet = "Tablet", phone = "Phone"
        var id: String { rawValue }
        var points: CGFloat? { switch self { case .desktop: nil; case .tablet: 768; case .phone: 375 } }
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                Button { model.webView.goBack() } label: { Image(systemName: "chevron.left") }
                    .buttonStyle(IconButtonStyle()).disabled(!model.canGoBack).accessibilityLabel("ย้อนกลับ")
                Button { model.webView.reload() } label: { Image(systemName: "arrow.clockwise") }
                    .buttonStyle(IconButtonStyle()).accessibilityLabel("โหลดใหม่")
                HStack(spacing: 8) {
                    Circle().fill(model.error == nil ? Palette.ok : Palette.bad).frame(width: 7, height: 7)
                    TextField("localhost:5173", text: $model.input).textFieldStyle(.plain).font(Typo.mono)
                        .onSubmit { model.load() }
                    if model.loading { ProgressView().controlSize(.mini) }
                }
                .padding(.horizontal, 10).frame(height: 30)
                .background(Palette.surface, in: RoundedRectangle(cornerRadius: 8))
                .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Palette.line))
                Picker("ขนาดจอ", selection: $width) { ForEach(Width.allCases) { Text($0.rawValue).tag($0) } }
                    .pickerStyle(.segmented).labelsHidden().fixedSize()
            }
            .padding(.horizontal, 12).frame(height: 46)
            Hairline()
            if let e = model.error {
                Text(e).font(Typo.caption).foregroundStyle(Palette.bad).padding(8).frame(maxWidth: .infinity, alignment: .leading)
            }
            WebHost(webView: model.webView)
                .clipShape(RoundedRectangle(cornerRadius: width == .desktop ? 0 : 10))
                .frame(maxWidth: width.points ?? .infinity)
                .padding(width == .desktop ? 0 : 20)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Palette.sidebar)
        }
        .onAppear { if model.webView.url == nil { model.load() } }
    }
}

// MARK: terminal drawer

/// Tabs of shells under the stage (⌃`), the design's drawer.
struct TerminalDrawer: View {
    @ObservedObject private var store = TerminalStore.shared
    let workspace: String

    var body: some View {
        let tabs = store.tabs(for: workspace)
        let current = store.current(for: workspace)
        VStack(spacing: 0) {
            HStack(spacing: 4) {
                ForEach(tabs) { t in
                    Button(t.title) { store.selected = t.id }
                        .buttonStyle(RowStyle(selected: current?.id == t.id, height: 24))
                        .fixedSize()
                        .contextMenu { Button("ปิด") { store.close(t) } }
                }
                Button { store.open(cwd: workspace, project: workspace, title: nil, sandboxed: false) } label: { Image(systemName: "plus") }
                    .buttonStyle(IconButtonStyle()).help("เทอร์มินัลใหม่").accessibilityLabel("เทอร์มินัลใหม่")
                Button { store.open(cwd: workspace, project: workspace, title: "ใน sandbox", sandboxed: true) } label: {
                    Image(systemName: "lock.shield")
                }
                .buttonStyle(IconButtonStyle()).help("เชลล์ที่เขียนได้เฉพาะในโปรเจกต์ เหมือนที่ agent โดนจำกัด")
                Spacer()
                if let t = current {
                    Button { t.clear() } label: { Image(systemName: "eraser") }.buttonStyle(IconButtonStyle()).help("ล้างหน้าจอ")
                    Button { store.close(t) } label: { Image(systemName: "xmark") }.buttonStyle(IconButtonStyle()).help("ปิดแท็บนี้")
                }
                Kbd("⌃`")
            }
            .padding(.horizontal, 10).frame(height: 34)
            if let t = current {
                if let n = t.sandboxNote {
                    Label(n, systemImage: "exclamationmark.triangle").font(Typo.caption).foregroundStyle(Palette.warn)
                        .padding(.horizontal, 10).frame(maxWidth: .infinity, alignment: .leading)
                }
                TerminalHost(tab: t).id(t.id)
            } else {
                Spacer()
            }
        }
        .background(Palette.sidebar)
        .onAppear { if tabs.isEmpty { store.open(cwd: workspace, project: workspace, title: nil, sandboxed: false) } }
    }
}
