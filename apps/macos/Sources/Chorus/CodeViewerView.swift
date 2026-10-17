import SwiftUI
import AppKit

/// High-performance & Beautiful Code Viewer (Center Stage Split).
/// Features SF Mono typography, syntax highlighting, gutter line numbers,
/// file breadcrumbs, and one-click external editor hand-off.
struct CodeViewerView: View {
    let filePath: String
    let cwd: String
    var isMaximized: Bool = false
    var onToggleMaximize: (() -> Void)? = nil
    var onClose: () -> Void

    @State private var fileContent: String = ""
    @State private var lines: [String] = []
    /// Coloured lines, built off the main thread from the core's spans.
    /// Empty until ready; rows fall back to plain text meanwhile.
    @State private var highlighted: [AttributedString] = []
    @State private var loadToken = UUID()
    @State private var hoveredLine: Int? = nil
    @State private var selectedLine: Int? = nil
    @State private var detectedEditor: String = "VS Code"
    @State private var copiedToClipboard: Bool = false

    private var fileExtension: String {
        (filePath as NSString).pathExtension.lowercased()
    }

    private var fileIcon: (name: String, color: Color) {
        switch fileExtension {
        case "rs": return ("shippingbox.fill", .orange)
        case "swift": return ("swift", .red)
        case "toml", "yaml", "yml": return ("gearshape.fill", .blue)
        case "json": return ("curlybraces", .yellow)
        case "md": return ("doc.plaintext.fill", .purple)
        case "ts", "js": return ("chevron.left.forwardslash.chevron.right", .green)
        case "sh", "zsh": return ("terminal.fill", .teal)
        default: return ("doc.text.fill", .secondary)
        }
    }

    private var breadcrumbParts: [String] {
        filePath.components(separatedBy: "/")
    }

    var body: some View {
        VStack(spacing: 0) {
            // Header Bar
            headerBar

            Divider().opacity(0.6)

            // Code Content with Gutter & Monospace Text
            ScrollView([.vertical, .horizontal]) {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(lines.enumerated()), id: \.offset) { idx, rawLine in
                        let lineNum = idx + 1
                        CodeLineRow(
                            lineNum: lineNum,
                            lineText: rawLine,
                            highlighted: idx < highlighted.count ? highlighted[idx] : nil,
                            isHovered: hoveredLine == lineNum,
                            isSelected: selectedLine == lineNum,
                            onHover: { h in
                                if h { hoveredLine = lineNum }
                                else if hoveredLine == lineNum { hoveredLine = nil }
                            },
                            onSelect: {
                                selectedLine = lineNum
                            }
                        )
                    }
                }
                .padding(.vertical, 8)
            }
            .chorusScrollIndicators()
            .background(Color.primary.opacity(0.02))
        }
        .onAppear {
            loadFile()
            detectInstalledEditor()
        }
        .onChange(of: filePath) { _, _ in
            loadFile()
        }
    }

    // MARK: - Header Bar
    private var headerBar: some View {
        HStack(spacing: 10) {
            // Back to Chat button
            Button(action: onClose) {
                HStack(spacing: 4) {
                    Image(systemName: "chevron.left")
                        .font(.system(size: 11, weight: .semibold))
                    Text("กลับไปที่แชท")
                        .font(.system(size: 11, weight: .medium))
                }
                .foregroundColor(.accentColor)
                .padding(.horizontal, 7)
                .padding(.vertical, 3.5)
                .background(Color.accentColor.opacity(0.12))
                .cornerRadius(6)
            }
            .buttonStyle(.plain)
            .keyboardShortcut(.escape, modifiers: [])
            .help("กลับไปที่บทสนทนา (Esc)")

            Divider()
                .frame(height: 14)

            // File Icon & Breadcrumb
            HStack(spacing: 6) {
                Image(systemName: fileIcon.name)
                    .foregroundColor(fileIcon.color)
                    .font(.system(size: 13, weight: .semibold))

                HStack(spacing: 4) {
                    let parts = breadcrumbParts
                    ForEach(Array(parts.enumerated()), id: \.offset) { i, part in
                        if i == parts.count - 1 {
                            Text(part)
                                .font(.system(size: 11.5, weight: .bold, design: .monospaced))
                                .foregroundColor(.primary)
                        } else {
                            Text(part)
                                .font(.system(size: 11, design: .monospaced))
                                .foregroundColor(.secondary)
                            Text("/")
                                .font(.system(size: 10, design: .monospaced))
                                .foregroundColor(.secondary.opacity(0.4))
                        }
                    }
                }
            }

            // Stats pill (lines count)
            Text("\(lines.count) lines")
                .font(.system(size: 9.5, weight: .semibold, design: .monospaced))
                .foregroundColor(.secondary)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(Color.primary.opacity(0.04))
                .clipShape(Capsule())

            Spacer()

            // Copy Content Button
            Button(action: copyContent) {
                HStack(spacing: 4) {
                    Image(systemName: copiedToClipboard ? "checkmark" : "doc.on.doc")
                        .font(.system(size: 10))
                        .foregroundColor(copiedToClipboard ? .green : .secondary)
                    if copiedToClipboard {
                        Text("Copied")
                            .font(.system(size: 10, weight: .medium))
                            .foregroundColor(.green)
                    }
                }
                .padding(4)
                .background(Color.primary.opacity(0.04))
                .cornerRadius(4)
            }
            .buttonStyle(.plain)
            .help("Copy file content")

            // External Editor Handoff Button
            Button(action: openInExternalEditor) {
                HStack(spacing: 4) {
                    Image(systemName: "arrow.up.forward.app")
                        .font(.system(size: 10.5))
                        .foregroundColor(.accentColor)
                    Text("Open in \(detectedEditor)")
                        .font(.system(size: 11, weight: .medium))
                    Text("⇧⌘E")
                        .font(.system(size: 9.5, weight: .semibold, design: .monospaced))
                        .foregroundColor(.secondary)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .background(Color.accentColor.opacity(0.1))
                .cornerRadius(5)
            }
            .buttonStyle(.plain)
            .keyboardShortcut("e", modifiers: [.shift, .command])
            .help("Open in \(detectedEditor) at line \(selectedLine ?? 1)")

            // Maximize / Restore Button
            if let onToggleMaximize {
                PanelMaximizeButton(isMaximized: isMaximized, onToggle: onToggleMaximize)
            }

            // Close Button
            Button(action: onClose) {
                Image(systemName: "xmark")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundColor(.secondary)
                    .padding(5)
                    .background(Color.primary.opacity(0.04))
                    .clipShape(Circle())
            }
            .buttonStyle(.plain)
            .help("Close file viewer")
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(Color(NSColor.controlBackgroundColor).opacity(0.6))
    }

    private func copyContent() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(fileContent, forType: .string)
        withAnimation {
            copiedToClipboard = true
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
            withAnimation {
                copiedToClipboard = false
            }
        }
    }

    private func loadFile() {
        guard !filePath.isEmpty, !cwd.isEmpty else { return }
        let fullPath = (cwd as NSString).appendingPathComponent(filePath)
        let path = filePath
        let token = UUID()
        loadToken = token
        highlighted = []
        // Read + highlight off the main thread; a big file used to freeze
        // the window while it loaded.
        DispatchQueue.global(qos: .userInitiated).async {
            let content = try? String(contentsOfFile: fullPath, encoding: .utf8)
            let lines = content?.components(separatedBy: "\n") ?? ["// Error: Unable to read file at \(path)"]
            let spans = content.flatMap { Engine.highlightSpans(path: path, text: $0) }
            let coloured = spans.map { CodeTheme.paint(lines: lines, spans: $0) } ?? []
            DispatchQueue.main.async {
                guard token == loadToken else { return }   // another file was opened since
                fileContent = content ?? ""
                self.lines = lines
                highlighted = coloured
            }
        }
    }

    private func detectInstalledEditor() {
        let ws = NSWorkspace.shared
        if ws.urlForApplication(withBundleIdentifier: "com.todesktop.230313mzl4w4u92") != nil ||
           ws.urlForApplication(withBundleIdentifier: "com.anysphere.cursor") != nil {
            detectedEditor = "Cursor"
        } else if ws.urlForApplication(withBundleIdentifier: "com.microsoft.VSCode") != nil {
            detectedEditor = "VS Code"
        } else if ws.urlForApplication(withBundleIdentifier: "dev.zed.Zed") != nil {
            detectedEditor = "Zed"
        } else {
            detectedEditor = "Editor"
        }
    }

    private func openInExternalEditor() {
        guard !filePath.isEmpty, !cwd.isEmpty else { return }
        let fullPath = (cwd as NSString).appendingPathComponent(filePath)
        let line = selectedLine ?? 1

        let task = Process()
        task.launchPath = "/usr/bin/env"

        let appName = detectedEditor == "VS Code" ? "code" : (detectedEditor == "Cursor" ? "cursor" : "zed")
        task.arguments = [appName, "--goto", "\(fullPath):\(line)"]

        do {
            try task.run()
        } catch {
            NSWorkspace.shared.open(URL(fileURLWithPath: fullPath))
        }
    }
}

// MARK: - Code Line Row with Gutter
private struct CodeLineRow: View {
    let lineNum: Int
    let lineText: String
    let highlighted: AttributedString?
    let isHovered: Bool
    let isSelected: Bool
    let onHover: (Bool) -> Void
    let onSelect: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            // Gutter with Line Number
            Text("\(lineNum)")
                .font(.system(size: 11.5, weight: .regular, design: .monospaced))
                .foregroundColor(isSelected ? .accentColor : (isHovered ? .primary : .secondary.opacity(0.42)))
                .frame(width: 48, alignment: .trailing)
                .padding(.trailing, 10)
                .background(Color.primary.opacity(0.015))

            // Gutter Divider
            Rectangle()
                .fill(Color.primary.opacity(0.08))
                .frame(width: 1)

            // Syntax Highlighted Code
            Text(highlighted ?? AttributedString(lineText.isEmpty ? " " : lineText))
                .font(.system(size: 12.5, weight: .regular, design: .monospaced))
                .textSelection(.enabled)
                .lineLimit(1)
                .fixedSize(horizontal: true, vertical: false)
                .padding(.leading, 12)
                .padding(.trailing, 24)

            Spacer(minLength: 0)
        }
        .padding(.vertical, 3.5)
        .background(rowBackground)
        .onHover { onHover($0) }
        .onTapGesture { onSelect() }
    }

    private var rowBackground: Color {
        if isSelected {
            return Color.accentColor.opacity(0.12)
        }
        if isHovered {
            return Color.primary.opacity(0.035)
        }
        return Color.clear
    }

}

// MARK: - Syntax theme
/// Xcode's default light/dark palette for the kinds `chorus_highlight`
/// reports. Colour only — which text is what kind is decided in the core.
enum CodeTheme {
    private static func dynamic(_ light: UInt32, _ dark: UInt32) -> Color {
        func ns(_ hex: UInt32) -> NSColor {
            NSColor(srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
                    blue: CGFloat(hex & 0xFF) / 255, alpha: 1)
        }
        return Color(nsColor: NSColor(name: nil) { appearance in
            appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? ns(dark) : ns(light)
        })
    }

    /// Index = `chorus_core::highlight::Kind`.
    static let colors: [Color] = [
        dynamic(0x9B2393, 0xFC5FA3), // keyword
        dynamic(0x0B4F79, 0x5DD8FF), // type
        dynamic(0xC41A16, 0xFC6A5D), // string
        dynamic(0x5D6C79, 0x6C7986), // comment
        dynamic(0x1C00CF, 0xD0BF69), // number
        dynamic(0x326D74, 0x67B7A4), // function
        dynamic(0x643820, 0xFD8F3F), // attribute / macro / key
        dynamic(0x9B2393, 0xFC5FA3), // constant (Xcode colours these as keywords)
    ]

    static func paint(lines: [String], spans: [[[Int]]]) -> [AttributedString] {
        lines.enumerated().map { i, line in
            var attr = AttributedString(line.isEmpty ? " " : line)
            guard i < spans.count, !line.isEmpty else { return attr }
            let utf8 = line.utf8
            for span in spans[i] where span.count == 3 {
                guard span[2] >= 0, span[2] < colors.count,
                      let lo = utf8.index(utf8.startIndex, offsetBy: span[0], limitedBy: utf8.endIndex),
                      let hi = utf8.index(lo, offsetBy: span[1], limitedBy: utf8.endIndex),
                      let r = Range(lo..<hi, in: attr) else { continue }
                attr[r].foregroundColor = colors[span[2]]
                if span[2] == 3 { attr[r].font = .system(size: 12.5, design: .monospaced).italic() }
            }
            return attr
        }
    }
}
