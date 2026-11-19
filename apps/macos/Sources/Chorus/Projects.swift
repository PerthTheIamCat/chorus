import AppKit
import SwiftUI

/// A project is an in-app grouping, not a directory: a name plus the folders
/// that belong to it. Sessions still live in each folder's own
/// `.chorus/store.db`; the project decides which of them the sidebar shows
/// together. Kept in Application Support so it survives without touching any
/// of the user's folders.
struct ChorusProject: Codable, Identifiable, Hashable {
    var id = UUID()
    var name: String
    var folders: [String]
}

@MainActor
final class ProjectStore: ObservableObject {
    @Published var projects: [ChorusProject] = [] { didSet { save() } }

    private static var file: URL {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Chorus", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("projects.json")
    }

    init() {
        if let d = try? Data(contentsOf: Self.file),
           let p = try? JSONDecoder().decode([ChorusProject].self, from: d) {
            projects = p
        } else {
            // First run after the upgrade: every recent folder becomes a
            // one-folder project, so nothing the user had disappears.
            let recents = UserDefaults.standard.stringArray(forKey: "chorus.recentWorkspaces") ?? []
            projects = recents.map { ChorusProject(name: ($0 as NSString).lastPathComponent, folders: [$0]) }
        }
    }

    private func save() {
        if let d = try? JSONEncoder().encode(projects) { try? d.write(to: Self.file, options: .atomic) }
    }

    func project(containing folder: String) -> ChorusProject? {
        projects.first { $0.folders.contains(folder) }
    }

    func upsert(_ p: ChorusProject) {
        if let i = projects.firstIndex(where: { $0.id == p.id }) { projects[i] = p } else { projects.insert(p, at: 0) }
    }

    /// The project's shared memory store, next to `projects.json`.
    static func memoryPath(_ p: ChorusProject) -> String {
        file.deletingLastPathComponent()
            .appendingPathComponent("projects/\(p.id.uuidString)/memory.db").path
    }

    func remove(_ p: ChorusProject) { projects.removeAll { $0.id == p.id } }

    static func pickFolders(message: String) -> [String] {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = true
        panel.canCreateDirectories = true
        panel.prompt = "เพิ่ม"
        panel.message = message
        guard panel.runModal() == .OK else { return [] }
        return panel.urls.map(\.path)
    }
}

