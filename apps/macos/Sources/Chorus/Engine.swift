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
    var id: Int64 { seq }
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
    struct Item: Decodable { let name: String }
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
    @Published var lastError: String?
    @Published var sessionID: Int64 = -1
    @Published var contextUsed: Int64?
    @Published var contextWindow: Int64?
    /// goal / done / pending for this session — the context tier the design
    /// never trims. Empty only before the first turn.
    @Published var taskState = ""
    /// Events with no stored summary rungs yet. Non-zero just means the next
    /// compaction pass has work, not that anything is wrong.
    @Published var uncompacted: Int64 = 0

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
    /// For FFI calls that take no engine handle (agent/session/skill lists).
    /// They used to sit on `queue` behind a running turn, so the sidebar and
    /// pickers stayed empty or stale until the agent finished — the main
    /// reason the window felt like it wasn't responding.
    private let ioQueue = DispatchQueue(label: "chorus.io", qos: .userInitiated, attributes: .concurrent)
    private let catalogQueue = DispatchQueue(label: "chorus.catalog", qos: .utility)
    private let routerSetupQueue = DispatchQueue(label: "chorus.routerSetup", qos: .utility)
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
        swap { chorus_engine_open(db, cwd, title) }
    }

    /// Reopen a session that already exists, without starting a new one.
    func reopen(db: String, sessionID: Int64) {
        swap { chorus_engine_reopen(db, sessionID) }
    }

    /// Open the new handle and read its transcript off the main thread, then
    /// publish handle + id + events in one main-thread step. The old thread
    /// stays on screen until then: clearing first is what made every switch
    /// blink through an empty view.
    private func swap(_ make: @escaping @Sendable () -> OpaquePointer?) {
        guard !busy, !switching else { return }
        let old = handle
        // Nothing may enqueue work against the old handle from here on: it
        // is freed on `queue` once the new one is up.
        handle = nil
        switching = true
        lastError = nil
        queue.async { [weak self] in
            let h = make()
            let eventsJSON = h.map { Self.string(from: chorus_transcript_json($0)) } ?? ""
            let attachmentsJSON = h.map { Self.string(from: chorus_list_attachments($0)) } ?? ""
            if let old { chorus_engine_free(old.raw) }
            DispatchQueue.main.async {
                guard let self else { return }
                self.switching = false
                self.memoryScope = nil
                self.live = LiveTurn()
                self.lastThoughts = ""
                guard let h else {
                    self.sessionID = -1
                    self.events = []
                    self.lastError = "เปิดเซสชันไม่ได้ (โฟลเดอร์ยังอยู่ไหม?)"
                    return
                }
                self.handle = Handle(raw: h)
                self.sessionID = chorus_engine_session_id(h)
                self.events = Self.decode([LedgerEvent].self, eventsJSON) ?? []
                self.attachments = Self.decode(SessionAttachmentList.self, attachmentsJSON)?.attachments ?? []
                self.refresh()
            }
        }
    }

    func close() {
        memoryScope = nil
        guard let old = handle else { return }
        handle = nil
        sessionID = -1
        events = []
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

    func deleteSession(db: String, id: Int64, isOpen: Bool) {
        if isOpen { close() }
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

    /// Turns block the engine queue until they finish, so the ledger cannot be
    /// re-read mid-turn. Without this the user's own message vanished into a
    /// blank thread until the answer landed. The synthetic row (negative seq)
    /// is replaced wholesale by the next `refresh()`.
    private var echoCounter: Int64 = 0
    private func echoUser(_ text: String) {
        compactWork?.cancel()
        echoCounter += 1
        live = LiveTurn()
        lastThoughts = ""
        events.append(LedgerEvent(seq: Int64.min + echoCounter, kind: "msg", actor: "user", body: text))
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
    @Published var live = LiveTurn()
    /// The finished turn's reasoning. Agents send it but it is not stored in
    /// the ledger, so it lives here until the next turn starts.
    @Published var lastThoughts = ""

    private var progressRegistered = false
    func registerProgress() {
        guard !progressRegistered else { return }
        progressRegistered = true
        let cb: @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>?, UnsafePointer<CChar>?) -> Void = { ctx, k, t in
            guard let ctx, let k, let t else { return }
            let kind = String(cString: k), text = String(cString: t)
            let engine = Unmanaged<Engine>.fromOpaque(ctx).takeUnretainedValue()
            DispatchQueue.main.async { engine.absorb(kind, text) }
        }
        chorus_set_progress_callback(cb, Unmanaged.passUnretained(self).toOpaque())
    }

    private func absorb(_ kind: String, _ text: String) {
        switch kind {
        case "thought": live.thoughts += text
        case "message": live.message += text
        case "tool": live.tools.append(.init(title: text))
        case "tool_done":
            let parts = text.split(separator: "\t", maxSplits: 1).map(String.init)
            let state = parts.first ?? "completed"
            let title = parts.count > 1 ? parts[1] : ""
            if let i = live.tools.lastIndex(where: { $0.state == "running" && (title.isEmpty || $0.title == title) })
                ?? live.tools.lastIndex(where: { $0.state == "running" }) {
                live.tools[i].state = state
            }
        default: break
        }
    }

    /// Turn finished: keep the reasoning, drop the live view (the ledger now has the rest).
    private func endLive() {
        loadUsage()
        loadAgents()
        if !live.thoughts.isEmpty { lastThoughts = live.thoughts }
        live = LiveTurn()
    }

    /// Stop button. Kills the agent process; the turn thread then returns an
    /// error, which `stopped` turns into a plain note instead of a red banner.
    func cancel() {
        _ = Self.string(from: chorus_cancel())
    }

    /// Copy a selected file into the session immediately. PDF text and image
    /// normalization stay local; the original bytes remain in the session.
    func importFile(_ url: URL, completion: @escaping (DraftAttachment?, String?) -> Void) {
        guard let h = handle else { completion(nil, "เปิดเซสชันก่อนแนบไฟล์"); return }
        queue.async {
            let access = url.startAccessingSecurityScopedResource()
            defer { if access { url.stopAccessingSecurityScopedResource() } }
            do {
                let values = try url.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey])
                guard values.isRegularFile == true else { throw ImportError.message("เลือกได้เฉพาะไฟล์") }
                guard (values.fileSize ?? 0) <= 20 * 1024 * 1024 else {
                    throw ImportError.message("ไฟล์หนึ่งต้องไม่เกิน 20 MB")
                }
                let bytes = try Data(contentsOf: url, options: .mappedIfSafe)
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
        guard let h = handle else { completion(nil, "เปิดเซสชันก่อนแนบรูป"); return }
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
        ioQueue.async {
            let json = taskJSON.withCString { task in
                candidatesJSON.withCString { candidates in
                    routerModel.withCString { model in
                        Self.string(from: chorus_route_json(task, candidates, model))
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
            let json = "qwen2.5:3b".withCString { Self.string(from: chorus_router_models_json($0)) }
            let result = Self.decode(RouterModelsResult.self, json)
            DispatchQueue.main.async {
                self?.routerModels = result?.ok == true ? result?.models?.map(\.name) : nil
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

    /// Run a turn where every permission request pops a dialog.
    func sendInteractive(_ prompt: String, agent: String, attachments: [Int64] = []) {
        guard let h = handle, !busy else { return }
        busy = true; lastError = nil
        echoUser(prompt)
        let ctx = CallbackContext(raw: Unmanaged.passUnretained(self).toOpaque())
        let idsJSON = Self.jsonIDs(attachments)
        let cb: @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<CChar>?) -> Int32 = { ctx, ask in
            guard let ctx, let ask else { return 0 }
            let engine = Unmanaged<Engine>.fromOpaque(ctx).takeUnretainedValue()
            return engine.askPermissionBlocking(String(cString: ask)) ? 1 : 0
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
                self?.busy = false
                self?.endLive()
                if r?.ok != true, !Self.isStop(r?.error) { self?.lastError = r?.error ?? "turn failed" }
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
            let obj = Self.decode(SimpleResult.self, json)
            DispatchQueue.main.async {
                self?.busy = false
                self?.endLive()
                if obj?.ok != true, !Self.isStop(obj?.error) { self?.lastError = obj?.error ?? "relay failed" }
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
        queue.async { [weak self] in
            let json = prompt.withCString { p in
                agents.withCString { a in
                    Self.string(from: chorus_compare(h.raw, a, p, level))
                }
            }
            DispatchQueue.main.async {
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
        queue.async { [weak self] in
            let eventsJSON = Self.string(from: chorus_transcript_json(h.raw))
            let attachmentsJSON = Self.string(from: chorus_list_attachments(h.raw))
            let diffJSON = Self.string(from: chorus_working_diff(h.raw))
            let memJSON = Self.string(from: chorus_memory_state(h.raw))
            let taskJSON = Self.string(from: chorus_task_state(h.raw))
            let warmJSON = Self.string(from: chorus_uncompacted(h.raw))
            DispatchQueue.main.async {
                guard let self else { return }
                if let rows = Self.decode([LedgerEvent].self, eventsJSON) {
                    self.events = rows
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
    }

    private struct AgentsResult: Codable { let ok: Bool?; let agents: [AgentInfo]? }
    private struct SessionsResult: Codable { let ok: Bool?; let sessions: [SessionSummary]? }
    private struct HandoffAutoResult: Codable {
        let ok: Bool?; let to: String?; let goal: String?; let echo_back: String?; let error: String?
    }

    @Published var agents: [AgentInfo] = []
    @Published var sessions: [SessionSummary] = []
    /// Sessions per project database, so the sidebar can list several
    /// projects at once instead of only the open one.
    @Published var sessionsByDB: [String: [SessionSummary]] = [:]

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
        let source: String      // "chorus" | "project" | "user"
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
    }
    struct RateLimits: Codable, Hashable { let primary: UsageWindow?; let secondary: UsageWindow? }
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
        var id: String { name }
        var title: String { account.isEmpty ? "ค่าเริ่มต้น" : account }
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
        ioQueue.async { [weak self] in
            let json = db.withCString { Self.string(from: chorus_list_sessions($0)) }
            let r = Self.decode(SessionsResult.self, json)
            DispatchQueue.main.async {
                self?.sessions = r?.sessions ?? []
                self?.sessionsByDB[db] = r?.sessions ?? []
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
        guard let h = handle else { return completion("(ยังไม่ได้เปิดเซสชัน)", 0) }
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

    /// On `queue`, never the main thread: the handle's SQLite connection may
    /// be in use by a running turn, and a main-thread call both raced it and
    /// froze the window while it waited.
    /// Which project's memory this session shares, for the Memory pane label.
    @Published var memoryScope: String?

    /// Share facts/approve queue with every folder in the project. Queued
    /// right after open, so it lands before the first turn can.
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
