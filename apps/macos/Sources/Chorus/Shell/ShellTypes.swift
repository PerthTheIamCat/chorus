import SwiftUI
import UniformTypeIdentifiers

struct DraftAttachment: Identifiable, Equatable {
    let id: Int64
    let name: String
    let mediaType: String
    let size: Int64
    let agentReadable: Bool
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


/// Mode of operation for the composer in a session.
enum SessionMode: String, CaseIterable, Identifiable {
    case autoRoute = "auto"
    case orchestra = "orchestra"
    case manual = "manual"

    var id: String { rawValue }
    var label: String {
        switch self {
        case .autoRoute: return "Auto route"
        case .orchestra: return "Orchestra"
        case .manual: return "เลือกเอง"
        }
    }
}
