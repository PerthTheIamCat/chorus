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

/// Create or edit a project: a name and the folders it covers.
struct ProjectEditor: View {
    @State var project: ChorusProject
    let isNew: Bool
    let onSave: (ChorusProject) -> Void
    let onDelete: (() -> Void)?
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(isNew ? "โปรเจกต์ใหม่" : "แก้ไขโปรเจกต์").font(.title3.bold())
            VStack(alignment: .leading, spacing: 4) {
                Text("ชื่อ").font(.caption).foregroundStyle(.secondary)
                TextField("เช่น Chorus, งานลูกค้า A", text: $project.name)
                    .textFieldStyle(.roundedBorder)
            }
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text("โฟลเดอร์ในโปรเจกต์นี้").font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    Button {
                        for f in ProjectStore.pickFolders(message: "เลือกโฟลเดอร์ที่เป็นส่วนหนึ่งของ \(project.name)")
                        where !project.folders.contains(f) {
                            project.folders.append(f)
                        }
                        if project.name.isEmpty, let f = project.folders.first {
                            project.name = (f as NSString).lastPathComponent
                        }
                    } label: { Label("เพิ่มโฟลเดอร์", systemImage: "plus") }
                    .controlSize(.small)
                }
                if project.folders.isEmpty {
                    Text("ยังไม่มีโฟลเดอร์ — เพิ่มอย่างน้อยหนึ่งอันเพื่อเริ่มเซสชัน")
                        .font(.caption).foregroundStyle(.tertiary)
                        .frame(maxWidth: .infinity, minHeight: 60)
                        .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 8))
                }
                ForEach(project.folders, id: \.self) { f in
                    HStack(spacing: 8) {
                        Image(systemName: "folder.fill").foregroundStyle(.blue)
                        VStack(alignment: .leading, spacing: 1) {
                            Text((f as NSString).lastPathComponent).font(.callout)
                            Text(f).font(.caption2).foregroundStyle(.tertiary)
                                .lineLimit(1).truncationMode(.middle)
                        }
                        Spacer()
                        Button { project.folders.removeAll { $0 == f } } label: {
                            Image(systemName: "minus.circle.fill")
                        }
                        .buttonStyle(.plain).foregroundStyle(.secondary)
                        .help("เอาออกจากโปรเจกต์ (ไม่ลบไฟล์)")
                    }
                    .padding(8)
                    .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 8))
                }
            }
            Spacer(minLength: 0)
            HStack {
                if let onDelete {
                    Button("ลบโปรเจกต์", role: .destructive) { onDelete(); dismiss() }
                        .help("ลบแค่รายการในแอป ไฟล์และเซสชันในโฟลเดอร์ยังอยู่ครบ")
                }
                Spacer()
                Button("ยกเลิก") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(isNew ? "สร้าง" : "บันทึก") { onSave(project); dismiss() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(project.name.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 480, height: 440)
    }
}
