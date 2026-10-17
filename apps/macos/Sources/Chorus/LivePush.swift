import Foundation

/// One signal from `chorus_set_push_callback`. Every push names its session,
/// so several turns at once (Orchestra children) never mix.
struct LivePush: Decodable, Sendable, Equatable {
    let session_id: Int64
    /// Newest ledger seq the session is known to have.
    let seq: Int64
    let kind: String
    let text: String
}

/// What one frame's batch of pushes asks the window to change. Pure, so the
/// routing is testable without a running turn.
struct LiveFold {
    var turn: Engine.LiveTurn
    var contextUsed: Int64?
    var contextWindow: Int64?
    /// The visible session's ledger moved past what is on screen: pull the delta.
    var pull = false
    /// A tool finished on screen: the workspace may have changed.
    var diffChanged = false
    /// `running` | `idle` | `failed`, for every session that reported.
    var statuses: [Int64: String] = [:]

    static func apply(_ batch: [LivePush], session: Int64, lastSeq: Int64,
                      turn: Engine.LiveTurn, contextUsed: Int64?, contextWindow: Int64?) -> LiveFold {
        var f = LiveFold(turn: turn, contextUsed: contextUsed, contextWindow: contextWindow)
        for p in batch {
            // Status is the one signal kept for sessions off screen: the
            // sidebar badges need it. Everything else is for the open thread.
            if p.kind == "status" { f.statuses[p.session_id] = p.text; continue }
            guard p.session_id == session else { continue }
            switch p.kind {
            case "thought": f.turn.thoughts += p.text
            case "message": f.turn.message += p.text
            case "tool": f.turn.tools.append(.init(title: p.text))
            case "tool_done":
                let parts = p.text.split(separator: "\t", maxSplits: 1).map(String.init)
                let state = parts.first ?? "completed"
                let title = parts.count > 1 ? parts[1] : ""
                if let i = f.turn.tools.lastIndex(where: { $0.state == "running" && (title.isEmpty || $0.title == title) })
                    ?? f.turn.tools.lastIndex(where: { $0.state == "running" }) {
                    f.turn.tools[i].state = state
                }
                f.diffChanged = true
            case "usage":
                // `used\tsize`, either side may be empty.
                let parts = p.text.split(separator: "\t", omittingEmptySubsequences: false)
                if let used = parts.first.flatMap({ Int64($0) }) { f.contextUsed = used }
                if parts.count > 1, let size = Int64(parts[1]) { f.contextWindow = size }
            case "events":
                if p.seq > lastSeq { f.pull = true }
            default: break
            }
        }
        return f
    }
}

/// Collects pushes from the turn's thread and hands them to the main thread
/// at most once per frame. A reply streams dozens of chunks a second; one
/// main-thread hop each is what made typing stutter during a turn.
final class PushCoalescer: @unchecked Sendable {
    private let lock = NSLock()
    private var pending: [LivePush] = []
    private var scheduled = false
    private let frame: TimeInterval
    private let deliver: @MainActor ([LivePush]) -> Void

    init(frame: TimeInterval = 1.0 / 60.0, deliver: @escaping @MainActor ([LivePush]) -> Void) {
        self.frame = frame
        self.deliver = deliver
    }

    /// Any thread.
    func enqueue(_ p: LivePush) {
        lock.lock()
        pending.append(p)
        let schedule = !scheduled
        scheduled = true
        lock.unlock()
        guard schedule else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + frame) { [self] in
            MainActor.assumeIsolated { deliver(drain()) }
        }
    }

    private func drain() -> [LivePush] {
        lock.lock()
        defer { lock.unlock() }
        let batch = pending
        pending = []
        scheduled = false
        return batch
    }
}
