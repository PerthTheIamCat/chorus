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
    /// Refused before import; shown red with the reason, never sent.
    @Published var rejected: [RejectedAttachment] = []
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


/// The composer box (variant A, `08-composer-layout`): attachment thumbnails,
/// then the Markdown text, then one control row — attach and the leading
/// controls on the left, dictation, the trailing controls (model pill) and
/// Send/Stop on the right — with a key hint line underneath.
///
/// Enter sends, Shift+Enter is a newline, and `/` or `@` open a completion
/// list driven from the keyboard (↑ ↓ to move, Enter/Tab to pick, Esc to
/// close). Holds the text in its own `ObservableObject` so typing re-renders
/// this view only, not the whole window.
struct ComposerField<Leading: View, Trailing: View>: View {
    @ObservedObject var box: DraftBox
    let suggestions: (_ trigger: Character, _ query: String) -> [Suggestion]
    let onSubmit: () -> Void
    var onImportURLs: ([URL]) -> Void = { _ in }
    var onPasteImage: (Data, String) -> Void = { _, _ in }
    var onRemoveAttachment: (Int64) -> Void = { _ in }
    /// Bump to pull keyboard focus into the field (session opened, turn sent).
    var focusToken = 0
    /// A turn is streaming: Send becomes Stop, Return still queues (steering).
    var isBusy = false
    /// A session exists to send into.
    var canSend = true
    var onStop: () -> Void = {}
    /// Show the info line when attachments narrow the router's choice.
    var routeNoteEnabled = true
    var placeholder = "ขอให้ทำอะไร… ( / คำสั่ง · @ ไฟล์หรือเอเจนต์ · วางจากเว็บได้ จะแปลงเป็น Markdown )"
    @ViewBuilder var leading: () -> Leading
    @ViewBuilder var trailing: () -> Trailing

    @AppStorage("chorus.enterSends") private var enterSends = true
    @State private var selected = 0
    @State private var dismissed = false
    @State private var showingImporter = false
    @State private var dropTargeted = false
    @State private var textHeight: CGFloat = 22
    @State private var focused = false
    @ObservedObject private var voiceDictation = VoiceDictationManager.shared
    @ObservedObject private var thumbnails = AttachmentThumbnails.shared

    /// About 220 px of text, then it scrolls inside (decided in ticket 08).
    private static var maxTextHeight: CGFloat { 220 }

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

    private var hasContent: Bool {
        !box.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !box.attachments.isEmpty
    }

    private var routeNote: String? {
        guard routeNoteEnabled else { return nil }
        return ComposerCore.routeNote(box.attachments.map(\.mediaType))
    }

    var body: some View {
        let list = items
        VStack(alignment: .leading, spacing: 4) {
            if !list.isEmpty { popup(list) }
            VStack(alignment: .leading, spacing: 0) {
                if !box.attachments.isEmpty || !box.rejected.isEmpty { attachmentStrip }
                if let routeNote {
                    Label(routeNote, systemImage: "info.circle")
                        .font(.caption).foregroundStyle(.orange)
                        .padding(.horizontal, 12).padding(.top, 4)
                }
                textArea(list)
                controlRow
            }
            .background(Color(nsColor: .textBackgroundColor).opacity(0.55), in: RoundedRectangle(cornerRadius: 12))
            .overlay {
                RoundedRectangle(cornerRadius: 12)
                    .strokeBorder(focused ? Color.accentColor : Color.primary.opacity(0.12), lineWidth: 1)
            }
            .background {
                if focused {
                    RoundedRectangle(cornerRadius: 12).stroke(Color.accentColor.opacity(0.18), lineWidth: 6)
                }
            }
            .overlay {
                if dropTargeted {
                    RoundedRectangle(cornerRadius: 12)
                        .strokeBorder(Color.accentColor, style: StrokeStyle(lineWidth: 2, dash: [6, 4]))
                        .background(Color.accentColor.opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
                        .allowsHitTesting(false)
                }
            }
            .onDrop(of: [UTType.fileURL, UTType.url, UTType.image], isTargeted: $dropTargeted, perform: receiveDrop)
            hintLine
        }
        .fileImporter(isPresented: $showingImporter, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            if case let .success(urls) = result, !urls.isEmpty { onImportURLs(urls) }
        }
        .onChange(of: box.text) { _, _ in selected = 0; dismissed = false }
    }

    private func textArea(_ list: [Suggestion]) -> some View {
        ZStack(alignment: .topLeading) {
            if box.text.isEmpty {
                Text(placeholder).foregroundStyle(.tertiary)
                    .font(.system(size: 13))
                    .lineLimit(1)
                    .padding(.leading, 5).padding(.top, 4)
                    .allowsHitTesting(false)
            }
            NativeComposerTextView(
                text: $box.text,
                height: $textHeight,
                focused: $focused,
                enterSends: enterSends,
                onPasteImage: onPasteImage,
                onImportURLs: onImportURLs,
                onSubmit: onSubmit,
                focusToken: focusToken,
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
            .frame(height: min(max(textHeight, 22), Self.maxTextHeight))
        }
        .padding(.horizontal, 8).padding(.top, 7)
    }

    private var controlRow: some View {
        HStack(spacing: 4) {
            Button { showingImporter = true } label: {
                Image(systemName: "plus").font(.system(size: 13, weight: .medium)).frame(width: 26, height: 26)
            }
            .buttonStyle(InteractiveStyle(cornerRadius: 7, padding: .init()))
            .foregroundStyle(.secondary)
            .help("แนบไฟล์หรือรูปภาพ")
            .accessibilityLabel("แนบไฟล์")
            leading()
            Spacer(minLength: 4)
            Button {
                voiceDictation.toggleRecording { transcribed in
                    if !transcribed.isEmpty { box.text = transcribed }
                }
            } label: {
                Image(systemName: voiceDictation.isRecording ? "waveform.circle.fill" : "mic")
                    .font(.system(size: 13))
                    .foregroundStyle(voiceDictation.isRecording ? Color.red : .secondary)
                    .frame(width: 26, height: 26)
            }
            .buttonStyle(InteractiveStyle(cornerRadius: 7, padding: .init()))
            .help(voiceDictation.isRecording ? "กำลังฟัง… กดอีกครั้งเพื่อหยุด" : "พิมพ์ด้วยเสียง (ไทย/อังกฤษ)")
            .accessibilityLabel("พิมพ์ด้วยเสียง")
            trailing()
            sendButton
        }
        .padding(.horizontal, 6).padding(.top, 2).padding(.bottom, 6)
    }

    @ViewBuilder private var sendButton: some View {
        if isBusy {
            Button(action: onStop) {
                Image(systemName: "stop.fill")
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(Color(nsColor: .textBackgroundColor))
                    .frame(width: 28, height: 28)
                    .background(Color.primary, in: RoundedRectangle(cornerRadius: 8))
            }
            .buttonStyle(.plain)
            .help("หยุด (⌘.) — ↩ ยังส่งข้อความเข้าคิวได้")
            .accessibilityLabel("หยุด")
        } else {
            Button(action: onSubmit) {
                Image(systemName: "arrow.up")
                    .font(.system(size: 13, weight: .bold))
                    .foregroundStyle(.white)
                    .frame(width: 28, height: 28)
                    .background(Color.accentColor, in: RoundedRectangle(cornerRadius: 8))
            }
            .buttonStyle(.plain)
            .disabled(!canSend || !hasContent)
            .opacity(canSend && hasContent ? 1 : 0.35)
            .help(enterSends ? "ส่ง (↩)" : "ส่ง (⌘↩)")
            .accessibilityLabel("ส่ง")
        }
    }

    private var hintLine: some View {
        HStack(spacing: 10) {
            if enterSends {
                keyHint("↩", "ส่ง"); keyHint("⇧↩", "ขึ้นบรรทัด")
            } else {
                keyHint("⌘↩", "ส่ง"); keyHint("↩", "ขึ้นบรรทัด")
            }
            keyHint("⌘V", "วางรูป/ไฟล์")
            if isBusy { keyHint("⌘.", "หยุด") }
            Spacer(minLength: 0)
        }
        .font(.caption2)
        .foregroundStyle(.tertiary)
        .padding(.horizontal, 4)
        .accessibilityHidden(true)
    }

    private func keyHint(_ key: String, _ label: String) -> some View {
        HStack(spacing: 3) {
            Text(key)
                .padding(.horizontal, 4)
                .overlay(RoundedRectangle(cornerRadius: 3).stroke(Color.primary.opacity(0.18)))
            Text(label)
        }
    }

    private var attachmentStrip: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 8) {
                ForEach(box.attachments) { a in
                    AttachmentChip(attachment: a, thumbnail: thumbnails.images[a.id],
                                   realPath: thumbnails.paths[a.id]) {
                        onRemoveAttachment(a.id)
                    }
                }
                ForEach(box.rejected) { r in
                    RejectedChip(item: r) { box.rejected.removeAll { $0.id == r.id } }
                }
            }
            .padding(.horizontal, 10).padding(.top, 8)
        }
        .scrollIndicators(.hidden)
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
                    if let url, url.isFileURL { DispatchQueue.main.async { self.onImportURLs([url]) } }
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

// MARK: - Attachment chips

private func byteString(_ n: Int64) -> String {
    ByteCountFormatter.string(fromByteCount: n, countStyle: .file)
}

/// Image: a real thumbnail. PDF: its first page with a red tag. Anything
/// else: a chip with its QuickLook icon, name and size. A preview never
/// opens or runs the file.
private struct AttachmentChip: View {
    let attachment: DraftAttachment
    let thumbnail: NSImage?
    let realPath: String?
    let onRemove: () -> Void
    @State private var hover = false

    private var isImage: Bool { attachment.mediaType.hasPrefix("image/") }
    private var isPDF: Bool { attachment.mediaType == "application/pdf" }

    var body: some View {
        content
            .clipShape(RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.primary.opacity(0.12)))
            .overlay(alignment: .topTrailing) {
                Button(action: onRemove) {
                    Image(systemName: "xmark.circle.fill")
                        .symbolRenderingMode(.palette)
                        .foregroundStyle(.white, .black.opacity(0.6))
                        .font(.system(size: 14))
                }
                .buttonStyle(.plain)
                .padding(2)
                .opacity(hover ? 1 : 0)
                .accessibilityLabel("เอา \(attachment.name) ออก")
            }
            .overlay(alignment: .bottomLeading) {
                if !attachment.agentReadable {
                    Image(systemName: "exclamationmark.circle.fill")
                        .foregroundStyle(.orange).padding(3)
                }
            }
            .onHover { hover = $0 }
            .help(helpText)
            .accessibilityElement(children: .contain)
            .accessibilityLabel("\(attachment.name), \(byteString(attachment.size)), \(attachment.mediaType)")
    }

    private var helpText: String {
        var parts = [attachment.name, byteString(attachment.size)]
        if let realPath { parts.append(realPath) }
        if !attachment.agentReadable { parts.append("agent ยังอ่านไฟล์นี้ไม่ได้ — ลบออกก่อนส่ง") }
        return parts.joined(separator: " · ")
    }

    @ViewBuilder private var content: some View {
        if isImage {
            thumb(fallback: "photo").frame(width: 64, height: 64)
        } else if isPDF {
            thumb(fallback: "doc.richtext")
                .frame(width: 52, height: 64)
                .background(Color.white)
                .overlay(alignment: .bottom) {
                    Text("PDF").font(.system(size: 9, weight: .bold)).foregroundStyle(.white)
                        .frame(maxWidth: .infinity).frame(height: 14).background(Color.red)
                }
        } else {
            HStack(spacing: 8) {
                Group {
                    if let thumbnail {
                        Image(nsImage: thumbnail).resizable().aspectRatio(contentMode: .fit)
                    } else {
                        Image(systemName: "doc").foregroundStyle(.secondary)
                    }
                }
                .frame(width: 24, height: 24)
                VStack(alignment: .leading, spacing: 1) {
                    Text(attachment.name).font(.caption).lineLimit(1)
                    Text(byteString(attachment.size)).font(.caption2).foregroundStyle(.secondary)
                }
                .frame(maxWidth: 140, alignment: .leading)
            }
            .padding(.leading, 8).padding(.trailing, 12)
            .frame(height: 64)
            .background(Color.primary.opacity(0.05))
        }
    }

    @ViewBuilder private func thumb(fallback: String) -> some View {
        if let thumbnail {
            Image(nsImage: thumbnail).resizable().aspectRatio(contentMode: .fill)
        } else {
            Image(systemName: fallback).font(.system(size: 20)).foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(Color.primary.opacity(0.05))
        }
    }
}

private struct RejectedChip: View {
    let item: RejectedAttachment
    let onRemove: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.red)
            VStack(alignment: .leading, spacing: 1) {
                Text(item.name).font(.caption).lineLimit(1)
                Text([item.size.map(byteString), item.reason].compactMap { $0 }.joined(separator: " · "))
                    .font(.caption2).foregroundStyle(.red).lineLimit(2)
            }
            .frame(maxWidth: 180, alignment: .leading)
            Button(action: onRemove) { Image(systemName: "xmark.circle.fill").foregroundStyle(.tertiary) }
                .buttonStyle(.plain)
                .accessibilityLabel("ปิด")
        }
        .padding(.horizontal, 8)
        .frame(height: 64)
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.red.opacity(0.8)))
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(item.name) ไม่ถูกแนบ: \(item.reason)")
    }
}

// MARK: - Control row (issue 14 wires Orchestra)

/// Mode: `Orchestra` (issue 14, not wired yet) or `Auto route`. Choosing a
/// model by hand turns Auto route off. The route preset chip sits next to
/// it in ContentView.
struct ComposerModeMenu: View {
    @Binding var autoRoute: Bool
    var openOrchestraSettings: () -> Void = {}

    var body: some View {
        Menu {
            Button {} label: { Text("Orchestra — เร็ว ๆ นี้") }.disabled(true)
            Toggle("Auto route", isOn: $autoRoute)
            Toggle("เลือกโมเดลเอง", isOn: Binding(get: { !autoRoute }, set: { autoRoute = !$0 }))
            Button("ตั้งค่าออร์เคสตรา…", action: openOrchestraSettings)
        } label: {
            Text(autoRoute ? "Auto route" : "เลือกเอง").font(.caption.weight(.medium))
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
        .padding(.horizontal, 8).padding(.vertical, 4)
        .background(autoRoute ? Color.accentColor.opacity(0.14) : Color.primary.opacity(0.06),
                    in: RoundedRectangle(cornerRadius: 7))
        .help(autoRoute ? "ให้ router เลือกเอเจนต์ โมเดล และระดับคิด" : "ใช้โมเดลที่เลือกในปุ่มโมเดลทางขวา")
    }
}

/// Route preset. Labels are decided (ticket 12); the router reads the
/// choice once issue 09 lands — until then it is stored only.
// MARK: - Native AppKit-backed text view

enum KeyPressNav {
    case up, down, escape, tab, enter
}

struct NativeComposerTextView: NSViewRepresentable {
    @Binding var text: String
    @Binding var height: CGFloat
    @Binding var focused: Bool
    var enterSends: Bool
    var onPasteImage: (Data, String) -> Void
    var onImportURLs: ([URL]) -> Void
    var onSubmit: () -> Void
    var focusToken: Int
    var onKeyNav: (KeyPressNav) -> Bool

    func makeCoordinator() -> Coordinator {
        Coordinator(self)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = false
        scroll.autohidesScrollers = true
        scroll.scrollerStyle = .overlay

        let tv = ChatNSTextView()
        tv.delegate = context.coordinator
        tv.isRichText = false
        tv.importsGraphics = false
        tv.allowsUndo = true
        tv.drawsBackground = false
        tv.font = MarkdownStyler.baseFont
        tv.textColor = NSColor.labelColor
        tv.typingAttributes = MarkdownStyler.baseAttributes
        tv.textContainerInset = NSSize(width: 2, height: 3)
        tv.isHorizontallyResizable = false
        tv.isVerticallyResizable = true
        tv.autoresizingMask = [.width]
        // Fill the visible height even when the text is one line, so a click
        // anywhere in the box lands on the text view, not the bare clip view.
        tv.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        tv.textContainer?.widthTracksTextView = true
        tv.isAutomaticQuoteSubstitutionEnabled = false
        tv.isAutomaticDashSubstitutionEnabled = false
        tv.parentView = self

        tv.registerForDraggedTypes([.fileURL, .png, .tiff]
            + NSFilePromiseReceiver.readableDraggedTypes.map { NSPasteboard.PasteboardType($0) })

        scroll.documentView = tv
        context.coordinator.textView = tv
        // Re-measure on width changes: wrapping changes the height.
        scroll.contentView.postsFrameChangedNotifications = true
        NotificationCenter.default.addObserver(
            context.coordinator, selector: #selector(Coordinator.frameChanged),
            name: NSView.frameDidChangeNotification, object: scroll.contentView)
        return scroll
    }

    func updateNSView(_ nsView: NSScrollView, context: Context) {
        guard let tv = nsView.documentView as? ChatNSTextView else { return }
        tv.parentView = self
        context.coordinator.parent = self
        if tv.string != text, !tv.hasMarkedText() {
            tv.string = text
            MarkdownStyler.apply(to: tv)
            context.coordinator.measure()
        }
        if context.coordinator.focusToken != focusToken {
            context.coordinator.focusToken = focusToken
            // Next runloop: the field may not be in a window yet on the
            // update that created it.
            DispatchQueue.main.async { tv.focusIfPossible() }
        }
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: NativeComposerTextView
        weak var textView: ChatNSTextView?
        var focusToken = Int.min

        init(_ parent: NativeComposerTextView) {
            self.parent = parent
        }

        func textDidChange(_ notification: Notification) {
            guard let tv = notification.object as? NSTextView else { return }
            parent.text = tv.string
            MarkdownStyler.apply(to: tv)
            measure()
        }

        @objc func frameChanged(_ note: Notification) {
            if let clip = note.object as? NSClipView, let tv = textView {
                tv.minSize = NSSize(width: 0, height: clip.bounds.height)
                if tv.frame.height < clip.bounds.height { tv.setFrameSize(NSSize(width: tv.frame.width, height: clip.bounds.height)) }
            }
            measure()
        }

        /// Content height → SwiftUI, which clamps it to the 220 px cap.
        func measure() {
            guard let tv = textView, let lm = tv.layoutManager, let tc = tv.textContainer else { return }
            lm.ensureLayout(for: tc)
            let h = ceil(lm.usedRect(for: tc).height + tv.textContainerInset.height * 2)
            guard abs(h - parent.height) > 0.5 else { return }
            DispatchQueue.main.async { self.parent.height = h }
        }
    }
}

final class ChatNSTextView: NSTextView {
    var parentView: NativeComposerTextView?
    /// File promises (Photos, Mail, Safari drags) land here, private to
    /// this user, then import like any dropped file.
    private static let promiseQueue: OperationQueue = {
        let q = OperationQueue()
        q.qualityOfService = .userInitiated
        return q
    }()

    override func becomeFirstResponder() -> Bool {
        let ok = super.becomeFirstResponder()
        if ok { setFocused(true) }
        return ok
    }

    override func resignFirstResponder() -> Bool {
        let ok = super.resignFirstResponder()
        if ok { setFocused(false) }
        return ok
    }

    private func setFocused(_ value: Bool) {
        DispatchQueue.main.async { [weak self] in self?.parentView?.focused = value }
    }

    /// Paste order (research 04 §3): file URLs → image data → HTML → RTF →
    /// plain text. Read only inside the user's paste, so macOS's pasteboard
    /// privacy prompt never fires.
    override func paste(_ sender: Any?) {
        let pb = NSPasteboard.general
        if attachMedia(from: pb) { return }
        if pasteRich(from: pb) { return }
        super.paste(sender)
    }

    /// A plain-text view disables Paste when the pasteboard holds only a
    /// file or an image; this one attaches those. Checks types only — no
    /// read, so no pasteboard-privacy prompt.
    override func validateUserInterfaceItem(_ item: NSValidatedUserInterfaceItem) -> Bool {
        if item.action == #selector(paste(_:)), isEditable {
            let pb = NSPasteboard.general
            if pb.availableType(from: [.fileURL]) != nil
                || (pb.types ?? []).contains(where: { UTType($0.rawValue)?.conforms(to: .image) == true }) {
                return true
            }
        }
        return super.validateUserInterfaceItem(item)
    }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        // ⌘V only when this field has focus: key equivalents walk the whole
        // view tree, so without the check a paste elsewhere landed here.
        if window?.firstResponder === self,
           event.modifierFlags.intersection(.deviceIndependentFlagsMask) == .command,
           event.charactersIgnoringModifiers == "v" {
            paste(nil)
            return true
        }
        return super.performKeyEquivalent(with: event)
    }

    private func attachMedia(from pb: NSPasteboard) -> Bool {
        // Finder puts the file's *icon* on the pasteboard next to its URL,
        // so file URLs must win or a copied file arrives as a picture.
        if let urls = pb.readObjects(forClasses: [NSURL.self],
                                     options: [.urlReadingFileURLsOnly: true]) as? [URL],
           !urls.isEmpty {
            parentView?.onImportURLs(urls)
            return true
        }
        if let (data, mime) = Self.imageData(pb) {
            parentView?.onPasteImage(data, mime)
            return true
        }
        return false
    }

    /// The first image flavour as raw bytes. Not decoded here: the engine
    /// normalises, and the chip thumbnails through ImageIO.
    private static func imageData(_ pb: NSPasteboard) -> (Data, String)? {
        for type in pb.types ?? [] {
            guard let ut = UTType(type.rawValue), ut.conforms(to: .image),
                  let data = pb.data(forType: type) else { continue }
            return (data, ut.preferredMIMEType ?? "image/png")
        }
        return nil
    }

    /// Rich text → Markdown in the core (`htmd`, capped; falls back to the
    /// plain flavour). RTF-only sources go through AppKit's RTF importer to
    /// HTML first — never `NSAttributedString(html:)`, which can fetch.
    private func pasteRich(from pb: NSPasteboard) -> Bool {
        let plain = pb.string(forType: .string) ?? ""
        var html = pb.string(forType: .html)
        if html == nil, let rtf = pb.data(forType: .rtf),
           let attributed = NSAttributedString(rtf: rtf, documentAttributes: nil),
           let data = try? attributed.data(from: NSRange(location: 0, length: attributed.length),
                                           documentAttributes: [.documentType: NSAttributedString.DocumentType.html]) {
            html = String(data: data, encoding: .utf8)
        }
        guard let html, let markdown = ComposerCore.htmlToMarkdown(html, plain: plain), !markdown.isEmpty else {
            return false
        }
        // insertText goes through the normal edit path: one undo step
        // (⌘Z restores what was there) and a textDidChange.
        insertText(markdown, replacementRange: rangeForUserTextChange)
        return true
    }

    /// Become first responder once the field is in a key window, unless the
    /// user is typing somewhere else (a sheet, search field, terminal).
    func focusIfPossible() {
        guard let window, window.attachedSheet == nil else { return }
        if let other = window.firstResponder as? NSText, other !== self, other.isEditable { return }
        window.makeFirstResponder(self)
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil { DispatchQueue.main.async { [weak self] in self?.focusIfPossible() } }
    }

    override func keyDown(with event: NSEvent) {
        let key = event.keyCode
        // IME composition (Thai, Japanese, …): Return/arrows/Esc belong to the
        // input method — commit or navigate the candidate, never send.
        if hasMarkedText() {
            super.keyDown(with: event)
            return
        }
        // Up Arrow: 126, Down Arrow: 125, Esc: 53, Tab: 48, Return: 36
        if key == 126 {
            if parentView?.onKeyNav(.up) == true { return }
        } else if key == 125 {
            if parentView?.onKeyNav(.down) == true { return }
        } else if key == 53 {
            if parentView?.onKeyNav(.escape) == true { return }
        } else if key == 48 {
            if parentView?.onKeyNav(.tab) == true { return }
        } else if key == 36 || key == 76 { // Return, keypad Enter
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
        let types = [NSPasteboard.PasteboardType.fileURL.rawValue,
                     NSPasteboard.PasteboardType.png.rawValue,
                     NSPasteboard.PasteboardType.tiff.rawValue] + NSFilePromiseReceiver.readableDraggedTypes
        if pb.canReadItem(withDataConformingToTypes: types) { return .copy }
        return super.draggingEntered(sender)
    }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        let pb = sender.draggingPasteboard
        if let urls = pb.readObjects(forClasses: [NSURL.self],
                                     options: [.urlReadingFileURLsOnly: true]) as? [URL],
           !urls.isEmpty {
            parentView?.onImportURLs(urls)
            return true
        }
        if let promises = pb.readObjects(forClasses: [NSFilePromiseReceiver.self]) as? [NSFilePromiseReceiver],
           !promises.isEmpty {
            let dir = FileManager.default.temporaryDirectory
                .appendingPathComponent("chorus-drops/\(UUID().uuidString)", isDirectory: true)
            try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            for promise in promises {
                promise.receivePromisedFiles(atDestination: dir, options: [:], operationQueue: Self.promiseQueue) { [weak self] url, error in
                    guard error == nil else { return }
                    DispatchQueue.main.async { self?.parentView?.onImportURLs([url]) }
                }
            }
            return true
        }
        if let (data, mime) = Self.imageData(pb) {
            parentView?.onPasteImage(data, mime)
            return true
        }
        return super.performDragOperation(sender)
    }
}
