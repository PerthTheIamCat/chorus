import AppKit
import SwiftUI

// Everything between "the person pressed send" and "an agent starts": Auto
// route, Orchestra, the confirm-the-route sheet, compare, and the command
// palette entries. The decisions themselves are Rust's (`proposeRoute`,
// `sendOrchestra`, `compare`); this only gathers the inputs and shows the
// answer (CLAUDE.md 2.3).

extension ShellModel {
    private var defaults: UserDefaults { .standard }

    // MARK: Auto route and Orchestra

    func routeOrDispatch(_ send: PendingSend) {
        if sessionMode == .orchestra && !send.explicitAgent {
            // Rust picks the lead and its backup, and runs it read-only.
            lastPrompt = send.text
            let level: Int32 = mode == .allow ? Level.allow : Level.ask
            engine.sendOrchestra(
                send.text,
                primary: defaults.string(forKey: SettingsKey.orchestratorModel) ?? "",
                backup: defaults.string(forKey: SettingsKey.orchestratorBackup) ?? "",
                concurrency: defaults.object(forKey: SettingsKey.orchestraConcurrency) as? Int ?? 3,
                maxChildren: defaults.object(forKey: SettingsKey.orchestraMaxChildren) as? Int ?? 10,
                maxMinutes: defaults.object(forKey: SettingsKey.orchestraMaxMinutes) as? Int ?? 30,
                level: level,
                attachments: send.attachments)
            return
        }
        let autoRoute = defaults.object(forKey: "chorus.autoRoute") as? Bool ?? true
        guard sessionMode == .autoRoute, autoRoute, !send.explicitAgent else {
            engine.recordLearnedRoute(
                workspace: workspace, prompt: send.text, model: send.model,
                agent: agent(for: send.model)?.id ?? "agent",
                reason: send.explicitAgent ? "ผู้ใช้ระบุ agent เจาะจง" : "ผู้ใช้เลือกโมเดลเองโดยตรง")
            dispatch(send.model, send.text, attachments: send.attachments)
            return
        }
        autoRouteAndDispatch(send)
    }

    private func autoRouteAndDispatch(_ send: PendingSend) {
        let routerModel = defaults.string(forKey: "chorus.routerModel") ?? "chorus-openthai-systemone:0.8b"
        let integrations = engine.catalog.filter(\.isEnabledForProject).map(\.id)
        let candidates = engine.agents.filter(\.ready).flatMap { agent in
            agent.models.map { choice -> [String: Any] in
                ["id": "\(agent.id):\(choice.id)", "agent": agent.id,
                 "model": ModelID(choice.id).base,
                 "effort": ModelID(choice.id).effort ?? "default",
                 "ready": true, "media_types": ["text/*", "image/*"],
                 "integrations": integrations]
            }
        }
        let task: [String: Any] = [
            "text": send.text,
            "attachments": [] as [Any],
            "enabled_integrations": integrations, "required_integrations": [] as [Any],
        ]
        guard let taskData = try? JSONSerialization.data(withJSONObject: task),
              let candidatesData = try? JSONSerialization.data(withJSONObject: ["candidates": candidates]),
              let taskJSON = String(data: taskData, encoding: .utf8),
              let candidatesJSON = String(data: candidatesData, encoding: .utf8),
              !routerModel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            reviewModel = send.model
            pendingRoute = PendingRoute(send: send, reason: "ตั้งค่าโมเดล router ในตั้งค่าก่อน หรือเลือกโมเดลเอง")
            return
        }
        let conserve = PowerManager.shared.shouldConserveEnergyForInference
        let effective = conserve ? "in-process-heuristic" : routerModel
        routingInProgress = true
        routeStatus = .thinking(routerModel: effective, candidates: candidates.count)
        let session = engine.sessionID
        engine.proposeRoute(taskJSON: taskJSON, candidatesJSON: candidatesJSON, routerModel: effective) { [weak self] json in
            guard let self else { return }
            self.routingInProgress = false
            guard session == self.engine.sessionID else { self.routeStatus = nil; return }
            let response = try? JSONDecoder().decode(RouterEnvelope.self, from: Data(json.utf8))
            if response?.ok == true, response?.route?.status == "proposal",
               let chosenID = response?.route?.result?.candidate_id,
               let chosen = self.engine.agents.flatMap({ a in
                   a.models.map { c in ("\(a.id):\(c.id)", c.id, a.id) }
               }).first(where: { $0.0 == chosenID }) {
                var reason = response?.route?.result?.reason ?? "เหมาะกับงานนี้"
                if conserve { reason += " [โหมดประหยัดแบตเตอรี่]" }
                let id = ModelID(chosen.1)
                self.routeStatus = .chosen(
                    agent: self.engine.agents.first { $0.id == chosen.2 }?.label ?? chosen.2,
                    model: id.base, effort: id.effort ?? "ค่าเริ่มต้น",
                    preset: self.engine.routePresetLabel, reason: reason)
                // The router's own pick is not learned: only overrides are.
                self.dispatch(chosen.1, send.text, attachments: send.attachments, agentOverride: chosen.2)
            } else {
                self.routeStatus = nil
                self.reviewModel = send.model
                self.pendingRoute = PendingRoute(send: send,
                    reason: response?.route?.result?.message ?? response?.error
                        ?? "Router ในเครื่องยังเลือกไม่ได้ เลือกโมเดลแล้วยืนยันส่งเอง")
            }
        }
    }

    func sendReviewedRoute() {
        guard let p = pendingRoute else { return }
        pendingRoute = nil
        routeStatus = nil
        engine.recordLearnedRoute(
            workspace: workspace, prompt: p.send.text, model: reviewModel,
            agent: agent(for: reviewModel)?.id ?? "agent", reason: "ผู้ใช้ยืนยันและเลือกโมเดลนี้เอง")
        dispatch(reviewModel, p.send.text, attachments: p.send.attachments)
    }

    func cancelReviewedRoute() {
        guard let p = pendingRoute else { return }
        routeStatus = nil
        text = p.send.text + (text.isEmpty ? "" : "\n" + text)
        pendingRoute = nil
        flushQueue()
    }

    // MARK: compare

    var canCompare: Bool { engine.sessionID >= 0 || engine.draft != nil }

    /// What a compare would ask: what is typed, else the last prompt.
    var comparePrompt: (text: String, fromComposer: Bool) {
        let typed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if !typed.isEmpty { return (typed, true) }
        return (lastPrompt.isEmpty ? "ทบทวนงานที่ค้างอยู่แล้วเสนอขั้นต่อไป" : lastPrompt, false)
    }

    /// One prompt to every ready agent, each in its own fork and at its own
    /// cost, so ask first.
    func runCompare() {
        guard engine.agents.filter(\.ready).count > 1, canCompare, !comparing else { return }
        confirmCompare = true
    }

    func performCompare() {
        let ready = engine.agents.filter(\.ready).map(\.id)
        guard ready.count > 1, canCompare, !comparing else { return }
        let source = comparePrompt
        if source.fromComposer { lastPrompt = source.text; text = "" }
        var lanes = ready
        if compareOrchestra, let first = ready.first { lanes.append(first + "+orchestra") }
        let options: String? = compareJudge ? "{\"judge\":true}" : nil
        let level: Int32 = mode == .allow ? Level.allow : 0
        let start = { [weak self] in
            guard let self else { return }
            self.comparing = true
            self.engine.compare(source.text, agents: lanes.joined(separator: ","), level: level, options: options) {
                self.comparing = false
                self.compareOutput = $0
            }
        }
        if engine.sessionID < 0 && engine.draft != nil {
            engine.commitDraft { [weak self] in self?.applyModel(); start() }
        } else {
            start()
        }
    }

    // MARK: after a turn

    /// Runs the project's own code, so only for a trusted project.
    func verifyAfterTurn() {
        if !workspace.isEmpty && engine.projectTrusted {
            engine.verifyWorkspace(ws: workspace)
        }
    }

    // MARK: ⌘K

    struct PaletteItem: Identifiable {
        let id = UUID()
        let title: String
        let icon: String
        let run: () -> Void
    }

    func paletteItems(openTool: @escaping (ShellTool) -> Void) -> [PaletteItem] {
        var items: [PaletteItem] = []
        for a in engine.agents where a.ready {
            items.append(.init(title: "สลับไปใช้ \(a.label)", icon: "person.crop.circle") { self.choose(a.defaultModel) })
        }
        items.append(.init(title: "session ใหม่", icon: "square.and.pencil") { self.newSession() })
        items.append(.init(title: "เทียบ agent", icon: "square.split.2x1") { self.runCompare() })
        if engine.sessionID >= 0 {
            items.append(.init(title: "Snapshot ตอนนี้", icon: "camera") { self.engine.snapshot() })
        }
        items.append(.init(title: "โต๊ะงาน · ที่แก้ไป", icon: "plus.forwardslash.minus") { self.stage = .workbench; self.workbenchTab = .changes })
        items.append(.init(title: "โต๊ะงาน · ไฟล์", icon: "folder") { self.stage = .workbench; self.workbenchTab = .files })
        items.append(.init(title: "โต๊ะงาน · พรีวิว", icon: "safari") { self.stage = .workbench; self.workbenchTab = .preview })
        items.append(.init(title: "Orchestra", icon: "point.3.connected.trianglepath.dotted") { self.stage = .orchestra })
        items.append(.init(title: "ส่งไม้…", icon: "arrow.left.arrow.right") { self.showHandoff = true })
        items.append(.init(title: "ย้อนเวลา (checkpoint)", icon: "clock.arrow.circlepath") {
            self.inspectorTab = .rewind; self.showInspector = true
        })
        for t in [ShellTool.ship, .monitor] {
            items.append(.init(title: t.title, icon: t.symbol) { openTool(t) })
        }
        items.append(.init(title: "ค้นทุก session", icon: "magnifyingglass") { self.showSearch = true })
        items.append(.init(title: "ตั้งค่า", icon: "gearshape") { ShellRoot.openSettings() })
        return items
    }

    // MARK: `@` and `/` completion

    /// What the popup above the field offers for the word being typed.
    func suggestions(for token: (trigger: Character, query: String)?) -> [Suggestion] {
        guard let token else { return [] }
        let q = token.query.lowercased()
        func hit(_ t: String) -> Bool { q.isEmpty || t.lowercased().contains(q) }
        if token.trigger == "@" {
            var out: [Suggestion] = engine.agents.filter { $0.ready && hit($0.id) }.map { a in
                Suggestion(title: "@" + a.id, detail: "ส่งข้อความนี้ให้ \(a.label)", icon: "person.crop.circle",
                           insert: "@\(a.id) ")
            }
            if engine.agents.contains(where: { $0.ready && $0.id == "antigravity" }) && hit("agy") {
                out.append(Suggestion(title: "@agy", detail: "ส่งข้อความนี้ให้ Antigravity",
                                      icon: "person.crop.circle", insert: "@agy "))
            }
            out += fileIndex.match(token.query, limit: 8 - min(out.count, 3)).map { f in
                Suggestion(title: (f as NSString).lastPathComponent, detail: f, icon: "doc.text", insert: "@\(f) ")
            }
            return out
        }
        var out: [Suggestion] = []
        let current = agent(for: model)?.id ?? ""
        for a in engine.agents where a.ready && a.id != current && engine.sessionID >= 0 {
            out.append(.init(title: "/handoff \(a.id)", detail: "ส่งงานต่อให้ \(a.label) พร้อม brief",
                             icon: "arrowshape.turn.up.right.fill") {
                self.engine.handoffAuto(to: a.id, level: self.mode == .allow ? Level.allow : 0)
            })
        }
        for a in engine.agents where a.ready {
            for m in a.baseModels {
                out.append(.init(title: "/model \(m.id)", detail: "\(a.label) · \(m.label)", icon: "cpu") {
                    let efforts = a.efforts(for: m.id)
                    self.choose(ModelID.compose(m.id, efforts.contains("high") ? "high" : efforts.first))
                })
            }
        }
        if let a = agent(for: model) {
            for e in a.efforts(for: ModelID(model).base) {
                out.append(.init(title: "/effort \(e)", detail: "ระดับการคิด", icon: "brain") {
                    self.choose(ModelID.compose(ModelID(self.model).base, e))
                })
            }
        }
        out.append(.init(title: "/new", detail: "session ใหม่ในโฟลเดอร์นี้", icon: "square.and.pencil") { self.newSession() })
        out.append(.init(title: "/compare", detail: "ถามทุก agent แล้วเทียบ", icon: "square.split.2x1") { self.runCompare() })
        out.append(.init(title: "/snapshot", detail: "บันทึกจุดย้อนกลับตอนนี้", icon: "camera") { self.engine.snapshot() })
        out.append(.init(title: "/ask", detail: "ถามก่อนเขียนไฟล์", icon: "hand.raised") { self.mode = .ask })
        out.append(.init(title: "/auto", detail: "ให้ทำเลยไม่ต้องถาม", icon: "bolt.fill") { self.mode = .allow })
        out.append(.init(title: "/plan", detail: "วางแผนก่อนลงมือ (อ่านอย่างเดียว)", icon: "list.bullet.clipboard") { self.mode = .plan })
        out.append(.init(title: "/settings", detail: "ตั้งค่าและ skills", icon: "gearshape") { ShellRoot.openSettings() })
        // Skills go in as text: Claude loads them by name, the others are told
        // where they live by the projected AGENTS.md index.
        for sk in engine.skills {
            out.append(.init(title: "/" + sk.name, detail: sk.description, icon: "sparkles", insert: "/\(sk.name) "))
        }
        return Array(out.filter { hit($0.title) || hit($0.detail) }.prefix(9))
    }

    /// The `@word` or `/word` at the end of the text, if that is what is being typed.
    static func completionToken(in text: String) -> (trigger: Character, query: String)? {
        guard let lastWord = text.split(separator: " ", omittingEmptySubsequences: false).last,
              let first = lastWord.first, first == "@" || first == "/" else { return nil }
        if first == "/" && text.count != lastWord.count { return nil } // slash commands start the message
        return (first, String(lastWord.dropFirst()))
    }
}
