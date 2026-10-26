import CChorus
import AppKit
import Foundation
import PDFKit
import UniformTypeIdentifiers

/// A ledger event as the shell sees it. Mirrors the FFI JSON.
struct LedgerEvent: Codable, Identifiable, Hashable {
    let seq: Int64
    let kind: String
    let actor: String
    let body: String
    let model_id: String?
    let cost_usd: Double?
    var id: Int64 { seq }

    init(seq: Int64, kind: String, actor: String, body: String, model_id: String?, cost_usd: Double? = nil) {
        self.seq = seq
        self.kind = kind
        self.actor = actor
        self.body = body
        self.model_id = model_id
        self.cost_usd = cost_usd
    }
}

struct SessionAttachment: Decodable, Identifiable, Hashable {
    let id: Int64
    let name: String
    let media_type: String
    let size: Int64
    let status: String
    let readable: Bool
    let event_seq: Int64?
}

private struct SessionAttachmentList: Decodable {
    let ok: Bool
    let attachments: [SessionAttachment]?
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

private struct ImportedAttachmentResult: Decodable {
    struct Item: Decodable {
        let id: Int64
        let name: String
        let media_type: String
        let size: Int64
        let readable: Bool
    }
    let ok: Bool
    let attachment: Item?
    let error: String?
}

private struct CatalogPlanResult: Decodable {
    let ok: Bool
    let error: String?
    let plan: InstallPlan?
}

private struct DashboardResult: Decodable {
    let ok: Bool
    let error: String?
    let dashboard: ToolDashboard?
}

private struct CatalogProgressResult: Decodable {
    let progress: CatalogProgress?
}

private struct CatalogListResult: Decodable {
    struct Entry: Decodable {
        struct Installed: Decodable {
            struct Health: Decodable { let healthy: Bool; let detail: String }
            let version: String
            let artifact_integrity: String?
            let health: Health
        }
        let id: String
        let title: String
        let source: String
        let maintainer: String
        let license: String
        let reviewed_version: String
        let modes: [String]
        let required_runtimes: [String]
        let actions: [String]
        let project_access: [String]
        let network: String
        let secret_names: [String]
        let installed: Installed?
        let enabled: Bool
        let can_remove: Bool
        let missing_installer: String?
        let brew_formula: String?
        let installed_method: String?
    }
    let ok: Bool
    let entries: [Entry]?
    let error: String?
}

private struct RouterModelsResult: Decodable {
    struct Item: Decodable {
        let name: String
        let size_bytes: UInt64?
    }
    let ok: Bool
    let models: [Item]?
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
    @Published var attachments: [SessionAttachment] = []
    @Published var busy = false
    /// The relay chain of a plan waiting for the user's approval (plan mode);
    /// nil when there is none. Cleared by any new message or session switch.
    @Published var pendingPlanAgents: String?
    @Published var lastError: String?
    @Published var sessionID: Int64 = -1
    @Published var contextUsed: Int64?
    @Published var contextWindow: Int64?
    @Published var sessionTokensIn: Int64 = 0
    @Published var sessionTokensOut: Int64 = 0
    @Published var sessionCostUSD: Double? = nil
    /// goal / done / pending for this session — the context tier the design
    /// never trims. Empty only before the first turn.
    @Published var taskState = ""
    /// Events with no stored summary rungs yet. Non-zero just means the next
    /// compaction pass has work, not that anything is wrong.
    @Published var uncompacted: Int64 = 0
    /// Active sessions currently being renamed by AI (for loading indicator feedback).
    @Published var renamingSessionIDs: Set<Int64> = []

    /// The Rust engine handle, carried onto `queue` and back.
    ///
    /// `OpaquePointer` is not `Sendable`, and the compiler is right to object:
    /// the Rust side owns a SQLite connection that must not be touched from two
    /// threads at once. What actually makes this safe is that *every* use goes
    /// through one **serial** `DispatchQueue`. This box states that invariant in
    /// the type system instead of leaving a dozen warnings that all repeat the
    /// same thing — and anyone who later reaches for a second queue to make
    /// compaction "not block" has to argue with this comment first.
    private struct Handle: @unchecked Sendable {
        let raw: OpaquePointer
    }

    /// Same bargain for the `self` pointer handed to the C permission callback:
    /// it is only ever dereferenced on the callback's own thread, which blocks
    /// until the main actor answers.
    private struct CallbackContext: @unchecked Sendable {
        let raw: UnsafeMutableRawPointer
    }

    private var handle: Handle?
    private let queue = DispatchQueue(label: "chorus.engine")
    /// The read-only twin of `handle` (`chorus_reader_open`). Used only on
    /// `readQueue`, never on `queue`: SQLite WAL lets it read while a turn on
    /// `queue` writes, so the thread and diff move mid-turn without breaking
    /// the one-writer rule above (CLAUDE.md 7.7). It never writes.
    private var reader: Handle?
    private let readQueue = DispatchQueue(label: "chorus.reader", qos: .userInitiated)
    /// Highest real ledger seq on screen; the delta pull starts after it.
    private var lastSeq: Int64 = 0
    /// `running` | `idle` | `failed` per session id, from `status` pushes —
    /// every session, not only the open one (sidebar badges).
    @Published private(set) var sessionStatus: [Int64: String] = [:]
    /// For FFI calls that take no engine handle (agent/session/skill lists).
    /// They used to sit on `queue` behind a running turn, so the sidebar and
    /// pickers stayed empty or stale until the agent finished — the main
    /// reason the window felt like it wasn't responding.
    private let ioQueue = DispatchQueue(label: "chorus.io", qos: .userInitiated, attributes: .concurrent)
    private let catalogQueue = DispatchQueue(label: "chorus.catalog", qos: .utility)
    private let routerSetupQueue = DispatchQueue(label: "chorus.routerSetup", qos: .utility)
    private let monitorQueue = DispatchQueue(label: "chorus.monitor", qos: .utility)
    @Published var catalog: [CatalogIntegration] = []
    @Published var catalogError: String?
    @Published var catalogActionError: String?
    @Published var catalogActionResult: CatalogActionResult?
    @Published var catalogWorking: Set<String> = []
    @Published var catalogBackgroundStatus: String?
    @Published var catalogProgress: CatalogProgress?
    /// An integration another screen wants the install sheet opened for.
    @Published var requestedInstall: String?
    private var catalogProgressTimer: Timer?
    @Published var routerModels: [String]?
    @Published var routerChecking = false
    @Published var routerDownloading = false
    @Published var routerSetupError: String?
    private var ollamaProcess: Process?
    /// Pending warm-tier pass; cancelled whenever the user sends something,
    /// so compaction never makes a new message wait.
    private var compactWork: DispatchWorkItem?

    /// True while a session is being opened or switched to. Separate from
    /// `busy` (a running turn): reusing `busy` flashed the "working…" row and
    /// disabled the composer on every sidebar click.
    @Published var switching = false

    /// Opening a session runs `git init` + a first snapshot of the whole
    /// workspace tree (checkpoint.rs) — slow on a big folder, so it runs on
    /// `queue`, not the main thread.
    func open(db: String, cwd: String, title: String) {
        checklist = []
        loadSubagents(db: db)
        swap { chorus_engine_open(db, cwd, title) }
    }

    /// Reopen a session that already exists, without starting a new one.
    func reopen(db: String, sessionID: Int64) {
        loadChecklist(db: db, sessionID: sessionID)
        loadSubagents(db: db)
        swap { chorus_engine_reopen(db, sessionID) }
    }

    /// A new session before its first message: in-memory only (Rust
    /// `drafts`), so pressing ✎ and walking away leaves no empty row.
    struct DraftInfo: Codable, Equatable { let id: UInt64; let db: String; let cwd: String }
    private struct DraftResult: Codable { let ok: Bool; let draft: DraftInfo?; let reused: Bool?; let error: String? }
    @Published private(set) var draft: DraftInfo?

    /// ✎ / "session ใหม่": show the draft for `cwd` — the unsent one if there
    /// is one (Rust decides). The open session is closed; nothing is written.
    func openDraft(db: String, cwd: String) {
        guard !busy, !switching else { return }
        let json = db.withCString { d in cwd.withCString { Self.string(from: chorus_draft_open(d, $0)) } }
        guard let d = Self.decode(DraftResult.self, json)?.draft else {
            lastError = "เปิด session ใหม่ไม่ได้"
            return
        }
        close()
        live = LiveTurn()
        lastThoughts = ""
        contextUsed = nil
        contextWindow = nil
        draft = d
    }

    /// First message of a draft: create its session, then `then()` once the
    /// handle is live (the caller sends from there). On failure the draft
    /// stays and the error shows, so the send can be retried.
    func commitDraft(title: String = "session", then: @escaping () -> Void) {
        guard let d = draft else { return }
        loadSubagents(db: d.db)
        swap(onReady: then) { title.withCString { chorus_draft_commit(d.id, $0) } }
    }

    private struct IDsResult: Codable { let ok: Bool; let ids: [Int64]?; let error: String? }

    /// Sessions with nothing in them across `dbs` (one per project folder).
    /// The open session is never counted.
    func emptySessionCount(dbs: [String], completion: @escaping (Int) -> Void) {
        let keep = sessionID
        ioQueue.async {
            let n = dbs.reduce(0) { sum, db in
                let json = db.withCString { Self.string(from: chorus_empty_sessions($0, keep)) }
                return sum + (Self.decode(IDsResult.self, json)?.ids?.count ?? 0)
            }
            DispatchQueue.main.async { completion(n) }
        }
    }

    /// "ลบ session ที่ว่าง": delete them; Rust re-checks each one first.
    func deleteEmptySessions(dbs: [String], completion: @escaping (Int) -> Void) {
        let keep = sessionID
        ioQueue.async { [weak self] in
            var deleted = 0
            var failure: String?
            for db in dbs {
                let json = db.withCString { Self.string(from: chorus_delete_empty_sessions($0, keep)) }
                let r = Self.decode(IDsResult.self, json)
                if r?.ok == true { deleted += r?.ids?.count ?? 0 } else { failure = r?.error ?? "unknown" }
            }
            DispatchQueue.main.async {
                if let failure { self?.lastError = "ลบ session ที่ว่างไม่ได้: \(failure)" }
                for db in dbs { self?.loadSessions(db: db) }
                completion(deleted)
            }
        }
    }

    /// Open the new handle and read its transcript off the main thread, then
    /// publish handle + id + events in one main-thread step. The old thread
    /// stays on screen until then: clearing first is what made every switch
    /// blink through an empty view.
    private func swap(onReady: (() -> Void)? = nil, _ make: @escaping @Sendable () -> OpaquePointer?) {
        guard !busy, !switching else { return }
        let old = handle
        let oldReader = reader
        // Nothing may enqueue work against the old handle from here on: it
        // is freed on `queue` once the new one is up.
        handle = nil
        reader = nil
        switching = true
        lastError = nil
        pendingPlanAgents = nil
        let askCtx = CallbackContext(raw: Unmanaged.passUnretained(self).toOpaque())
        queue.async { [weak self] in
            let h = make()
            // Every session starts with the user's computer-use switch.
            if let h {
                // Agents may ask the person a question in any mode, not only
                // "ask first" — a question is not a permission.
                let qcb: @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>?, UnsafeMutablePointer<CChar>?, Int) -> Int32 = { ctx, json, answer, cap in
                    guard let ctx, let json, let answer, cap > 0 else { return 0 }
                    let engine = Unmanaged<Engine>.fromOpaque(ctx).takeUnretainedValue()
                    guard let text = engine.askQuestionBlocking(String(cString: json)) else { return 0 }
                    let bytes = Engine.answerBytes(text, cap: cap)
                    for (i, b) in bytes.enumerated() { answer[i] = CChar(bitPattern: b) }
                    answer[bytes.count] = 0
                    return 1
                }
                chorus_set_question_callback(h, qcb, askCtx.raw)
                chorus_set_computer_use(h, UserDefaults.standard.bool(forKey: Self.computerUseKey) ? 1 : 0)
                chorus_set_claude_chrome(h, UserDefaults.standard.bool(forKey: Self.claudeChromeKey) ? 1 : 0)
                chorus_set_hide_secret_files(h, Self.hideSecretFiles ? 1 : 0)
            }
            let eventsJSON = h.map { Self.string(from: chorus_transcript_json($0)) } ?? ""
            let presetJSON = h.map { Self.initialRoutePreset($0) } ?? ""
            let attachmentsJSON = h.map { Self.string(from: chorus_list_attachments($0)) } ?? ""
            let usageJSON = h.map { Self.string(from: chorus_session_usage($0)) } ?? ""
            let r = h.flatMap { chorus_reader_open($0) }.map(Handle.init)
            if let old { chorus_engine_free(old.raw) }
            if let oldReader { self?.readQueue.async { chorus_reader_free(oldReader.raw) } }
            DispatchQueue.main.async {
                guard let self else { return }
                self.switching = false
                self.memoryScope = nil
                self.live = LiveTurn()
                self.lastThoughts = ""
                // Seed the meter from the stored usage; nil only for a session
                // that never reported any.
                let usage = Self.decode(SessionUsageResult.self, usageJSON)?.usage
                self.contextUsed = usage?.context_used
                self.contextWindow = usage?.context_window
                self.sessionTokensIn = usage?.tokens_in ?? 0
                self.sessionTokensOut = usage?.tokens_out ?? 0
                self.sessionCostUSD = usage?.cost_usd
                guard let h else {
                    self.sessionID = -1
                    self.events = []
                    self.lastError = "เปิด session ไม่ได้ (โฟลเดอร์ยังอยู่ไหม?)"
                    return
                }
                self.handle = Handle(raw: h)
                self.reader = r
                self.draft = nil
                self.sessionID = chorus_engine_session_id(h)
                self.events = Self.decode([LedgerEvent].self, eventsJSON) ?? []
                self.lastSeq = self.events.last(where: { $0.seq > 0 })?.seq ?? 0
                self.attachments = Self.decode(SessionAttachmentList.self, attachmentsJSON)?.attachments ?? []
                self.applyRoutePreset(presetJSON)
                self.refresh()
                onReady?()
            }
        }
    }

    struct SessionUsage: Codable {
        let context_used: Int64?
        let context_window: Int64?
        let tokens_in: Int64?
        let tokens_out: Int64?
        let cost_usd: Double?
    }
    struct SessionUsageResult: Codable { let ok: Bool; let usage: SessionUsage? }

    func close() {
        memoryScope = nil
        pendingPlanAgents = nil
        sessionCostUSD = nil
        sessionTokensIn = 0
        sessionTokensOut = 0
        checklist = []
        if let oldReader = reader {
            reader = nil
            readQueue.async { chorus_reader_free(oldReader.raw) }
        }
        guard let old = handle else { return }
        handle = nil
        sessionID = -1
        events = []
        lastSeq = 0
        attachments = []
        queue.async { chorus_engine_free(old.raw) }
    }

    /// Rename or delete a session by store path; neither needs it open.
    func renameSession(db: String, id: Int64, title: String) {
        ioQueue.async { [weak self] in
            let json = db.withCString { d in title.withCString { Self.string(from: chorus_rename_session(d, id, $0)) } }
            let r = Self.decode(SimpleResult.self, json)
            DispatchQueue.main.async {
                if r?.ok != true { self?.lastError = "เปลี่ยนชื่อไม่ได้: \(r?.error ?? "unknown")" }
                self?.loadSessions(db: db)
            }
        }
    }

    /// Automatically generate a concise title for a session using AI (local Ollama fast model or intelligent fallback).
    func renameSessionWithAI(db: String, id: Int64, sampleText: String? = nil, completion: ((String?) -> Void)? = nil) {
        renamingSessionIDs.insert(id)
        let installedModels = self.routerModels ?? []

        let currentPrompt = (sampleText?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false)
            ? sampleText
            : self.events.first(where: { $0.actor == "user" || $0.kind == "msg" })?.body

        ioQueue.async { [weak self] in
            let textToSummarize = (currentPrompt ?? "New Session")
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .replacingOccurrences(of: "\n", with: " ")

            let cleanPrompt = String(textToSummarize.prefix(80))

            // Candidate models for local title summarization in order of quality/speed
            // (Exclude router policies like chorus-openthai-systemone / tev1 which only output route JSON)
            let candidateModels = ["qwen2.5:3b", "qwen3:1.7b", "llama3.2:1b", "iapp/openthai-systemone:0.8b"]
            let chosenModel = candidateModels.first(where: { installedModels.contains($0) }) ?? "qwen2.5:3b"

            guard let url = URL(string: "http://127.0.0.1:11434/api/generate") else {
                let fallback = Self.fallbackTitle(from: cleanPrompt)
                DispatchQueue.main.async {
                    self?.renamingSessionIDs.remove(id)
                    self?.renameSession(db: db, id: id, title: fallback)
                    completion?(fallback)
                }
                return
            }

            var req = URLRequest(url: url)
            req.httpMethod = "POST"
            req.timeoutInterval = 8.0
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
            let payload: [String: Any] = [
                "model": chosenModel,
                "prompt": "ข้อความ: \"\(cleanPrompt)\"\nสรุปเป็นชื่อหัวข้อสั้นๆ ภาษาไทย 3-4 คำเท่านั้น ตอบเฉพาะชื่อ (ห้ามใส่เครื่องหมายคำพูดหรือวงเล็บ):",
                "stream": false,
                "options": [
                    "num_predict": 18,
                    "temperature": 0.2
                ]
            ]
            req.httpBody = try? JSONSerialization.data(withJSONObject: payload)

            let task = URLSession.shared.dataTask(with: req) { data, _, _ in
                var generatedTitle: String? = nil
                if let data = data,
                   let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                    var raw = json["response"] as? String ?? ""
                    if let thinkEnd = raw.range(of: "</think>") {
                        raw = String(raw[thinkEnd.upperBound...])
                    }
                    let cleaned = raw.trimmingCharacters(in: .whitespacesAndNewlines)
                        .trimmingCharacters(in: CharacterSet(charactersIn: "\"'`“”。.:：-—"))
                    if !cleaned.isEmpty && cleaned.count <= 40 && !cleaned.contains("\n") {
                        generatedTitle = cleaned
                    }
                }

                let finalTitle = generatedTitle ?? Self.fallbackTitle(from: cleanPrompt)
                DispatchQueue.main.async {
                    self?.renamingSessionIDs.remove(id)
                    self?.renameSession(db: db, id: id, title: finalTitle)
                    completion?(finalTitle)
                }
            }
            task.resume()
        }
    }

    nonisolated private static func fallbackTitle(from prompt: String) -> String {
        let trimmed = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty { return "session ใหม่" }
        let words = trimmed.split(separator: " ").prefix(4).joined(separator: " ")
        return String(words.prefix(30))
    }

    func deleteSession(db: String, id: Int64, isOpen: Bool) {
        if isOpen { close() }
        SpotlightIndexer.shared.deleteSession(id: id)
        // After `close()`'s free on `queue`, so the open handle is gone first.
        queue.async { [weak self] in
            let json = db.withCString { Self.string(from: chorus_delete_session($0, id)) }
            let r = Self.decode(SimpleResult.self, json)
            DispatchQueue.main.async {
                if r?.ok != true { self?.lastError = "ลบไม่ได้: \(r?.error ?? "unknown")" }
                self?.loadSessions(db: db)
            }
        }
    }

    // MARK: interactive permission

    struct PermissionAsk: Identifiable {
        let id = UUID()
        /// Agent name the turn runs on (may carry `@account`), from Rust.
        let agent: String
        let tool: String
        let kind: String
        let detail: String
    }
    @Published var pendingAsk: PermissionAsk?
    private let permGate = DispatchSemaphore(value: 0)
    /// What the callback returns to Rust: 0 deny, 1 once, 2 for this session.
    private var permAnswer: Int32 = 0

    enum PermissionAnswer: Int32 { case deny = 0, once = 1, session = 2 }

    /// Called from the C callback on the engine's background thread. Publishes
    /// the ask to the UI and blocks until `answerPermission` is called.
    fileprivate func askPermissionBlocking(_ json: String) -> Int32 {
        struct Raw: Codable { let agent: String?; let tool: String?; let kind: String?; let detail: String? }
        let raw = (try? JSONDecoder().decode(Raw.self, from: Data(json.utf8)))
        DispatchQueue.main.async {
            self.pendingAsk = PermissionAsk(
                agent: raw?.agent ?? self.liveAgent ?? "agent",
                tool: raw?.tool ?? "action", kind: raw?.kind ?? "", detail: raw?.detail ?? "")
        }
        permGate.wait()
        return permAnswer
    }

    func answerPermission(_ answer: PermissionAnswer) {
        permAnswer = answer.rawValue
        pendingAsk = nil
        permGate.signal()
    }

    // MARK: agent questions (`chorus_ask_user`)

    struct AgentQuestion: Identifiable {
        let id = UUID()
        let agent: String
        let question: String
        let options: [String]
        let allowOther: Bool
    }
    @Published var pendingQuestion: AgentQuestion?
    nonisolated private let questionGate = DispatchSemaphore(value: 0)
    /// Serialises questions from parallel agents: one sheet at a time.
    nonisolated private let questionLock = NSLock()
    /// Written on the main actor, read by the waiting MCP thread after the
    /// semaphore — the gate orders the two, so no actor protects it.
    nonisolated(unsafe) private var questionAnswer: String?

    /// UTF-8 of `text` that fits `cap - 1` bytes, cut on a character
    /// boundary so Rust never sees half a code point.
    fileprivate nonisolated static func answerBytes(_ text: String, cap: Int) -> [UInt8] {
        var bytes: [UInt8] = []
        for ch in text {
            let b = Array(String(ch).utf8)
            if bytes.count + b.count > cap - 1 { break }
            bytes += b
        }
        return bytes
    }

    /// Called on an MCP server thread. Publishes the question and blocks
    /// until `answerQuestion`; `nil` = closed without an answer.
    nonisolated fileprivate func askQuestionBlocking(_ json: String) -> String? {
        struct Raw: Codable { let agent: String?; let question: String?; let options: [String]?; let allow_other: Bool? }
        guard let raw = try? JSONDecoder().decode(Raw.self, from: Data(json.utf8)) else { return nil }
        questionLock.lock()
        defer { questionLock.unlock() }
        let agent = raw.agent, question = raw.question ?? ""
        let options = raw.options ?? [], allowOther = raw.allow_other ?? true
        Task { @MainActor in
            self.pendingQuestion = AgentQuestion(
                agent: agent ?? self.liveAgent ?? "agent",
                question: question, options: options, allowOther: allowOther)
            NSApp.requestUserAttention(.informationalRequest)
        }
        questionGate.wait()
        return questionAnswer
    }

    func answerQuestion(_ text: String?) {
        guard pendingQuestion != nil else { return }
        questionAnswer = text
        pendingQuestion = nil
        questionGate.signal()
    }

    /// Turns block the engine queue until they finish, so the ledger cannot be
    /// re-read mid-turn. Without this the user's own message vanished into a
    /// blank thread until the answer landed. The synthetic row (negative seq)
    /// is replaced wholesale by the next `refresh()`.
    private var echoCounter: Int64 = 0
    private func echoUser(_ text: String) {
        compactWork?.cancel()
        pendingPlanAgents = nil
        echoCounter += 1
        live = LiveTurn()
        lastThoughts = ""
        events.append(LedgerEvent(seq: Int64.min + echoCounter, kind: "msg", actor: "user", body: text, model_id: nil))
        turnStartedAt = Date()
    }

    @Published var turnStartedAt: Date?

    /// What the running turn is doing right now, streamed from the agent.
    /// Display only — the ledger refresh after the turn is the record.
    struct LiveTurn {
        struct Tool: Identifiable { let id = UUID(); var title: String; var state = "running" }
        var thoughts = ""
        var message = ""
        var tools: [Tool] = []
        var isEmpty: Bool { thoughts.isEmpty && message.isEmpty && tools.isEmpty }
    }
    /// Its own observable so a streamed token re-renders the live row only,
    /// not every view that observes `Engine` (the whole ~4k-line window).
    let liveStream = LiveStream()
    var live: LiveTurn {
        get { liveStream.turn }
        set { liveStream.turn = newValue }
    }
    @Published var liveAgent: String? = nil
    /// The finished turn's reasoning. Agents send it but it is not stored in
    /// the ledger, so it lives here until the next turn starts.
    @Published var lastThoughts = ""
    /// The active file the agent is looking at or editing (Security D3 confined).
    @Published var agentFocusFile: String? = nil

    private var pushRegistered = false
    private lazy var pushes = PushCoalescer { [weak self] batch in self?.absorb(batch) }

    /// Subscribe to every session's pushes. The callback runs on the turn's
    /// thread; it only decodes and queues — the window hears at most one
    /// batch per frame.
    func registerProgress() {
        guard !pushRegistered else { return }
        pushRegistered = true
        // The context is the coalescer, not the engine: it is the one thing
        // here that may be touched off the main actor. `self` keeps it alive.
        let cb: @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>?) -> Void = { ctx, json in
            guard let ctx, let json else { return }
            guard let p = try? JSONDecoder().decode(LivePush.self, from: Data(String(cString: json).utf8)) else { return }
            Unmanaged<PushCoalescer>.fromOpaque(ctx).takeUnretainedValue().enqueue(p)
        }
        chorus_set_push_callback(cb, Unmanaged.passUnretained(pushes).toOpaque())
    }

    private func absorb(_ batch: [LivePush]) {
        let f = LiveFold.apply(batch, session: sessionID, lastSeq: lastSeq, turn: live,
                               contextUsed: contextUsed, contextWindow: contextWindow)
        // One assignment per frame, and only when something on screen moved.
        // Not after the turn: a batch still in flight when it ended would
        // put the finished reply back as a ghost live row.
        if busy, f.turn.thoughts != live.thoughts || f.turn.message != live.message
            || f.turn.tools.map(\.state) != live.tools.map(\.state) || f.turn.tools.count != live.tools.count {
            live = f.turn
        }
        if f.contextUsed != contextUsed { contextUsed = f.contextUsed }
        if f.contextWindow != contextWindow { contextWindow = f.contextWindow }
        for (id, status) in f.statuses where sessionStatus[id] != status { sessionStatus[id] = status }
        if let focus = f.focusFile { agentFocusFile = focus }
        if f.pull { pullDelta() }
        if f.diffChanged { scheduleLiveDiff() }
    }

    private var pulling = false
    private var pullAgain = false
    /// Fetch rows past `lastSeq` on the read-only handle and append them.
    /// One pull in flight at a time; pushes that land meanwhile ask for one
    /// more, so a burst becomes at most two reads.
    private func pullDelta() {
        guard let r = reader else { return }
        if pulling { pullAgain = true; return }
        pulling = true
        let after = lastSeq, sid = sessionID
        readQueue.async { [weak self] in
            let json = Self.string(from: chorus_reader_events_since(r.raw, after))
            let rows = Self.decode([LedgerEvent].self, json)
            DispatchQueue.main.async {
                guard let self else { return }
                self.pulling = false
                if sid == self.sessionID, let rows { self.merge(rows) }
                if self.pullAgain { self.pullAgain = false; self.pullDelta() }
            }
        }
    }

    /// Append a delta. The echoed user row (negative seq) goes once the real
    /// one lands — or once the turn is over, if it never did.
    private func merge(_ rows: [LedgerEvent]) {
        let fresh = rows.filter { $0.seq > lastSeq }
        let dropEcho = !busy || fresh.contains { $0.actor == "user" }
        guard !fresh.isEmpty || (dropEcho && events.contains { $0.seq < 0 }) else { return }
        var next = dropEcho ? events.filter { $0.seq >= 0 } : events
        next.append(contentsOf: fresh)
        events = next
        if let top = fresh.last?.seq { lastSeq = max(lastSeq, top) }
    }

    @Published var diffScope: String = "session" {
        didSet { scheduleLiveDiff() }
    }
    @Published var showChorusWrittenFiles: Bool = false {
        didSet { scheduleLiveDiff() }
    }

    private struct MarkReviewedResult: Codable {
        let ok: Bool?
        let base: String?
    }

    private struct CheckUserCommitResult: Codable {
        let ok: Bool?
        let moved: Bool?
    }

    func markReviewed(completion: @escaping (String?) -> Void = { _ in }) {
        guard let h = handle else { completion(nil); return }
        queue.async { [weak self] in
            let json = Self.string(from: chorus_mark_reviewed(h.raw))
            let r = Self.decode(MarkReviewedResult.self, json)
            DispatchQueue.main.async {
                guard let self else { return }
                self.scheduleLiveDiff()
                completion(r?.base)
            }
        }
    }

    func checkUserCommit() {
        guard let h = handle else { return }
        queue.async { [weak self] in
            let json = Self.string(from: chorus_check_user_commit(h.raw))
            let r = Self.decode(CheckUserCommitResult.self, json)
            if r?.moved == true {
                DispatchQueue.main.async {
                    self?.scheduleLiveDiff()
                }
            }
        }
    }

    private var liveDiffPending = false
    private var liveDiffAt = Date.distantPast
    /// Recompute the diff on the read-only handle, at most once a second
    /// (it shells out to git). Feeds the Changes pane and the tree badges.
    func scheduleLiveDiff() {
        guard !liveDiffPending, reader != nil else { return }
        liveDiffPending = true
        let wait = max(0, 1 - Date().timeIntervalSince(liveDiffAt))
        DispatchQueue.main.asyncAfter(deadline: .now() + wait) { [weak self] in
            guard let self, let r = self.reader else { self?.liveDiffPending = false; return }
            let sid = self.sessionID
            let scope = self.diffScope
            let incProjected = self.showChorusWrittenFiles ? 1 : 0
            self.readQueue.async { [weak self] in
                let json = scope.withCString { cScope in
                    Self.string(from: chorus_reader_working_diff_scoped(r.raw, cScope, Int32(incProjected)))
                }
                let diff = Self.decode(DiffResult.self, json)?.diff
                let hunks = diff.map(parseHunks)
                DispatchQueue.main.async {
                    guard let self else { return }
                    self.liveDiffPending = false
                    self.liveDiffAt = Date()
                    if sid == self.sessionID, let diff, let hunks, diff != self.workingDiff {
                        self.setWorkingDiff(diff, hunks: hunks)
                    }
                }
            }
        }
    }

    /// Turn finished: keep the reasoning, drop the live view (the ledger now has the rest).
    private func endLive() {
        loadUsage()
        loadAgents()
        if !live.thoughts.isEmpty { lastThoughts = live.thoughts }
        live = LiveTurn()
        liveAgent = nil
        refreshSessionUsage()
    }

    /// Refresh token usage and cost from reader.
    func refreshSessionUsage() {
        guard let r = reader else { return }
        readQueue.async { [weak self] in
            let json = Self.string(from: chorus_reader_usage(r.raw))
            let u = Self.decode(SessionUsageResult.self, json)?.usage
            DispatchQueue.main.async {
                guard let self else { return }
                if let u {
                    if let cu = u.context_used { self.contextUsed = cu }
                    if let cw = u.context_window { self.contextWindow = cw }
                    self.sessionTokensIn = u.tokens_in ?? 0
                    self.sessionTokensOut = u.tokens_out ?? 0
                    self.sessionCostUSD = u.cost_usd
                }
            }
        }
    }

    /// Stop button. Kills the agent process; the turn thread then returns an
    /// error, which `stopped` turns into a plain note instead of a red banner.
    func cancel() {
        _ = Self.string(from: chorus_cancel())
    }

    /// Matches `MAX_FILE_BYTES` in chorus-ffi's `chorus_import_attachment`.
    nonisolated static let maxAttachmentBytes = 20 * 1024 * 1024

    /// Size gate that runs before a single byte is read or mapped, so a
    /// 2 GB video is refused at once instead of after queueing behind a turn.
    static func attachmentRejection(_ url: URL) -> String? {
        guard let values = try? url.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey]) else {
            return "อ่านข้อมูลไฟล์ไม่ได้"
        }
        guard values.isRegularFile == true else { return "เลือกได้เฉพาะไฟล์" }
        guard let size = values.fileSize, size <= maxAttachmentBytes else { return "ไฟล์หนึ่งต้องไม่เกิน 20 MB" }
        return nil
    }

    /// Copy a selected file into the session immediately. PDF text and image
    /// normalization stay local; the original bytes remain in the session.
    func importFile(_ url: URL, completion: @escaping (DraftAttachment?, String?) -> Void) {
        guard let h = handle else { completion(nil, "เปิด session ก่อนแนบไฟล์"); return }
        do {
            let access = url.startAccessingSecurityScopedResource()
            defer { if access { url.stopAccessingSecurityScopedResource() } }
            if let reason = Self.attachmentRejection(url) { completion(nil, reason); return }
        }
        queue.async {
            let access = url.startAccessingSecurityScopedResource()
            defer { if access { url.stopAccessingSecurityScopedResource() } }
            do {
                // Bounded read: the file may have grown since the size check.
                let fh = try FileHandle(forReadingFrom: url)
                defer { try? fh.close() }
                let bytes = try fh.read(upToCount: Self.maxAttachmentBytes + 1) ?? Data()
                guard bytes.count <= Self.maxAttachmentBytes else {
                    throw ImportError.message("ไฟล์หนึ่งต้องไม่เกิน 20 MB")
                }
                let mime = UTType(filenameExtension: url.pathExtension)?.preferredMIMEType ?? "application/octet-stream"
                let prepared = Self.prepareAttachment(bytes, mime: mime, filename: url.lastPathComponent)
                let json = Self.callImport(h.raw, name: url.lastPathComponent, mime: mime,
                                           bytes: bytes, derivedText: prepared.text,
                                           normalized: prepared.image, normalizedMime: prepared.imageMime)
                let result = Self.decode(ImportedAttachmentResult.self, json)
                DispatchQueue.main.async {
                    if let item = result?.attachment, result?.ok == true {
                        completion(DraftAttachment(id: item.id, name: item.name, mediaType: item.media_type,
                                                   size: item.size, agentReadable: item.readable), nil)
                    } else { completion(nil, result?.error ?? "แนบไฟล์ไม่สำเร็จ") }
                }
            } catch {
                DispatchQueue.main.async { completion(nil, error.localizedDescription) }
            }
        }
    }

    func importImage(_ data: Data, mime: String, completion: @escaping (DraftAttachment?, String?) -> Void) {
        guard let h = handle else { completion(nil, "เปิด session ก่อนแนบรูป"); return }
        queue.async {
            let prepared = Self.prepareAttachment(data, mime: mime, filename: "pasted-image.png")
            let json = Self.callImport(h.raw, name: "pasted-image.png", mime: mime, bytes: data,
                                       derivedText: nil, normalized: prepared.image, normalizedMime: prepared.imageMime)
            let result = Self.decode(ImportedAttachmentResult.self, json)
            DispatchQueue.main.async {
                if let item = result?.attachment, result?.ok == true {
                    completion(DraftAttachment(id: item.id, name: item.name, mediaType: item.media_type,
                                               size: item.size, agentReadable: item.readable), nil)
                } else { completion(nil, result?.error ?? "แนบรูปไม่สำเร็จ") }
            }
        }
    }

    func deleteAttachment(_ id: Int64) {
        guard let h = handle else { return }
        queue.async { [weak self] in
            let json = Self.string(from: chorus_delete_attachment(h.raw, id))
            let result = Self.decode(SimpleResult.self, json)
            DispatchQueue.main.async {
                if result?.ok != true { self?.lastError = result?.error ?? "ลบไฟล์ไม่ได้" }
                self?.loadAttachments()
            }
        }
    }

    func loadAttachments() {
        guard let h = handle else { return }
        queue.async { [weak self] in
            let json = Self.string(from: chorus_list_attachments(h.raw))
            let rows = Self.decode(SessionAttachmentList.self, json)?.attachments
            DispatchQueue.main.async { if let rows { self?.attachments = rows } }
        }
    }

    private enum ImportError: LocalizedError {
        case message(String)
        var errorDescription: String? { if case .message(let value) = self { value } else { nil } }
    }

    nonisolated private static func prepareAttachment(_ bytes: Data, mime: String, filename: String)
        -> (text: String?, image: Data?, imageMime: String?) {
        if mime == "application/pdf" {
            let text = PDFDocument(data: bytes)?.string
            return (text?.isEmpty == false ? text : nil, nil, nil)
        }
        if mime.hasPrefix("image/") {
            if mime == "image/png" || mime == "image/jpeg" { return (nil, nil, nil) }
            let bitmap = NSBitmapImageRep(data: bytes)
                ?? NSImage(data: bytes)?.tiffRepresentation.flatMap(NSBitmapImageRep.init(data:))
            return (nil, bitmap?.representation(using: .png, properties: [:]), "image/png")
        }
        let sourceExtensions: Set<String> = ["txt", "md", "json", "jsonl", "yaml", "yml", "toml", "xml", "csv", "tsv", "swift", "rs", "ts", "tsx", "js", "jsx", "py", "go", "java", "kt", "c", "h", "cpp", "hpp", "css", "html", "sql", "sh", "zsh", "rb", "php", "svelte", "vue", "log"]
        if mime.hasPrefix("text/") || sourceExtensions.contains(URL(fileURLWithPath: filename).pathExtension.lowercased()) {
            return (String(data: bytes, encoding: .utf8), nil, nil)
        }
        return (nil, nil, nil)
    }

    nonisolated private static func callImport(_ h: OpaquePointer, name: String, mime: String,
                                                bytes: Data, derivedText: String?, normalized: Data?,
                                                normalizedMime: String?) -> String {
        name.withCString { namePointer in
            mime.withCString { mimePointer in
                bytes.withUnsafeBytes { raw in
                    let original = raw.bindMemory(to: UInt8.self).baseAddress
                    return (derivedText ?? "").withCString { textPointer in
                        (normalized ?? Data()).withUnsafeBytes { imageRaw in
                            let image = imageRaw.bindMemory(to: UInt8.self).baseAddress
                            return (normalizedMime ?? "").withCString { imageMimePointer in
                                string(from: chorus_import_attachment(h, namePointer, mimePointer,
                                    original, bytes.count, derivedText == nil ? nil : textPointer,
                                    normalized == nil ? nil : image, normalized?.count ?? 0,
                                    normalizedMime == nil ? nil : imageMimePointer))
                            }
                        }
                    }
                }
            }
        }
    }

    nonisolated private static func jsonIDs(_ ids: [Int64]) -> String {
        guard let data = try? JSONEncoder().encode(ids), let text = String(data: data, encoding: .utf8) else { return "[]" }
        return text
    }

    /// Router runs on the engine's independent I/O queue and receives only a
    /// summary of this new message, never the session history or file bytes.
    func proposeRoute(taskJSON: String, candidatesJSON: String, routerModel: String,
                      completion: @escaping (String) -> Void) {
        let preset = routePreset
        ioQueue.async {
            let json = taskJSON.withCString { task in
                candidatesJSON.withCString { candidates in
                    routerModel.withCString { model in
                        preset.withCString { p in
                            Self.string(from: chorus_route_json(task, candidates, model, p))
                        }
                    }
                }
            }
            DispatchQueue.main.async { completion(json) }
        }
    }

    func loadCatalog(project: String, automaticUpdates: Bool = false) {
        if automaticUpdates { catalogBackgroundStatus = "กำลังตรวจเวอร์ชันเครื่องมือผ่าน Homebrew…" }
        catalogQueue.async { [weak self] in
            func list() -> CatalogListResult? {
                let json = project.withCString { Self.string(from: chorus_catalog_list_json($0)) }
                return Self.decode(CatalogListResult.self, json)
            }
            var result = list()
            var updateFailures: [String] = []
            if automaticUpdates, let entries = result?.entries {
                for entry in entries {
                    guard let installed = entry.installed, entry.brew_formula != nil,
                          installed.version.compare(entry.reviewed_version, options: .numeric) == .orderedAscending else { continue }
                    DispatchQueue.main.async {
                        self?.catalogBackgroundStatus = "Homebrew กำลังอัปเดต \(entry.title)…"
                        self?.catalogWorking.insert(entry.id)
                    }
                    let json = project.withCString { path in
                        entry.id.withCString { id in
                            "update".withCString { action in
                                Self.string(from: chorus_catalog_action_json(path, id, action))
                            }
                        }
                    }
                    if Self.decode(SimpleResult.self, json)?.ok != true {
                        updateFailures.append(entry.title)
                    }
                    DispatchQueue.main.async { self?.catalogWorking.remove(entry.id) }
                }
                result = list()
            }
            DispatchQueue.main.async {
                guard let self else { return }
                if automaticUpdates { self.catalogBackgroundStatus = nil }
                if result?.ok == true, let entries = result?.entries {
                    self.catalog = entries.compactMap { entry in
                        guard let source = URL(string: entry.source) else { return nil }
                        return CatalogIntegration(
                            id: entry.id, title: entry.title, maintainer: entry.maintainer,
                            license: entry.license, sourceURL: source,
                            reviewedVersion: entry.reviewed_version, modes: entry.modes,
                            requiredRuntimes: entry.required_runtimes, actions: entry.actions,
                            projectAccess: entry.project_access, networkBehavior: entry.network,
                            requiredSecretNames: entry.secret_names,
                            installedVersion: entry.installed?.version,
                            isHealthy: entry.installed?.health.healthy,
                            healthMessage: entry.installed?.health.detail,
                            integrity: entry.installed?.artifact_integrity,
                            isEnabledForProject: entry.enabled,
                            isWorking: self.catalogWorking.contains(entry.id),
                            canRemove: entry.can_remove,
                            missingInstaller: entry.missing_installer,
                            brewFormula: entry.brew_formula,
                            installedMethod: entry.installed_method)
                    }
                    self.catalogError = updateFailures.isEmpty ? nil
                        : "อัปเดต \(updateFailures.joined(separator: ", ")) ไม่สำเร็จ"
                } else { self.catalogError = result?.error ?? "โหลดรายการเครื่องมือไม่ได้" }
            }
        }
    }

    func loadDashboard(project: String, id: String, completion: @escaping (ToolDashboard?, String?) -> Void) {
        catalogQueue.async {
            let json = project.withCString { path in
                id.withCString { Self.string(from: chorus_catalog_dashboard_json(path, $0)) }
            }
            let result = Self.decode(DashboardResult.self, json)
            DispatchQueue.main.async { completion(result?.dashboard, result?.ok == true ? nil : (result?.error ?? "โหลดแดชบอร์ดไม่ได้")) }
        }
    }

    private struct ToolActionResult: Decodable {
        let ok: Bool
        let error: String?
        let result: ToolActionOutput?
    }

    func runToolAction(project: String, id: String, key: String,
                       completion: @escaping (ToolActionOutput?, String?) -> Void) {
        catalogQueue.async {
            let json = project.withCString { path in
                id.withCString { identifier in
                    key.withCString { Self.string(from: chorus_catalog_run_json(path, identifier, $0)) }
                }
            }
            let result = Self.decode(ToolActionResult.self, json)
            DispatchQueue.main.async { completion(result?.result, result?.ok == true ? nil : (result?.error ?? "สั่งงานไม่ได้")) }
        }
    }

    /// Ask the core what `action` would create or remove. Touches nothing.
    func loadCatalogPlan(project: String, id: String, action: String,
                         completion: @escaping (InstallPlan?, String?) -> Void) {
        catalogQueue.async {
            let json = project.withCString { path in
                id.withCString { identifier in
                    action.withCString { operation in
                        Self.string(from: chorus_catalog_plan_json(path, identifier, operation))
                    }
                }
            }
            let result = Self.decode(CatalogPlanResult.self, json)
            DispatchQueue.main.async { completion(result?.plan, result?.ok == true ? nil : (result?.error ?? "อ่านรายการไม่ได้")) }
        }
    }

    private func pollCatalogProgress() {
        catalogProgressTimer?.invalidate()
        catalogProgressTimer = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { [weak self] _ in
            // A separate queue: the action holds `catalogQueue` until it ends.
            DispatchQueue.global(qos: .userInitiated).async {
                let json = Self.string(from: chorus_catalog_progress_json())
                let progress = Self.decode(CatalogProgressResult.self, json)?.progress
                DispatchQueue.main.async { if progress != nil { self?.catalogProgress = progress } }
            }
        }
    }

    private func stopCatalogProgress() {
        catalogProgressTimer?.invalidate()
        catalogProgressTimer = nil
        catalogProgress = nil
    }

    /// `action`: install, install_enable, update, remove, enable, disable.
    func catalogAction(project: String, id: String, action: String) {
        catalogActionError = nil
        catalogActionResult = nil
        catalogWorking.insert(id)
        catalog = catalog.map { entry in
            var entry = entry
            if entry.id == id { entry.isWorking = true }
            return entry
        }
        pollCatalogProgress()
        catalogQueue.async { [weak self] in
            let json = project.withCString { path in
                id.withCString { identifier in
                    action.withCString { operation in
                        Self.string(from: chorus_catalog_action_json(path, identifier, operation))
                    }
                }
            }
            let result = Self.decode(SimpleResult.self, json)
            DispatchQueue.main.async {
                self?.stopCatalogProgress()
                self?.catalogWorking.remove(id)
                let failure = result?.ok == true ? nil : (result?.error ?? "จัดการเครื่องมือไม่สำเร็จ")
                self?.catalogActionError = failure
                self?.catalogActionResult = CatalogActionResult(id: id, action: action, error: failure)
                self?.loadCatalog(project: project)
            }
        }
    }

    /// The router uses a local Ollama service. Inventory is a cheap readiness
    /// check and avoids sending a real prompt merely to discover setup state.
    func loadRouterModels() {
        routerChecking = true
        routerSetupQueue.async { [weak self] in
            let json = "chorus-openthai-systemone:0.8b".withCString { Self.string(from: chorus_router_models_json($0)) }
            let result = Self.decode(RouterModelsResult.self, json)
            DispatchQueue.main.async {
                // A router must be a local model within the agreed 2 GB cap.
                self?.routerModels = result?.ok == true
                    ? result?.models?.filter {
                        guard let size = $0.size_bytes else { return false }
                        return size > 0 && size <= 2_000_000_000
                    }.map(\.name)
                    : nil
                self?.routerChecking = false
            }
        }
    }

    /// Download only after the user presses the setup card's button. The Rust
    /// router uses the same loopback transport as normal routing, so macOS ATS
    /// does not make the setup check disagree with actual routing.
    func pullRouterModel(_ model: String) {
        guard !routerDownloading, routerModels != nil,
              !model.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        routerDownloading = true
        routerSetupError = nil
        routerSetupQueue.async { [weak self] in
            let json = model.withCString { Self.string(from: chorus_router_pull_json($0)) }
            let result = Self.decode(SimpleResult.self, json)
            DispatchQueue.main.async {
                self?.routerDownloading = false
                self?.routerSetupError = result?.ok == true ? nil : (result?.error ?? "ดาวน์โหลดโมเดลไม่สำเร็จ")
                if result?.ok == true { self?.loadRouterModels() }
            }
        }
    }

    func startOllama(executable: URL) {
        guard ollamaProcess?.isRunning != true else { loadRouterModels(); return }
        routerSetupError = nil
        let process = Process()
        process.executableURL = executable
        process.arguments = ["serve"]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        do {
            try process.run()
            ollamaProcess = process
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
                self?.loadRouterModels()
            }
        } catch {
            routerSetupError = "เปิด Ollama ไม่สำเร็จ: \(error.localizedDescription)"
        }
    }

    // MARK: - 1-Click In-Process Local Models Manager

    struct LocalModelStatus: Identifiable, Decodable {
        var id: String { spec.id }
        struct Spec: Decodable {
            let id: String
            let display_name: String
            let filename: String
            let download_url: String
            let expected_size_bytes: Int64
            let min_disk_free_bytes: Int64
            let purpose: String
        }
        let status: String
        let spec: Spec
        let path: String?
        let size_bytes: Int64?
        let free_disk_bytes: Int64?
        let can_download: Bool?
        let reason: String?
        
        var isInstalled: Bool {
            status == "ready"
        }
    }

    private struct ModelManagerListResponse: Decodable {
        let ok: Bool
        let models: [LocalModelStatus]?
        let error: String?
    }

    @Published var localModels: [LocalModelStatus] = []
    @Published var isDownloadingLocalModel: Bool = false
    @Published var localModelDownloadProgress: Double = 0.0
    @Published var localModelDownloadStatusText: String = ""
    @Published var localModelError: String? = nil
    private var modelDownloadTask: URLSessionDownloadTask?
    private var modelDownloadID: UUID?
    var canCancelLocalModelDownload: Bool { modelDownloadTask != nil }

    /// System monitor request (`chorus_monitor_json`; the window builds the
    /// JSON, Rust decides). Snapshots touch no SQLite, so they run on their
    /// own queue and never wait behind a turn. Clean-ups that write a store
    /// or the shadow repo go through `queue` with the handle, so they cannot
    /// race the engine's connection and the open session's checkpoint base
    /// is protected from a prune.
    func monitor(_ request: String, onEngineQueue: Bool = false, completion: @escaping (String) -> Void) {
        if onEngineQueue {
            let h = handle
            queue.async {
                let s = Self.string(from: chorus_monitor_json(h?.raw, request))
                DispatchQueue.main.async { completion(s) }
            }
        } else {
            monitorQueue.async {
                let s = Self.string(from: chorus_monitor_json(nil, request))
                DispatchQueue.main.async { completion(s) }
            }
        }
    }

    func loadLocalModels() {
        routerSetupQueue.async { [weak self] in
            let json = Self.string(from: chorus_model_manager_list_json())
            let res = Self.decode(ModelManagerListResponse.self, json)
            DispatchQueue.main.async {
                if let models = res?.models {
                    self?.localModels = models
                }
            }
        }
    }

    func downloadLocalModel(_ spec: LocalModelStatus.Spec) {
        guard !isDownloadingLocalModel else { return }
        guard let url = URL(string: spec.download_url), url.scheme?.lowercased() == "https" else {
            localModelError = "URL สำหรับดาวน์โหลดโมเดลไม่ถูกต้อง"
            return
        }

        isDownloadingLocalModel = true
        localModelDownloadProgress = 0.0
        localModelDownloadStatusText = "กำลังเริ่มดาวน์โหลด..."
        localModelError = nil
        let downloadID = UUID()
        modelDownloadID = downloadID

        let sessionConfig = URLSessionConfiguration.default
        let session = URLSession(configuration: sessionConfig)

        let task = session.downloadTask(with: url) { [weak self] tempURL, response, error in
            // URLSession owns tempURL only until this callback returns.
            // Move it synchronously before dispatching installation to another queue.
            let ownedFile: Result<URL, Error> = Result {
                if let error { throw error }
                guard let response = response as? HTTPURLResponse,
                      response.statusCode == 200,
                      response.url?.scheme?.lowercased() == "https" else {
                    throw URLError(.badServerResponse)
                }
                guard let tempURL else { throw URLError(.fileDoesNotExist) }
                let destination = FileManager.default.temporaryDirectory
                    .appendingPathComponent("chorus-model-\(downloadID.uuidString).gguf")
                try FileManager.default.moveItem(at: tempURL, to: destination)
                return destination
            }
            session.finishTasksAndInvalidate()
            DispatchQueue.main.async {
                guard let self, self.modelDownloadID == downloadID else {
                    if case .success(let url) = ownedFile { Self.removeStagedModelDownload(at: url) }
                    return
                }
                self.modelDownloadTask = nil
                switch ownedFile {
                case .failure(let error):
                    self.isDownloadingLocalModel = false
                    self.modelDownloadID = nil
                    self.localModelError = "ดาวน์โหลดไม่สำเร็จ: \(error.localizedDescription)"
                case .success(let url):
                    self.localModelDownloadStatusText = "กำลังตรวจสอบและติดตั้งโมเดล..."
                    self.finalizeLocalModel(tempPath: url.path, modelId: spec.id, downloadID: downloadID)
                }
            }
        }

        self.modelDownloadTask = task
        task.resume()
    }

    func cancelLocalModelDownload() {
        // Once the transfer has completed, installation is already committed to its queue.
        guard modelDownloadTask != nil else { return }
        modelDownloadTask?.cancel()
        modelDownloadTask = nil
        modelDownloadID = nil
        isDownloadingLocalModel = false
        localModelDownloadStatusText = "ยกเลิกการดาวน์โหลดแล้ว"
    }

    private func finalizeLocalModel(tempPath: String, modelId: String, downloadID: UUID) {
        routerSetupQueue.async { [weak self] in
            defer { Self.removeStagedModelDownload(at: URL(fileURLWithPath: tempPath)) }
            let json = tempPath.withCString { cTemp in
                modelId.withCString { cId in
                    Self.string(from: chorus_model_manager_finalize_json(cTemp, cId))
                }
            }
            struct FinRes: Decodable { let ok: Bool; let error: String? }
            let res = Self.decode(FinRes.self, json)
            DispatchQueue.main.async {
                guard let self, self.modelDownloadID == downloadID else { return }
                self.isDownloadingLocalModel = false
                self.modelDownloadID = nil
                if res?.ok == true {
                    self.localModelDownloadStatusText = "ติดตั้งสำเร็จเรียบร้อย"
                    self.localModelError = nil
                    self.loadLocalModels()
                } else {
                    self.localModelError = res?.error ?? "การตรวจสอบและติดตั้งโมเดลล้มเหลว"
                }
            }
        }
    }

    private nonisolated static func removeStagedModelDownload(at url: URL) {
        guard FileManager.default.fileExists(atPath: url.path) else { return }
        do {
            try FileManager.default.removeItem(at: url)
        } catch {
            NSLog("Local model staging cleanup failed (error %ld)", (error as NSError).code)
        }
    }

    func deleteLocalModel(modelId: String) {
        routerSetupQueue.async { [weak self] in
            let json = modelId.withCString { cId in
                Self.string(from: chorus_model_manager_delete_json(cId))
            }
            struct DelRes: Decodable { let ok: Bool; let error: String? }
            let res = Self.decode(DelRes.self, json)
            DispatchQueue.main.async {
                if res?.ok == true {
                    self?.loadLocalModels()
                } else {
                    self?.localModelError = res?.error ?? "ไม่สามารถลบโมเดลได้"
                }
            }
        }
    }

    // MARK: - Auto Router Learning Memory Flywheel

    struct LearnedRoute: Identifiable, Codable, Hashable {
        let id: Int64
        let prompt: String
        let chosen_model: String
        let chosen_agent: String
        let reason: String
        let created_ms: Int64
        let preset: String?
    }

    struct RoutePresetOption: Codable, Hashable, Identifiable {
        let id: String
        let label: String
        let hint: String
    }
    private struct RoutePresetState: Decodable {
        let ok: Bool
        let preset: String?
        let label: String?
        let presets: [RoutePresetOption]?
    }

    /// This session's route preset (`economy` | `balanced` | `max`), its Thai
    /// label, and every option, all as Rust reports them.
    @Published var routePreset = "balanced"
    @Published var routePresetLabel = "สมดุล"
    @Published var routePresets: [RoutePresetOption] = []

    nonisolated static let defaultRoutePresetKey = "chorus.routePreset.default"

    /// Read the session's preset on `queue`; a session that has none yet this
    /// run gets the Settings default.
    nonisolated private static func initialRoutePreset(_ h: OpaquePointer) -> String {
        let json = string(from: chorus_route_preset(h))
        guard let state = decode(RoutePresetState.self, json), state.preset == nil else { return json }
        let fallback = UserDefaults.standard.string(forKey: defaultRoutePresetKey) ?? "balanced"
        return fallback.withCString { string(from: chorus_set_route_preset(h, $0)) }
    }

    private func applyRoutePreset(_ json: String) {
        guard let state = Self.decode(RoutePresetState.self, json), state.ok else { return }
        if let presets = state.presets { routePresets = presets }
        if let preset = state.preset { routePreset = preset }
        if let label = state.label { routePresetLabel = label }
    }

    func setRoutePreset(_ id: String) {
        guard let h = handle else { return }
        queue.async { [weak self] in
            let json = id.withCString { Self.string(from: chorus_set_route_preset(h.raw, $0)) }
            DispatchQueue.main.async { self?.applyRoutePreset(json) }
        }
    }

    @Published var learnedRoutes: [LearnedRoute] = []
    @Published var isLoadingLearnedRoutes = false

    func loadLearnedRoutes(workspace: String? = nil) {
        isLoadingLearnedRoutes = true
        let ws = workspace ?? ""
        routerSetupQueue.async { [weak self] in
            let json = ws.withCString { cWs in
                Self.string(from: chorus_router_list_learned_json(cWs))
            }
            struct R: Codable { let ok: Bool; let routes: [LearnedRoute]? }
            let res = Self.decode(R.self, json)
            DispatchQueue.main.async {
                self?.isLoadingLearnedRoutes = false
                if let routes = res?.routes {
                    self?.learnedRoutes = routes
                }
            }
        }
    }

    /// Record a choice the user made, under this session's preset. Never
    /// call it with the router's own pick.
    func recordLearnedRoute(workspace: String? = nil, prompt: String, model: String, agent: String, reason: String) {
        guard !prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        let ws = workspace ?? ""
        let preset = routePreset
        routerSetupQueue.async { [weak self] in
            let _ = ws.withCString { cWs in
                prompt.withCString { cP in
                    model.withCString { cM in
                        agent.withCString { cA in
                            reason.withCString { cR in
                                preset.withCString { cPr in
                                    Self.string(from: chorus_router_record_learned_json(cWs, cP, cM, cA, cR, cPr))
                                }
                            }
                        }
                    }
                }
            }
            DispatchQueue.main.async {
                self?.loadLearnedRoutes(workspace: workspace)
            }
        }
    }

    func clearLearnedRoutes(workspace: String? = nil, completion: (() -> Void)? = nil) {
        let ws = workspace ?? ""
        routerSetupQueue.async { [weak self] in
            let _ = ws.withCString { cWs in
                Self.string(from: chorus_router_clear_learned_json(cWs))
            }
            DispatchQueue.main.async {
                self?.learnedRoutes = []
                completion?()
            }
        }
    }

    /// Run a turn where every permission request pops a dialog.
    func sendInteractive(_ prompt: String, agent: String, attachments: [Int64] = []) {
        guard let h = handle, !busy else { return }
        busy = true; lastError = nil
        let clean = agent.split(separator: "@", maxSplits: 1).first.map(String.init) ?? agent
        liveAgent = clean
        PowerManager.shared.beginActivity(reason: "Chorus Interactive Turn (\(clean))")
        echoUser(prompt)
        let ctx = CallbackContext(raw: Unmanaged.passUnretained(self).toOpaque())
        let idsJSON = Self.jsonIDs(attachments)
        let cb: @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>?) -> Int32 = { ctx, ask in
            guard let ctx, let ask else { return 0 }
            let engine = Unmanaged<Engine>.fromOpaque(ctx).takeUnretainedValue()
            return engine.askPermissionBlocking(String(cString: ask))
        }
        queue.async { [weak self] in
            let json = prompt.withCString { p in
                agent.withCString { a in
                    idsJSON.withCString { ids in
                        Self.string(from: chorus_run_turn_interactive_with_attachments(h.raw, a, p, ids, cb, ctx.raw))
                    }
                }
            }
            let r = Self.decode(TurnResult.self, json)
            DispatchQueue.main.async {
                PowerManager.shared.endActivity()
                self?.busy = false
                self?.endLive()
                if let r, r.ok {
                    self?.contextUsed = r.context_used
                    self?.contextWindow = r.context_window
                } else if !Self.isStop(r?.error) {
                    self?.lastError = r?.error ?? "turn failed"
                }
                self?.refresh()
                self?.compactIfIdle()
            }
        }
    }

    /// Run one turn, then refresh the transcript.
    /// `level` mirrors `Policy::from_level` in chorus-cli/session.rs:
    /// 0 deny · 1 read-only (auto-allow reads, deny writes/commands/network) · 2 allow all.
    func send(_ prompt: String, agent: String, level: Int32, attachments: [Int64] = []) {
        guard let h = handle, !busy else { return }
        busy = true
        let clean = agent.split(separator: "@", maxSplits: 1).first.map(String.init) ?? agent
        liveAgent = clean
        PowerManager.shared.beginActivity(reason: "Chorus Turn (\(clean))")
        echoUser(prompt)
        lastError = nil
        let idsJSON = Self.jsonIDs(attachments)
        queue.async { [weak self] in
            let json = prompt.withCString { p in
                agent.withCString { a in
                    idsJSON.withCString { ids in
                        Self.string(from: chorus_run_turn_with_attachments(h.raw, a, p, ids, level))
                    }
                }
            }
            let result = Self.decode(TurnResult.self, json)
            DispatchQueue.main.async {
                PowerManager.shared.endActivity()
                self?.busy = false
                self?.endLive()
                if let r = result, r.ok {
                    self?.contextUsed = r.context_used
                    self?.contextWindow = r.context_window
                } else {
                    if !Self.isStop(result?.error) { self?.lastError = result?.error ?? "turn failed" }
                }
                self?.refresh()
                self?.compactIfIdle()
            }
        }
    }

    /// Run one turn against a relay chain (comma-separated agent names).
    func sendRelay(_ prompt: String, agents: String, level: Int32, attachments: [Int64] = []) {
        guard let h = handle, !busy else { return }
        busy = true; lastError = nil
        let first = agents.split(separator: ",").first.map(String.init) ?? agents
        let clean = first.split(separator: "@", maxSplits: 1).first.map(String.init) ?? first
        liveAgent = clean
        PowerManager.shared.beginActivity(reason: "Chorus Relay (\(clean))")
        echoUser(prompt)
        let idsJSON = Self.jsonIDs(attachments)
        queue.async { [weak self] in
            let json = prompt.withCString { p in
                agents.withCString { a in
                    idsJSON.withCString { ids in
                        Self.string(from: chorus_run_relay_with_attachments(h.raw, a, p, ids, level))
                    }
                }
            }
            let obj = Self.decode(TurnResult.self, json)
            DispatchQueue.main.async {
                PowerManager.shared.endActivity()
                self?.busy = false
                self?.endLive()
                if let r = obj, r.ok {
                    // Every composer turn goes through relay; dropping these
                    // left the context pane stuck on "no turn yet".
                    if let u = r.context_used { self?.contextUsed = u }
                    if let w = r.context_window { self?.contextWindow = w }
                } else if !Self.isStop(obj?.error) { self?.lastError = obj?.error ?? "relay failed" }
                self?.refresh()
                self?.compactIfIdle()
            }
        }
    }

    // MARK: - Computer use

    /// `nonisolated`: read on `queue` when a session handle is created.
    nonisolated static let computerUseKey = "chorus.computerUse"

    struct ComputerPermissions: Codable, Equatable {
        let supported: Bool
        let screen_recording: Bool
        let accessibility: Bool
    }

    /// What macOS has granted Chorus. `request` shows the Screen Recording
    /// prompt first (macOS shows it once per app).
    func computerPermissions(request: Bool = false) -> ComputerPermissions? {
        Self.decode(ComputerPermissions.self, Self.string(from: chorus_computer_permissions_json(request ? 1 : 0)))
    }

    nonisolated static let claudeChromeKey = "chorus.claudeChrome"

    nonisolated static let hideSecretFilesKey = "chorus.hideSecretFiles"
    /// On unless the user turned it off (`bool(forKey:)` would read unset as off).
    nonisolated static var hideSecretFiles: Bool {
        UserDefaults.standard.object(forKey: hideSecretFilesKey) as? Bool ?? true
    }

    func setHideSecretFiles(_ on: Bool) {
        UserDefaults.standard.set(on, forKey: Self.hideSecretFilesKey)
        guard let h = handle else { return }
        queue.async { chorus_set_hide_secret_files(h.raw, on ? 1 : 0) }
    }

    struct SecretEntry: Decodable, Identifiable {
        let name: String
        let kind: String
        var id: String { name }
    }

    /// Names Chorus remembers, never values (the values are in the Keychain).
    func secrets() -> [SecretEntry] {
        Self.decode([SecretEntry].self, Self.string(from: chorus_secrets_json())) ?? []
    }

    func forgetSecret(_ name: String) {
        _ = Self.string(from: chorus_secret_forget(name))
    }

    /// Claude in Chrome (Claude's own extension). Stored and applied like
    /// `setComputerUse`; takes effect from the next turn's bridge.
    func setClaudeChrome(_ on: Bool) {
        UserDefaults.standard.set(on, forKey: Self.claudeChromeKey)
        guard let h = handle else { return }
        queue.async { chorus_set_claude_chrome(h.raw, on ? 1 : 0) }
    }

    /// Settings switch. Stored for the next sessions and applied to the open one.
    func setComputerUse(_ on: Bool) {
        UserDefaults.standard.set(on, forKey: Self.computerUseKey)
        guard let h = handle else { return }
        queue.async { chorus_set_computer_use(h.raw, on ? 1 : 0) }
    }

    /// Plan mode: one read-only turn that answers with a plan. Rust forces
    /// read-only and adds the plan instructions to the prompt; on success the
    /// chain is kept so `approvePlan` can hand the same agents the go-ahead.
    func sendPlan(_ prompt: String, agents: String, attachments: [Int64] = []) {
        guard let h = handle, !busy else { return }
        busy = true; lastError = nil
        let first = agents.split(separator: ",").first.map(String.init) ?? agents
        liveAgent = first.split(separator: "@", maxSplits: 1).first.map(String.init) ?? first
        PowerManager.shared.beginActivity(reason: "Chorus Plan (\(liveAgent ?? first))")
        echoUser(prompt)
        let idsJSON = Self.jsonIDs(attachments)
        queue.async { [weak self] in
            let json = prompt.withCString { p in
                agents.withCString { a in
                    idsJSON.withCString { ids in
                        Self.string(from: chorus_run_plan_with_attachments(h.raw, a, p, ids))
                    }
                }
            }
            let obj = Self.decode(TurnResult.self, json)
            DispatchQueue.main.async {
                PowerManager.shared.endActivity()
                self?.busy = false
                self?.endLive()
                if let r = obj, r.ok {
                    if let u = r.context_used { self?.contextUsed = u }
                    if let w = r.context_window { self?.contextWindow = w }
                    self?.pendingPlanAgents = agents
                } else if !Self.isStop(obj?.error) { self?.lastError = obj?.error ?? "plan failed" }
                self?.refresh()
                self?.compactIfIdle()
            }
        }
    }

    /// The approval message for a plan — the text lives in Rust.
    var planApprovedPrompt: String { Self.string(from: chorus_plan_approved_prompt()) }

    /// Orchestra mode turn. Who leads, its read-only permission, failover to
    /// the backup and the children's caps are all decided in Rust; this only
    /// passes the Settings values through.
    func sendOrchestra(_ prompt: String, primary: String, backup: String,
                       concurrency: Int, maxChildren: Int, maxMinutes: Int,
                       level: Int32, attachments: [Int64] = []) {
        guard let h = handle, !busy else { return }
        busy = true; lastError = nil
        PowerManager.shared.beginActivity(reason: "Chorus Orchestra")
        echoUser(prompt)
        let idsJSON = Self.jsonIDs(attachments)
        queue.async { [weak self] in
            let json = prompt.withCString { p in
                idsJSON.withCString { ids in
                    primary.withCString { pr in
                        backup.withCString { bk in
                            Self.string(from: chorus_run_orchestra(
                                h.raw, p, ids, pr, bk,
                                Int32(concurrency), Int32(maxChildren), Int32(maxMinutes), level))
                        }
                    }
                }
            }
            let obj = Self.decode(TurnResult.self, json)
            DispatchQueue.main.async {
                PowerManager.shared.endActivity()
                self?.busy = false
                self?.endLive()
                if let r = obj, r.ok {
                    if let u = r.context_used { self?.contextUsed = u }
                    if let w = r.context_window { self?.contextWindow = w }
                } else if !Self.isStop(obj?.error) { self?.lastError = obj?.error ?? "orchestra failed" }
                self?.refresh()
                self?.compactIfIdle()
            }
        }
    }

    /// Compare mode — fork per agent, run the prompt on each.
    func compare(_ prompt: String, agents: String, level: Int32,
                 completion: @escaping (String) -> Void) {
        guard let h = handle, !busy else { return }
        busy = true; lastError = nil
        PowerManager.shared.beginActivity(reason: "Chorus Compare")
        queue.async { [weak self] in
            let json = prompt.withCString { p in
                agents.withCString { a in
                    Self.string(from: chorus_compare(h.raw, a, p, level))
                }
            }
            DispatchQueue.main.async {
                PowerManager.shared.endActivity()
                self?.busy = false
                self?.endLive()
                self?.refresh()
                completion(json)
            }
        }
    }

    /// Hand the session to another agent.
    func handoff(to: String, reason: String, goal: String, done: [String], pending: [String], level: Int32,
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
                                Self.string(from: chorus_handoff(h.raw, t, r, g, d, p, level))
                            }
                        }
                    }
                }
            }
            let result = Self.decode(HandoffResult.self, json)
            DispatchQueue.main.async {
                self?.busy = false
                self?.endLive()
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
    static func isStop(_ e: String?) -> Bool { e?.contains("หยุดโดยผู้ใช้") == true }

    func refresh() {
        guard let h = handle else { return }
        // The thread comes from the delta pull on the read-only handle; only
        // without one (no file-backed store) does it fall back to a full read.
        if reader != nil { pullDelta() }
        let full = reader == nil
        queue.async { [weak self] in
            let eventsJSON = full ? Self.string(from: chorus_transcript_json(h.raw)) : ""
            let attachmentsJSON = Self.string(from: chorus_list_attachments(h.raw))
            let diffJSON = Self.string(from: chorus_working_diff(h.raw))
            // Parse here, off the main thread, once per refresh — views read
            // `diffHunks` instead of re-parsing the diff on every render.
            let diff = Self.decode(DiffResult.self, diffJSON).map { $0.diff ?? "" }
            let hunks = diff.map(parseHunks)
            let memJSON = Self.string(from: chorus_memory_state(h.raw))
            let taskJSON = Self.string(from: chorus_task_state(h.raw))
            let warmJSON = Self.string(from: chorus_uncompacted(h.raw))
            DispatchQueue.main.async {
                guard let self else { return }
                if full, let rows = Self.decode([LedgerEvent].self, eventsJSON) {
                    self.events = rows
                    self.lastSeq = rows.last(where: { $0.seq > 0 })?.seq ?? 0
                }
                if let rows = Self.decode(SessionAttachmentList.self, attachmentsJSON)?.attachments {
                    self.attachments = rows
                }
                if let t = Self.decode(TaskStateResult.self, taskJSON) {
                    self.taskState = t.text ?? ""
                }
                if let w = Self.decode(WarmTierResult.self, warmJSON) {
                    self.uncompacted = w.pending ?? 0
                }
                if let diff, let hunks, diff != self.workingDiff {
                    self.setWorkingDiff(diff, hunks: hunks)
                }
                if let s = Self.decode(MemoryState.self, memJSON) {
                    self.facts = s.facts ?? []
                    self.pending = s.pending ?? []
                }
            }
        }
    }

    private struct TaskStateResult: Codable {
        let ok: Bool?
        let text: String?
    }
    private struct WarmTierResult: Codable {
        let ok: Bool?
        let pending: Int64?
    }
    private struct CompactResult: Codable {
        let ok: Bool?
        let compacted: Int?
        let remaining: Int?
        let model: String?
        let error: String?
    }


    // MARK: agents, sessions, one-click handoff

    struct ModelChoice: Codable, Hashable {
        let id: String
        let label: String
        let is_default: Bool
    }

    struct AgentInfo: Codable, Identifiable, Hashable {
        let id: String
        let label: String
        let models: [ModelChoice]
        let auth: String        // "ok" | "needs_login" | "missing_cli"
        let hint: String        // the exact command that fixes it
        let detail: String

        var ready: Bool { auth == "ok" }
        var defaultModel: String { models.first(where: { $0.is_default })?.id ?? models.first?.id ?? id }
    }

    struct SessionSummary: Codable, Identifiable, Hashable {
        let id: Int64
        let title: String
        let cwd: String
        let started_ms: Int64
        let last_ms: Int64
        let events: Int64
        let agents: [String]
        let goal: String
        let forked_from: Int64?
        let cost_usd: Double?
    }

    private struct AgentsResult: Codable { let ok: Bool?; let agents: [AgentInfo]? }
    private struct SessionsResult: Codable { let ok: Bool?; let sessions: [SessionSummary]? }
    private struct SubagentsResult: Codable { let ok: Bool?; let subagents: [SubagentInfo]? }
    private struct HandoffAutoResult: Codable {
        let ok: Bool?; let to: String?; let goal: String?; let echo_back: String?; let error: String?
    }

    struct SubagentInfo: Codable, Identifiable, Hashable {
        let id: Int64
        let parent_sess: Int64?
        let child_sess: Int64?
        let kind: String
        let role: String
        let task: String
        let worktree: String?
        let status: String
        let summary: String?
        let diff_stat: String?
        let files_touched: [String]
        let created_ms: Int64
        let updated_ms: Int64
        let model: String?
    }

    @Published var agents: [AgentInfo] = []
    @Published var sessions: [SessionSummary] = []
    /// Sessions per project database, so the sidebar can list several
    /// projects at once instead of only the open one.
    @Published var sessionsByDB: [String: [SessionSummary]] = [:]
    /// Subagents per project database and per parent session.
    @Published var subagentsByDB: [String: [SubagentInfo]] = [:]
    @Published var subagentsByParent: [Int64: [SubagentInfo]] = [:]
    /// Live task checklist from chorus_task_update for the current session.
    @Published var checklist: [ChecklistItem] = []

    /// Pin the model an agent runs. Must be called for the picker to mean
    /// anything: the engine defaults to whatever the agent's own CLI is
    /// configured for, so without this the picker changes a label and nothing
    /// else — which is exactly what it did until this existed.
    func setModel(agent: String, model: String) {
        guard let h = handle else { return }
        queue.async {
            agent.withCString { a in
                model.withCString { m in
                    _ = Self.string(from: chorus_set_model(h.raw, a, m))
                }
            }
        }
    }

    /// Which agents can actually run here. Cheap enough to call on every
    /// appearance and after the user comes back from logging in.
    struct SkillInfo: Codable, Identifiable, Hashable {
        let name: String
        let description: String
        let path: String
        let source: String      // "chorus" | "project" | "agents" | "user" (Claude) | "codex" | "gemini"
        var id: String { source + path }
    }
    @Published var skills: [SkillInfo] = []

    func loadSkills(cwd: String) {
        ioQueue.async { [weak self] in
            let json = cwd.withCString { Self.string(from: chorus_list_skills($0)) }
            struct R: Codable { let skills: [SkillInfo]? }
            let r = Self.decode(R.self, json)
            DispatchQueue.main.async { self?.skills = r?.skills ?? [] }
        }
    }

    @Published var refreshingModels = false
    private var modelsRefreshed = false

    /// Re-ask each ready agent for its model list. The cache it reads is
    /// otherwise only filled by `chorus models`, so a list from weeks ago
    /// (missing Sonnet 1M, Fable…) was what the picker kept showing.
    /// Spawns each bridge once, in a scratch dir, off every other queue.
    func refreshModels(force: Bool = false) {
        guard force || !modelsRefreshed, !refreshingModels else { return }
        modelsRefreshed = true
        let ids = agents.filter(\.ready).map(\.id)
        guard !ids.isEmpty else { return }
        refreshingModels = true
        let dir = (NSTemporaryDirectory() as NSString).appendingPathComponent("chorus-model-probe")
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        ioQueue.async { [weak self] in
            for id in ids {
                _ = id.withCString { a in dir.withCString { d in Self.string(from: chorus_refresh_models(a, d)) } }
            }
            DispatchQueue.main.async {
                self?.refreshingModels = false
                self?.loadAgents()
            }
        }
    }

    // MARK: accounts & usage

    struct UsageWindow: Codable, Hashable {
        let used_percent: Double
        let window_minutes: Int64?
        let resets_at: Int64?
        let resets_label: String?
    }
    struct ModelQuotaGroup: Codable, Hashable, Identifiable {
        var id: String { name }
        let name: String
        let description: String?
        let weekly: UsageWindow?
        let five_hour: UsageWindow?
    }
    struct RateLimits: Codable, Hashable {
        let primary: UsageWindow?
        let secondary: UsageWindow?
        let groups: [ModelQuotaGroup]?
    }
    struct LimitHit: Codable, Hashable { let at_ms: Int64; let resets: String? }
    struct Account: Codable, Identifiable, Hashable {
        let name: String        // what goes in a relay chain: codex, claude@work
        let agent: String
        let account: String     // "" = the CLI's own login
        let dir: String
        let ready: Bool
        let login: String
        let rate_limits: RateLimits?
        let hit: LimitHit?
        /// "ไม่ทราบ" text from Rust when the quota cannot be known right now.
        let quota_note: String?
        /// The user's own name / icon (`sf:<symbol>`, nil = the provider's mark) / `#RRGGBB`.
        let label: String?
        let icon: String?
        let color: String?
        var id: String { name }
        var title: String { label ?? (account.isEmpty ? "ค่าเริ่มต้น" : account) }
        /// Limited right now, as far as we know.
        var limited: Bool {
            if let hit, Date().timeIntervalSince1970 * 1000 - Double(hit.at_ms) < 5 * 3600 * 1000 { return true }
            return (rate_limits?.primary?.used_percent ?? 0) >= 100 || (rate_limits?.secondary?.used_percent ?? 0) >= 100
        }
        /// The fullest window, 0…1, for a one-bar summary.
        var worst: Double? {
            let v = [rate_limits?.primary?.used_percent, rate_limits?.secondary?.used_percent].compactMap { $0 }.max()
            return v.map { min($0 / 100, 1) } ?? (limited ? 1 : nil)
        }
    }
    @Published var accounts: [Account] = []

    func loadUsage() {
        ioQueue.async { [weak self] in
            let json = Self.string(from: chorus_usage_report())
            struct R: Codable { let accounts: [Account]? }
            let r = Self.decode(R.self, json)
            DispatchQueue.main.async { if let a = r?.accounts { self?.accounts = a } }
        }
    }

    /// Create an account dir, then open Terminal on its login command.
    func addAccount(agent: String, name: String) {
        ioQueue.async { [weak self] in
            let json = agent.withCString { a in name.withCString { Self.string(from: chorus_add_account(a, $0)) } }
            struct R: Codable { let ok: Bool?; let login: String?; let error: String? }
            let r = Self.decode(R.self, json)
            DispatchQueue.main.async {
                guard let self else { return }
                if let login = r?.login, r?.ok == true { self.runLoginCommand(login) }
                else { self.lastError = "เพิ่มบัญชีไม่ได้: \(r?.error ?? "unknown")" }
                self.loadUsage()
            }
        }
    }

    /// New account from a look: Rust derives a unique folder name (from
    /// `slug`, else the label), saves the look, then Terminal opens on login.
    func createAccount(agent: String, label: String, slug: String?, icon: String?, color: String?) {
        var spec: [String: String] = ["label": label]
        if let slug { spec["slug"] = slug }
        if let icon { spec["icon"] = icon }
        if let color { spec["color"] = color }
        guard let data = try? JSONSerialization.data(withJSONObject: spec),
              let json = String(data: data, encoding: .utf8) else { return }
        ioQueue.async { [weak self] in
            let reply = agent.withCString { a in json.withCString { Self.string(from: chorus_create_account(a, $0)) } }
            struct R: Codable { let ok: Bool?; let login: String?; let error: String? }
            let r = Self.decode(R.self, reply)
            DispatchQueue.main.async {
                guard let self else { return }
                if let login = r?.login, r?.ok == true { self.runLoginCommand(login) }
                else { self.lastError = "เพิ่มบัญชีไม่ได้: \(r?.error ?? "unknown")" }
                self.loadUsage()
            }
        }
    }

    /// Rename / re-icon an account. `name` is the folder name, "" for the CLI's own login.
    func setAccountLook(agent: String, name: String, label: String, icon: String?, color: String?) {
        var look: [String: String] = [:]
        if !label.isEmpty { look["label"] = label }
        if let icon { look["icon"] = icon }
        if let color { look["color"] = color }
        guard let data = try? JSONSerialization.data(withJSONObject: look),
              let json = String(data: data, encoding: .utf8) else { return }
        ioQueue.async { [weak self] in
            _ = agent.withCString { a in name.withCString { n in json.withCString { Self.string(from: chorus_set_account_look(a, n, $0)) } } }
            DispatchQueue.main.async { self?.loadUsage() }
        }
    }

    func removeAccount(agent: String, name: String) {
        ioQueue.async { [weak self] in
            _ = agent.withCString { a in name.withCString { Self.string(from: chorus_remove_account(a, $0)) } }
            DispatchQueue.main.async { self?.loadUsage() }
        }
    }

    func loadAgents() {
        ioQueue.async { [weak self] in
            let json = Self.string(from: chorus_list_agents())
            let r = Self.decode(AgentsResult.self, json)
            DispatchQueue.main.async { self?.agents = r?.agents ?? [] }
        }
    }

    /// Every past session in this project's store, newest activity first.
    func loadSessions(db: String) {
        loadSubagents(db: db)
        ioQueue.async { [weak self] in
            let json = db.withCString { Self.string(from: chorus_list_sessions($0)) }
            let r = Self.decode(SessionsResult.self, json)
            DispatchQueue.main.async {
                let list = r?.sessions ?? []
                self?.sessions = list
                self?.sessionsByDB[db] = list
                for s in list {
                    SpotlightIndexer.shared.indexSession(id: s.id, title: s.title, goal: s.goal, cwd: s.cwd)
                }
            }
        }
    }

    /// Load subagents for a project or parent session.
    func loadSubagents(db: String, parentSess: Int64 = 0) {
        ioQueue.async { [weak self] in
            let json = db.withCString { Self.string(from: chorus_list_subagents($0, parentSess)) }
            let r = Self.decode(SubagentsResult.self, json)
            DispatchQueue.main.async {
                guard let self else { return }
                let list = r?.subagents ?? []
                self.subagentsByDB[db] = list
                for sub in list {
                    if let p = sub.parent_sess {
                        var existing = self.subagentsByParent[p] ?? []
                        if let idx = existing.firstIndex(where: { $0.id == sub.id }) {
                            existing[idx] = sub
                        } else {
                            existing.append(sub)
                        }
                        self.subagentsByParent[p] = existing
                    }
                }
            }
        }
    }

    /// Cancel a running or waiting subagent.
    func cancelSubagent(db: String, id: Int64) {
        ioQueue.async { [weak self] in
            _ = db.withCString { Self.string(from: chorus_cancel_subagent($0, id)) }
            DispatchQueue.main.async {
                self?.loadSubagents(db: db)
            }
        }
    }

    /// Send a steering message to a subagent.
    func messageSubagent(db: String, id: Int64, message: String) {
        ioQueue.async { [weak self] in
            db.withCString { dbPtr in
                message.withCString { msgPtr in
                    _ = Self.string(from: chorus_message_subagent(dbPtr, id, msgPtr))
                }
            }
            DispatchQueue.main.async {
                self?.loadSubagents(db: db)
            }
        }
    }

    /// Resume a waiting child subagent (e.g. paused at a cap).
    func resumeSubagent(db: String, id: Int64) {
        ioQueue.async { [weak self] in
            _ = db.withCString { Self.string(from: chorus_resume_subagent($0, id)) }
            DispatchQueue.main.async {
                self?.loadSubagents(db: db)
            }
        }
    }

    /// Cancel all children of an orchestration.
    func cancelOrchestration(db: String, parentSess: Int64) {
        ioQueue.async { [weak self] in
            _ = db.withCString { Self.string(from: chorus_cancel_orchestration($0, parentSess)) }
            DispatchQueue.main.async {
                self?.loadSubagents(db: db)
            }
        }
    }

    /// Task checklist items for a session.
    struct ChecklistItem: Identifiable, Codable, Equatable {
        var id: String { text }
        let text: String
        let done: Bool
        let in_progress: Bool
    }
    struct ChecklistResult: Codable {
        let ok: Bool
        let items: [ChecklistItem]?
    }

    func loadChecklist(db: String, sessionID: Int64) {
        guard sessionID >= 0 else {
            self.checklist = []
            return
        }
        ioQueue.async { [weak self] in
            let json = db.withCString { Self.string(from: chorus_get_checklist_json($0, sessionID)) }
            let r = Self.decode(ChecklistResult.self, json)
            DispatchQueue.main.async {
                self?.checklist = r?.items ?? []
            }
        }
    }

    /// Hand the session to another agent with nothing typed by the user: the
    /// brief is built from task state the ledger already holds. This is the
    /// difference between "switch agent" being a form and being a button.
    func handoffAuto(to: String, level: Int32) {
        guard let h = handle, !busy else { return }
        busy = true
        lastError = nil
        queue.async { [weak self] in
            let json = to.withCString { Self.string(from: chorus_handoff_auto(h.raw, $0, level)) }
            let r = Self.decode(HandoffAutoResult.self, json)
            DispatchQueue.main.async {
                guard let self else { return }
                self.busy = false
                if r?.ok != true { self.lastError = r?.error ?? "ส่งต่อไม่สำเร็จ" }
                self.refresh()
                self.compactIfIdle()
            }
        }
    }

    /// Open Terminal on the command that fixes an agent's login. The app can
    /// not complete an OAuth flow itself, and pretending otherwise would just
    /// hide the one step the user has to take.
    func runLoginCommand(_ command: String) {
        // Account logins carry a quoted path (CODEX_HOME="…"); escape it for
        // the AppleScript string it is embedded in.
        let escaped = command.replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
        let script = "tell application \"Terminal\"\nactivate\ndo script \"\(escaped)\"\nend tell"
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        p.arguments = ["-e", script]
        do { try p.run() } catch { lastError = "เปิด Terminal ไม่ได้: \(error.localizedDescription)" }
    }

    private struct AssembledContext: Codable {
        let ok: Bool?
        let text: String?
        let tokens: Int64?
        let error: String?
    }

    /// Exactly what a turn would hand the agent right now — not a summary of
    /// it, the thing itself. The app claims to carry context between agents;
    /// this is the button that lets someone check.
    func assembledContext(budget: Int64 = 120_000, completion: @escaping (String, Int64) -> Void) {
        guard let h = handle else { return completion("(ยังไม่ได้เปิด session)", 0) }
        queue.async {
            let json = Self.string(from: chorus_assembled_context(h.raw, budget))
            let r = Self.decode(AssembledContext.self, json)
            DispatchQueue.main.async {
                completion(r?.text ?? r?.error ?? "(ประกอบ context ไม่สำเร็จ)", r?.tokens ?? 0)
            }
        }
    }

    /// Fill in missing summary rungs — the warm tier the context assembler
    /// reads. This is the one call that is *allowed* to be slow (it may run a
    /// local model per event), which is exactly why context assembly never
    /// does this work itself.
    ///
    /// It shares `queue` with turns on purpose: the Rust engine handle owns a
    /// SQLite connection that is not safe to touch from two threads, and
    /// `queue` being serial is what keeps that true today. So each pass is
    /// deliberately small — a long session catches up over several turns
    /// instead of making the next turn wait on one big pass.
    func compactIfIdle(limit: Int32 = 40) {
        compactWork?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.compactNow(limit: limit) }
        compactWork = work
        // Wait for the user to go quiet first: a pass holds the engine queue
        // for up to the compaction budget, and the next message would wait on it.
        DispatchQueue.main.asyncAfter(deadline: .now() + 8, execute: work)
    }

    func compactNow(limit: Int32) {
        guard let h = handle, !busy else { return }
        queue.async { [weak self] in
            let json = Self.string(from: chorus_compact(h.raw, limit))
            let r = Self.decode(CompactResult.self, json)
            DispatchQueue.main.async {
                guard let self else { return }
                if let remaining = r?.remaining {
                    self.uncompacted = Int64(remaining)
                }
            }
        }
    }

    /// Set together through `setWorkingDiff` so they never disagree.
    @Published private(set) var workingDiff = ""
    @Published private(set) var diffHunks: [DiffHunk] = []
    /// Files in `diffHunks`, in diff order.
    @Published private(set) var diffFiles: [String] = []

    func setWorkingDiff(_ diff: String, hunks: [DiffHunk]) {
        workingDiff = diff
        diffHunks = hunks
        diffFiles = hunks.map(\.file).reduce(into: [String]()) { if !$0.contains($1) { $0.append($1) } }
    }
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

    /// On `queue`, never the main thread: the handle's SQLite connection may
    /// be in use by a running turn, and a main-thread call both raced it and
    /// froze the window while it waited.
    /// Which project's memory this session shares, for the Memory pane label.
    @Published var memoryScope: String?

    /// Share facts/approve queue with every folder in the project. Queued
    /// right after open, so it lands before the first turn can.
    /// The project's folders, so agents in this session see the others too.
    func setProjectFolders(_ folders: [String]) {
        guard let h = handle,
              let data = try? JSONEncoder().encode(folders),
              let json = String(data: data, encoding: .utf8) else { return }
        queue.async { [weak self] in
            let res = json.withCString { Self.string(from: chorus_set_project_folders(h.raw, $0)) }
            let r = Self.decode(SimpleResult.self, res)
            DispatchQueue.main.async {
                if r?.ok != true { self?.lastError = "ตั้งโฟลเดอร์ของโปรเจกต์ไม่ได้: \(r?.error ?? "unknown")" }
            }
        }
    }

    func setMemoryDB(_ path: String, projectName: String) {
        guard let h = handle else { return }
        queue.async { [weak self] in
            let json = path.withCString { Self.string(from: chorus_set_memory_db(h.raw, $0)) }
            let r = Self.decode(SimpleResult.self, json)
            DispatchQueue.main.async {
                guard let self else { return }
                if r?.ok == true { self.memoryScope = projectName } else {
                    self.memoryScope = nil
                    self.lastError = "ผูกความจำโปรเจกต์ไม่ได้: \(r?.error ?? "unknown")"
                }
                self.refreshMemory()
            }
        }
    }

    func refreshMemory() {
        guard let h = handle else { return }
        queue.async { [weak self] in
            let json = Self.string(from: chorus_memory_state(h.raw))
            let s = Self.decode(MemoryState.self, json)
            DispatchQueue.main.async {
                guard let self, let s else { return }
                self.facts = s.facts ?? []
                self.pending = s.pending ?? []
            }
        }
    }

    func review(_ id: Int64, approve: Bool) {
        guard let h = handle else { return }
        // Optimistic: drop the row now, the store catches up on `queue`.
        pending.removeAll { $0.id == id }
        queue.async { _ = Self.string(from: chorus_memory_review(h.raw, id, approve ? 1 : 0)) }
        refreshMemory()
    }

    // All three below run `git` under the hood (checkpoint.rs) — off the
    // main thread for the same reason as `open`/`refresh`.

    func snapshot() {
        guard let h = handle else { return }
        queue.async { [weak self] in
            _ = "manual checkpoint".withCString { chorus_snapshot(h.raw, $0) }
            DispatchQueue.main.async { self?.refresh() }
        }
    }

    /// Reverse-apply `patch` (the hunks the user rejected).
    func rejectHunks(_ patch: String) {
        guard let h = handle else { return }
        queue.async { [weak self] in
            let r = patch.withCString { Self.string(from: chorus_reject_hunks(h.raw, $0)) }
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
            let r = "".withCString { Self.string(from: chorus_restore(h.raw, $0)) }
            let obj = Self.decode(SimpleResult.self, r)
            DispatchQueue.main.async {
                if obj?.ok != true { self?.lastError = obj?.error ?? "restore failed" }
                self?.refresh()
            }
        }
    }

    // ---- Phase 2-5: Workbench File Tree, Diff, Review, Worktrees, Search ----

    struct FileNode: Codable, Identifiable, Hashable {
        let name: String
        let rel_path: String
        let is_dir: Bool
        let icon: String?
        let size: Int64
        let git_status: String?
        let is_focus: Bool
        let children: [FileNode]
        var id: String { rel_path }
    }

    struct FileTreeResult: Codable {
        let root_path: String
        let total_files: Int
        let nodes: [FileNode]
    }

    private struct FileTreeResponse: Codable {
        let ok: Bool
        let tree: FileTreeResult?
        let error: String?
    }

    func fetchFileTree(cwd: String, focusFile: String? = nil, maxDepth: Int = 6, completion: @escaping (FileTreeResult?) -> Void) {
        guard !cwd.isEmpty else { completion(nil); return }
        DispatchQueue.global(qos: .utility).async {
            let json = cwd.withCString { cPath in
                if let focus = focusFile {
                    return focus.withCString { cFocus in
                        Self.string(from: chorus_file_tree_json(cPath, cFocus, Int32(maxDepth)))
                    }
                } else {
                    return Self.string(from: chorus_file_tree_json(cPath, nil, Int32(maxDepth)))
                }
            }
            let res = Self.decode(FileTreeResponse.self, json)
            DispatchQueue.main.async {
                completion(res?.tree)
            }
        }
    }

    struct DiffLine: Codable, Hashable {
        let line_num: Int
        let text: String
        let kind: String
    }

    struct AlignedLinePair: Codable, Identifiable, Hashable {
        let left: DiffLine?
        let right: DiffLine?
        var id: String { "\(left?.line_num ?? 0)_\(right?.line_num ?? 0)_\(left?.kind ?? "")_\(right?.kind ?? "")" }
    }

    struct AlignedHunk: Codable, Identifiable, Hashable {
        let header: String
        let old_start: Int
        let new_start: Int
        let pairs: [AlignedLinePair]
        var id: String { header }
    }

    struct AlignedFileDiff: Codable, Identifiable, Hashable {
        let path: String
        let status: String
        let additions: Int
        let deletions: Int
        let hunks: [AlignedHunk]
        var id: String { path }
    }

    private struct AlignedDiffResponse: Codable {
        let ok: Bool
        let diff: AlignedFileDiff?
        let error: String?
    }

    func fetchAlignedDiff(oldText: String, newText: String, path: String, completion: @escaping (AlignedFileDiff?) -> Void) {
        queue.async {
            let json = oldText.withCString { cOld in
                newText.withCString { cNew in
                    path.withCString { cPath in
                        Self.string(from: chorus_aligned_diff_json(cOld, cNew, cPath))
                    }
                }
            }
            let res = Self.decode(AlignedDiffResponse.self, json)
            DispatchQueue.main.async {
                completion(res?.diff)
            }
        }
    }

    struct ReviewComment: Codable, Identifiable, Hashable {
        let id: Int64
        let session_id: Int64
        let file_path: String
        let line_number: Int
        let comment_text: String
        let code_snippet: String?
        let created_ms: Int64
        let status: String
    }

    private struct PendingCommentsResponse: Codable {
        let ok: Bool
        let comments: [ReviewComment]?
    }

    private struct SteeringPromptResponse: Codable {
        let ok: Bool
        let prompt: String?
    }

    func addReviewComment(path: String, line: Int, comment: String, snippet: String?) {
        guard let h = handle else { return }
        queue.async {
            _ = path.withCString { cPath in
                comment.withCString { cComment in
                    if let s = snippet {
                        s.withCString { cSnip in
                            Self.string(from: chorus_add_review_comment(h.raw, cPath, Int32(line), cComment, cSnip))
                        }
                    } else {
                        Self.string(from: chorus_add_review_comment(h.raw, cPath, Int32(line), cComment, nil))
                    }
                }
            }
        }
    }

    func fetchPendingReviewComments(completion: @escaping ([ReviewComment]) -> Void) {
        guard let h = handle else { completion([]); return }
        queue.async {
            let json = Self.string(from: chorus_pending_review_comments(h.raw))
            let res = Self.decode(PendingCommentsResponse.self, json)
            DispatchQueue.main.async {
                completion(res?.comments ?? [])
            }
        }
    }

    func clearReviewComments() {
        guard let h = handle else { return }
        queue.async {
            _ = Self.string(from: chorus_clear_review_comments(h.raw))
        }
    }

    func fetchReviewSteeringPrompt(completion: @escaping (String) -> Void) {
        guard let h = handle else { completion(""); return }
        queue.async {
            let json = Self.string(from: chorus_review_steering_prompt(h.raw))
            let res = Self.decode(SteeringPromptResponse.self, json)
            DispatchQueue.main.async {
                completion(res?.prompt ?? "")
            }
        }
    }

    struct WorktreeInfo: Codable, Identifiable, Hashable {
        let id: String
        let branch: String
        let path: String
        let role: String
        let allocated_port: Int
        let status: String
    }

    private struct WorktreesResponse: Codable {
        let ok: Bool
        let worktrees: [WorktreeInfo]?
    }

    func fetchWorktrees(cwd: String, completion: @escaping ([WorktreeInfo]) -> Void) {
        guard !cwd.isEmpty else { completion([]); return }
        queue.async {
            let json = cwd.withCString { cPath in
                Self.string(from: chorus_list_worktrees(cPath))
            }
            let res = Self.decode(WorktreesResponse.self, json)
            DispatchQueue.main.async {
                completion(res?.worktrees ?? [])
            }
        }
    }

    func createWorktree(cwd: String, role: String, completion: @escaping (WorktreeInfo?) -> Void) {
        guard !cwd.isEmpty else { completion(nil); return }
        queue.async {
            let json = cwd.withCString { cCwd in
                role.withCString { cRole in
                    Self.string(from: chorus_create_worktree(cCwd, cRole))
                }
            }
            struct CreateRes: Codable { let ok: Bool; let worktree: WorktreeInfo? }
            let res = Self.decode(CreateRes.self, json)
            DispatchQueue.main.async {
                completion(res?.worktree)
            }
        }
    }

    func mergeWorktree(cwd: String, branch: String, completion: @escaping (Bool, String) -> Void) {
        guard !cwd.isEmpty, !branch.isEmpty else { completion(false, "Invalid args"); return }
        queue.async {
            let json = cwd.withCString { cCwd in
                branch.withCString { cBranch in
                    Self.string(from: chorus_merge_worktree(cCwd, cBranch))
                }
            }
            struct MergeOutcome: Codable {
                let success: Bool
                let fast_forward: Bool
                let has_conflicts: Bool
                let message: String
            }
            struct MergeRes: Codable {
                let ok: Bool
                let outcome: MergeOutcome?
            }
            let res = Self.decode(MergeRes.self, json)
            DispatchQueue.main.async {
                completion(res?.outcome?.success ?? false, res?.outcome?.message ?? "Error")
            }
        }
    }

    func discardWorktree(cwd: String, branch: String, completion: @escaping (Bool) -> Void) {
        guard !cwd.isEmpty, !branch.isEmpty else { completion(false); return }
        queue.async {
            let json = cwd.withCString { cCwd in
                branch.withCString { cBranch in
                    Self.string(from: chorus_remove_worktree(cCwd, cBranch))
                }
            }
            struct DiscardRes: Codable { let ok: Bool }
            let res = Self.decode(DiscardRes.self, json)
            DispatchQueue.main.async {
                completion(res?.ok ?? false)
            }
        }
    }

    func updateSessionCost(db: String, id: Int64) {
        guard id >= 0, !db.isEmpty else {
            self.sessionCostUSD = nil
            return
        }
        ioQueue.async {
            let cost = db.withCString { cDb in
                chorus_session_cost_usd(cDb, id)
            }
            DispatchQueue.main.async {
                self.sessionCostUSD = cost < 0.0 ? nil : cost
            }
        }
    }

    // MARK: - Agent Roles & AI Role Synthesis

    struct AgentRole: Identifiable, Codable, Equatable {
        var id: String { name }
        var name: String
        var description: String
        var system_prompt: String
        var tools: [String]
        var model: String?

        var isBuiltin: Bool {
            let n = name.lowercased()
            return n == "orchestrator" || n == "investigator" || n == "builder" || n == "reviewer"
        }

        var iconName: String {
            switch name.lowercased() {
            case "orchestrator": return "brain.head.profile"
            case "investigator": return "magnifyingglass"
            case "builder": return "hammer.fill"
            case "reviewer": return "checklist.checked"
            case let n where n.contains("database") || n.contains("sql"): return "cylinder.split.1x2"
            case let n where n.contains("security") || n.contains("audit"): return "lock.shield.fill"
            case let n where n.contains("test") || n.contains("spec"): return "testtube.2"
            case let n where n.contains("ui") || n.contains("frontend"): return "paintpalette"
            case let n where n.contains("doc"): return "doc.text"
            case let n where n.contains("bug") || n.contains("fix"): return "ladybug.fill"
            default: return "person.badge.shield.checkmark"
            }
        }
    }

    private struct RolesResponse: Codable {
        let ok: Bool
        let roles: [AgentRole]?
    }

    func fetchRoles(cwd: String, completion: @escaping ([AgentRole]) -> Void) {
        queue.async {
            let json = cwd.withCString { cPath in
                Self.string(from: chorus_list_roles(cPath))
            }
            let res = Self.decode(RolesResponse.self, json)
            DispatchQueue.main.async {
                completion(res?.roles ?? [])
            }
        }
    }

    func saveRole(cwd: String, role: AgentRole, completion: @escaping (Bool, String?) -> Void) {
        guard !cwd.isEmpty else { completion(false, "ต้องเลือกโฟลเดอร์โปรเจกต์ก่อน"); return }
        guard let data = try? JSONEncoder().encode(role),
              let jsonStr = String(data: data, encoding: .utf8) else {
            completion(false, "แปลงข้อมูล Role ไม่สำเร็จ")
            return
        }
        queue.async {
            let json = cwd.withCString { cPath in
                jsonStr.withCString { cJson in
                    Self.string(from: chorus_save_role(cPath, cJson))
                }
            }
            struct Res: Codable { let ok: Bool; let error: String? }
            let res = Self.decode(Res.self, json)
            DispatchQueue.main.async {
                completion(res?.ok == true, res?.error)
            }
        }
    }

    func deleteRole(cwd: String, name: String, completion: @escaping (Bool, String?) -> Void) {
        guard !cwd.isEmpty else { completion(false, "ต้องเลือกโฟลเดอร์โปรเจกต์ก่อน"); return }
        queue.async {
            let json = cwd.withCString { cPath in
                name.withCString { cName in
                    Self.string(from: chorus_delete_role(cPath, cName))
                }
            }
            struct Res: Codable { let ok: Bool; let error: String? }
            let res = Self.decode(Res.self, json)
            DispatchQueue.main.async {
                completion(res?.ok == true, res?.error)
            }
        }
    }

    func synthesizeRole(task: String, writeMode: Bool, nameHint: String?, completion: @escaping (AgentRole?) -> Void) {
        queue.async {
            let json = task.withCString { cTask in
                (nameHint ?? "").withCString { cHint in
                    Self.string(from: chorus_synthesize_role(cTask, writeMode, cHint))
                }
            }
            struct Res: Codable { let ok: Bool; let role: AgentRole? }
            let res = Self.decode(Res.self, json)
            DispatchQueue.main.async {
                completion(res?.role)
            }
        }
    }

    struct SearchHit: Codable, Identifiable, Hashable {
        let session_id: Int64
        let session_title: String
        let seq: Int64
        let actor: String
        let snippet: String
        var id: String { "\(session_id)_\(seq)" }
    }

    private struct SearchResponse: Codable {
        let ok: Bool
        let hits: [SearchHit]?
    }

    func searchSessions(dbPath: String, query: String, completion: @escaping ([SearchHit]) -> Void) {
        guard !dbPath.isEmpty else { completion([]); return }
        queue.async {
            let json = dbPath.withCString { cDb in
                query.withCString { cQ in
                    Self.string(from: chorus_search_sessions_json(cDb, cQ))
                }
            }
            let res = Self.decode(SearchResponse.self, json)
            DispatchQueue.main.async {
                completion(res?.hits ?? [])
            }
        }
    }

    // MARK: - Sessions from other tools (Claude Code, Codex, Gemini CLI)

    struct ExternalSession: Codable, Identifiable {
        let provider: String
        let account: String?
        let id: String
        let path: String
        let cwd: String?
        let title: String
        let updated_ms: Int64
        let size_bytes: UInt64
        let snippet: String?
        /// The agent name the import records replies under (`claude@work`).
        var agentName: String { account.map { "\(provider)@\($0)" } ?? provider }
    }

    private struct ExternalSessionsResponse: Codable {
        let ok: Bool
        let sessions: [ExternalSession]?
        let error: String?
    }

    struct ExternalImportResult: Codable {
        let ok: Bool
        let session_id: Int64?
        let events: Int?
        let reused: Bool?
        let not_appended: Int?
        let error: String?
    }

    /// Scans the other tools' transcript folders — file IO only, so it runs
    /// off the engine queue and never waits behind a turn.
    func externalSessions(query: String, limit: Int32 = 100, completion: @escaping ([ExternalSession], String?) -> Void) {
        ioQueue.async {
            let json = query.withCString { cQ in
                Self.string(from: chorus_external_sessions_json(cQ, limit))
            }
            let res = Self.decode(ExternalSessionsResponse.self, json)
            DispatchQueue.main.async {
                completion(res?.sessions ?? [], res?.ok == true ? nil : (res?.error ?? "อ่านรายการ session ไม่สำเร็จ"))
            }
        }
    }

    /// Imports one into the ledger, then opens it so the next message
    /// continues that conversation here.
    func importExternal(_ s: ExternalSession, db: String, completion: @escaping (ExternalImportResult?) -> Void) {
        queue.async {
            let json = db.withCString { cDb in
                s.provider.withCString { cP in
                    s.path.withCString { cId in
                        Self.string(from: chorus_external_import_json(cDb, cP, cId, nil))
                    }
                }
            }
            let res = Self.decode(ExternalImportResult.self, json)
            DispatchQueue.main.async {
                if res?.ok == true, let id = res?.session_id {
                    self.reopen(db: db, sessionID: id)
                }
                completion(res)
            }
        }
    }

    // MARK: - Super App & Autonomous Harness Engine Extensions

    struct RepoMapResponse: Codable {
        let ok: Bool
        let symbols_count: Int?
        let repo_map: String?
        let error: String?
    }

    /// Generate a high-density Repo Map using Personalized PageRank in Rust.
    func generateRepoMap(workspace: String, maxTokens: UInt32 = 1024, completion: @escaping (String?, Int?, String?) -> Void) {
        ioQueue.async {
            let json = workspace.withCString { cWs in
                Self.string(from: chorus_repo_map_generate_json(cWs, maxTokens))
            }
            let res = Self.decode(RepoMapResponse.self, json)
            DispatchQueue.main.async {
                if res?.ok == true {
                    completion(res?.repo_map, res?.symbols_count, nil)
                } else {
                    completion(nil, nil, res?.error ?? "สร้าง Repo Map ไม่สำเร็จ")
                }
            }
        }
    }

    struct StructuralDiffAnalysis: Codable {
        let total_lines_changed: Int
        let cosmetic_lines: Int
        let semantic_lines: Int
        let is_purely_cosmetic: Bool
    }

    private struct StructuralDiffResponse: Codable {
        let ok: Bool
        let analysis: StructuralDiffAnalysis?
        let error: String?
    }

    /// Analyze structural diff between old and new code to differentiate cosmetic vs semantic edits.
    func analyzeStructuralDiff(oldCode: String, newCode: String, completion: @escaping (StructuralDiffAnalysis?, String?) -> Void) {
        ioQueue.async {
            let json = oldCode.withCString { cOld in
                newCode.withCString { cNew in
                    Self.string(from: chorus_structural_diff_json(cOld, cNew))
                }
            }
            let res = Self.decode(StructuralDiffResponse.self, json)
            DispatchQueue.main.async {
                if res?.ok == true {
                    completion(res?.analysis, nil)
                } else {
                    completion(nil, res?.error ?? "วิเคราะห์ structural diff ไม่สำเร็จ")
                }
            }
        }
    }

    // MARK: - Time-Travel Checkpoint & Selective Rewind (Ticket 10 & 21)

    public struct CheckpointRecord: Identifiable, Codable {
        public let seq: Int64
        public let session_id: Int64
        public let commit_sha: String
        public let message: String
        public let created_ms: Int64

        public var id: Int64 { seq }

        public var shortSha: String {
            String(commit_sha.prefix(7))
        }

        public var formattedDate: String {
            let date = Date(timeIntervalSince1970: Double(created_ms) / 1000.0)
            let formatter = RelativeDateTimeFormatter()
            formatter.unitsStyle = .short
            return formatter.localizedString(for: date, relativeTo: Date())
        }
    }

    private struct CheckpointListResponse: Codable {
        let ok: Bool
        let checkpoints: [CheckpointRecord]?
        let error: String?
    }

    private struct CheckpointRewindResponse: Codable {
        let ok: Bool
        let summary: CheckpointRewindSummary?
        let error: String?
    }

    public struct CheckpointRewindSummary: Codable {
        public let session_id: Int64
        public let target_seq: Int64
        public let mode: String
        public let restored_commit: String?
        public let events_rolled_back: Int
    }

    func listCheckpoints(db: String, sessionID: Int64, completion: @escaping ([CheckpointRecord]) -> Void) {
        guard sessionID >= 0, !db.isEmpty else { return completion([]) }
        ioQueue.async {
            let json = db.withCString { cDb in
                Self.string(from: chorus_checkpoint_list_json(cDb, sessionID))
            }
            let res = Self.decode(CheckpointListResponse.self, json)
            DispatchQueue.main.async {
                completion(res?.checkpoints ?? [])
            }
        }
    }

    /// Timeline buttons. Same rewind as a message's right-click menu: the
    /// chat continues in a fork (nothing is deleted) and files are restored
    /// after a snapshot of the current tree. `ws`/`sessionID` are kept for the
    /// call sites; the open engine handle is the session that is rewound.
    func rewindCheckpoint(db: String, ws: String, sessionID: Int64, targetSeq: Int64, mode: String, completion: @escaping (Bool, String?) -> Void) {
        guard sessionID == self.sessionID, !ws.isEmpty else {
            completion(false, "session หรือโฟลเดอร์ไม่ถูกต้อง")
            return
        }
        rewind(beforeSeq: targetSeq,
               code: mode != "ledger_only",
               conversation: mode != "code_only",
               db: db) { _, error in completion(error == nil, error) }
    }

    private struct RewindResult: Codable {
        let ok: Bool
        let session_id: Int64?
        let text: String?
        let error: String?
    }

    /// Rewind to just before the user's message `seq`; Rust decides what that
    /// means (fork, file restore, backup snapshot). On success the fork is
    /// opened and `completion` gets the message text to offer again.
    func rewind(beforeSeq seq: Int64, code: Bool, conversation: Bool, db: String,
                completion: @escaping (_ text: String?, _ error: String?) -> Void) {
        guard let h = handle, !busy, !switching else {
            completion(nil, "รอให้เทิร์นนี้จบก่อนแล้วค่อยย้อน")
            return
        }
        let current = sessionID
        queue.async { [weak self] in
            let json = Self.string(from: chorus_rewind_before_json(h.raw, seq, code ? 1 : 0, conversation ? 1 : 0))
            let r = Self.decode(RewindResult.self, json)
            DispatchQueue.main.async {
                guard let self else { return }
                guard let r, r.ok, let id = r.session_id else {
                    completion(nil, r?.error ?? "ย้อนกลับไม่สำเร็จ")
                    return
                }
                if code { self.setWorkingDiff("", hunks: []) }
                if id != current { self.reopen(db: db, sessionID: id) } else { self.refresh() }
                completion(r.text, nil)
            }
        }
    }

    // MARK: - Autonomous Verification & Self-Healing Engine (Ticket 03 & 21)

    public struct DiagnosticItem: Codable, Identifiable {
        public let file: String
        public let line: Int
        public let col: Int
        public let severity: String
        public let message: String

        public var id: String { "\(file):\(line):\(col):\(message)" }
        public var tool: String { severity }
        public var column: Int? { col }
        public var rule: String? { nil }
    }

    public typealias VerifyFinding = DiagnosticItem

    public struct VerificationData: Codable {
        public let passed: Bool
        public let checker_used: String
        public let duration_ms: UInt64
        public let diagnostics: [DiagnosticItem]
        public let raw_output: String

        public var tool_used: String { checker_used }
        public var findings: [DiagnosticItem] { diagnostics }
    }

    public typealias VerificationResult = VerificationData

    private struct VerifyWorkspaceResponse: Codable {
        let ok: Bool
        let verification: VerificationData?
        let error: String?
    }

    @Published public var lastVerification: VerificationData? = nil
    @Published public var isVerifyingWorkspace: Bool = false
    public var isVerifying: Bool { isVerifyingWorkspace }

    /// Syntax spans per line from the core (`chorus_highlight`): each span
    /// is `[start, len, kind]` in UTF-8 bytes within its line. Blocking —
    /// call off the main thread.
    nonisolated static func highlightSpans(path: String, text: String) -> [[[Int]]]? {
        struct Res: Decodable { let ok: Bool; let lines: [[[Int]]]? }
        let json = path.withCString { p in text.withCString { t in string(from: chorus_highlight(p, t)) } }
        return decode(Res.self, json)?.lines
    }

    func verifyWorkspace(ws: String, completion: ((VerificationData?, String?) -> Void)? = nil) {
        guard !ws.isEmpty else { return }
        isVerifyingWorkspace = true
        ioQueue.async { [weak self] in
            let json = ws.withCString { cWs in
                Self.string(from: chorus_verify_workspace_json(cWs))
            }
            let res = Self.decode(VerifyWorkspaceResponse.self, json)
            DispatchQueue.main.async { [weak self] in
                self?.isVerifyingWorkspace = false
                if res?.ok == true {
                    self?.lastVerification = res?.verification
                    completion?(res?.verification, nil)
                } else {
                    completion?(nil, res?.error ?? "ตรวจสอบโค้ดไม่สำเร็จ")
                }
            }
        }
    }

    /// Language-server diagnostics for `files` (relative to `ws`), computed by
    /// the Rust core under the same trust and sandbox as `verifyWorkspace`.
    func lspDiagnostics(ws: String, files: [String], completion: @escaping (VerificationData?, String?) -> Void) {
        guard !ws.isEmpty, !files.isEmpty else { completion(nil, "ไม่มีไฟล์ที่เปลี่ยนให้ตรวจ"); return }
        let filesJSON = Self.jsonArray(files)
        ioQueue.async {
            let json = ws.withCString { cWs in filesJSON.withCString { Self.string(from: chorus_lsp_diagnostics_json(cWs, $0)) } }
            let res = Self.decode(VerifyWorkspaceResponse.self, json)
            DispatchQueue.main.async {
                if res?.ok == true { completion(res?.verification, nil) }
                else { completion(nil, res?.error ?? "ตรวจด้วย language server ไม่สำเร็จ") }
            }
        }
    }

    /// "Trust this project": the Rust core runs checks and hooks only when on.
    @Published public var projectTrusted: Bool = false

    private struct TrustResponse: Codable { let ok: Bool; let trusted: Bool? }

    func loadProjectTrust(ws: String) {
        guard !ws.isEmpty else { projectTrusted = false; return }
        let json = ws.withCString { Self.string(from: chorus_project_trusted_json($0)) }
        projectTrusted = Self.decode(TrustResponse.self, json)?.trusted ?? false
    }

    func setProjectTrusted(ws: String, _ trusted: Bool) {
        guard !ws.isEmpty else { return }
        let json = ws.withCString { Self.string(from: chorus_set_project_trusted($0, trusted)) }
        let res = Self.decode(TrustResponse.self, json)
        projectTrusted = res?.ok == true ? (res?.trusted ?? false) : false
        if !projectTrusted { lastVerification = nil }
    }

    func verifyWorkspace(workspace: String, completion: @escaping (VerificationResult?, String?) -> Void) {
        verifyWorkspace(ws: workspace) { data, err in
            completion(data, err)
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

/// The running turn's streamed state, split off `Engine` (see `liveStream`).
final class LiveStream: ObservableObject {
    @Published var turn = Engine.LiveTurn()
}
