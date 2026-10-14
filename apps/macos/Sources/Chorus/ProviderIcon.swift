import AppKit
import SwiftUI

/// Provider marks from Iconify's logos set and Simple Icons, bundled locally.
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
        case "antigravity", "agy": "provider-gemini"
        case "gemini", "google": "provider-gemini"
        default: nil
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
        guard let asset, let path = Bundle.module.path(forResource: asset, ofType: "svg"),
              let image = NSImage(contentsOfFile: path) else { return nil }
        image.isTemplate = true
        return image
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
