import AppKit
import Foundation

// Things an agent puts in front of the person mid-turn, beyond a yes/no
// permission: a credential (`chorus_request_secret`) and versions to choose
// from (`chorus_offer_variants`). Rust validates everything and decides what
// the agent is told; this file only decodes what Rust sends, holds the pending
// state, and hands the answer back (rule 2.3).

// MARK: what Rust sends

struct SecretRequest: Identifiable, Decodable {
    let id = UUID()
    let agent: String
    /// The variable name the agent asked for, e.g. `STRIPE_API_KEY`.
    let name: String
    let reason: String

    private enum CodingKeys: String, CodingKey { case agent, name, reason }
}

struct VariantOffer: Identifiable, Decodable {
    struct Variant: Identifiable, Decodable {
        var id: String { "\(label)|\(target)" }
        let label: String
        /// `image` (a file in the project) or `url` (a local dev server).
        let kind: String
        let target: String
        let note: String
    }

    let id = UUID()
    let agent: String
    let title: String
    let variants: [Variant]
    let recommended: Int?

    private enum CodingKeys: String, CodingKey { case agent, title, variants, recommended }
}

// MARK: what comes back

/// How long a typed credential is kept. The numbers are the C ABI's
/// (`ChorusSecretCallback` in chorus.h).
enum SecretRemember: Int32, CaseIterable, Identifiable {
    case once = 1
    case project = 2
    case all = 3

    var id: Int32 { rawValue }

    var label: String {
        switch self {
        case .once: return "แค่ตอนนี้"
        case .project: return "โปรเจกต์นี้"
        case .all: return "ทุกโปรเจกต์"
        }
    }

    var detail: String {
        switch self {
        case .once: return "อยู่ในหน่วยความจำจนกว่าจะปิด Chorus — ไม่ลง Keychain"
        case .project: return "เก็บใน Keychain ใช้เฉพาะเทิร์นของโปรเจกต์นี้"
        case .all: return "เก็บใน Keychain ใช้กับทุกโปรเจกต์"
        }
    }
}

/// A typed credential. Never logged, never printed: the only place the value
/// goes is the C buffer Rust handed us.
struct SecretAnswer {
    let value: String
    let remember: SecretRemember
}

struct VariantPin: Identifiable, Encodable, Equatable {
    var id = UUID()
    var variant: Int
    /// 0…1 across the image; nil for a comment on the whole variant.
    var x: Double?
    var y: Double?
    var text = ""

    private enum CodingKeys: String, CodingKey { case variant, x, y, text }
}

struct VariantChoice: Encodable {
    var chosen: Int?
    var comments: [VariantPin]
    var note: String
}

// MARK: waiting for the person

/// Blocks the thread an MCP call arrived on until the main actor answers.
/// Calls are serialised: parallel agents are asked one at a time.
final class PromptGate<Answer>: @unchecked Sendable {
    private let lock = NSLock()
    private let gate = DispatchSemaphore(value: 0)
    private var answer: Answer?

    /// `publish` puts the prompt on screen (it hops to the main actor itself).
    func ask(_ publish: () -> Void) -> Answer? {
        lock.lock()
        defer { lock.unlock() }
        answer = nil
        publish()
        gate.wait()
        return answer
    }

    /// Only call this for a prompt that is actually waiting: an extra signal
    /// would make the next `ask` return at once.
    func resolve(_ value: Answer?) {
        answer = value
        gate.signal()
    }
}

/// Text of the system notification when Chorus is in the background.
enum AgentPromptNotice {
    static func question(agent: String, question: String) -> (title: String, body: String) {
        ("Chorus: \(label(agent)) ถามคุณ", TurnNotice.firstLine(question))
    }

    static func secret(agent: String, name: String) -> (title: String, body: String) {
        ("Chorus: \(label(agent)) ขอ \(name)", "ใส่ค่าในหน้าต่าง Chorus — agent จะไม่เห็นค่า")
    }

    static func variants(agent: String, title: String) -> (title: String, body: String) {
        ("Chorus: \(label(agent)) ให้เลือกแบบ", TurnNotice.firstLine(title))
    }

    /// "claude@work" → "claude".
    private static func label(_ agent: String) -> String {
        agent.split(separator: "@", maxSplits: 1).first.map(String.init) ?? agent
    }

    /// The sidebar badge: "รอคุณ 1 คำถาม" (an unspecific "คำถาม" covers a
    /// secret and a choice too — they all wait on the same person).
    static func badge(_ count: Int) -> String? {
        count > 0 ? "รอคุณ \(count) คำถาม" : nil
    }
}

// MARK: C callbacks

/// The C-ABI entry points. Rust calls these on an MCP server thread; they
/// block until the person answers.
enum AgentPromptCallbacks {
    /// Up to `cap - 1` bytes of `text`, cut on a character boundary so Rust
    /// never sees half a code point.
    static func utf8Prefix(_ text: String, cap: Int) -> [UInt8] {
        var bytes: [UInt8] = []
        for ch in text {
            let b = Array(String(ch).utf8)
            if bytes.count + b.count > cap - 1 { break }
            bytes += b
        }
        return bytes
    }

    static let secret: @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>?, UnsafeMutablePointer<CChar>?, Int) -> Int32 = { ctx, json, buf, cap in
        guard let ctx, let json, let buf, cap > 0 else { return 0 }
        let engine = Unmanaged<Engine>.fromOpaque(ctx).takeUnretainedValue()
        guard let answer = engine.askSecretBlocking(String(cString: json)) else { return 0 }
        let bytes = utf8Prefix(answer.value, cap: cap)
        for (i, b) in bytes.enumerated() { buf[i] = CChar(bitPattern: b) }
        buf[bytes.count] = 0
        return answer.remember.rawValue
    }

    static let variants: @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>?, UnsafeMutablePointer<CChar>?, Int) -> Int32 = { ctx, json, buf, cap in
        guard let ctx, let json, let buf, cap > 0 else { return 0 }
        let engine = Unmanaged<Engine>.fromOpaque(ctx).takeUnretainedValue()
        guard let text = engine.askVariantsBlocking(String(cString: json)) else { return 0 }
        let bytes = utf8Prefix(text, cap: cap)
        for (i, b) in bytes.enumerated() { buf[i] = CChar(bitPattern: b) }
        buf[bytes.count] = 0
        return 1
    }
}

extension Engine {
    /// Called on an MCP server thread. `nil` = cancelled.
    nonisolated func askSecretBlocking(_ json: String) -> SecretAnswer? {
        guard let req = try? JSONDecoder().decode(SecretRequest.self, from: Data(json.utf8)) else { return nil }
        return secretGate.ask {
            Task { @MainActor in
                self.pendingSecret = req
                self.announceWaiting(AgentPromptNotice.secret(agent: req.agent, name: req.name))
            }
        }
    }

    /// Called on an MCP server thread. The answer is the JSON Rust reads
    /// back; `nil` = the card was closed.
    nonisolated func askVariantsBlocking(_ json: String) -> String? {
        guard let offer = try? JSONDecoder().decode(VariantOffer.self, from: Data(json.utf8)) else { return nil }
        let choice = variantsGate.ask {
            Task { @MainActor in
                self.pendingVariants = offer
                self.announceWaiting(AgentPromptNotice.variants(agent: offer.agent, title: offer.title))
            }
        }
        guard let choice, let data = try? JSONEncoder().encode(choice) else { return nil }
        return String(data: data, encoding: .utf8)
    }

    func answerSecret(_ answer: SecretAnswer?) {
        guard pendingSecret != nil else { return }
        pendingSecret = nil
        secretGate.resolve(answer)
    }

    func answerVariants(_ choice: VariantChoice?) {
        guard pendingVariants != nil else { return }
        pendingVariants = nil
        variantsGate.resolve(choice)
    }

    /// The turn was stopped: whatever waited for the person will never be
    /// read, and the blocked MCP threads must be let go.
    func dismissAgentPrompts() {
        answerQuestion(nil)
        answerSecret(nil)
        answerVariants(nil)
    }

    /// Bring the person back: dock bounce, and a system notification when
    /// Chorus is not the frontmost app.
    func announceWaiting(_ notice: (title: String, body: String)) {
        NSApp.requestUserAttention(.informationalRequest)
        if !NSApp.isActive {
            TurnNotice.post(id: "prompt-\(UUID().uuidString)", title: notice.title, body: notice.body)
        }
    }
}
