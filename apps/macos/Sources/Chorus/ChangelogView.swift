import SwiftUI

/// Settings › มีอะไรใหม่: the bundled CHANGELOG.md (copied in by build.sh).
/// A line-based reader for that file's shape — `## ` versions, `### `
/// sections, `- ` bullets — not a general Markdown renderer; inline
/// `**bold**` and `code` still render through `Text`.
struct ChangelogSettings: View {
    @State private var lines: [Line] = []

    enum Line {
        case version(String), section(String), bullet(String), text(String)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                switch line {
                case .version(let v):
                    HStack(spacing: 8) {
                        Text(v).font(.title3.weight(.semibold))
                        if isCurrent(v) {
                            Text("เวอร์ชันนี้")
                                .font(.caption2.weight(.semibold))
                                .padding(.horizontal, 6).padding(.vertical, 2)
                                .background(Color.accentColor.opacity(0.15), in: Capsule())
                                .foregroundStyle(Color.accentColor)
                        }
                    }
                    .padding(.top, 14)
                case .section(let s):
                    Text(s).font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                        .padding(.top, 4)
                case .bullet(let b):
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text("•").foregroundStyle(.secondary)
                        Text(LocalizedStringKey(b)).fixedSize(horizontal: false, vertical: true)
                    }
                    .font(.callout)
                case .text(let t):
                    Text(LocalizedStringKey(t)).font(.callout).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .textSelection(.enabled)
        .onAppear(perform: load)
    }

    /// `0.1.3 - 2026-10-17` heading is this build's version.
    private func isCurrent(_ heading: String) -> Bool {
        heading.split(separator: " ").first.map(String.init) == Updater.appVersion
    }

    private func load() {
        guard let url = Bundle.main.url(forResource: "CHANGELOG", withExtension: "md"),
              let raw = try? String(contentsOf: url, encoding: .utf8)
        else {
            lines = [.text("build นี้ไม่มี CHANGELOG.md — ดูได้ที่ github.com/PerthTheIamCat/chorus/releases")]
            return
        }
        lines = Self.parse(raw)
    }

    static func parse(_ raw: String) -> [Line] {
        var out: [Line] = []
        var started = false   // skip the file's title and intro paragraph
        for rawLine in raw.split(separator: "\n", omittingEmptySubsequences: false) {
            let line = String(rawLine)
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if line.hasPrefix("## ") {
                started = true
                // `[0.1.3] - date` → `0.1.3 - date`; `[Unreleased]` → `Unreleased`
                out.append(.version(String(line.dropFirst(3)).replacingOccurrences(of: "[", with: "")
                    .replacingOccurrences(of: "]", with: "")))
            } else if !started || trimmed.isEmpty {
                continue
            } else if line.hasPrefix("### ") {
                out.append(.section(String(line.dropFirst(4))))
            } else if line.hasPrefix("- ") {
                out.append(.bullet(String(line.dropFirst(2))))
            } else if line.hasPrefix("  "), case .bullet(let prev)? = out.last {
                out[out.count - 1] = .bullet(prev + " " + trimmed)
            } else {
                out.append(.text(trimmed))
            }
        }
        // An empty `[Unreleased]` right after a release is just a heading.
        return out.enumerated().filter { i, line in
            guard case .version = line else { return true }
            if i + 1 == out.count { return false }
            if case .version = out[i + 1] { return false }
            return true
        }.map(\.element)
    }
}
