import AppKit
import SwiftUI

/// Provider marks bundled locally: Claude and Gemini from Simple Icons (CC0),
/// OpenAI from Iconify's logos set, Antigravity from LobeHub's lobe-icons (MIT) — see
/// docs/provider-icons.md. The marks themselves remain their owners'
/// trademarks and are used only to say which provider a row belongs to.
/// Model names stay visible beside the mark, so the icon identifies the family
/// while text identifies the selected model.
struct ProviderIcon: View {
    let provider: String
    var size: CGFloat = 15

    private var key: String {
        let raw = provider.lowercased()
        let withoutColon = raw.split(separator: ":", maxSplits: 1).first.map(String.init) ?? raw
        return withoutColon.split(separator: "@", maxSplits: 1).first.map(String.init) ?? withoutColon
    }

    private var asset: String? {
        switch key {
        case "codex", "openai": "provider-openai"
        case "claude", "anthropic": "provider-claude"
        case "antigravity", "agy": "provider-antigravity"
        case "gemini", "google": "provider-gemini"
        default: nil
        }
    }

    /// Each mark is decoded once. `body` runs on every redraw, and reading
    /// an SVG from disk there showed up in long lists of rows.
    private static let cache = NSCache<NSString, NSImage>()

    /// The brand's own accent, for tiles and tints.
    static func brandColor(for provider: String) -> Color {
        let raw = provider.lowercased()
        let base = raw.split(separator: ":", maxSplits: 1).first.map(String.init) ?? raw
        switch base.split(separator: "@", maxSplits: 1).first.map(String.init) ?? base {
        case "claude", "anthropic": return Color(hex: "#D97757")
        case "codex", "openai": return Color(hex: "#10A37F")
        case "antigravity", "agy": return Color(hex: "#4285F4")
        case "gemini", "google": return Color(hex: "#4285F4")
        default: return .gray
        }
    }

    private var fallback: String {
        switch key {
        case "codex", "openai": "circle.hexagongrid"
        case "claude", "anthropic": "sparkle"
        case "antigravity", "agy": "atom"
        case "gemini", "google": "sparkles"
        default: "cpu"
        }
    }

    private var providerImage: NSImage? {
        guard let asset else { return nil }
        if let hit = Self.cache.object(forKey: asset as NSString) { return hit }
        guard let path = ChorusResources.path(forResource: asset, ofType: "svg"),
              let image = NSImage(contentsOfFile: path) else { return nil }
        image.isTemplate = true
        Self.cache.setObject(image, forKey: asset as NSString)
        return image
    }

    /// The mark as a plain `Image` already at `size` points. Menu and toolbar
    /// labels are flattened to text + image by AppKit, which ignores a view's
    /// `.frame` and draws the SVG at its own (large) size; this one keeps its size.
    static func menuImage(_ provider: String, size: CGFloat = 14) -> Image {
        let icon = ProviderIcon(provider: provider, size: size)
        guard let source = icon.providerImage, let copy = source.copy() as? NSImage else {
            return Image(systemName: icon.fallback)
        }
        copy.size = NSSize(width: size, height: size)
        copy.isTemplate = true
        return Image(nsImage: copy)
    }

    var body: some View {
        Group {
            if let image = providerImage {
                Image(nsImage: image)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
                    .foregroundStyle(.primary)
            } else {
                Image(systemName: fallback)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
                    .foregroundStyle(.secondary)
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}
