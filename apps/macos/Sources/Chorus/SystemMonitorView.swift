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
    /// "local" (loopback only) or "network". Decided in Rust (`monitor::reach`).
    let reach: String
    /// "mine" | "chorus" | "background" | "other" (`monitor::Origin`).
    let origin: String
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

// ---- popover -----------------------------------------------------------------

struct SystemMonitorView: View {
    @EnvironmentObject private var engine: Engine
    @StateObject private var model = MonitorModel()
    @StateObject private var projects = ProjectStore()
    @State private var tab: Tab = .services
    @State private var confirming: Confirm?
    @State private var reachFilter: ReachFilter = .all

    enum Tab: String, CaseIterable { case services = "บริการ", chorus = "Chorus", disk = "พื้นที่" }
    enum ReachFilter: String, CaseIterable { case all = "ทั้งหมด", local = "localhost", network = "เครือข่าย" }

    /// The groups the services list is split into, in Rust's `Origin` order.
    struct ServiceGroup: Identifiable {
        let id: String
        let title: String
        let rows: [MonService]
    }

    private static let groupTitles: [(String, String)] = [
        ("mine", "รันเอง (จาก terminal)"),
        ("chorus", "Chorus"),
        ("background", "แอปและบริการเบื้องหลัง"),
        ("other", "ระบบและผู้ใช้อื่น"),
    ]

    private var serviceGroups: [ServiceGroup] {
        let all = (model.snapshot?.services ?? []).filter { r in
            switch reachFilter {
            case .all: return true
            case .local: return r.reach == "local"
            case .network: return r.reach == "network"
            }
        }
        return Self.groupTitles.compactMap { key, title in
            let rows = all.filter { $0.origin == key }
            return rows.isEmpty ? nil : ServiceGroup(id: key, title: "\(title) · \(rows.count)", rows: rows)
        }
    }

    struct Confirm: Equatable {
        let pid: Int32
        let group: Bool
    }

    private var workspaces: [String] {
        var seen = Set<String>()
        return projects.projects.flatMap(\.folders).filter { seen.insert($0).inserted }
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            if let n = model.notice ?? model.error {
                HStack(spacing: 6) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .font(.system(size: 10))
                    Text(n)
                        .font(.caption2)
                        .lineLimit(2)
                    Spacer()
                    Image(systemName: "xmark")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundStyle(.secondary)
                }
                .foregroundStyle(.orange)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .background(Color.orange.opacity(0.1))
                .contentShape(Rectangle())
                .onTapGesture { model.notice = nil; model.error = nil }
            }
            Divider().opacity(0.5)
            switch tab {
            case .services: servicesList
            case .chorus: chorusList
            case .disk: DiskPane(model: model, workspaces: workspaces)
            }
        }
        .frame(width: 390, height: 460)
        .onAppear {
            model.engine = engine
            model.setVisible(true)
        }
        .onDisappear { model.setVisible(false) }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: "memorychip")
                    .font(.system(size: 12, weight: .bold))
                    .foregroundStyle(.secondary)
                Text("System Monitor")
                    .font(.system(size: 12.5, weight: .bold))
                Spacer()
                if let s = model.snapshot {
                    HStack(spacing: 6) {
                        Label(MonFormat.bytes(s.chorusFootprint), systemImage: "memorychip")
                        Text(String(format: "CPU %.1f%%", s.chorusCpuPct))
                        if s.modelFootprint > 0 {
                            Text("· โมเดล \(MonFormat.bytes(s.modelFootprint))")
                        }
                    }
                    .font(.system(size: 10.5, weight: .medium, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Color.primary.opacity(0.05), in: Capsule())
                }
            }

            HStack(spacing: 8) {
                Picker("", selection: $tab) {
                    ForEach(Tab.allCases, id: \.self) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .controlSize(.small)

                Spacer()

                if tab == .services {
                    Toggle("UDP", isOn: $model.udp)
                        .toggleStyle(.checkbox)
                        .controlSize(.small)
                        .font(.caption)
                        .help("แสดง UDP ด้วย (ซ่อนพอร์ต 0 และพอร์ตชั่วคราว 49152 ขึ้นไป)")
                }
                if let s = model.snapshot {
                    Text(String(format: "%.0f ms", s.sweepMs))
                        .font(.system(size: 9.5, design: .monospaced))
                        .foregroundStyle(.tertiary)
                }
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
    }

    // ---- services compact list -----------------------------------------------

    private var servicesList: some View {
        let groups = serviceGroups
        return VStack(spacing: 0) {
            Picker("", selection: $reachFilter) {
                ForEach(ReachFilter.allCases, id: \.self) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .controlSize(.small)
            .padding(.horizontal, 10)
            .padding(.top, 8)
            .help("localhost = เข้าได้จากเครื่องนี้เท่านั้น · เครือข่าย = เครื่องอื่นใน network เดียวกันก็เข้าได้")
            if groups.isEmpty {
                VStack(spacing: 6) {
                    Image(systemName: "network")
                        .font(.system(size: 20))
                        .foregroundStyle(.tertiary)
                    Text(reachFilter == .all ? "ไม่มีพอร์ตที่เปิดรับการเชื่อมต่อ" : "ไม่มีพอร์ตแบบ \(reachFilter.rawValue)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 6) {
                        ForEach(groups) { g in
                            Text(g.title)
                                .font(.system(size: 10.5, weight: .semibold))
                                .foregroundStyle(.secondary)
                                .padding(.top, g.id == groups.first?.id ? 0 : 6)
                            ForEach(g.rows) { r in
                                serviceRow(r)
                            }
                        }
                    }
                    .padding(10)
                }
            }
        }
    }

    private func serviceRow(_ r: MonService) -> some View {
        let p = r.process
        let isConfirming = confirming == Confirm(pid: p.pid, group: false)
        return VStack(alignment: .leading, spacing: 5) {
            HStack(alignment: .center, spacing: 6) {
                HStack(spacing: 3) {
                    Text(verbatim: ":\(r.port)")
                        .font(.system(size: 11.5, weight: .semibold, design: .monospaced))
                    if r.udp {
                        Text("UDP")
                            .font(.system(size: 8.5, weight: .bold))
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(.horizontal, 5)
                .padding(.vertical, 2)
                .background(Color.accentColor.opacity(0.12), in: RoundedRectangle(cornerRadius: 4, style: .continuous))
                .help(r.addrs.joined(separator: ", "))

                reachBadge(r)

                Text(p.name)
                    .font(.system(size: 12, weight: .medium))
                    .lineLimit(1)

                if let s = r.service {
                    Text(s.source == "port" ? "(\(s.name))" : "· \(s.name)")
                        .font(.system(size: 10.5))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                Spacer(minLength: 4)

                actionControl(p, group: false)
            }

            HStack(spacing: 8) {
                Text(verbatim: "PID \(p.pid)")
                if let cpu = p.cpuPct {
                    Text(String(format: "CPU %.1f%%", cpu))
                }
                if let ram = p.footprint {
                    Text(MonFormat.bytes(ram))
                        .help("Memory แบบเดียวกับ Activity Monitor (phys_footprint)")
                }
                if let u = p.uptimeS {
                    Text(MonFormat.uptime(u))
                }
                if let by = p.startedBy {
                    Text("· เปิดโดย \(by)")
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
            }
            .font(.system(size: 10, design: .monospaced))
            .foregroundStyle(.tertiary)

            if isConfirming {
                confirmBanner(p, group: false)
            }
        }
        .padding(8)
        .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .help(([p.path] + (p.cmdline.isEmpty ? [] : [p.cmdline.joined(separator: " ")])).joined(separator: "\n"))
        .dimmed(!p.mine)
    }

    private func reachBadge(_ r: MonService) -> some View {
        let local = r.reach == "local"
        return Image(systemName: local ? "lock.fill" : "network")
            .font(.system(size: 9.5, weight: .semibold))
            .foregroundStyle(local ? Color.secondary : Color.orange)
            .help(local
                  ? "เฉพาะเครื่องนี้ (\(r.addrs.joined(separator: ", "))) — เครื่องอื่นเข้าไม่ได้"
                  : "เปิดสู่เครือข่าย (\(r.addrs.joined(separator: ", "))) — เครื่องอื่นใน network เดียวกันเข้าได้")
    }

    // ---- chorus compact list -------------------------------------------------

    private var chorusList: some View {
        let rows = model.snapshot?.chorus ?? []
        return ScrollView {
            LazyVStack(spacing: 6) {
                ForEach(rows) { r in
                    chorusRow(r)
                }
            }
            .padding(10)
        }
    }

    private func chorusRow(_ r: MonRow) -> some View {
        let group = r.groupStoppable
        let canStop = group || (r.role != "app" && r.role != "daemon")
        let isConfirming = confirming == Confirm(pid: r.pid, group: group)
        return VStack(alignment: .leading, spacing: 5) {
            HStack(alignment: .center, spacing: 6) {
                Text(MonFormat.role(r.role))
                    .font(.system(size: 9.5, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 5)
                    .padding(.vertical, 2)
                    .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 4, style: .continuous))

                Text(r.name)
                    .font(.system(size: 12, weight: .medium))
                    .lineLimit(1)

                Spacer(minLength: 4)

                if canStop {
                    actionControl(r, group: group)
                }
            }

            HStack(spacing: 8) {
                Text(verbatim: "PID \(r.pid)")
                if let cpu = r.cpuPct {
                    Text(String(format: "CPU %.1f%%", cpu))
                }
                if let ram = r.footprint {
                    Text(MonFormat.bytes(ram))
                }
                if let u = r.uptimeS {
                    Text(MonFormat.uptime(u))
                }
                if let by = r.startedBy {
                    Text("· \(by)")
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
            }
            .font(.system(size: 10, design: .monospaced))
            .foregroundStyle(.tertiary)

            if isConfirming {
                confirmBanner(r, group: group)
            }
        }
        .padding(8)
        .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .help(([r.path] + (r.cmdline.isEmpty ? [] : [r.cmdline.joined(separator: " ")])).joined(separator: "\n"))
    }

    // ---- inline action & confirmation ----------------------------------------

    @ViewBuilder
    private func actionControl(_ p: MonRow, group: Bool) -> some View {
        if let id = p.ident {
            switch model.phases[id] {
            case .stopping?, .killing?:
                ProgressView().controlSize(.mini)
            case .stillAlive?:
                Button("บังคับปิด") { model.forceKill(p, group: group) }
                    .buttonStyle(.borderedProminent)
                    .tint(.red)
                    .controlSize(.mini)
                    .help("ส่ง SIGKILL — process จะไม่ได้เก็บกวาดก่อนปิด")
            case nil:
                let target = Confirm(pid: p.pid, group: group)
                if confirming == target {
                    HStack(spacing: 4) {
                        Button("ยืนยันหยุด") {
                            confirming = nil
                            model.stop(p, group: group)
                        }
                        .buttonStyle(.borderedProminent)
                        .tint(.red)
                        .controlSize(.mini)

                        Button("ยกเลิก") {
                            confirming = nil
                        }
                        .buttonStyle(.bordered)
                        .controlSize(.mini)
                    }
                } else {
                    Button(group ? "หยุดกลุ่ม" : "หยุด") {
                        confirming = target
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.mini)
                    .help(group ? "หยุดทุกอย่างที่ agent นี้เปิด (killpg)" : "ส่ง SIGTERM หลังยืนยัน")
                }
            }
        } else if p.mine, let note = p.refusalNote {
            Image(systemName: "lock")
                .font(.system(size: 10))
                .foregroundStyle(.tertiary)
                .help(note)
        }
    }

    private func confirmBanner(_ p: MonRow, group: Bool) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(p.path)
                .font(.system(size: 9.5, design: .monospaced))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            if group {
                Text("ส่ง SIGTERM ถึงทั้ง process group ที่ Chorus สร้าง")
                    .font(.system(size: 9.5))
                    .foregroundStyle(.orange)
            } else if let by = p.startedBy {
                Text("เปิดโดย: \(by) · ส่ง SIGTERM")
                    .font(.system(size: 9.5))
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.top, 2)
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
            VStack(alignment: .leading, spacing: 14) {
                if let d = model.disk {
                    categories(d)
                    if !d.projects.isEmpty {
                        projects(d)
                    }
                } else if model.diskLoading {
                    HStack {
                        Spacer()
                        ProgressView("กำลังวัดพื้นที่…").controlSize(.small)
                        Spacer()
                    }
                    .padding(.vertical, 12)
                }
                optimise
            }
            .padding(12)
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
        let rows: [(String, UInt64)] = [
            ("ledger (store.db)", c.ledger), ("checkpoints", c.checkpoints), ("worktrees", c.worktrees),
            ("models", c.models), ("memory", c.memory), ("cache", c.cache), ("อื่น ๆ", c.other),
        ]
        return VStack(alignment: .leading, spacing: 5) {
            HStack {
                Text("พื้นที่ที่ Chorus ใช้").font(.system(size: 11.5, weight: .semibold))
                Text(MonFormat.bytes(d.total)).font(.system(size: 11.5, weight: .semibold, design: .monospaced)).foregroundStyle(.secondary)
                Spacer()
                Button("วัดใหม่") { model.loadDisk(workspaces: workspaces) }
                    .controlSize(.mini)
                    .disabled(model.diskLoading)
            }
            ForEach(rows, id: \.0) { name, bytes in
                HStack(spacing: 6) {
                    Text(name)
                        .font(.system(size: 10.5))
                        .frame(width: 105, alignment: .leading)
                        .lineLimit(1)
                    GeometryReader { g in
                        RoundedRectangle(cornerRadius: 2.5).fill(Color.accentColor.opacity(0.55))
                            .frame(width: d.total == 0 ? 0 : max(2, g.size.width * CGFloat(Double(bytes) / Double(d.total))))
                    }.frame(height: 6)
                    Text(MonFormat.bytes(bytes))
                        .font(.system(size: 10, design: .monospaced))
                        .frame(width: 64, alignment: .trailing)
                }
            }
        }
    }

    private func projects(_ d: MonDisk) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("แยกตามโปรเจกต์").font(.system(size: 11.5, weight: .semibold))
            ForEach(d.projects) { p in
                DisclosureGroup {
                    VStack(alignment: .leading, spacing: 2) {
                        line("ledger", p.ledger)
                        line("checkpoints", p.checkpoints)
                        line("worktrees", p.worktrees)
                        line("อื่น ๆ", p.other)
                        if p.freelistBytes > 0 { line("หน้าว่างใน store.db", p.freelistBytes) }
                    }
                    .font(.system(size: 10))
                    .padding(.leading, 6)
                } label: {
                    HStack {
                        Text((p.path as NSString).abbreviatingWithTildeInPath)
                            .font(.system(size: 10.5))
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Spacer()
                        Text(MonFormat.bytes(p.total))
                            .font(.system(size: 10, design: .monospaced))
                    }
                }
            }
        }
    }

    private func line(_ name: String, _ b: UInt64) -> some View {
        HStack { Text(name).foregroundStyle(.secondary); Spacer(); Text(MonFormat.bytes(b)).monospacedDigit() }
    }

    private var optimise: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("ทำให้เล็กลง").font(.system(size: 11.5, weight: .semibold))
                Spacer()
                if !workspaces.isEmpty {
                    Picker("", selection: $workspace) {
                        ForEach(workspaces, id: \.self) { Text(($0 as NSString).lastPathComponent).tag($0) }
                    }
                    .labelsHidden()
                    .controlSize(.small)
                    .frame(maxWidth: 160)
                }
            }
            Text("ทำเมื่อกดเองเท่านั้น · ประเมินก่อนเสมอ").font(.system(size: 10)).foregroundStyle(.secondary)
            OptimiseRow(model: model, title: "Blob GC", detail: "ลบ blob ที่ไม่มีไฟล์อ้างถึง",
                        action: ["action": "blob_gc", "workspace": workspace], engineQueue: true, onDone: reload)
            OptimiseRow(model: model, title: "VACUUM", detail: "เขียน store.db ใหม่ (เฉพาะตอนว่าง)",
                        action: ["action": "vacuum", "workspace": workspace], engineQueue: true, onDone: reload)
            OptimiseRow(model: model, title: "Pack checkpoints", detail: "รวม object ของ shadow repo เป็น pack",
                        action: ["action": "pack_checkpoints", "workspace": workspace], engineQueue: true, onDone: reload)
            OptimiseRow(model: model, title: "ลบ checkpoint เก่า", detail: "เก่ากว่า \(days) วัน (เก็บ review_base)",
                        action: ["action": "prune_checkpoints", "workspace": workspace, "days": days], engineQueue: true, onDone: reload) {
                Stepper("\(days) วัน", value: $days, in: 1...365)
                    .controlSize(.mini)
                    .font(.system(size: 10))
                    .fixedSize()
            }
            OptimiseRow(model: model, title: "ลบ worktree", detail: "เฉพาะที่ merge/ทิ้งแล้ว หรือไม่มีเจ้าของ",
                        action: ["action": "prune_worktrees", "workspace": workspace], engineQueue: true, onDone: reload)
            OptimiseRow(model: model, title: "Unload โมเดล", detail: "เอาโมเดลออกจาก RAM ของ Ollama",
                        action: ["action": "unload_model"], onDone: reload)
            OptimiseRow(model: model, title: "ล้าง cache", detail: "~/.cache/chorus",
                        action: ["action": "clear_caches"], onDone: reload)
            OptimiseRow(model: model, title: "ลบโมเดลที่ไม่ใช้", detail: "เลือกไฟล์โมเดลที่จะลบ",
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
        VStack(alignment: .leading, spacing: 5) {
            HStack(alignment: .center, spacing: 6) {
                VStack(alignment: .leading, spacing: 1) {
                    Text(title).font(.system(size: 11.5, weight: .medium))
                    Text(detail).font(.system(size: 10)).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer(minLength: 4)
                extra()
                if let p = preview {
                    Text("~\(MonFormat.bytes(p.items.isEmpty ? p.bytes : pickedBytes(p)))")
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(.secondary)
                }
                if confirming {
                    Button(preview?.destructive == true ? "ยืนยันลบ" : "ยืนยัน") {
                        confirming = false
                        apply()
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(preview?.destructive == true ? .red : .accentColor)
                    .controlSize(.mini)

                    Button("ยกเลิก") {
                        confirming = false
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.mini)
                } else {
                    Button("ประเมิน") { runPreview() }
                        .controlSize(.mini)
                        .disabled(busy)
                    if preview != nil {
                        Button(preview?.destructive == true ? "ลบ…" : "ทำเลย…") { confirming = true }
                            .controlSize(.mini)
                            .disabled(busy || preview?.allowed == false || (preview?.items.isEmpty == false && picked.isEmpty))
                    }
                }
            }
            if let p = preview {
                Text(p.note).font(.system(size: 9.5)).foregroundStyle(.secondary)
                ForEach(p.items) { it in
                    Toggle(isOn: Binding(get: { picked.contains(it.id) },
                                         set: { if $0 { picked.insert(it.id) } else { picked.remove(it.id) } })) {
                        HStack {
                            Text(it.label).font(.system(size: 10)).lineLimit(1).truncationMode(.middle)
                            Text(it.kind).font(.system(size: 9)).foregroundStyle(.secondary)
                            Spacer()
                            Text(MonFormat.bytes(it.bytes)).font(.system(size: 9.5, design: .monospaced))
                        }
                    }
                    .toggleStyle(.checkbox)
                    .controlSize(.small)
                    .disabled(!it.selectable)
                }
            }
            if let result { Text(result).font(.system(size: 10)).foregroundStyle(.green) }
        }
        .padding(8)
        .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 7, style: .continuous))
        .onChange(of: actionKey) { _, _ in preview = nil; picked = []; result = nil; confirming = false }
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
        confirming = false
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

// ---- sidebar footer label --------------------------------------------------------

/// "Chorus 412 MB" in the sidebar footer; toggles the monitor popover. Reads
/// only the Chorus group (no port sweep), every 5 s while the app is active.
struct ChorusRAMLabel: View {
    @EnvironmentObject private var engine: Engine
    @Binding var isPresented: Bool
    @State private var bytes: UInt64?
    private let tick = Timer.publish(every: 5, on: .main, in: .common).autoconnect()

    var body: some View {
        Button { isPresented.toggle() } label: {
            HStack(spacing: 3) {
                Image(systemName: "memorychip").font(.system(size: 9))
                Text(bytes.map(MonFormat.bytes) ?? "—").font(.system(size: 10, weight: .medium)).monospacedDigit()
            }
            .foregroundStyle(.secondary)
            .padding(.horizontal, 6).padding(.vertical, 4)
            .background(Color.primary.opacity(isPresented ? 0.1 : 0.04), in: RoundedRectangle(cornerRadius: 6, style: .continuous))
        }
        .buttonStyle(.plain)
        .popover(isPresented: $isPresented, arrowEdge: .top) {
            SystemMonitorView()
                .environmentObject(engine)
        }
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

