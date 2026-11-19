import SwiftUI

// Orchestra: what a lead has handed out. The plan, each agent it started
// (state, task, files touched), and the controls to steer or stop one.
// Planning, spawning and verifying are Rust's; this only shows and forwards.

struct OrchestraTool: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @State private var messaging: Engine.SubagentInfo?
    @State private var steer = ""

    private var subs: [Engine.SubagentInfo] {
        let all = engine.subagentsByDB[shell.currentDB] ?? []
        return all.sorted { $0.created_ms > $1.created_ms }
    }

    var body: some View {
        List {
            if !engine.checklist.isEmpty {
                Section("แผนของ lead") {
                    ForEach(engine.checklist) { item in
                        Label {
                            Text(item.text).strikethrough(item.done)
                        } icon: {
                            Image(systemName: item.done ? "checkmark.circle.fill"
                                  : item.in_progress ? "circle.dotted" : "circle")
                                .foregroundStyle(item.done ? Color.green : item.in_progress ? Color.accentColor : Color.secondary)
                        }
                    }
                }
            }
            Section("agent ย่อย (\(subs.count))") {
                if subs.isEmpty {
                    Text("ยังไม่มี เปลี่ยนโหมดการส่งงานเป็น Orchestra ในช่องพิมพ์ แล้วให้ lead แตกงาน")
                        .foregroundStyle(.secondary)
                }
                ForEach(subs) { s in row(s) }
            }
        }
        .onAppear { engine.loadSubagents(db: shell.currentDB) }
        .alert("ส่งข้อความถึง agent ย่อย", isPresented: Binding(get: { messaging != nil }, set: { if !$0 { messaging = nil } }),
               presenting: messaging) { sub in
            TextField("ข้อความ", text: $steer)
            Button("ส่ง") {
                let t = steer.trimmingCharacters(in: .whitespacesAndNewlines)
                if !t.isEmpty { engine.messageSubagent(db: shell.currentDB, id: sub.id, message: t) }
                messaging = nil
            }
            Button("ยกเลิก", role: .cancel) { messaging = nil }
        } message: { sub in Text(sub.task) }
    }

    private func row(_ s: Engine.SubagentInfo) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(s.role).font(.callout.weight(.semibold))
                if let m = s.model { Text(m).font(.caption).foregroundStyle(.secondary) }
                Spacer()
                Text(label(s.status)).font(.caption).foregroundStyle(color(s.status))
            }
            Text(s.task).font(.callout).lineLimit(3)
            if let sum = s.summary, !sum.isEmpty {
                Text(sum).font(.caption).foregroundStyle(.secondary).lineLimit(3)
            }
            if !s.files_touched.isEmpty {
                Text(s.files_touched.prefix(4).joined(separator: ", ") + (s.files_touched.count > 4 ? " …" : ""))
                    .font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1)
            }
            HStack {
                if let child = s.child_sess {
                    Button("เปิดแชท") { shell.open(SessionKey(folder: shell.workspace, id: child)) }
                }
                if s.status == "running" || s.status == "waiting" {
                    Button("ส่งข้อความ…") { steer = ""; messaging = s }
                    Button("หยุด", role: .destructive) { engine.cancelSubagent(db: shell.currentDB, id: s.id) }
                }
            }
            .buttonStyle(.link).font(.caption)
        }
        .padding(.vertical, 2)
    }

    private func label(_ status: String) -> String {
        switch status {
        case "running": "กำลังทำ"
        case "waiting": "รอคุณ"
        case "failed": "ล้มเหลว"
        case "cancelled": "ยกเลิก"
        default: "เสร็จ"
        }
    }

    private func color(_ status: String) -> Color {
        switch status {
        case "running": .accentColor
        case "waiting": .orange
        case "failed": .red
        case "cancelled": .secondary
        default: .green
        }
    }
}
