import AppKit
import CChorus
import ImageIO
import QuickLookThumbnailing
import SwiftUI

/// Composer calls into the core (`chorus-ffi/src/composer.rs`). Pure and
/// fast, so they run inline on the main thread.
enum ComposerCore {
    struct Converted: Decodable { let ok: Bool; let markdown: String?; let fallback: String? }
    struct Span: Decodable { let start: Int; let end: Int; let kind: String; let token: Int? }
    private struct Spans: Decodable { let ok: Bool; let spans: [Span]? }
    private struct Note: Decodable { let ok: Bool; let note: String? }

    /// Pasted HTML → Markdown; the core falls back to `plain` past its caps.
    static func htmlToMarkdown(_ html: String, plain: String) -> String? {
        let json = html.withCString { h in plain.withCString { p in string(chorus_html_to_markdown(h, p)) } }
        return decode(Converted.self, json)?.markdown
    }

    static func spans(_ text: String) -> [Span] {
        decode(Spans.self, text.withCString { string(chorus_markdown_spans($0)) })?.spans ?? []
    }

    static func routeNote(_ mediaTypes: [String]) -> String? {
        guard !mediaTypes.isEmpty,
              let data = try? JSONEncoder().encode(mediaTypes),
              let arg = String(data: data, encoding: .utf8) else { return nil }
        return decode(Note.self, arg.withCString { string(chorus_attachment_route_note($0)) })?.note
    }

    private static func string(_ p: UnsafeMutablePointer<CChar>?) -> String {
        guard let p else { return "" }
        defer { chorus_string_free(p) }
        return String(cString: p)
    }

    private static func decode<T: Decodable>(_ type: T.Type, _ json: String) -> T? {
        try? JSONDecoder().decode(T.self, from: Data(json.utf8))
    }
}

/// Paints the core's Markdown spans onto the draft. Characters are never
/// changed — markers are dimmed, not hidden — so the text stays the source.
@MainActor
enum MarkdownStyler {
    static let baseFont = NSFont.systemFont(ofSize: 13)
    private static let mono = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
    private static let codeBackground = NSColor.secondaryLabelColor.withAlphaComponent(0.1)
    private static var tokenColors: [NSColor] { CodeTheme.colors.map { NSColor($0) } }

    static var baseAttributes: [NSAttributedString.Key: Any] {
        [.font: baseFont, .foregroundColor: NSColor.labelColor]
    }

    static func apply(to tv: NSTextView) {
        // Restyling mid-composition (Thai, Japanese IME) breaks the marked
        // range; the next change after commit restyles.
        guard !tv.hasMarkedText(), let storage = tv.textStorage else { return }
        let length = storage.length
        let spans = ComposerCore.spans(storage.string)
        let colors = tokenColors
        storage.beginEditing()
        storage.setAttributes(baseAttributes, range: NSRange(location: 0, length: length))
        // Markers last, so no container colour paints over them.
        for span in spans.filter({ $0.kind != "marker" }) + spans.filter({ $0.kind == "marker" }) {
            guard span.start >= 0, span.start < span.end, span.end <= length else { continue }
            let r = NSRange(location: span.start, length: span.end - span.start)
            switch span.kind {
            case "heading1": setFont(storage, r) { _ in .systemFont(ofSize: 17, weight: .bold) }
            case "heading2": setFont(storage, r) { _ in .systemFont(ofSize: 15, weight: .bold) }
            case "heading3": setFont(storage, r) { _ in .systemFont(ofSize: 13.5, weight: .semibold) }
            case "strong": setFont(storage, r) { NSFontManager.shared.convert($0, toHaveTrait: .boldFontMask) }
            case "emphasis": setFont(storage, r) { NSFontManager.shared.convert($0, toHaveTrait: .italicFontMask) }
            case "strike": storage.addAttribute(.strikethroughStyle, value: NSUnderlineStyle.single.rawValue, range: r)
            case "inline_code", "code_block":
                storage.addAttributes([.font: mono, .backgroundColor: codeBackground], range: r)
            case "link":
                // Underlined text only: no `.link`, so nothing in a draft is clickable.
                storage.addAttributes([.underlineStyle: NSUnderlineStyle.single.rawValue,
                                       .foregroundColor: NSColor.linkColor], range: r)
            case "quote": storage.addAttribute(.foregroundColor, value: NSColor.secondaryLabelColor, range: r)
            case "token":
                if let t = span.token, colors.indices.contains(t) {
                    storage.addAttribute(.foregroundColor, value: colors[t], range: r)
                }
            case "marker": storage.addAttribute(.foregroundColor, value: NSColor.tertiaryLabelColor, range: r)
            default: break
            }
        }
        storage.endEditing()
        tv.typingAttributes = baseAttributes
    }

    private static func setFont(_ storage: NSTextStorage, _ range: NSRange, _ f: (NSFont) -> NSFont) {
        storage.enumerateAttribute(.font, in: range) { value, sub, _ in
            storage.addAttribute(.font, value: f((value as? NSFont) ?? baseFont), range: sub)
        }
    }
}

/// A file refused before import (too big, unreadable). Shown red with the
/// reason until the user removes it or sends.
struct RejectedAttachment: Identifiable, Equatable {
    let id = UUID()
    let name: String
    let size: Int64?
    let reason: String
}

/// Chip thumbnails. In memory only (research 03 §4: never a persistent cache
/// outside `~/.chorus`), keyed by attachment id. Never decodes a full image:
/// QuickLook for file URLs, ImageIO with a max pixel size for raw bytes.
@MainActor
final class AttachmentThumbnails: ObservableObject {
    static let shared = AttachmentThumbnails()
    @Published private(set) var images: [Int64: NSImage] = [:]
    /// Resolved real path (symlinks, aliases) for the chip tooltip.
    @Published private(set) var paths: [Int64: String] = [:]

    nonisolated private static let pixels: CGFloat = 128

    func load(id: Int64, url: URL) {
        paths[id] = url.resolvingSymlinksInPath().path
        let access = url.startAccessingSecurityScopedResource()
        let scale = NSScreen.main?.backingScaleFactor ?? 2
        let request = QLThumbnailGenerator.Request(
            fileAt: url, size: CGSize(width: Self.pixels / 2, height: Self.pixels / 2),
            scale: scale, representationTypes: .all)
        request.iconMode = false
        // Icon first, then the real thumbnail when QuickLook has it.
        QLThumbnailGenerator.shared.generateRepresentations(for: request) { rep, type, _ in
            let image = rep?.nsImage
            DispatchQueue.main.async {
                if let image { self.images[id] = image }
                if access, type == .thumbnail { url.stopAccessingSecurityScopedResource() }
            }
        }
    }

    func load(id: Int64, data: Data) {
        Task.detached(priority: .userInitiated) {
            let options: [CFString: Any] = [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceShouldCacheImmediately: true,
                kCGImageSourceThumbnailMaxPixelSize: Self.pixels,
            ]
            guard let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
                  let cg = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return }
            let image = NSImage(cgImage: cg, size: NSSize(width: cg.width / 2, height: cg.height / 2))
            await MainActor.run { self.images[id] = image }
        }
    }

    func forget(_ id: Int64) {
        images[id] = nil
        paths[id] = nil
    }
}
