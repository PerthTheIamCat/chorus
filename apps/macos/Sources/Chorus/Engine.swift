import CChorus
import Foundation

/// A ledger event as the shell sees it. Mirrors the FFI JSON.
struct LedgerEvent: Codable, Identifiable, Hashable {
    let seq: Int64
    let kind: String
    let actor: String
    let body: String
    var id: Int64 { seq }
}

struct TurnResult: Codable {
    let ok: Bool
    let user_seq: Int64?
    let agent_event_count: Int?
    let message: String?
    let context_used: Int64?
    let context_window: Int64?
    let stop_reason: String?
    let error: String?
}

struct DiffResult: Codable {
    let ok: Bool?
    let diff: String?
}

struct SimpleResult: Codable {
    let ok: Bool?
    let error: String?
}

struct HandoffResult: Codable {
    let ok: Bool
    let handoff_seq: Int64?
    let from: String?
    let to: String?
    let brief_markdown: String?
    let echo_back: String?
    let stop_reason: String?
    let error: String?
}

/// Thin Swift front for the Chorus C ABI. All logic is in Rust; this class
/// only marshals strings and hops threads (design rule 2.3).
@MainActor
final class Engine: ObservableObject {
    @Published var events: [LedgerEvent] = []
    @Published var busy = false
    @Published var lastError: String?
    @Published var sessionID: Int64 = -1
    @Published var contextUsed: Int64?
    @Published var contextWindow: Int64?

    private var handle: OpaquePointer?
    private let queue = DispatchQueue(label: "chorus.engine")

    func open(db: String, cwd: String, title: String) {
        close()
        handle = db.withCString { d in
            cwd.withCString { c in
                title.withCString { t in
                    chorus_engine_open(d, c, t)
                }
            }
        }
        guard let h = handle else {
            lastError = "could not open a session (is the workspace path valid?)"
            return
        }
        sessionID = chorus_engine_session_id(h)
        refresh()
    }

    func close() {
        if let h = handle { chorus_engine_free(h) }
        handle = nil
    }

    // MARK: interactive permission

    struct PermissionAsk: Identifiable {
        let id = UUID()
        let tool: String
        let kind: String
        let detail: String
    }
    @Published var pendingAsk: PermissionAsk?
    private let permGate = DispatchSemaphore(value: 0)
    private var permAnswer = false

    /// Called from the C callback on the engine's background thread. Publishes
    /// the ask to the UI and blocks until `answerPermission` is called.
    fileprivate func askPermissionBlocking(_ json: String) -> Bool {
        struct Raw: Codable { let tool: String?; let kind: String?; let detail: String? }
        let raw = (try? JSONDecoder().decode(Raw.self, from: Data(json.utf8)))
        DispatchQueue.main.async {
            self.pendingAsk = PermissionAsk(
                tool: raw?.tool ?? "action", kind: raw?.kind ?? "", detail: raw?.detail ?? "")
        }
        permGate.wait()
        return permAnswer
    }

    func answerPermission(_ allow: Bool) {
        permAnswer = allow
        pendingAsk = nil
        permGate.signal()
    }

    /// Run a turn where every permission request pops a dialog.
    func sendInteractive(_ prompt: String, agent: String) {
        guard let h = handle, !busy else { return }
        busy = true; lastError = nil
        let ctx = Unmanaged.passUnretained(self).toOpaque()
        let cb: @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>?) -> Int32 = { ctx, ask in
            guard let ctx, let ask else { return 0 }
            let engine = Unmanaged<Engine>.fromOpaque(ctx).takeUnretainedValue()
            return engine.askPermissionBlocking(String(cString: ask)) ? 1 : 0
        }
        queue.async { [weak self] in
            let json = prompt.withCString { p in
                agent.withCString { a in
                    Self.string(from: chorus_run_turn_interactive(h, a, p, cb, ctx))
                }
            }
            let r = Self.decode(TurnResult.self, json)
            DispatchQueue.main.async {
                self?.busy = false
                if r?.ok != true { self?.lastError = r?.error ?? "turn failed" }
                self?.refresh()
            }
        }
    }

    /// Run one turn, then refresh the transcript.
    func send(_ prompt: String, agent: String, allow: Bool) {
        guard let h = handle, !busy else { return }
        busy = true
        lastError = nil
        queue.async { [weak self] in
            let json = prompt.withCString { p in
                agent.withCString { a in
                    Self.string(from: chorus_run_turn(h, a, p, allow ? 1 : 0))
                }
            }
            let result = Self.decode(TurnResult.self, json)
            DispatchQueue.main.async {
                self?.busy = false
                if let r = result, r.ok {
                    self?.contextUsed = r.context_used
                    self?.contextWindow = r.context_window
                } else {
                    self?.lastError = result?.error ?? "turn failed"
                }
                self?.refresh()
            }
        }
    }

    /// Run one turn against a relay chain (comma-separated agent names).
    func sendRelay(_ prompt: String, agents: String, allow: Bool) {
        guard let h = handle, !busy else { return }
        busy = true; lastError = nil
        queue.async { [weak self] in
            let json = prompt.withCString { p in
                agents.withCString { a in
                    Self.string(from: chorus_run_relay(h, a, p, allow ? 1 : 0))
                }
            }
            let obj = Self.decode(SimpleResult.self, json)
            DispatchQueue.main.async {
                self?.busy = false
                if obj?.ok != true { self?.lastError = obj?.error ?? "relay failed" }
                self?.refresh()
            }
        }
    }

    /// Compare mode — fork per agent, run the prompt on each.
    func compare(_ prompt: String, agents: String, allow: Bool,
                 completion: @escaping (String) -> Void) {
        guard let h = handle, !busy else { return }
        busy = true; lastError = nil
        queue.async { [weak self] in
            let json = prompt.withCString { p in
                agents.withCString { a in
                    Self.string(from: chorus_compare(h, a, p, allow ? 1 : 0))
                }
            }
            DispatchQueue.main.async {
                self?.busy = false
                self?.refresh()
                completion(json)
            }
        }
    }

    /// Hand the session to another agent.
    func handoff(to: String, reason: String, goal: String, done: [String], pending: [String], allow: Bool,
                 completion: @escaping (HandoffResult?) -> Void) {
        guard let h = handle, !busy else { return }
        busy = true
        lastError = nil
        let doneJSON = Self.jsonArray(done)
        let pendingJSON = Self.jsonArray(pending)
        queue.async { [weak self] in
            let json = to.withCString { t in
                reason.withCString { r in
                    goal.withCString { g in
                        doneJSON.withCString { d in
                            pendingJSON.withCString { p in
                                Self.string(from: chorus_handoff(h, t, r, g, d, p, allow ? 1 : 0))
                            }
                        }
                    }
                }
            }
            let result = Self.decode(HandoffResult.self, json)
            DispatchQueue.main.async {
                self?.busy = false
                if result?.ok != true { self?.lastError = result?.error ?? "handoff failed" }
                self?.refresh()
                completion(result)
            }
        }
    }

    func refresh() {
        guard let h = handle else { return }
        let json = Self.string(from: chorus_transcript_json(h))
        if let rows = Self.decode([LedgerEvent].self, json) {
            events = rows
        }
        let d = Self.string(from: chorus_working_diff(h))
        if let obj = Self.decode(DiffResult.self, d) {
            workingDiff = obj.diff ?? ""
        }
        refreshMemory()
    }

    @Published var workingDiff = ""
    @Published var facts: [String] = []
    @Published var pending: [Proposal] = []

    struct Proposal: Codable, Identifiable {
        let id: Int64
        let scope: Int64
        let text: String
    }
    private struct MemoryState: Codable {
        let ok: Bool?
        let facts: [String]?
        let pending: [Proposal]?
    }

    func refreshMemory() {
        guard let h = handle else { return }
        let json = Self.string(from: chorus_memory_state(h))
        if let s = Self.decode(MemoryState.self, json) {
            facts = s.facts ?? []
            pending = s.pending ?? []
        }
    }

    func review(_ id: Int64, approve: Bool) {
        guard let h = handle else { return }
        _ = Self.string(from: chorus_memory_review(h, id, approve ? 1 : 0))
        refreshMemory()
    }

    func snapshot() {
        guard let h = handle else { return }
        _ = "manual checkpoint".withCString { chorus_snapshot(h, $0) }
        refresh()
    }

    /// Roll the workspace back to the session start. Destructive.
    func revertToStart() {
        guard let h = handle else { return }
        let r = "".withCString { Self.string(from: chorus_restore(h, $0)) }
        if let obj = Self.decode(SimpleResult.self, r), obj.ok != true {
            lastError = obj.error ?? "restore failed"
        }
        refresh()
    }

    // ---- helpers (thread-agnostic) ----

    nonisolated private static func string(from p: UnsafeMutablePointer<CChar>?) -> String {
        guard let p else { return "" }
        defer { chorus_string_free(p) }
        return String(cString: p)
    }

    nonisolated private static func decode<T: Decodable>(_ type: T.Type, _ json: String) -> T? {
        guard let data = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(T.self, from: data)
    }

    nonisolated private static func jsonArray(_ items: [String]) -> String {
        guard let data = try? JSONEncoder().encode(items),
              let s = String(data: data, encoding: .utf8) else { return "[]" }
        return s
    }
}
