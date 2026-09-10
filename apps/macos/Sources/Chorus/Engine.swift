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
