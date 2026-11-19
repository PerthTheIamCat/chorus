import SwiftUI

// The role library: the jobs an Orchestra lead can hand out. Four built-in
// roles plus your own, saved as .chorus/agents/*.md. A role is a name, what
// it is for, the tools it may use, and the instructions it starts with.

struct RoleLibrary: View {
    @EnvironmentObject private var engine: Engine
    let workspace: String
    @State private var roles: [Engine.AgentRole] = []
    @State private var editing: Engine.AgentRole?
    @State private var creating = false
    @State private var deleting: Engine.AgentRole?

    var body: some View {
        Group {
            ForEach(roles) { r in
                LabeledContent {
                    HStack {
                        Button("แก้") { editing = r }
                        if !r.isBuiltin {
                            Button("ลบ", role: .destructive) { deleting = r }
                        }
                    }
                    .buttonStyle(.borderless)
                } label: {
                    Label {
                        VStack(alignment: .leading, spacing: 1) {
                            Text(r.name)
                            Text(r.description).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                        }
                    } icon: { Image(systemName: r.iconName) }
                }
            }
            Button("บทบาทใหม่…") { creating = true }
        }
        .onAppear(perform: reload)
        .sheet(isPresented: $creating) { RoleEditor(workspace: workspace, role: nil) { reload() } }
        .sheet(item: $editing) { r in RoleEditor(workspace: workspace, role: r) { reload() } }
        .confirmationDialog("ลบบทบาท \(deleting?.name ?? "")?", isPresented: Binding(
            get: { deleting != nil }, set: { if !$0 { deleting = nil } })) {
            Button("ลบ", role: .destructive) {
                if let r = deleting { engine.deleteRole(cwd: workspace, name: r.name) { _, _ in reload() } }
                deleting = nil
            }
            Button("ยกเลิก", role: .cancel) { deleting = nil }
        }
    }

    private func reload() { engine.fetchRoles(cwd: workspace) { roles = $0 } }
}

private struct RoleEditor: View {
    @EnvironmentObject private var engine: Engine
    @Environment(\.dismiss) private var dismiss
    let workspace: String
    let role: Engine.AgentRole?
    let onSaved: () -> Void

    @State private var name = ""
    @State private var detail = ""
    @State private var prompt = ""
    @State private var tools: Set<String> = ["read", "write", "edit", "bash", "diff"]
    @State private var model = ""
    @State private var idea = ""
    @State private var writes = true
    @State private var working = false
    @State private var error: String?

    private static let standardTools = ["read", "write", "edit", "bash", "diff", "grep", "find"]

    var body: some View {
        VStack(spacing: 0) {
            Form {
                if role == nil {
                    Section("ให้ AI ร่างให้") {
                        TextField("อยากได้บทบาทไว้ทำอะไร", text: $idea, axis: .vertical)
                        Toggle("บทบาทนี้แก้ไฟล์ได้", isOn: $writes)
                        Button(working ? "กำลังร่าง…" : "ร่างบทบาท") { synthesize() }
                            .disabled(working || idea.trimmingCharacters(in: .whitespaces).isEmpty)
                    }
                }
                Section("บทบาท") {
                    TextField("ชื่อ (ตัวพิมพ์เล็ก)", text: $name).disabled(role != nil)
                    TextField("ไว้ทำอะไร", text: $detail)
                    TextField("โมเดลที่ใช้ (เว้นว่าง = ให้ lead เลือก)", text: $model)
                }
                Section("เครื่องมือที่ใช้ได้") {
                    ForEach(Self.standardTools, id: \.self) { t in
                        Toggle(t, isOn: Binding(
                            get: { tools.contains(t) },
                            set: { on in if on { tools.insert(t) } else { tools.remove(t) } }))
                    }
                }
                Section("คำสั่งตั้งต้น") {
                    TextEditor(text: $prompt).font(.callout.monospaced()).frame(minHeight: 120)
                }
                if let error { Text(error).foregroundStyle(.red).font(.caption) }
            }
            .formStyle(.grouped)
            Divider()
            HStack {
                Spacer()
                Button("ยกเลิก") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("บันทึก", action: save).keyboardShortcut(.defaultAction).buttonStyle(.borderedProminent)
            }
            .padding(12)
        }
        .frame(width: 520, height: 600)
        .onAppear {
            if let r = role {
                name = r.name; detail = r.description; prompt = r.system_prompt
                tools = Set(r.tools); model = r.model ?? ""
            }
        }
    }

    private func synthesize() {
        working = true; error = nil
        engine.synthesizeRole(task: idea, writeMode: writes, nameHint: nil) { r in
            working = false
            guard let r else { error = "ร่างบทบาทไม่สำเร็จ ลองใหม่อีกครั้ง"; return }
            name = r.name; detail = r.description; prompt = r.system_prompt; tools = Set(r.tools)
        }
    }

    private func save() {
        let n = name.trimmingCharacters(in: .whitespaces).lowercased()
        let p = prompt.trimmingCharacters(in: .whitespaces)
        guard !n.isEmpty else { error = "ใส่ชื่อบทบาท"; return }
        guard !p.isEmpty else { error = "ใส่คำสั่งตั้งต้น"; return }
        let m = model.trimmingCharacters(in: .whitespaces)
        let d = detail.trimmingCharacters(in: .whitespaces)
        let saved = Engine.AgentRole(name: n, description: d.isEmpty ? "Custom agent role \(n)" : d,
                                     system_prompt: p, tools: tools.isEmpty ? ["read"] : tools.sorted(),
                                     model: m.isEmpty ? nil : m)
        engine.saveRole(cwd: workspace, role: saved) { ok, err in
            if ok { onSaved(); dismiss() } else { error = err ?? "บันทึกไม่สำเร็จ" }
        }
    }
}
