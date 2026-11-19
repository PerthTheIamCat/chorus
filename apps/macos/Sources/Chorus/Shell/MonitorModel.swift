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

