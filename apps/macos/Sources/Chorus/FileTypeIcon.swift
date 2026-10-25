import AppKit
import SwiftUI

/// File-tree glyph for a file. The core picks the key (`FileNode.icon`); this
/// view only maps it to the bundled Catppuccin SVG for the current appearance
/// (Latte light, Mocha dark) and falls back to the generic `doc` symbol.
/// Workspace tree only, never app branding: the glyphs depict third-party marks.
struct FileTypeIcon: View {
    let key: String?
    var size: CGFloat = 13
    @Environment(\.colorScheme) private var colorScheme

    private var image: NSImage? {
        guard let key else { return nil }
        let flavour = colorScheme == .dark ? "mocha" : "latte"
        return Self.load("fi-\(flavour)-\(key)")
    }

    var body: some View {
        Group {
            if let image {
                // Colour glyphs: no isTemplate, which would flatten them.
                Image(nsImage: image)
                    .resizable()
                    .interpolation(.high)
                    .aspectRatio(contentMode: .fit)
            } else {
                Image(systemName: "doc")
                    .font(.system(size: 12))
                    .foregroundColor(.secondary)
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }

    private static var cache: [String: NSImage] = [:]

    private static func load(_ asset: String) -> NSImage? {
        if let hit = cache[asset] { return hit }
        guard let path = ChorusResources.path(forResource: asset, ofType: "svg"),
              let image = NSImage(contentsOfFile: path) else { return nil }
        cache[asset] = image
        return image
    }
}

/// Standard About panel plus the credit MIT asks for the bundled file icons.
struct AboutCommand: Commands {
    var body: some Commands {
        CommandGroup(replacing: .appInfo) {
            Button("About Chorus") {
                let credits = NSAttributedString(
                    string: "File icons: Catppuccin Icons for VS Code v1.26.0\n© 2023 Catppuccin, © 2023 thang-nm · MIT License\nhttps://github.com/catppuccin/vscode-icons",
                    attributes: [
                        .font: NSFont.systemFont(ofSize: NSFont.smallSystemFontSize),
                        .foregroundColor: NSColor.secondaryLabelColor,
                    ]
                )
                NSApp.orderFrontStandardAboutPanel(options: [.credits: credits])
            }
        }
    }
}
