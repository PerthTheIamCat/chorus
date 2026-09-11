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

    /// Opening a session runs `git init` + a first snapshot of the whole
    /// workspace tree (checkpoint.rs) — on a large or slow-to-enumerate
    /// directory (a home folder, an iCloud-synced tree) that can take a
    /// long time. Do it off the main thread so the UI stays responsive
    /// (`busy` drives a spinner) instead of looking hung.
    func open(db: String, cwd: String, title: String) {
        close()
        busy = true
        lastError = nil
        queue.async { [weak self] in
            let h = db.withCString { d in
                cwd.withCString { c in
                    title.withCString { t in
                        chorus_engine_open(d, c, t)
                    }
                }
            }
            DispatchQueue.main.async {
                guard let self else { return }
                self.busy = false
                guard let h else {
                    self.lastError = "could not open a session (is the workspace path valid?)"
                    return
                }
                self.handle = h
                self.sessionID = chorus_engine_session_id(h)
                self.refresh()
            }
        }
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

    /// `chorus_working_diff` runs `git add -A` under the hood (checkpoint.rs)
    /// — on a large or iCloud-synced workspace that can take a while, so
    /// this hops to the background queue rather than blocking whatever
    /// thread called `refresh()` (including the main thread, since several
    /// call sites are already inside a `DispatchQueue.main.async`).
    func refresh() {
        guard let h = handle else { return }
        queue.async { [weak self] in
            let eventsJSON = Self.string(from: chorus_transcript_json(h))
            let diffJSON = Self.string(from: chorus_working_diff(h))
            let memJSON = Self.string(from: chorus_memory_state(h))
            DispatchQueue.main.async {
                guard let self else { return }
                if let rows = Self.decode([LedgerEvent].self, eventsJSON) {
                    self.events = rows
                }
                if let obj = Self.decode(DiffResult.self, diffJSON) {
                    self.workingDiff = obj.diff ?? ""
                }
                if let s = Self.decode(MemoryState.self, memJSON) {
                    self.facts = s.facts ?? []
                    self.pending = s.pending ?? []
                }
            }
        }
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

    // All three below run `git` under the hood (checkpoint.rs) — off the
    // main thread for the same reason as `open`/`refresh`.

    func snapshot() {
        guard let h = handle else { return }
        queue.async {
            _ = "manual checkpoint".withCString { chorus_snapshot(h, $0) }
            DispatchQueue.main.async { [weak self] in self?.refresh() }
        }
    }

    /// Reverse-apply `patch` (the hunks the user rejected).
    func rejectHunks(_ patch: String) {
        guard let h = handle else { return }
        queue.async { [weak self] in
            let r = patch.withCString { Self.string(from: chorus_reject_hunks(h, $0)) }
            let obj = Self.decode(SimpleResult.self, r)
            DispatchQueue.main.async {
                if obj?.ok != true { self?.lastError = obj?.error ?? "reject failed" }
                self?.refresh()
            }
        }
    }

    /// Roll the workspace back to the session start. Destructive.
    func revertToStart() {
        guard let h = handle else { return }
        queue.async { [weak self] in
            let r = "".withCString { Self.string(from: chorus_restore(h, $0)) }
            let obj = Self.decode(SimpleResult.self, r)
            DispatchQueue.main.async {
                if obj?.ok != true { self?.lastError = obj?.error ?? "restore failed" }
                self?.refresh()
            }
        }
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
