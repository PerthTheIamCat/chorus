import AppKit
import SwiftUI

// The state of the v0.4 window. It owns no logic of its own: every decision
// (what a turn does, what a handoff carries, what the router would pick) is
// Rust's, reached through `Engine`. This only remembers what the person is
// looking at and what they have typed (CLAUDE.md 2.3).

/// How the agent may act. One switch, three values (board 1).
enum ActMode: String, CaseIterable, Identifiable {
    case ask, plan, allow
    var id: String { rawValue }

    var label: String {
        switch self {
        case .ask: "ถามก่อน"
        case .plan: "วางแผนก่อน"
        case .allow: "ให้ทำเลย"
        }
    }

    var symbol: String {
        switch self {
        case .ask: "hand.raised"
        case .plan: "list.bullet.clipboard"
        case .allow: "bolt.fill"
        }
    }

    var detail: String {
        switch self {
        case .ask: "ถามก่อนเขียนไฟล์หรือรันคำสั่ง"
        case .plan: "อ่านอย่างเดียว แล้วตอบเป็นแผนให้อนุมัติ"
        case .allow: "ทำได้ทุกอย่างโดยไม่ถาม ใช้กับโฟลเดอร์ที่ลบทิ้งได้"
        }
    }
}

/// One row of the sidebar: a session inside one project folder.
struct SessionKey: Hashable {
    let folder: String
    let id: Int64
}

enum InspectorTab: String, CaseIterable, Identifiable {
    case context, changes, memory
    var id: String { rawValue }

    var title: String {
        switch self {
        case .context: "คอนเท็กซ์"
        case .changes: "ที่แก้"
        case .memory: "ความจำ"
        }
    }

    var symbol: String {
        switch self {
        case .context: "gauge.with.dots.needle.33percent"
        case .changes: "plus.forwardslash.minus"
        case .memory: "brain"
        }
    }
}

@MainActor
final class ShellModel: ObservableObject {
    let engine: Engine
    let projects = ProjectStore()

    // What is on screen.
    @Published var workspace = ""
    @Published var selection: SessionKey?
    @Published var showInspector = false
    @Published var inspectorTab: InspectorTab = .context
    @Published var query = ""
    @Published var pendingDelete: Engine.SessionSummary?
    @Published var showSettings = false

    // The composer.
    @Published var text = ""
    @Published var hint: String?
    @Published private(set) var attachments: [DraftAttachment] = []
    @Published private(set) var importing = 0
    struct Queued { let send: PendingSend }
    @Published private(set) var queued: [Queued] = []
    @Published var model = ""
    /// Auto route, Orchestra, or the model picked by hand.
    @Published var sessionMode: SessionMode = SessionMode(
        rawValue: UserDefaults.standard.string(forKey: SettingsKey.defaultMode) ?? "auto") ?? .autoRoute
    @Published var routeStatus: RouteStatus?
    @Published var pendingRoute: PendingRoute?
    @Published var reviewModel = ""
    @Published private(set) var routingInProgress = false
    @Published var lastPrompt = ""
    let fileIndex = FileIndex()
    // Compare.
    @Published var comparing = false
    @Published var confirmCompare = false
    @Published var compareJudge = false
    @Published var compareOrchestra = false
    @Published var compareOutput: String?
    // ⌘K.
    @Published var showPalette = false

    struct PendingSend {
        let model: String
        let text: String
        let attachments: [Int64]
        let explicitAgent: Bool
    }
    struct PendingRoute: Identifiable {
        let id = UUID()
        let send: PendingSend
        let reason: String
    }
    struct RouterEnvelope: Decodable {
        struct Route: Decodable {
            struct Result: Decodable {
                let candidate_id: String?
                let reason: String?
                let message: String?
            }
            let status: String
            let result: Result?
        }
        let ok: Bool
        let route: Route?
        let error: String?
    }
    /// The ABI's permission numbers (`ChorusPermission` in chorus.h).
    enum Level { static let ask: Int32 = -1; static let allow: Int32 = 2 }
    /// What "ask" or "allow" was last picked, so a plan runs with it.
    private var lastActing: ActMode = .ask
    @Published var mode: ActMode {
        didSet {
            let d = UserDefaults.standard
            d.set(mode == .plan, forKey: "chorus.planFirst")
            if mode != .plan {
                lastActing = mode
                d.set(Int(mode == .ask ? Level.ask : Level.allow), forKey: "chorus.defaultPermission")
            }
        }
    }

    init(engine: Engine) {
        self.engine = engine
        let d = UserDefaults.standard
        if d.bool(forKey: "chorus.planFirst") {
            mode = .plan
        } else if let raw = d.object(forKey: "chorus.defaultPermission") as? Int,
                  Int32(raw) == Level.allow {
            mode = .allow
        } else {
            mode = .ask
        }
        lastActing = mode == .plan ? .ask : mode
    }

    // MARK: paths

    func dbPath(for folder: String) -> String {
        (folder as NSString).appendingPathComponent(".chorus/store.db")
    }

    var currentDB: String { dbPath(for: workspace) }

    func sessions(in folder: String) -> [Engine.SessionSummary] {
        engine.sessionsByDB[dbPath(for: folder)] ?? []
    }

    /// A title the person set wins; otherwise the first thing they asked.
    func title(of s: Engine.SessionSummary) -> String {
        let t = s.title.trimmingCharacters(in: .whitespaces)
        if !t.isEmpty && t != "session" { return t }
        return s.goal.isEmpty ? "session #\(s.id)" : s.goal
    }

    // MARK: lifecycle

    /// Once, when the window first appears.
    func start() {
        if workspace.isEmpty {
            let recents = UserDefaults.standard.stringArray(forKey: "chorus.recentWorkspaces") ?? []
            if let f = projects.projects.first(where: { !$0.folders.isEmpty })?.folders.first,
               f != NSHomeDirectory() {
                workspace = f
            } else if let r = recents.first, r != NSHomeDirectory() {
                workspace = r
            }
        }
        engine.loadAgents()
        engine.registerProgress()
        engine.loadUsage()
        for p in projects.projects { loadSessions(of: p) }
        guard !workspace.isEmpty else { return }
        engine.loadProjectTrust(ws: workspace)
        engine.loadSkills(cwd: workspace)
        engine.loadSessions(db: currentDB)
        if engine.sessionID < 0 {
            if let first = sessions(in: workspace).first {
                open(SessionKey(folder: workspace, id: first.id))
            } else {
                newSession(in: workspace)
            }
        }
    }

    func loadSessions(of p: ChorusProject) {
        for f in p.folders { engine.loadSessions(db: dbPath(for: f)) }
    }

    func open(_ key: SessionKey) {
        guard !engine.busy, !engine.switching else { return }
        if key.folder != workspace {
            workspace = key.folder
            engine.loadProjectTrust(ws: key.folder)
            engine.loadSkills(cwd: key.folder)
        }
        if key.id != engine.sessionID {
            engine.reopen(db: dbPath(for: key.folder), sessionID: key.id)
        }
    }

    /// A draft, not a row: the session exists once the first message is sent.
    func newSession(in folder: String? = nil) {
        guard !engine.busy, !engine.switching else { return }
        let f = folder ?? workspace
        guard !f.isEmpty else { addProject(); return }
        workspace = f
        let db = dbPath(for: f)
        try? FileManager.default.createDirectory(
            atPath: (db as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
        selection = nil
        engine.openDraft(db: db, cwd: f)
    }

    func addProject() {
        let folders = ProjectStore.pickFolders(message: "เลือกโฟลเดอร์โปรเจกต์ให้ Chorus")
        guard let first = folders.first else { return }
        let p = projects.project(containing: first)
            ?? ChorusProject(name: (first as NSString).lastPathComponent, folders: folders)
        projects.upsert(p)
        loadSessions(of: p)
        newSession(in: first)
    }

    /// The open session changed under us (opened, or a draft just committed).
    func sessionChanged(_ id: Int64) {
        engine.metrics = nil
        guard !workspace.isEmpty else { return }
        engine.refreshMetrics(db: currentDB)
        engine.loadSessions(db: currentDB)
        guard id >= 0 else { return }
        selection = SessionKey(folder: workspace, id: id)
        if let p = projects.project(containing: workspace) {
            engine.setMemoryDB(ProjectStore.memoryPath(p), projectName: p.name)
            engine.setProjectFolders(p.folders)
        }
        engine.updateSessionCost(db: currentDB, id: id)
    }

    /// A turn started or ended.
    func busyChanged(_ busy: Bool) {
        if busy { engine.nextHint = nil }
        if !busy {
            engine.refreshMetrics(db: currentDB)
            if engine.sessionID >= 0 { engine.updateSessionCost(db: currentDB, id: engine.sessionID) }
            flushQueue()
            verifyAfterTurn()
            if UserDefaults.standard.object(forKey: "chorus.suggestNext") as? Bool ?? true {
                engine.loadNextHint(model: UserDefaults.standard.string(forKey: "chorus.routerModel") ?? "")
            }
            if engine.sessionID >= 0 && !NSApp.isActive {
                let reply = engine.events.last(where: { $0.actor != "user" && $0.kind == "msg" })
                let n = TurnNotice.content(lastError: engine.lastError, lastReply: reply?.body, agent: reply?.actor)
                TurnNotice.post(id: "turn-done-\(engine.sessionID)", title: n.title, body: n.body)
            }
        }
    }

    // MARK: models and handoff

    func agent(for modelID: String) -> Engine.AgentInfo? {
        let base = ModelID(modelID).base
        return engine.agents.first { a in
            a.id == modelID || a.id == base || a.models.contains { ModelID($0.id).base == base || $0.id == modelID }
        }
    }

    /// Keep the choice pointed at something that exists, preferring an agent
    /// that can answer a turn right now.
    func syncModel() {
        let known = engine.agents.flatMap { $0.models.map(\.id) }
        guard !known.isEmpty else { return }
        if !known.contains(model) {
            model = engine.agents.first(where: \.ready)?.defaultModel ?? engine.agents[0].defaultModel
        }
        applyModel()
    }

    func applyModel() {
        guard let a = agent(for: model) else { return }
        engine.setModel(agent: a.id, model: model)
    }

    /// The person picked a model. Another provider's model prepares the
    /// handoff in the background (the next message carries it); the same
    /// provider's, or going back to whoever is working, cancels it.
    func choose(_ modelID: String) {
        sessionMode = .manual
        model = modelID
        applyModel()
        guard engine.sessionID >= 0, let last = engine.lastAnsweringAgent,
              let target = agent(for: modelID)?.id, !target.isEmpty else { return }
        if target == last { engine.cancelHandoffPrep() } else { engine.prepareHandoff(to: target) }
    }

    // MARK: sending

    /// `@codex fix the test` sends that turn to Codex without touching the
    /// picker. Returns the model to use and the text with the tag removed.
    func route(_ text: String) -> (model: String, text: String, explicit: Bool) {
        guard text.hasPrefix("@") else { return (model, text, false) }
        let parts = text.dropFirst().split(separator: " ", maxSplits: 1, omittingEmptySubsequences: true)
        guard let tag = parts.first?.lowercased(),
              let a = engine.agents.first(where: {
                  $0.ready && ($0.id.lowercased() == tag || $0.label.lowercased() == tag
                               || ($0.id == "antigravity" && tag == "agy"))
              })
        else { return (model, text, false) }
        return (a.defaultModel, parts.count > 1 ? String(parts[1]) : "", true)
    }

    func send() {
        let raw = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !raw.isEmpty || !attachments.isEmpty else { return }
        guard importing == 0 else { hint = "รอแนบไฟล์ให้เสร็จก่อนส่ง"; return }
        guard attachments.allSatisfy(\.agentReadable) else {
            hint = "มีไฟล์ที่ agent ยังอ่านไม่ได้ ลบไฟล์นั้นก่อนส่ง"
            return
        }
        guard engine.sessionID >= 0 || engine.draft != nil else {
            hint = "เปิดโปรเจกต์ก่อนส่งข้อความ"
            return
        }
        let routed = route(raw)
        if routed.text.isEmpty && attachments.isEmpty {
            hint = "พิมพ์ข้อความต่อจาก \(raw) ด้วย"
            return
        }
        let send = PendingSend(model: routed.model,
                               text: routed.text.isEmpty ? "ช่วยดูไฟล์ที่แนบมา" : routed.text,
                               attachments: attachments.map(\.id),
                               explicitAgent: routed.explicit)
        hint = nil
        text = ""
        attachments = []
        if engine.sessionID < 0 && engine.draft != nil {
            // First message: the draft becomes a session now; pin the model
            // (picked while there was no engine to tell) before sending.
            engine.commitDraft { [weak self] in
                self?.applyModel()
                self?.routeOrDispatch(send)
            }
            return
        }
        if engine.busy || routingInProgress || pendingRoute != nil { queued.append(Queued(send: send)); return }
        routeOrDispatch(send)
    }

    // MARK: attachments

    /// A file or image needs a live session to be stored in; a draft has none.
    func attach(urls: [URL]) {
        guard engine.sessionID >= 0 else { hint = "ส่งข้อความแรกก่อน แล้วค่อยแนบไฟล์"; return }
        for url in urls {
            importing += 1
            engine.importFile(url) { [weak self] item, error in
                guard let self else { return }
                self.importing -= 1
                if let item { self.attachments.append(item) } else { self.hint = "\(url.lastPathComponent): \(error ?? "แนบไม่สำเร็จ")" }
            }
        }
    }

    func attach(image data: Data, mime: String) {
        guard engine.sessionID >= 0 else { hint = "ส่งข้อความแรกก่อน แล้วค่อยแนบรูป"; return }
        importing += 1
        engine.importImage(data, mime: mime) { [weak self] item, error in
            guard let self else { return }
            self.importing -= 1
            if let item { self.attachments.append(item) } else { self.hint = error ?? "แนบรูปไม่สำเร็จ" }
        }
    }

    func removeAttachment(_ id: Int64) {
        attachments.removeAll { $0.id == id }
        engine.deleteAttachment(id)
    }

    func flushQueue() {
        guard !engine.busy, !routingInProgress, pendingRoute == nil, !queued.isEmpty,
              engine.lastError == nil else { return }
        routeOrDispatch(queued.removeFirst().send)
    }

    func stop() {
        engine.cancel()
        queued.removeAll()
    }

    func dismissQueued(at i: Int) {
        guard queued.indices.contains(i) else { return }
        queued.remove(at: i)
    }

    /// The relay chain for a turn as agent names (with accounts), never model
    /// ids: the engine picks the bridge from the name. The chosen agent's
    /// usable accounts first, then every other ready agent's.
    func chain(for agentID: String) -> [String] {
        let d = UserDefaults.standard
        let usable = engine.accounts.filter { $0.ready && !$0.limited }
        func names(_ agent: String) -> [String] {
            let own = usable.filter { $0.agent == agent }.map(\.name)
            return own.isEmpty ? [agent] : own
        }
        let chosenReady = engine.agents.first { $0.id == agentID }?.ready ?? true
        var out = chosenReady ? names(agentID) : []
        if let pick = d.string(forKey: "chorus.preferred.\(agentID)"), let i = out.firstIndex(of: pick) {
            out.insert(out.remove(at: i), at: 0)
        }
        if (d.object(forKey: "chorus.failoverAccounts") as? Bool ?? true) == false { out = Array(out.prefix(1)) }
        if d.object(forKey: "chorus.failoverProviders") as? Bool ?? true {
            for a in engine.agents where a.ready && a.id != agentID { out += names(a.id) }
        }
        return out
    }

    func dispatch(_ target: String, _ prompt: String, attachments: [Int64] = [], agentOverride: String? = nil) {
        lastPrompt = prompt
        let agentID = agentOverride ?? agent(for: target)?.id ?? target
        // `@codex …` and Auto route pick a model other than the picker's.
        if target != model {
            model = target
            engine.setModel(agent: agentID, model: target)
        }
        engine.liveAgent = agentID
        engine.consumeHandoffPrep(for: agentID)
        var names = chain(for: agentID)
        var notice: String?
        if names.first.map({ $0.split(separator: "@").first.map(String.init) ?? $0 }) != agentID {
            let label = agent(for: target)?.label ?? agentID
            if let next = names.first {
                notice = "\(label) ยังไม่ได้ login หรือติดลิมิตทุกบัญชี ส่งให้ \(next) แทน"
            } else {
                notice = "\(label) ยังไม่ได้ login ไปที่ ตั้งค่า แล้วกด Login"
                names = [agentID]
            }
        }
        defer { if let notice { engine.lastError = notice } }
        switch mode {
        case .plan:
            engine.sendPlan(prompt, agents: names.joined(separator: ","), attachments: attachments)
        case .ask:
            engine.sendInteractive(prompt, agent: names.first ?? agentID, attachments: attachments)
        case .allow:
            engine.sendRelay(prompt, agents: names.joined(separator: ","), level: Level.allow, attachments: attachments)
        }
    }

    /// After a plan: carry it out with the permission picked before planning.
    func approvePlan(agents: String) {
        let go = engine.planApprovedPrompt
        guard !go.isEmpty else { return }
        engine.pendingPlanAgents = nil
        if lastActing == .ask {
            engine.sendInteractive(go, agent: agents.split(separator: ",").first.map(String.init) ?? agents)
        } else {
            engine.sendRelay(go, agents: agents, level: Level.allow)
        }
    }

    // MARK: waiting on the person

    func agentLabel(_ name: String) -> String {
        let parts = name.split(separator: "@", maxSplits: 1).map(String.init)
        let id = parts.first ?? name
        let label = engine.agents.first { $0.id == id }?.label ?? id
        return parts.count > 1 ? "\(label) (\(parts[1]))" : label
    }
}
