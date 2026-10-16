import SwiftUI
import UniformTypeIdentifiers

struct DraftAttachment: Identifiable, Equatable {
    let id: Int64
    let name: String
    let mediaType: String
    let size: Int64
    let agentReadable: Bool
}

final class DraftBox: ObservableObject {
    @Published var text = ""
    @Published var attachments: [DraftAttachment] = []
}

/// One row of the `/` or `@` popup.
struct Suggestion: Identifiable {
    let id = UUID()
    let title: String
    var detail: String = ""
    let icon: String
    /// Replaces the token being typed. `nil` with an `action` = run and clear.
    var insert: String?
    var action: (() -> Void)?
}

/// Files under the workspace for `@` mentions. Built once per folder off the
/// main thread; skips the same noise the checkpoint skips.
@MainActor
final class FileIndex: ObservableObject {
    @Published private(set) var files: [String] = []
    private var root = ""

    nonisolated private static let skip: Set<String> = [
        ".git", ".chorus", "node_modules", "target", ".build", "dist", "build", ".next",
        ".svelte-kit", ".turbo", ".cache", "Pods", "DerivedData", ".venv", "__pycache__", "coverage",
    ]

    func load(_ root: String) {
        guard root != self.root, !root.isEmpty else { return }
        self.root = root
        files = []
        Task.detached(priority: .utility) {
            let result = Self.scan(root)
            await MainActor.run { if self.root == root { self.files = result } }
        }
    }

    nonisolated private static func scan(_ root: String) -> [String] {
        var out: [String] = []
        let base = URL(fileURLWithPath: root)
        guard let e = FileManager.default.enumerator(
            at: base, includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles, .skipsPackageDescendants]) else { return out }
        while let url = e.nextObject() as? URL {
            if skip.contains(url.lastPathComponent) { e.skipDescendants(); continue }
            let isDir = (try? url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) ?? false
            if !isDir {
                out.append(String(url.path.dropFirst(base.path.count + 1)))
                if out.count >= 8000 { break }
            }
        }
        return out
    }

    /// Fuzzy-ish: filename prefix first, then path contains.
    func match(_ q: String, limit: Int = 8) -> [String] {
        guard !q.isEmpty else { return Array(files.prefix(limit)) }
        let ql = q.lowercased()
        let byName = files.filter { ($0 as NSString).lastPathComponent.lowercased().hasPrefix(ql) }
        let byPath = files.filter { $0.lowercased().contains(ql) && !byName.contains($0) }
        return Array((byName + byPath).prefix(limit))
    }
}

/// The text box: Enter sends, Shift+Enter is a newline, and `/` or `@` open a
/// completion list driven from the keyboard (↑ ↓ to move, Enter/Tab to pick,
/// Esc to close) — what Claude, Codex and Cursor all do.
///
/// Holds the text in its own `ObservableObject` so typing re-renders this view
/// only, not the whole window.
struct ComposerField: View {
    @ObservedObject var box: DraftBox
    let suggestions: (_ trigger: Character, _ query: String) -> [Suggestion]
    let onSubmit: () -> Void
    var onImportURLs: ([URL]) -> Void = { _ in }
    var onPasteImage: (Data, String) -> Void = { _, _ in }
    var onRemoveAttachment: (Int64) -> Void = { _ in }
    var placeholder = "ขอให้ทำอะไร… ( / คำสั่ง · @ ไฟล์หรือเอเจนต์ )"

    @AppStorage("chorus.enterSends") private var enterSends = true
    @State private var selected = 0
    @State private var dismissed = false
    @State private var showingImporter = false
    @State private var dropTargeted = false
    @FocusState private var focused: Bool
    @ObservedObject private var voiceDictation = VoiceDictationManager.shared

    /// The token under completion: `/xyz` at the very start, or the last
    /// whitespace-separated word if it starts with `@`.
    private var token: (trigger: Character, query: String, range: Range<String.Index>)? {
        let t = box.text
        if t.hasPrefix("/"), !t.contains(" "), !t.contains("\n") {
            return ("/", String(t.dropFirst()), t.startIndex..<t.endIndex)
        }
        let start = t.lastIndex(where: { $0 == " " || $0 == "\n" }).map { t.index(after: $0) } ?? t.startIndex
        let word = t[start...]
        if word.hasPrefix("@") {
            return ("@", String(word.dropFirst()), start..<t.endIndex)
        }
        return nil
    }

    private var items: [Suggestion] {
        guard !dismissed, let tok = token else { return [] }
        return suggestions(tok.trigger, tok.query)
    }

    private var dynamicHeight: CGFloat {
        let lines = box.text.components(separatedBy: "\n").count
        let chars = box.text.count
        let estimatedWrapping = max(lines, Int(ceil(Double(chars) / 55.0)))
        if box.text.isEmpty || estimatedWrapping <= 1 {
            return 32
        }
        let h = 32 + CGFloat(estimatedWrapping - 1) * 19
        return min(max(h, 32), 140)
    }

    var body: some View {
        let list = items
        VStack(alignment: .leading, spacing: 4) {
            if !list.isEmpty { popup(list) }
            if !box.attachments.isEmpty { attachmentChips }
            ZStack(alignment: .topLeading) {
                if box.text.isEmpty {
                    Text(placeholder).foregroundStyle(.tertiary)
                        .font(.system(size: 13))
                        .padding(.leading, 5).padding(.top, 5)
                        .allowsHitTesting(false)
                }
                NativeComposerTextView(
                    text: $box.text,
                    enterSends: enterSends,
                    onPasteImage: onPasteImage,
                    onImportURLs: onImportURLs,
                    onSubmit: onSubmit,
                    onKeyNav: { nav in
                        guard !list.isEmpty else { return false }
                        switch nav {
                        case .up:
                            selected = (selected - 1 + list.count) % list.count
                            return true
                        case .down:
                            selected = (selected + 1) % list.count
                            return true
                        case .escape:
                            dismissed = true
                            return true
                        case .tab, .enter:
                            apply(list[min(selected, list.count - 1)])
                            return true
                        }
                    }
                )
                .frame(height: dynamicHeight)
                .animation(.easeInOut(duration: 0.12), value: dynamicHeight)
            }
            .overlay {
                if dropTargeted {
                    RoundedRectangle(cornerRadius: 7)
                        .strokeBorder(Color.accentColor, style: StrokeStyle(lineWidth: 2, dash: [6, 4]))
                        .background(Color.accentColor.opacity(0.08), in: RoundedRectangle(cornerRadius: 7))
                        .allowsHitTesting(false)
                }
            }
            .onDrop(of: [UTType.fileURL, UTType.url, UTType.image], isTargeted: $dropTargeted, perform: receiveDrop)
            .onPasteCommand(of: [.image, .fileURL, .url], perform: receivePaste)
            HStack(spacing: 8) {
                Button { showingImporter = true } label: {
                    Label("แนบไฟล์", systemImage: "paperclip")
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .help("แนบไฟล์หรือรูปภาพ")
                .accessibilityHint("เลือกไฟล์หรือรูปภาพเพื่อแนบในข้อความนี้")

                // Voice Dictation Button (Native Apple Speech Recognition)
                Button {
                    voiceDictation.toggleRecording { transcribed in
                        if !transcribed.isEmpty {
                            box.text = transcribed
                        }
                    }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: voiceDictation.isRecording ? "waveform.circle.fill" : "mic")
                            .font(.system(size: 12))
                            .foregroundStyle(voiceDictation.isRecording ? Color.red : .secondary)
                        if voiceDictation.isRecording {
                            Text("กำลังฟัง…")
                                .font(.caption2.weight(.bold))
                                .foregroundStyle(.red)
                        } else {
                            Text("สั่งด้วยเสียง")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                .buttonStyle(.plain)
                .help("สั่งการด้วยเสียงพูด (ไทย/อังกฤษ) ด้วย Apple Speech.framework")

                Spacer(minLength: 0)
                if box.attachments.isEmpty {
                    if voiceDictation.isRecording {
                        Text("กำลังบันทึกเสียงพูด...")
                            .font(.caption)
                            .foregroundStyle(Color.red.opacity(0.8))
                            .accessibilityHidden(true)
                    } else {
                        Text("ลากไฟล์มาวาง หรือกดวางรูปภาพได้ทันที")
                            .font(.caption)
                            .foregroundStyle(.tertiary)
                            .accessibilityHidden(true)
                    }
                }
            }
        }
        .fileImporter(isPresented: $showingImporter, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            if case let .success(urls) = result, !urls.isEmpty { onImportURLs(urls) }
        }
        .onChange(of: box.text) { _, _ in selected = 0; dismissed = false }
        .onAppear { focused = true }
    }

    private var attachmentChips: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 6) {
                ForEach(box.attachments) { attachment in
                    HStack(spacing: 6) {
                        Image(systemName: attachment.mediaType.hasPrefix("image/") ? "photo" : "doc")
                            .foregroundStyle(.secondary)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(attachment.name).lineLimit(1)
                            Text("\(ByteCountFormatter.string(fromByteCount: attachment.size, countStyle: .file)) · \(attachment.mediaType)")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                        }
                        if !attachment.agentReadable {
                            Image(systemName: "exclamationmark.circle")
                                .foregroundStyle(.secondary)
                                .help("This file is stored with the session but is not currently readable by the agent")
                        }
                        Button {
                            onRemoveAttachment(attachment.id)
                        } label: {
                            Image(systemName: "xmark.circle.fill")
                                .foregroundStyle(.tertiary)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Remove \(attachment.name)")
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 5)
                    .background(Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: 7))
                    .accessibilityElement(children: .contain)
                    .accessibilityLabel("\(attachment.name), \(ByteCountFormatter.string(fromByteCount: attachment.size, countStyle: .file)), \(attachment.mediaType), \(attachment.agentReadable ? "agent readable" : "not agent readable")")
                }
            }
        }
        .scrollIndicators(.hidden)
        .frame(maxHeight: 48)
    }

    private func receiveDrop(_ providers: [NSItemProvider]) -> Bool {
        var handled = false
        for provider in providers {
            if provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier)
                || provider.hasItemConformingToTypeIdentifier(UTType.url.identifier) {
                handled = true
                let type = provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier)
                    ? UTType.fileURL.identifier : UTType.url.identifier
                provider.loadItem(forTypeIdentifier: type, options: nil) { item, _ in
                    let url = (item as? URL) ?? (item as? NSURL).map { $0 as URL } ?? (item as? Data).flatMap { URL(dataRepresentation: $0, relativeTo: nil) }
                    if let url { DispatchQueue.main.async { self.onImportURLs([url]) } }
                }
            } else if let imageType = provider.registeredTypeIdentifiers.first(where: { UTType($0)?.conforms(to: .image) == true }) {
                handled = true
                provider.loadDataRepresentation(forTypeIdentifier: imageType) { data, _ in
                    guard let data else { return }
                    let mime = UTType(imageType)?.preferredMIMEType ?? "application/octet-stream"
                    DispatchQueue.main.async { self.onPasteImage(data, mime) }
                }
            }
        }
        return handled
    }

    private func receivePaste(_ providers: [NSItemProvider]) {
        for provider in providers {
            if provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier)
                || provider.hasItemConformingToTypeIdentifier(UTType.url.identifier) {
                let type = provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier)
                    ? UTType.fileURL.identifier : UTType.url.identifier
                provider.loadItem(forTypeIdentifier: type, options: nil) { item, _ in
                    let url = (item as? URL) ?? (item as? NSURL).map { $0 as URL } ?? (item as? Data).flatMap { URL(dataRepresentation: $0, relativeTo: nil) }
                    if let url { DispatchQueue.main.async { self.onImportURLs([url]) } }
                }
                continue
            }
            guard let imageType = provider.registeredTypeIdentifiers.first(where: { UTType($0)?.conforms(to: .image) == true }) else { continue }
            provider.loadDataRepresentation(forTypeIdentifier: imageType) { data, _ in
                guard let data else { return }
                let mime = UTType(imageType)?.preferredMIMEType ?? "application/octet-stream"
                DispatchQueue.main.async { self.onPasteImage(data, mime) }
            }
        }
    }

    private func pasteFromClipboard() -> Bool {
        let pb = NSPasteboard.general
        // 1. Direct image data in pasteboard (PNG, TIFF)
        if let pngData = pb.data(forType: .png) {
            onPasteImage(pngData, "image/png")
            return true
        }
        if let tiffData = pb.data(forType: .tiff) {
            if let rep = NSBitmapImageRep(data: tiffData),
               let pngData = rep.representation(using: .png, properties: [:]) {
                onPasteImage(pngData, "image/png")
                return true
            }
        }
        // 2. NSImage fallback (covers screenshots from shortcuts)
        let imageTypes = [NSPasteboard.PasteboardType.png.rawValue,
                          NSPasteboard.PasteboardType.tiff.rawValue,
                          "public.image"]
        if pb.canReadItem(withDataConformingToTypes: imageTypes),
           let image = NSImage(pasteboard: pb),
           let tiffData = image.tiffRepresentation,
           let rep = NSBitmapImageRep(data: tiffData),
           let pngData = rep.representation(using: .png, properties: [:]) {
            onPasteImage(pngData, "image/png")
            return true
        }
        // 3. Copied file URLs from Finder
        if let urls = pb.readObjects(forClasses: [NSURL.self], options: nil) as? [URL] {
            let fileUrls = urls.filter(\.isFileURL)
            if !fileUrls.isEmpty {
                onImportURLs(fileUrls)
                return true
            }
        }
        return false
    }

    private func handle(_ press: KeyPress, _ list: [Suggestion]) -> KeyPress.Result {
        if !list.isEmpty {
            switch press.key {
            case .upArrow: selected = (selected - 1 + list.count) % list.count; return .handled
            case .downArrow: selected = (selected + 1) % list.count; return .handled
            case .escape: dismissed = true; return .handled
            case .return, .tab: apply(list[min(selected, list.count - 1)]); return .handled
            default: return .ignored
            }
        }
        if press.key == .return {
            let send = enterSends ? !press.modifiers.contains(.shift) : press.modifiers.contains(.command)
            if send { onSubmit(); return .handled }
        }
        return .ignored
    }

    private func apply(_ s: Suggestion) {
        guard let tok = token else { return }
        if let insert = s.insert {
            box.text.replaceSubrange(tok.range, with: insert)
        } else {
            box.text.removeSubrange(tok.range)
        }
        s.action?()
    }

    private func popup(_ list: [Suggestion]) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(list.enumerated()), id: \.element.id) { i, s in
                HStack(spacing: 8) {
                    Image(systemName: s.icon).frame(width: 16).foregroundStyle(.secondary)
                    Text(s.title).font(.callout).lineLimit(1)
                    if !s.detail.isEmpty {
                        Text(s.detail).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                    Spacer(minLength: 0)
                    if i == selected {
                        Text("↩").font(.caption2).foregroundStyle(.tertiary)
                    }
                }
                .padding(.horizontal, 8).padding(.vertical, 5)
                .background(i == selected ? Color.accentColor.opacity(0.18) : .clear,
                            in: RoundedRectangle(cornerRadius: 5))
                .contentShape(Rectangle())
                .onTapGesture { apply(s) }
                .onHover { if $0 { selected = i } }
            }
        }
        .padding(4)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(.quaternary))
    }
}

// MARK: - Native AppKit-backed Composer TextView for Reliable Image/File Pasting & Drop

enum KeyPressNav {
    case up, down, escape, tab, enter
}

struct NativeComposerTextView: NSViewRepresentable {
    @Binding var text: String
    var enterSends: Bool
    var onPasteImage: (Data, String) -> Void
    var onImportURLs: ([URL]) -> Void
    var onSubmit: () -> Void
    var onKeyNav: (KeyPressNav) -> Bool

    func makeCoordinator() -> Coordinator {
        Coordinator(self)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = false
        scroll.hasHorizontalScroller = false
        scroll.autohidesScrollers = true

        let tv = ChatNSTextView()
        tv.delegate = context.coordinator
        tv.isRichText = false
        tv.importsGraphics = false
        tv.allowsUndo = true
        tv.drawsBackground = false
        tv.font = NSFont.systemFont(ofSize: 13)
        tv.textColor = NSColor.labelColor
        tv.textContainerInset = NSSize(width: 2, height: 4)
        tv.isHorizontallyResizable = false
        tv.isVerticallyResizable = true
        tv.autoresizingMask = [.width]
        tv.textContainer?.widthTracksTextView = true
        tv.parentView = self

        // Register for Drag & Drop
        tv.registerForDraggedTypes([
            .fileURL,
            .png,
            .tiff
        ])

        scroll.documentView = tv
        context.coordinator.textView = tv
        return scroll
    }

    func updateNSView(_ nsView: NSScrollView, context: Context) {
        guard let tv = nsView.documentView as? ChatNSTextView else { return }
        tv.parentView = self
        if tv.string != text {
            tv.string = text
        }
    }

    class Coordinator: NSObject, NSTextViewDelegate {
        var parent: NativeComposerTextView
        weak var textView: ChatNSTextView?

        init(_ parent: NativeComposerTextView) {
            self.parent = parent
        }

        func textDidChange(_ notification: Notification) {
            guard let tv = notification.object as? NSTextView else { return }
            parent.text = tv.string
        }
    }
}

final class ChatNSTextView: NSTextView {
    var parentView: NativeComposerTextView?

    override func paste(_ sender: Any?) {
        if checkPasteboardForMedia() { return }
        super.paste(sender)
    }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        if event.modifierFlags.contains(.command) && event.charactersIgnoringModifiers == "v" {
            if checkPasteboardForMedia() { return true }
        }
        return super.performKeyEquivalent(with: event)
    }

    private func checkPasteboardForMedia() -> Bool {
        let pb = NSPasteboard.general
        // 1. Direct PNG
        if let png = pb.data(forType: .png) {
            parentView?.onPasteImage(png, "image/png")
            return true
        }
        // 2. TIFF to PNG
        if let tiff = pb.data(forType: .tiff),
           let rep = NSBitmapImageRep(data: tiff),
           let png = rep.representation(using: .png, properties: [:]) {
            parentView?.onPasteImage(png, "image/png")
            return true
        }
        // 3. NSImage fallback (screenshot shortcut Cmd+Ctrl+Shift+4)
        let imageTypes = [NSPasteboard.PasteboardType.png.rawValue,
                          NSPasteboard.PasteboardType.tiff.rawValue,
                          "public.image"]
        if pb.canReadItem(withDataConformingToTypes: imageTypes),
           let image = NSImage(pasteboard: pb),
           let tiffData = image.tiffRepresentation,
           let rep = NSBitmapImageRep(data: tiffData),
           let png = rep.representation(using: .png, properties: [:]) {
            parentView?.onPasteImage(png, "image/png")
            return true
        }
        // 4. Copied file URLs from Finder
        if let urls = pb.readObjects(forClasses: [NSURL.self], options: nil) as? [URL] {
            let files = urls.filter(\.isFileURL)
            if !files.isEmpty {
                parentView?.onImportURLs(files)
                return true
            }
        }
        return false
    }

    override func keyDown(with event: NSEvent) {
        let key = event.keyCode
        // Up Arrow: 126, Down Arrow: 125, Esc: 53, Tab: 48, Return: 36
        if key == 126 {
            if parentView?.onKeyNav(.up) == true { return }
        } else if key == 125 {
            if parentView?.onKeyNav(.down) == true { return }
        } else if key == 53 {
            if parentView?.onKeyNav(.escape) == true { return }
        } else if key == 48 {
            if parentView?.onKeyNav(.tab) == true { return }
        } else if key == 36 {
            if parentView?.onKeyNav(.enter) == true { return }
            let enterSends = parentView?.enterSends ?? true
            if enterSends {
                if event.modifierFlags.contains(.shift) {
                    insertNewlineIgnoringFieldEditor(nil)
                    return
                } else {
                    parentView?.onSubmit()
                    return
                }
            } else {
                if event.modifierFlags.contains(.command) {
                    parentView?.onSubmit()
                    return
                } else {
                    insertNewlineIgnoringFieldEditor(nil)
                    return
                }
            }
        }
        super.keyDown(with: event)
    }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        let pb = sender.draggingPasteboard
        if pb.canReadItem(withDataConformingToTypes: [NSPasteboard.PasteboardType.fileURL.rawValue,
                                                     NSPasteboard.PasteboardType.png.rawValue,
                                                     NSPasteboard.PasteboardType.tiff.rawValue]) {
            return .copy
        }
        return super.draggingEntered(sender)
    }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        let pb = sender.draggingPasteboard
        if let urls = pb.readObjects(forClasses: [NSURL.self], options: nil) as? [URL] {
            let files = urls.filter(\.isFileURL)
            if !files.isEmpty {
                parentView?.onImportURLs(files)
                return true
            }
        }
        if let png = pb.data(forType: .png) {
            parentView?.onPasteImage(png, "image/png")
            return true
        }
        if let tiff = pb.data(forType: .tiff),
           let rep = NSBitmapImageRep(data: tiff),
           let png = rep.representation(using: .png, properties: [:]) {
            parentView?.onPasteImage(png, "image/png")
            return true
        }
        return super.performDragOperation(sender)
    }
}
