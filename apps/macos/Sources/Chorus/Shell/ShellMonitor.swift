import SwiftUI

// System monitor: what listens on this Mac's ports, what Chorus itself is
// running, and what it keeps on disk. Scanning, labelling and every stop
// guard live in Rust (crates/chorus-daemon/src/monitor); this draws the
// answer and forwards the clicks.

struct MonitorTool: View {
    @EnvironmentObject private var engine: Engine
    @StateObject private var model = MonitorModel()
    @StateObject private var projects = ProjectStore()
    @State private var tab: Tab = .services
    @State private var confirming: MonRow?

    enum Tab: String, CaseIterable, Identifiable {
        case services = "บริการ", chorus = "Chorus", disk = "พื้นที่"
        var id: String { rawValue }
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Picker("แท็บ", selection: $tab) { ForEach(Tab.allCases) { Text($0.rawValue).tag($0) } }
                    .pickerStyle(.segmented).labelsHidden().frame(width: 260)
                Spacer()
                if let s = model.snapshot {
                    Text("Chorus \(MonFormat.bytes(s.chorusFootprint)) · CPU \(Int(s.chorusCpuPct))%")
                        .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                }
            }
            .padding(10)
            if let n = model.notice ?? model.error {
                Label(n, systemImage: "exclamationmark.triangle.fill")
                    .font(.caption).foregroundStyle(.orange)
                    .padding(.horizontal, 10).padding(.bottom, 6)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .onTapGesture { model.notice = nil; model.error = nil }
            }
            Divider()
            switch tab {
            case .services: services
            case .chorus: chorus
            case .disk: DiskPane(model: model, workspaces: workspaces)
            }
        }
        .onAppear {
            model.engine = engine
            model.setVisible(true)
        }
        .onDisappear { model.setVisible(false) }
        .confirmationDialog("หยุด \(confirming?.name ?? "")?", isPresented: Binding(
            get: { confirming != nil }, set: { if !$0 { confirming = nil } })) {
            Button("หยุด (SIGTERM)", role: .destructive) {
                if let r = confirming { model.stop(r) }
                confirming = nil
            }
            Button("ยกเลิก", role: .cancel) { confirming = nil }
        } message: {
            if let r = confirming {
                Text("pid \(r.pid) · \(r.path)").font(.caption)
            }
        }
    }

    private var workspaces: [String] {
        var seen = Set<String>()
        return projects.projects.flatMap(\.folders).filter { seen.insert($0).inserted }
    }

    private var services: some View {
        List {
            Toggle("รวมพอร์ต UDP", isOn: $model.udp)
            ForEach(model.snapshot?.services ?? []) { s in
                HStack {
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 6) {
                            Text(verbatim: "\(s.port)").font(.callout.monospacedDigit().weight(.semibold))
                            Text(s.service?.name ?? s.process.name).font(.callout)
                            Text(s.reach == "network" ? "เครือข่าย" : "localhost")
                                .font(.caption2)
                                .foregroundStyle(s.reach == "network" ? Color.orange : Color.secondary)
                        }
                        Text("\(s.process.name) · pid \(s.process.pid) · \(originTitle(s.origin))")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    stopControl(s.process)
                }
                .opacity(s.process.mine ? 1 : 0.5)
            }
        }
    }

    private var chorus: some View {
        List(model.snapshot?.chorus ?? []) { r in
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(r.name).font(.callout)
                    Text("\(MonFormat.role(r.role)) · pid \(r.pid) · \(MonFormat.uptime(r.uptimeS))")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Text(r.footprint.map(MonFormat.bytes) ?? "—").font(.caption.monospacedDigit())
                if r.role != "app" { stopControl(r) }
            }
        }
    }

    @ViewBuilder
    private func stopControl(_ r: MonRow) -> some View {
        if let id = r.ident, let phase = model.phases[id] {
            switch phase {
            case .stopping, .killing: ProgressView().controlSize(.small)
            case .stillAlive: Button("บังคับปิด", role: .destructive) { model.forceKill(r) }
            }
        } else if r.mine, r.refusal == nil, r.ident != nil {
            Button("หยุด") { confirming = r }
        } else if let note = r.refusalNote {
            Image(systemName: "lock").foregroundStyle(.secondary).help(note)
        }
    }

    private func originTitle(_ o: String) -> String {
        switch o {
        case "mine": "รันเอง"
        case "chorus": "Chorus"
        case "background": "เบื้องหลัง"
        default: "ระบบ/ผู้ใช้อื่น"
        }
    }
}

private struct DiskPane: View {
    @ObservedObject var model: MonitorModel
    let workspaces: [String]
    @State private var workspace = ""
    @State private var days = 30
    @State private var action: [String: Any]?
    @State private var title = ""
    @State private var preview: MonPreview?
    @State private var picked: Set<String> = []
    @State private var busy = false
    @State private var result: String?

    var body: some View {
        List {
            if let d = model.disk {
                Section("ที่ Chorus เก็บ · \(MonFormat.bytes(d.total))") {
                    row("ledger", d.categories.ledger)
                    row("checkpoint", d.categories.checkpoints)
                    row("worktree", d.categories.worktrees)
                    row("โมเดล local", d.categories.models)
                    row("ความจำ", d.categories.memory)
                    row("cache", d.categories.cache)
                    row("อื่น ๆ", d.categories.other)
                }
            } else if model.diskLoading {
                ProgressView("กำลังวัดพื้นที่…")
            }
            Section("เก็บกวาด") {
                if workspaces.count > 1 {
                    Picker("โปรเจกต์", selection: $workspace) {
                        ForEach(workspaces, id: \.self) { Text(($0 as NSString).lastPathComponent).tag($0) }
                    }
                }
                cleanup("ล้าง blob ที่ไม่มีใครอ้าง", ["action": "blob_gc", "workspace": workspace])
                cleanup("บีบฐานข้อมูล (vacuum)", ["action": "vacuum", "workspace": workspace])
                cleanup("แพ็ก checkpoint", ["action": "pack_checkpoints", "workspace": workspace])
                HStack {
                    cleanup("ลบ checkpoint เก่ากว่า \(days) วัน", ["action": "prune_checkpoints", "workspace": workspace, "days": days])
                    Stepper("", value: $days, in: 1...365).labelsHidden()
                }
                cleanup("ลบ worktree ที่เลิกใช้", ["action": "prune_worktrees", "workspace": workspace])
                cleanup("เอาโมเดลออกจากหน่วยความจำ", ["action": "unload_model"])
                cleanup("ล้าง cache", ["action": "clear_caches"])
                cleanup("ลบโมเดลที่ดาวน์โหลดไว้", ["action": "delete_models"])
            }
            if let p = preview {
                Section(title) {
                    Text(p.note).font(.caption).foregroundStyle(.secondary)
                    ForEach(p.items) { it in
                        Toggle(isOn: Binding(
                            get: { picked.contains(it.id) },
                            set: { on in if on { picked.insert(it.id) } else { picked.remove(it.id) } }
                        )) {
                            HStack {
                                Text(it.label).lineLimit(1).truncationMode(.middle)
                                Spacer()
                                Text(MonFormat.bytes(it.bytes)).font(.caption.monospacedDigit())
                            }
                        }
                        .disabled(!it.selectable)
                    }
                    HStack {
                        Text("จะคืนพื้นที่ \(MonFormat.bytes(p.items.isEmpty ? p.bytes : bytes(of: p)))")
                            .font(.caption)
                        Spacer()
                        Button("ยกเลิก") { preview = nil }
                        Button(p.destructive ? "ลบ" : "ทำเลย", role: p.destructive ? .destructive : nil) { apply(p) }
                            .disabled(!p.allowed || busy || (!p.items.isEmpty && picked.isEmpty))
                    }
                }
            }
            if let result { Text(result).font(.caption).foregroundStyle(.green) }
        }
        .onAppear {
            if workspace.isEmpty { workspace = workspaces.first ?? "" }
            model.loadDisk(workspaces: workspaces)
        }
    }

    private func row(_ name: String, _ bytes: UInt64) -> some View {
        HStack { Text(name); Spacer(); Text(MonFormat.bytes(bytes)).font(.callout.monospacedDigit()).foregroundStyle(.secondary) }
    }

    private func cleanup(_ name: String, _ a: [String: Any]) -> some View {
        Button(name) { runPreview(name, a) }.disabled(busy)
    }

    private func bytes(of p: MonPreview) -> UInt64 {
        p.items.filter { picked.contains($0.id) }.reduce(0) { $0 + $1.bytes }
    }

    private func runPreview(_ name: String, _ a: [String: Any]) {
        guard let engine = model.engine else { return }
        busy = true; result = nil; title = name; action = a
        let onQueue = ["blob_gc", "vacuum", "pack_checkpoints", "prune_checkpoints", "prune_worktrees"]
            .contains(a["action"] as? String ?? "")
        MonitorModel.call(engine, ["op": "preview", "action": a], MonPreview.self, onEngineQueue: onQueue) { r in
            busy = false
            switch r {
            case .success(let p): preview = p; picked = []
            case .failure(let e): model.notice = e.message
            }
        }
    }

    private func apply(_ p: MonPreview) {
        guard let engine = model.engine, var a = action else { return }
        if !p.items.isEmpty { a["ids"] = Array(picked) }
        busy = true
        let onQueue = ["blob_gc", "vacuum", "pack_checkpoints", "prune_checkpoints", "prune_worktrees"]
            .contains(a["action"] as? String ?? "")
        MonitorModel.call(engine, ["op": "apply", "action": a], MonApplied.self, onEngineQueue: onQueue) { r in
            busy = false
            preview = nil; picked = []
            switch r {
            case .success(let x): result = x.message; model.loadDisk(workspaces: workspaces)
            case .failure(let e): model.notice = e.message
            }
        }
    }
}
