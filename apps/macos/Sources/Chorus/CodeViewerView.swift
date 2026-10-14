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
                            ext: fileExtension,
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
        if let content = try? String(contentsOfFile: fullPath, encoding: .utf8) {
            self.fileContent = content
            self.lines = content.components(separatedBy: "\n")
        } else {
            self.lines = ["// Error: Unable to read file at \(filePath)"]
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
    let ext: String
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
            Text(highlightedText)
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

    private var highlightedText: AttributedString {
        SyntaxHighlighter.highlight(line: lineText, ext: ext)
    }
}

// MARK: - Lightweight Syntax Highlighter
enum SyntaxHighlighter {
    static func highlight(line: String, ext: String) -> AttributedString {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        if trimmed.isEmpty {
            return AttributedString(" ")
        }

        var attr = AttributedString(line)

        // 1. Comments
        if trimmed.hasPrefix("#") || trimmed.hasPrefix("//") {
            attr.foregroundColor = .secondary.opacity(0.65)
            attr.font = .system(size: 12.5, weight: .regular, design: .monospaced).italic()
            return attr
        }

        // 2. TOML Section headers: [package], [[bin]], [dependencies]
        if (ext == "toml" || ext == "ini") && trimmed.hasPrefix("[") && trimmed.hasSuffix("]") {
            attr.foregroundColor = .blue
            attr.font = .system(size: 12.5, weight: .bold, design: .monospaced)
            return attr
        }

        // 3. Highlight Strings in Green
        highlightRegex(pattern: "\"[^\"]*\"|'[^']*'", in: &attr, color: .green)

        // 4. Highlight Keys in TOML / YAML (e.g. `name =`, `version.workspace =`)
        if ext == "toml" || ext == "yaml" || ext == "yml" {
            highlightRegex(pattern: "^\\s*([a-zA-Z0-9_.-]+)\\s*(?==)", in: &attr, color: .primary)
            highlightRegex(pattern: "\\b(true|false)\\b", in: &attr, color: .orange)
            highlightRegex(pattern: "\\b\\d+(\\.\\d+)?\\b", in: &attr, color: .purple)
        }

        // 5. Highlight Rust / Swift / TS / JS code
        if ext == "rs" || ext == "swift" || ext == "ts" || ext == "js" {
            let kwPattern = "\\b(pub|fn|struct|enum|impl|let|mut|match|if|else|return|import|export|const|function|var|class|type|interface|use|mod|crate|async|await|guard|case|default|where|for|in|while|loop)\\b"
            highlightRegex(pattern: kwPattern, in: &attr, color: .purple)
            // Types in Cyan
            highlightRegex(pattern: "\\b([A-Z][a-zA-Z0-9_]+)\\b", in: &attr, color: .cyan)
            // Booleans & Optionals in Orange
            highlightRegex(pattern: "\\b(true|false|nil|null|None|Some|Ok|Err)\\b", in: &attr, color: .orange)
            // Numbers in Teal
            highlightRegex(pattern: "\\b\\d+(\\.\\d+)?\\b", in: &attr, color: .teal)
        }

        return attr
    }

    private static func highlightRegex(pattern: String, in attr: inout AttributedString, color: Color) {
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return }
        let fullString = String(attr.characters)
        let matches = regex.matches(in: fullString, range: NSRange(location: 0, length: fullString.utf16.count))

        for match in matches {
            guard let range = Range(match.range, in: fullString),
                  let attrRange = Range(range, in: attr) else { continue }
            attr[attrRange].foregroundColor = color
        }
    }
}
