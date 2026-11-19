import SwiftUI

// Name a project and choose which folders belong to it. A grouped `Form`; the
// folder picker is the system open panel.

struct ShellProjectEditor: View {
    @State var project: ChorusProject
    let onSave: (ChorusProject) -> Void
    let onDelete: () -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            Form {
                Section("ชื่อ") {
                    TextField("ชื่อ", text: $project.name, prompt: Text("เช่น Chorus, งานลูกค้า A"))
                        .labelsHidden()
                }
                Section {
                    if project.folders.isEmpty {
                        Text("ยังไม่มีโฟลเดอร์ เพิ่มอย่างน้อยหนึ่งอันเพื่อเริ่ม session")
                            .foregroundStyle(.secondary)
                    }
                    ForEach(project.folders, id: \.self) { f in
                        LabeledContent {
                            Button { project.folders.removeAll { $0 == f } } label: { Image(systemName: "minus.circle") }
                                .buttonStyle(.borderless)
                                .help("เอาออกจากโปรเจกต์ (ไม่ลบไฟล์)")
                        } label: {
                            Label {
                                VStack(alignment: .leading) {
                                    Text((f as NSString).lastPathComponent)
                                    Text(f).font(.caption).foregroundStyle(.secondary)
                                        .lineLimit(1).truncationMode(.middle)
                                }
                            } icon: { Image(systemName: "folder") }
                        }
                    }
                } header: {
                    HStack {
                        Text("โฟลเดอร์ในโปรเจกต์นี้")
                        Spacer()
                        Button("เพิ่มโฟลเดอร์…") {
                            for f in ProjectStore.pickFolders(message: "เลือกโฟลเดอร์ที่เป็นส่วนหนึ่งของ \(project.name)")
                            where !project.folders.contains(f) { project.folders.append(f) }
                            if project.name.isEmpty, let f = project.folders.first {
                                project.name = (f as NSString).lastPathComponent
                            }
                        }
                    }
                }
            }
            .formStyle(.grouped)
            Divider()
            HStack {
                Button("ลบโปรเจกต์", role: .destructive) { onDelete(); dismiss() }
                    .help("ลบแค่รายการในแอป ไฟล์และ session ในโฟลเดอร์ยังอยู่ครบ")
                Spacer()
                Button("ยกเลิก") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("บันทึก") { onSave(project); dismiss() }
                    .keyboardShortcut(.defaultAction)
                    .buttonStyle(.borderedProminent)
                    .disabled(project.name.trimmingCharacters(in: .whitespaces).isEmpty)
            }
            .padding(12)
        }
        .frame(width: 480, height: 420)
    }
}
