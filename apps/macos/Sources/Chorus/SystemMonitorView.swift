import AppKit
import Combine
import SwiftUI

// System monitor window (⌥⌘M). Draws what `chorus_monitor_json` returns and
// forwards clicks; scanning, labelling, the stop guards and every clean-up
// decision live in Rust (crates/chorus-daemon/src/monitor).

// ---- wire types (mirror monitor/*.rs, snake_case on the wire) ------------

struct MonIdent: Codable, Hashable {
    let pid: Int32
    let startSec: UInt64
    let startUsec: UInt64
    let path: String
}

struct MonRow: Decodable, Hashable {
    let pid: Int32
    let ppid: Int32
    let name: String
    let path: String
    let cmdline: [String]
    let cpuPct: Double?
    let footprint: UInt64?
    let uptimeS: UInt64?
    let mine: Bool
    let role: String?
    let startedBy: String?
    let ident: MonIdent?
    let refusal: String?
    let refusalNote: String?
    let groupStoppable: Bool
}

extension MonRow: Identifiable { var id: Int32 { pid } }

struct MonServiceName: Decodable, Hashable {
    let name: String
    let source: String
}

struct MonService: Decodable, Hashable, Identifiable {
    let port: UInt16
    let udp: Bool
    let addrs: [String]
    let service: MonServiceName?
    let process: MonRow
    var id: String { "\(port)-\(udp)-\(process.pid)" }
}

struct MonSnapshot: Decodable {
    let chorus: [MonRow]
    let services: [MonService]
    let chorusFootprint: UInt64
    let chorusCpuPct: Double
    let modelFootprint: UInt64
    let sweepMs: Double
    let supported: Bool
}

struct MonStopOutcome: Decodable {
    let state: String
    let refusal: String?
    let message: String
}

struct MonDisk: Decodable {
    struct Categories: Decodable {
        let ledger, checkpoints, worktrees, models, memory, cache, other: UInt64
    }
    struct Table: Decodable, Hashable { let name: String; let bytes: UInt64 }
    struct Project: Decodable, Identifiable {
        let path: String
        let ledger, checkpoints, worktrees, other, total: UInt64
        let tables: [Table]
        let freelistBytes: UInt64
        var id: String { path }
    }
    let categories: Categories
    let total: UInt64
    let projects: [Project]
}

struct MonItem: Decodable, Hashable, Identifiable {
    let id: String
    let label: String
    let bytes: UInt64
    let kind: String
    let selectable: Bool
}

struct MonPreview: Decodable {
    let bytes: UInt64
    let destructive: Bool
    let allowed: Bool
    let note: String
    let items: [MonItem]
}

struct MonApplied: Decodable { let freedBytes: UInt64; let message: String }

private struct Envelope<T: Decodable>: Decodable {
    let ok: Bool
    let result: T?
    let error: String?
}

enum MonFormat {
    static func bytes(_ b: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(clamping: b), countStyle: .memory)
    }

    static func uptime(_ s: UInt64?) -> String {
        guard let s else { return "—" }
        if s < 60 { return "\(s) วิ" }
        if s < 3600 { return "\(s / 60) นาที" }
        if s < 86_400 { return "\(s / 3600) ชม. \(s % 3600 / 60) น." }
        return "\(s / 86_400) วัน"
    }

    static func role(_ r: String?) -> String {
        switch r {
        case "app": "แอป"
        case "daemon": "daemon"
        case "bridge": "agent bridge"
        case "child": "เปิดโดย Chorus"
        case "model": "โมเดล local"
        default: "—"
        }
    }
}

// ---- model -----------------------------------------------------------------

@MainActor
final class MonitorModel: ObservableObject {
    enum Phase: Equatable { case stopping, stillAlive, killing }

    @Published var snapshot: MonSnapshot?
    @Published var udp = false { didSet { refresh() } }
    @Published var error: String?
    @Published var notice: String?
    @Published var phases: [MonIdent: Phase] = [:]
    @Published var disk: MonDisk?
    @Published var diskLoading = false

    var engine: Engine?
    private var timer: Timer?
    private var inFlight = false

    /// 1 s while the window is visible; nothing at all while it is hidden or
    /// minimised (ticket 14 §4).
    func setVisible(_ visible: Bool) {
        timer?.invalidate()
        timer = nil
        guard visible else { return }
        refresh()
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
    }

    func refresh() {
        guard !inFlight, let engine else { return }
        inFlight = true
        Self.call(engine, ["op": "snapshot", "udp": udp], MonSnapshot.self) { [weak self] r in
            guard let self else { return }
            self.inFlight = false
            switch r {
            case .success(let s): self.snapshot = s; self.error = nil
            case .failure(let e): self.error = e.message
            }
        }
    }

    func loadDisk(workspaces: [String]) {
        guard let engine, !diskLoading else { return }
        diskLoading = true
        Self.call(engine, ["op": "disk", "workspaces": workspaces], MonDisk.self) { [weak self] r in
            self?.diskLoading = false
            if case .success(let d) = r { self?.disk = d }
            if case .failure(let e) = r { self?.error = e.message }
        }
    }

    // Stop flow: the confirm sheet already ran. TERM → Rust polls up to 3 s →
    // still alive shows "บังคับปิด", which the user must click for SIGKILL.
    func stop(_ row: MonRow, group: Bool = false) {
        guard let id = row.ident else { return }
        phases[id] = .stopping
        signal(id, "term", group: group) { [weak self] alive in
            self?.phases[id] = alive ? .stillAlive : nil
            if alive { self?.notice = "\(row.name) (\(row.pid)) ยังไม่ปิดหลัง 3 วินาที" }
        }
    }

    func forceKill(_ row: MonRow, group: Bool = false) {
        guard let id = row.ident else { return }
        phases[id] = .killing
        signal(id, "kill", group: group) { [weak self] alive in
            self?.phases[id] = alive ? .stillAlive : nil
        }
    }

    private func signal(_ id: MonIdent, _ sig: String, group: Bool, done: @escaping (Bool) -> Void) {
        guard let engine else { return }
        let ident = Self.identJSON(id)
        Self.call(engine, ["op": "stop", "ident": ident, "signal": sig, "group": group], MonStopOutcome.self) { [weak self] r in
            guard let self else { return }
            switch r {
            case .failure(let e): self.notice = e.message; done(false)
            case .success(let o) where o.state != "signalled":
                self.notice = o.message
                done(false)
                self.refresh()
            case .success:
                Self.call(engine, ["op": "wait_exit", "ident": ident, "timeout_ms": 3000], MonStopOutcome.self) { r in
                    let alive = (try? r.get())?.state == "signalled"
                    done(alive)
                    self.refresh()
                }
            }
        }
    }

    struct CallError: Error { let message: String }

    static func identJSON(_ i: MonIdent) -> [String: Any] {
        ["pid": i.pid, "start_sec": i.startSec, "start_usec": i.startUsec, "path": i.path]
    }

    static func call<T: Decodable>(_ engine: Engine, _ req: [String: Any], _ type: T.Type, onEngineQueue: Bool = false,
                                   done: @escaping (Result<T, CallError>) -> Void) {
        guard let data = try? JSONSerialization.data(withJSONObject: req),
              let json = String(data: data, encoding: .utf8) else {
            done(.failure(CallError(message: "bad request")))
            return
        }
        engine.monitor(json, onEngineQueue: onEngineQueue) { out in
            let dec = JSONDecoder()
            dec.keyDecodingStrategy = .convertFromSnakeCase
            guard let env = try? dec.decode(Envelope<T>.self, from: Data(out.utf8)) else {
                done(.failure(CallError(message: "อ่านผลจาก monitor ไม่ได้")))
                return
            }
            if let r = env.result, env.ok { done(.success(r)) } else { done(.failure(CallError(message: env.error ?? "error"))) }
        }
    }
}

// ---- window ------------------------------------------------------------------

struct SystemMonitorView: View {
    @EnvironmentObject private var engine: Engine
    @StateObject private var model = MonitorModel()
    @StateObject private var projects = ProjectStore()
    @State private var tab: Tab = .services
    @State private var confirm: Confirm?

    enum Tab: String, CaseIterable { case services = "บริการ", chorus = "Chorus", disk = "พื้นที่" }

    struct Confirm: Identifiable {
        let row: MonRow
        let group: Bool
        var id: String { "\(row.pid)-\(group)" }
    }

    private var workspaces: [String] {
        var seen = Set<String>()
        return projects.projects.flatMap(\.folders).filter { seen.insert($0).inserted }
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            switch tab {
            case .services: servicesTable
            case .chorus: chorusList
            case .disk: DiskPane(model: model, workspaces: workspaces)
            }
        }
        .frame(minWidth: 860, minHeight: 420)
        .background(WindowVisibility { model.setVisible($0) })
        .onAppear { model.engine = engine }
        .onDisappear { model.setVisible(false) }
        .alert(item: $confirm) { c in
            Alert(
                title: Text(c.group ? "หยุดทุกอย่างที่ \(c.row.startedBy ?? c.row.name) เปิดไว้?" : "หยุด \(c.row.name)?"),
                message: Text(confirmText(c)),
                primaryButton: .destructive(Text("หยุด (SIGTERM)")) { model.stop(c.row, group: c.group) },
                secondaryButton: .cancel(Text("ยกเลิก"))
            )
        }
    }

    private func confirmText(_ c: Confirm) -> String {
        var lines = ["pid \(c.row.pid)", c.row.path]
        if let by = c.row.startedBy { lines.append("เปิดโดย: \(by) (ข้อมูลประกอบ ไม่ใช่หลักฐาน)") }
        if let f = c.row.footprint { lines.append("หน่วยความจำ \(MonFormat.bytes(f))") }
        if c.group { lines.append("ส่งสัญญาณถึงทั้ง process group ที่ Chorus สร้าง") }
        return lines.joined(separator: "\n")
    }

    private var header: some View {
        HStack(spacing: 16) {
            Picker("", selection: $tab) {
                ForEach(Tab.allCases, id: \.self) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .frame(width: 260)
            if let s = model.snapshot {
                stat("Chorus", MonFormat.bytes(s.chorusFootprint), String(format: "CPU %.1f%%", s.chorusCpuPct))
                if s.modelFootprint > 0 { stat("โมเดล local", MonFormat.bytes(s.modelFootprint), nil) }
            }
            Spacer()
            if tab == .services {
                Toggle("UDP", isOn: $model.udp).toggleStyle(.checkbox)
                    .help("แสดง UDP ด้วย (ซ่อนพอร์ต 0 และพอร์ตชั่วคราว 49152 ขึ้นไป)")
            }
            if let s = model.snapshot {
                Text(String(format: "สแกน %.1f ms", s.sweepMs)).font(.caption2).foregroundStyle(.tertiary).monospacedDigit()
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 10)
        .overlay(alignment: .bottomLeading) {
            if let n = model.notice ?? model.error {
                Text(n).font(.caption).foregroundStyle(.orange)
                    .padding(.horizontal, 14).offset(y: 14)
                    .onTapGesture { model.notice = nil }
            }
        }
    }

    private func stat(_ title: String, _ value: String, _ sub: String?) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(title).font(.caption2).foregroundStyle(.secondary)
            HStack(spacing: 6) {
                Text(value).font(.system(size: 13, weight: .semibold)).monospacedDigit()
                if let sub { Text(sub).font(.caption).foregroundStyle(.secondary).monospacedDigit() }
            }
        }
    }

    // Port · Service · Process · PID · CPU · RAM · started by · uptime
    private var servicesTable: some View {
        Table(model.snapshot?.services ?? []) {
            TableColumn("Port") { r in
                HStack(spacing: 4) {
                    Text(verbatim: ":\(r.port)").monospacedDigit()
                    if r.udp { Text("UDP").font(.caption2).foregroundStyle(.secondary) }
                }
                .help(r.addrs.joined(separator: ", "))
                .dimmed(!r.process.mine)
            }.width(min: 60, ideal: 70)
            TableColumn("Service") { r in
                Group {
                    if let s = r.service {
                        Text(s.source == "port" ? "มักเป็น \(s.name)" : s.name)
                            .foregroundStyle(s.source == "port" ? .secondary : .primary)
                    } else { Text("—").foregroundStyle(.tertiary) }
                }.dimmed(!r.process.mine)
            }.width(min: 90, ideal: 130)
            TableColumn("Process") { r in processCell(r.process) }.width(min: 100, ideal: 150)
            TableColumn("PID") { r in Text(verbatim: "\(r.process.pid)").monospacedDigit().dimmed(!r.process.mine) }
                .width(min: 50, ideal: 60)
            TableColumn("CPU") { r in cpuCell(r.process) }.width(min: 50, ideal: 60)
            TableColumn("RAM") { r in ramCell(r.process) }.width(min: 60, ideal: 80)
            TableColumn("เปิดโดย") { r in
                Text(r.process.startedBy ?? "—").foregroundStyle(r.process.startedBy == nil ? .tertiary : .secondary)
                    .dimmed(!r.process.mine)
            }.width(min: 80, ideal: 120)
            TableColumn("Uptime") { r in Text(MonFormat.uptime(r.process.uptimeS)).monospacedDigit().dimmed(!r.process.mine) }
                .width(min: 60, ideal: 80)
            TableColumn("") { r in actionCell(r.process, group: false) }.width(min: 90, ideal: 100)
        }
    }

    private var chorusList: some View {
        Table(model.snapshot?.chorus ?? []) {
            TableColumn("ส่วน") { r in Text(MonFormat.role(r.role)) }.width(min: 90, ideal: 110)
            TableColumn("Process") { r in processCell(r) }.width(min: 120, ideal: 180)
            TableColumn("PID") { r in Text(verbatim: "\(r.pid)").monospacedDigit() }.width(min: 50, ideal: 60)
            TableColumn("CPU") { r in cpuCell(r) }.width(min: 50, ideal: 60)
            TableColumn("RAM") { r in ramCell(r) }.width(min: 60, ideal: 80)
            TableColumn("เปิดโดย") { r in Text(r.startedBy ?? "—").foregroundStyle(.secondary) }.width(min: 80, ideal: 130)
            TableColumn("Uptime") { r in Text(MonFormat.uptime(r.uptimeS)).monospacedDigit() }.width(min: 60, ideal: 80)
            TableColumn("") { r in
                if r.groupStoppable { actionCell(r, group: true) } else if r.role != "app" && r.role != "daemon" { actionCell(r, group: false) }
            }.width(min: 120, ideal: 150)
        }
    }

    private func processCell(_ p: MonRow) -> some View {
        Text(p.name)
            .lineLimit(1)
            .help(([p.path] + (p.cmdline.isEmpty ? [] : [p.cmdline.joined(separator: " ")])).joined(separator: "\n"))
            .dimmed(!p.mine)
    }

    private func cpuCell(_ p: MonRow) -> some View {
        Text(p.cpuPct.map { String(format: "%.1f%%", $0) } ?? "—").monospacedDigit().dimmed(!p.mine)
    }

    private func ramCell(_ p: MonRow) -> some View {
        Text(p.footprint.map(MonFormat.bytes) ?? "—").monospacedDigit().dimmed(!p.mine)
            .help("Memory แบบเดียวกับ Activity Monitor (phys_footprint)")
    }

    @ViewBuilder
    private func actionCell(_ p: MonRow, group: Bool) -> some View {
        if let id = p.ident {
            switch model.phases[id] {
            case .stopping?, .killing?:
                ProgressView().controlSize(.small)
            case .stillAlive?:
                Button("บังคับปิด") { model.forceKill(p, group: group) }
                    .foregroundStyle(.red)
                    .help("ส่ง SIGKILL — process จะไม่ได้เก็บกวาดก่อนปิด")
            case nil:
                Button(group ? "หยุดทั้งกลุ่ม" : "หยุด") { confirm = Confirm(row: p, group: group) }
                    .help(group ? "หยุดทุกอย่างที่ agent นี้เปิด (killpg)" : "ส่ง SIGTERM หลังยืนยัน")
            }
        } else if p.mine, let note = p.refusalNote {
            Image(systemName: "lock").foregroundStyle(.tertiary).help(note)
        }
    }
}

private extension View {
    /// Other users' rows: drawn, but greyed and without actions.
    func dimmed(_ on: Bool) -> some View { opacity(on ? 0.45 : 1) }
}

// ---- disk + optimise ---------------------------------------------------------

private struct DiskPane: View {
    @ObservedObject var model: MonitorModel
    let workspaces: [String]
    @State private var workspace: String = ""
    @State private var days = 30

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                if let d = model.disk {
                    categories(d)
                    projects(d)
                } else if model.diskLoading {
                    ProgressView("กำลังวัดพื้นที่…")
                }
                optimise
            }
            .padding(16)
        }
        .onAppear {
            if workspace.isEmpty { workspace = workspaces.first(where: hasStore) ?? workspaces.first ?? "" }
            model.loadDisk(workspaces: workspaces)
        }
    }

    private func hasStore(_ w: String) -> Bool {
        FileManager.default.fileExists(atPath: (w as NSString).appendingPathComponent(".chorus"))
    }

    private func categories(_ d: MonDisk) -> some View {
        let c = d.categories
        let rows: [(String, UInt64)] = [("ledger (store.db)", c.ledger), ("checkpoints", c.checkpoints), ("worktrees", c.worktrees),
                                        ("models", c.models), ("memory", c.memory), ("cache", c.cache), ("อื่น ๆ ใน .chorus", c.other)]
        return VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("พื้นที่ที่ Chorus ใช้").font(.headline)
                Text(MonFormat.bytes(d.total)).font(.headline).monospacedDigit().foregroundStyle(.secondary)
                Spacer()
                Button("วัดใหม่") { model.loadDisk(workspaces: workspaces) }.disabled(model.diskLoading)
            }
            ForEach(rows, id: \.0) { name, bytes in
                HStack {
                    Text(name).frame(width: 150, alignment: .leading)
                    GeometryReader { g in
                        RoundedRectangle(cornerRadius: 3).fill(Color.accentColor.opacity(0.55))
                            .frame(width: d.total == 0 ? 0 : max(2, g.size.width * CGFloat(Double(bytes) / Double(d.total))))
                    }.frame(height: 8)
                    Text(MonFormat.bytes(bytes)).monospacedDigit().frame(width: 80, alignment: .trailing)
                }.font(.callout)
            }
        }
    }

    private func projects(_ d: MonDisk) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("แยกตามโปรเจกต์").font(.headline)
            ForEach(d.projects) { p in
                DisclosureGroup {
                    VStack(alignment: .leading, spacing: 3) {
                        line("ledger", p.ledger)
                        line("checkpoints", p.checkpoints)
                        line("worktrees", p.worktrees)
                        line("อื่น ๆ", p.other)
                        if p.freelistBytes > 0 { line("หน้าว่างใน store.db (VACUUM คืนได้)", p.freelistBytes) }
                        if !p.tables.isEmpty {
                            Text("ตารางใน store.db").font(.caption).foregroundStyle(.secondary).padding(.top, 4)
                            ForEach(p.tables, id: \.self) { t in line(t.name, t.bytes).font(.caption) }
                        }
                    }.padding(.leading, 8)
                } label: {
                    HStack {
                        Text((p.path as NSString).abbreviatingWithTildeInPath).lineLimit(1).truncationMode(.middle)
                        Spacer()
                        Text(MonFormat.bytes(p.total)).monospacedDigit()
                    }
                }
            }
        }
    }

    private func line(_ name: String, _ b: UInt64) -> some View {
        HStack { Text(name).foregroundStyle(.secondary); Spacer(); Text(MonFormat.bytes(b)).monospacedDigit() }
    }

    private var optimise: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("ทำให้เล็กลง").font(.headline)
                Spacer()
                Picker("Workspace", selection: $workspace) {
                    ForEach(workspaces, id: \.self) { Text(($0 as NSString).lastPathComponent).tag($0) }
                }.frame(maxWidth: 260)
            }
            Text("ทุกอย่างทำเมื่อกดเองเท่านั้น · ประเมินก่อนเสมอ").font(.caption).foregroundStyle(.secondary)
            OptimiseRow(model: model, title: "Blob GC", detail: "ลบ blob ที่ไม่มี event หรือไฟล์แนบอ้างถึง",
                        action: ["action": "blob_gc", "workspace": workspace], engineQueue: true, onDone: reload)
            OptimiseRow(model: model, title: "VACUUM", detail: "เขียน store.db ใหม่ให้ไม่มีหน้าว่าง (ทำได้เฉพาะตอนไม่มีเทิร์นรัน)",
                        action: ["action": "vacuum", "workspace": workspace], engineQueue: true, onDone: reload)
            OptimiseRow(model: model, title: "Pack checkpoints", detail: "รวม object ของ shadow repo เป็น pack — ไม่ลบอะไร",
                        action: ["action": "pack_checkpoints", "workspace": workspace], engineQueue: true, onDone: reload)
            OptimiseRow(model: model, title: "ลบ checkpoint เก่า", detail: "เก่ากว่า \(days) วัน · เก็บ review_base และ base ของ session ที่เปิดอยู่",
                        action: ["action": "prune_checkpoints", "workspace": workspace, "days": days], engineQueue: true, onDone: reload) {
                Stepper("\(days) วัน", value: $days, in: 1...365).fixedSize()
            }
            OptimiseRow(model: model, title: "ลบ worktree", detail: "เฉพาะที่ merge แล้ว ทิ้งแล้ว หรือไม่มีเจ้าของ (ที่ยังมีงานค้างเลือกไม่ได้)",
                        action: ["action": "prune_worktrees", "workspace": workspace], engineQueue: true, onDone: reload)
            OptimiseRow(model: model, title: "Unload โมเดล", detail: "เอาโมเดลออกจาก RAM ของ Ollama (ไฟล์ยังอยู่)",
                        action: ["action": "unload_model"], onDone: reload)
            OptimiseRow(model: model, title: "ล้าง cache", detail: "~/.cache/chorus — สร้างใหม่ได้เอง",
                        action: ["action": "clear_caches"], onDone: reload)
            OptimiseRow(model: model, title: "ลบโมเดลที่ไม่ใช้", detail: "ไฟล์ใน models ของ Chorus — เลือกเองว่าจะลบตัวไหน",
                        action: ["action": "delete_models"], onDone: reload)
        }
    }

    private func reload() { model.loadDisk(workspaces: workspaces) }
}

private struct OptimiseRow<Extra: View>: View {
    @ObservedObject var model: MonitorModel
    let title: String
    let detail: String
    let action: [String: Any]
    var engineQueue = false
    let onDone: () -> Void
    @ViewBuilder var extra: () -> Extra

    @State private var preview: MonPreview?
    @State private var picked: Set<String> = []
    @State private var busy = false
    @State private var result: String?
    @State private var confirming = false

    init(model: MonitorModel, title: String, detail: String, action: [String: Any], engineQueue: Bool = false,
         onDone: @escaping () -> Void, @ViewBuilder extra: @escaping () -> Extra = { EmptyView() }) {
        self.model = model
        self.title = title
        self.detail = detail
        self.action = action
        self.engineQueue = engineQueue
        self.onDone = onDone
        self.extra = extra
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).font(.body.weight(.medium))
                    Text(detail).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                extra()
                if let p = preview {
                    Text("~\(MonFormat.bytes(p.items.isEmpty ? p.bytes : pickedBytes(p)))").monospacedDigit().foregroundStyle(.secondary)
                }
                Button("ประเมิน") { runPreview() }.disabled(busy)
                Button(preview?.destructive == true ? "ลบ…" : "ทำเลย…") { confirming = true }
                    .disabled(busy || preview == nil || preview?.allowed == false || (preview?.items.isEmpty == false && picked.isEmpty))
            }
            if let p = preview {
                Text(p.note).font(.caption).foregroundStyle(.secondary)
                ForEach(p.items) { it in
                    Toggle(isOn: Binding(get: { picked.contains(it.id) },
                                         set: { if $0 { picked.insert(it.id) } else { picked.remove(it.id) } })) {
                        HStack {
                            Text(it.label).lineLimit(1).truncationMode(.middle)
                            Text(it.kind).font(.caption2).foregroundStyle(.secondary)
                            Spacer()
                            Text(MonFormat.bytes(it.bytes)).monospacedDigit().font(.caption)
                        }
                    }
                    .toggleStyle(.checkbox)
                    .disabled(!it.selectable)
                }
            }
            if let result { Text(result).font(.caption).foregroundStyle(.green) }
        }
        .padding(10)
        .background(Color.primary.opacity(0.03), in: RoundedRectangle(cornerRadius: 8))
        .onChange(of: actionKey) { _, _ in preview = nil; picked = []; result = nil }
        .alert(isPresented: $confirming) {
            Alert(
                title: Text(preview?.destructive == true ? "\(title)? ลบแล้วกู้คืนไม่ได้" : "\(title)?"),
                message: Text("ประมาณ \(MonFormat.bytes(preview.map { $0.items.isEmpty ? $0.bytes : pickedBytes($0) } ?? 0))"),
                primaryButton: preview?.destructive == true ? .destructive(Text("ลบ")) { apply() } : .default(Text("ทำเลย")) { apply() },
                secondaryButton: .cancel(Text("ยกเลิก"))
            )
        }
    }

    private var actionKey: String {
        (try? JSONSerialization.data(withJSONObject: action, options: .sortedKeys)).flatMap { String(data: $0, encoding: .utf8) } ?? ""
    }

    private func pickedBytes(_ p: MonPreview) -> UInt64 {
        p.items.filter { picked.contains($0.id) }.reduce(0) { $0 + $1.bytes }
    }

    private func runPreview() {
        guard let engine = model.engine else { return }
        busy = true
        result = nil
        MonitorModel.call(engine, ["op": "preview", "action": action], MonPreview.self, onEngineQueue: engineQueue) { r in
            busy = false
            switch r {
            case .success(let p): preview = p; picked = []
            case .failure(let e): result = nil; model.notice = e.message
            }
        }
    }

    private func apply() {
        guard let engine = model.engine else { return }
        var a = action
        if preview?.items.isEmpty == false { a["ids"] = Array(picked) }
        busy = true
        MonitorModel.call(engine, ["op": "apply", "action": a], MonApplied.self, onEngineQueue: engineQueue) { r in
            busy = false
            preview = nil
            picked = []
            switch r {
            case .success(let x): result = x.message; onDone()
            case .failure(let e): model.notice = e.message
            }
        }
    }
}

/// Reports whether the hosting window is actually on screen (not hidden,
/// minimised or fully covered), so polling stops when nobody is looking.
private struct WindowVisibility: NSViewRepresentable {
    let onChange: (Bool) -> Void

    func makeNSView(context: Context) -> NSView {
        let v = Probe()
        v.onChange = onChange
        return v
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        (nsView as? Probe)?.onChange = onChange
    }

    final class Probe: NSView {
        var onChange: ((Bool) -> Void)?
        private var tokens: [NSObjectProtocol] = []
        private var last: Bool?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            tokens.forEach(NotificationCenter.default.removeObserver)
            tokens = []
            guard let w = window else { report(false); return }
            let names: [Notification.Name] = [NSWindow.didChangeOcclusionStateNotification, NSWindow.didMiniaturizeNotification,
                                              NSWindow.didDeminiaturizeNotification, NSWindow.willCloseNotification]
            for n in names {
                tokens.append(NotificationCenter.default.addObserver(forName: n, object: w, queue: .main) { [weak self] note in
                    let closing = note.name == NSWindow.willCloseNotification
                    MainActor.assumeIsolated { self?.evaluate(closing: closing) }
                })
            }
            evaluate(closing: false)
        }

        private func evaluate(closing: Bool) {
            guard let w = window, !closing else { report(false); return }
            report(w.occlusionState.contains(.visible) && !w.isMiniaturized)
        }

        private func report(_ v: Bool) {
            guard v != last else { return }
            last = v
            onChange?(v)
        }

        deinit { tokens.forEach(NotificationCenter.default.removeObserver) }
    }
}

// ---- sidebar footer label --------------------------------------------------------

/// "Chorus 412 MB" in the sidebar footer; opens the monitor window. Reads
/// only the Chorus group (no port sweep), every 5 s while the app is active.
struct ChorusRAMLabel: View {
    @EnvironmentObject private var engine: Engine
    @Environment(\.openWindow) private var openWindow
    @State private var bytes: UInt64?
    private let tick = Timer.publish(every: 5, on: .main, in: .common).autoconnect()

    var body: some View {
        Button { openWindow(id: SystemMonitorView.windowID) } label: {
            HStack(spacing: 3) {
                Image(systemName: "memorychip").font(.system(size: 9))
                Text(bytes.map(MonFormat.bytes) ?? "—").font(.system(size: 10, weight: .medium)).monospacedDigit()
            }
            .foregroundStyle(.secondary)
            .padding(.horizontal, 6).padding(.vertical, 4)
            .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 6, style: .continuous))
        }
        .buttonStyle(.plain)
        .help("RAM ที่ Chorus ใช้ · เปิด System Monitor (⌥⌘M)")
        .onAppear(perform: load)
        .onReceive(tick) { _ in if NSApp.isActive { load() } }
    }

    private func load() {
        MonitorModel.call(engine, ["op": "snapshot", "chorus_only": true], MonSnapshot.self) { r in
            if case .success(let s) = r { bytes = s.chorusFootprint }
        }
    }
}

extension SystemMonitorView {
    static let windowID = "system-monitor"
}
