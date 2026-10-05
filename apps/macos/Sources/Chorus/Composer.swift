import SwiftUI

final class DraftBox: ObservableObject {
    @Published var text = ""
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
    var placeholder = "ขอให้ทำอะไร… ( / คำสั่ง · @ ไฟล์หรือเอเจนต์ )"

    @State private var selected = 0
    @State private var dismissed = false
    @FocusState private var focused: Bool

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

    var body: some View {
        let list = items
        VStack(alignment: .leading, spacing: 4) {
            if !list.isEmpty { popup(list) }
            ZStack(alignment: .topLeading) {
                if box.text.isEmpty {
                    Text(placeholder).foregroundStyle(.tertiary)
                        .padding(.leading, 5).padding(.top, 1)
                        .allowsHitTesting(false)
                }
                TextEditor(text: $box.text)
                    .font(.body)
                    .scrollContentBackground(.hidden)
                    .frame(minHeight: 40, maxHeight: 160)
                    .fixedSize(horizontal: false, vertical: true)
                    .focused($focused)
                    .onKeyPress(keys: [.upArrow, .downArrow, .return, .tab, .escape], phases: .down) { press in
                        handle(press, list)
                    }
            }
        }
        .onChange(of: box.text) { _, _ in selected = 0; dismissed = false }
        .onAppear { focused = true }
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
        if press.key == .return && !press.modifiers.contains(.shift) {
            onSubmit()
            return .handled
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
