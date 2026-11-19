import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

// ContentView, split out of ContentView.swift: agent/model/folder helpers and session start.
// Moved verbatim; only `private` was dropped so the pieces can see each other.

extension ContentView {
    func shortAgentLabel(_ a: Engine.AgentInfo) -> String {
        switch a.id.lowercased() {
        case "antigravity", "agy": return "Agy"
        case "claude": return "Claude"
        case "codex": return "Codex"
        default: return a.label
        }
    }

    func agentHelpTooltip(_ a: Engine.AgentInfo, accs: [Engine.Account], limited: Bool, worst: Double?) -> String {
        var desc = a.label
        if accs.count > 1 {
            desc += " (\(accs.count) บัญชี)"
        }
        if limited {
            desc += ": ทุกบัญชีติดลิมิต"
        } else if let worst {
            desc += ": บัญชีที่ว่างที่สุดใช้ไป \(Int(worst * 100))%"
        } else if a.ready {
            desc += ": พร้อมใช้งาน"
        } else {
            desc += ": \(a.detail)"
        }
        return desc
    }

    /// The agent whose model id is currently selected.
    func agentFor(_ modelID: String) -> Engine.AgentInfo? {
        let base = ModelID(modelID).base
        return engine.agents.first { a in
            a.id == modelID || a.id == base || a.models.contains { m in
                m.id == modelID || ModelID(m.id).base == base
            }
        }
    }
    /// Keep the model picker pointed at something that actually exists.
    ///
    /// The picker's tags are *model* ids (`gpt-5.1-codex`), and it was seeded
    /// with an *agent* id (`"codex"`). SwiftUI renders a Picker whose selection
    /// matches no tag as blank — so the app opened with an empty agent box and
    /// no way to tell whether detection had failed or the list was just not
    /// drawn. Seed from the loaded agents instead: a ready one first, because a
    /// picker should not default to something that cannot answer a turn.
    /// Tell the engine which model the picked one is. Without this the
    /// selection is decoration — the agent runs whatever its own CLI defaults
    /// to, and the label lies.
    func applyModel(_ modelID: String) {
        guard let a = agentFor(modelID) else { return }
        engine.setModel(agent: a.id, model: modelID)
    }

    /// Picking another provider's model in the composer prepares the handoff
    /// right away; picking the same provider's other model (Opus → Sonnet) or
    /// going back to the agent that is already working cancels it. There is no
    /// "hand over" button: the next message is the handoff.
    func prepareHandoffIfSwitched(to m: String) {
        guard engine.sessionID >= 0, let last = engine.lastAnsweringAgent,
              let target = agentFor(m)?.id, !target.isEmpty else { return }
        if target == last {
            engine.cancelHandoffPrep()
        } else {
            engine.prepareHandoff(to: target)
        }
    }

    func syncModelSelection() {
        let known = engine.agents.flatMap { $0.models.map(\.id) }
        guard !known.isEmpty else { return }
        if known.contains(model) {
            applyModel(model)
            return
        }
        model = engine.agents.first(where: \.ready)?.defaultModel
            ?? engine.agents[0].defaultModel
        applyModel(model)
    }


    var currentAgentID: String { agentFor(model)?.id ?? "" }

    func pickFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Select"
        panel.message = "เลือกโฟลเดอร์โปรเจกต์ให้ Chorus"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        addRecent(url.path)
        let p = projects.project(containing: url.path)
            ?? ChorusProject(name: url.lastPathComponent, folders: [url.path])
        projects.upsert(p)
        expanded.insert(p.id.uuidString)
        loadSessions(of: p)
    }

    func addRecent(_ path: String) {
        var list = recents.filter { $0 != path }
        list.insert(path, at: 0)
        if list.count > 30 { list = Array(list.prefix(30)) }
        recents = list
        persistRecents()
    }

    func persistRecents() {
        UserDefaults.standard.set(recents, forKey: "chorus.recentWorkspaces")
    }

    func startSession() {
        let db = (workspace as NSString).appendingPathComponent(".chorus/store.db")
        try? FileManager.default.createDirectory(
            atPath: (db as NSString).deletingLastPathComponent,
            withIntermediateDirectories: true)
        // A draft, not a row: the session exists once the first message is sent.
        engine.openDraft(db: db, cwd: workspace)
    }

    var dbPath: String {
        (workspace as NSString).appendingPathComponent(".chorus/store.db")
    }
}
