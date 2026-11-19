import SwiftUI

// One checkpoint per turn that changed files. Going back is a fork of the
// conversation plus a restore of the files, never a delete (CLAUDE.md 2.2).

struct CheckpointsTool: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @State private var items: [Engine.CheckpointRecord] = []
    @State private var loading = false
    @State private var status: String?
    @State private var pending: (seq: Int64, mode: String)?

    var body: some View {
        Group {
            if engine.sessionID < 0 {
                ContentUnavailableView("เปิด session ก่อน", systemImage: "clock.arrow.circlepath")
            } else if loading && items.isEmpty {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if items.isEmpty {
                ContentUnavailableView("ยังไม่มี checkpoint ใน session นี้", systemImage: "clock.badge.questionmark",
                                       description: Text("เมื่อ agent แก้ไฟล์ ระบบจะบันทึก snapshot ให้ทุกเทิร์น"))
            } else {
                VStack(spacing: 0) {
                    if let status { Text(status).font(.caption).foregroundStyle(.secondary).padding(8) }
                    List(items.reversed()) { c in
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(c.message.isEmpty ? "เทิร์น #\(c.seq)" : c.message).lineLimit(2)
                                Text("\(c.shortSha) · \(c.formattedDate)").font(.caption.monospaced()).foregroundStyle(.secondary)
                            }
                            Spacer()
                            Menu("ย้อนกลับ") {
                                Button("ย้อนทั้งไฟล์และแชท") { pending = (c.seq, "code_and_ledger") }
                                Button("ย้อนเฉพาะไฟล์") { pending = (c.seq, "code_only") }
                                Button("ย้อนเฉพาะแชท") { pending = (c.seq, "ledger_only") }
                            }
                            .fixedSize()
                            .disabled(engine.busy)
                        }
                    }
                }
            }
        }
        .onAppear(perform: load)
        .confirmationDialog("ย้อนกลับไปก่อนเทิร์น #\(pending?.seq ?? 0)?", isPresented: Binding(
            get: { pending != nil }, set: { if !$0 { pending = nil } })) {
            Button("ย้อนกลับ", role: .destructive) {
                if let p = pending { rewind(p.seq, p.mode) }
                pending = nil
            }
            Button("ยกเลิก", role: .cancel) { pending = nil }
        } message: {
            Text("ระบบเก็บ snapshot ของตอนนี้ไว้ก่อนย้อนเสมอ แชทเดิมไม่ถูกลบ")
        }
    }

    private func load() {
        guard engine.sessionID >= 0 else { return }
        loading = true
        engine.listCheckpoints(db: shell.currentDB, sessionID: engine.sessionID) { list in
            items = list
            loading = false
        }
    }

    private func rewind(_ seq: Int64, _ mode: String) {
        status = "กำลังย้อนกลับ…"
        engine.rewindCheckpoint(db: shell.currentDB, ws: shell.workspace, sessionID: engine.sessionID,
                                targetSeq: seq, mode: mode) { ok, err in
            status = ok ? "ย้อนกลับแล้ว" : "ย้อนไม่สำเร็จ: \(err ?? "เกิดข้อผิดพลาด")"
            load()
        }
    }
}
