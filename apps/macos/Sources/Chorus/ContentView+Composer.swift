import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

// ContentView, split out of ContentView.swift: composer, send pipeline and queue.
// Moved verbatim; only `private` was dropped so the pieces can see each other.

extension ContentView {
    @ViewBuilder
    var composer: some View {
        let currentSub = engine.subagentsByDB[dbPath(for: workspace)]?.first { $0.child_sess == engine.sessionID }
        if currentSub?.kind == "provider" {
            HStack(spacing: 12) {
                Image(systemName: "eye.fill")
                    .foregroundStyle(.secondary)
                    .font(.system(size: 14))
                VStack(alignment: .leading, spacing: 2) {
                    Text("Provider subagent (อ่านอย่างเดียว)")
                        .font(.caption.weight(.semibold))
                    Text("การสนทนาของ subagent ฝั่ง Provider บันทึกเพื่อให้อ่านย้อนหลัง ไม่สามารถส่งข้อความโดยตรงได้")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                if engine.busy {
                    Button(role: .destructive) {
                        engine.cancel()
                        queued.removeAll()
                    } label: {
                        Label("หยุดเทิร์นของ agent หลัก", systemImage: "stop.circle.fill")
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
                }
            }
            .padding(12)
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).stroke(Color.primary.opacity(0.1), lineWidth: 1))
            .shadow(color: .black.opacity(0.06), radius: 8, y: 3)
        } else {
            VStack(spacing: 6) {
                if !queued.isEmpty {
                VStack(alignment: .leading, spacing: 3) {
                    ForEach(Array(queued.enumerated()), id: \.offset) { i, q in
                        HStack(spacing: 6) {
                            Image(systemName: "clock").font(.caption2)
                            Text(q.text).font(.caption).lineLimit(1)
                            Spacer()
                            Button { queued.remove(at: i) } label: { Image(systemName: "xmark.circle.fill") }
                                .buttonStyle(.chorusPlain).foregroundStyle(.secondary)
                        }
                    }
                    Text("รอคิว — จะส่งต่อเมื่อเทิร์นนี้จบ").font(.caption2).foregroundStyle(.tertiary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            if let routeStatus {
                RouteStatusCard(status: routeStatus) { self.routeStatus = nil }
                    .transition(.opacity)
            }
            if let hint { Text(hint).font(.caption).foregroundStyle(.orange).frame(maxWidth: .infinity, alignment: .leading) }
            if let planAgents = engine.pendingPlanAgents, !engine.busy {
                planApprovalBar(agents: planAgents)
            }
            HandoffPrepStrip(prep: engine.handoffPrep) { id in
                engine.agents.first { $0.id == id }?.label ?? id
            }
            ComposerField(box: draftBox, suggestions: suggest, onSubmit: submit,
                          onImportURLs: importFiles,
                          onPasteImage: { data, mime in importPastedImage(data, mime: mime) },
                          onRemoveAttachment: { id in
                              draftBox.attachments.removeAll { $0.id == id }
                              AttachmentThumbnails.shared.forget(id)
                              engine.deleteAttachment(id)
                          },
                          focusToken: composerFocus,
                          isBusy: engine.busy,
                          canSend: engine.sessionID >= 0 || engine.draft != nil,
                          onStop: { engine.cancel(); queued.removeAll() },
                          routeNoteEnabled: currentSessionMode == .autoRoute,
                          ghost: suggestNext ? engine.nextHint : nil) {
                ComposerModeMenu(mode: Binding(
                    get: { currentSessionMode },
                    set: { currentSessionMode = $0 }
                )) {
                    settingsSection = .orchestration
                    showSettings = true
                }
                if currentSessionMode == .autoRoute, !engine.routePresets.isEmpty {
                    ChipPicker(
                        icon: "gauge.with.dots.needle.50percent",
                        value: engine.routePreset,
                        options: engine.routePresets.map(\.id),
                        labels: Dictionary(uniqueKeysWithValues: engine.routePresets.map { ($0.id, $0.label) }),
                        selected: engine.routePreset
                    ) { engine.setRoutePreset($0) }
                    .help(engine.routePresets.first { $0.id == engine.routePreset }.map { "\($0.label) — \($0.hint)" } ?? "")
                }
                // One switch for how the agent may act: ask first, plan first
                // (read-only, then carry the plan out with the last ask/allow
                // pick), or just do it. Orchestra plans by itself, so it only
                // offers the two.
                let modes = currentSessionMode == .orchestra ? ["ask", "allow"] : ["ask", "plan", "allow"]
                let modeLabels = ["ask": PermissionLevel.ask.label, "plan": "วางแผนก่อน", "allow": PermissionLevel.allowAll.label]
                let modeValue = planFirst && currentSessionMode != .orchestra ? "plan" : (permission == .ask ? "ask" : "allow")
                ChipPicker(
                    icon: modeValue == "plan" ? "list.bullet.clipboard" : (modeValue == "ask" ? "hand.raised" : "bolt.fill"),
                    value: modeValue,
                    options: modes,
                    labels: modeLabels,
                    selected: modeValue
                ) { v in
                    switch v {
                    case "plan": planFirst = true
                    case "ask": planFirst = false; permission = .ask
                    default: planFirst = false; permission = .allowAll
                    }
                }
                .help(modeValue == "plan"
                      ? "วางแผนก่อน — agent อ่านอย่างเดียวแล้วตอบเป็นแผน กด ทำตามแผน แล้วลงมือด้วยสิทธิ์ \(permission.label)"
                      : "\(permission.label) — \(permission.hint)")
            } trailing: {
                if currentSessionMode == .orchestra {
                    let orchLabel = configuredOrchestrator.isEmpty ? "Claude Opus 5.5" : ModelID(configuredOrchestrator).base
                    Button {
                        settingsSection = .orchestration
                        showSettings = true
                    } label: {
                        CompactLabel(text: "นำโดย \(orchLabel)") {
                            Image(systemName: "crown.fill")
                                .font(.system(size: 9))
                                .foregroundStyle(.yellow)
                        }
                        .font(.caption.weight(.medium))
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 7, padding: .init(top: 4, leading: 8, bottom: 4, trailing: 8)))
                    .help("นำโดย \(orchLabel) — Orchestra: วางแผนและแจกงานให้ agent ย่อย (คลิกเพื่อตั้งค่าโมเดล)")
                } else if currentSessionMode == .autoRoute {
                    Button { currentSessionMode = .manual } label: {
                        CompactLabel(text: "router เลือก", spacing: 3) {
                            Image(systemName: "arrow.triangle.branch")
                        }
                        .font(.caption.weight(.medium))
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 7, padding: .init(top: 4, leading: 8, bottom: 4, trailing: 8)))
                    .help("Auto route: router เสนอเอเจนต์และโมเดลตอนส่ง — คลิกเพื่อเลือกโมเดลเอง")
                } else {
                    ModelPicker(model: $model, onNeedsLogin: { settingsSection = .agents; showSettings = true })
                }
            }
        }
        .padding(.horizontal, 14).padding(.top, 12).padding(.bottom, 10)
        .floatingPanel(cornerRadius: 14)
        }
    }

    /// `@codex fix this` → ("codex's default model", "fix this"). Matches agent
    /// id or label, case-insensitively; anything else is sent untouched.
    func route(_ text: String) -> (model: String, text: String) {
        guard text.hasPrefix("@") else { return (model, text) }
        let parts = text.dropFirst().split(separator: " ", maxSplits: 1, omittingEmptySubsequences: true)
        guard let tag = parts.first?.lowercased(),
              let a = engine.agents.first(where: {
                  $0.ready && ($0.id.lowercased() == tag
                               || $0.label.lowercased() == tag
                               || ($0.id == "antigravity" && tag == "agy"))
              })
        else { return (model, text) }
        return (a.defaultModel, parts.count > 1 ? String(parts[1]) : "")
    }

    func ensureSessionReady() -> Bool {
        if engine.sessionID >= 0 || engine.draft != nil { return true }
        if workspace.isEmpty {
            if let p = projects.projects.first(where: { !$0.folders.isEmpty }),
               let f = p.folders.first {
                workspace = f
            }
        }
        if !workspace.isEmpty && !engine.busy {
            startSession()
            return true
        }
        return false
    }

    /// Menu bar → window. The menu (`ChorusCommands`) owns every shortcut;
    /// this only maps a command onto the same state the toolbar buttons flip.
    func run(_ command: AppCommand) {
        let spring = Animation.spring(response: 0.28, dampingFraction: 0.85)
        switch command {
        case .newSession:
            if workspace.isEmpty || engine.busy {
                editing = ChorusProject(name: "", folders: []); editingIsNew = true
            } else { startSession() }
        case .settings: showSettings = true
        case .commandPalette: showPalette = true
        case .searchSessions: showSearchSheet = true
        case .toggleSidebar: showSidebar.toggle()
        case .toggleExplorer: showExplorer.toggle()
        case .toggleInspector: showInspector.toggle()
        case .inspectorTab(let i):
            guard RightTab.allCases.indices.contains(i) else { return }
            rightTab = RightTab.allCases[i]
            showInspector = true
        case .toggleOrchestrator:
            settingsSection = .orchestration
            showSettings = true
        case .toggleLivePreview:
            withAnimation(spring) { centerStageMode = centerStageMode == .livePreview ? .thread : .livePreview }
        case .toggleTerminal:
            withAnimation(spring) { showTerminalDrawer.toggle() }
        case .toggleFocusMode:
            // One meaning: leave whatever panel is maximized, else focus the thread.
            withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                maximizedPanel = maximizedPanel == .none ? .thread : .none
            }
        case .send:
            // A sheet (project editor, settings) owns ⌘↩ while it is up.
            guard NSApp.mainWindow?.attachedSheet == nil else { return }
            submit()
        case .stop:
            guard engine.busy else { return }
            engine.cancel(); queued.removeAll()
        case .compareAgents:
            runCompare()
        case .chooseModel, .nextEffort, .sendReview, .openInEditor:
            break // handled by ModelPicker / DiffReviewerView / CodeViewerView
        case .systemMonitor:
            if !showSidebar || maximizedPanel != .none {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.85)) {
                    maximizedPanel = .none
                    showSidebar = true
                }
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) {
                    showSystemMonitorPopover.toggle()
                }
            } else {
                showSystemMonitorPopover.toggle()
            }
        }
    }

    /// "claude@work" → "Claude (work)"; falls back to the raw name.
    func askingAgentLabel(_ name: String) -> String {
        let parts = name.split(separator: "@", maxSplits: 1).map(String.init)
        let id = parts.first ?? name
        let label = engine.agents.first { $0.id == id }?.label ?? id
        return parts.count > 1 ? "\(label) (\(parts[1]))" : label
    }

    func syncCommandState() {
        let c = AppCommandCenter.shared
        c.hasSession = engine.sessionID >= 0
        c.canCompare = canCompare
        c.busy = engine.busy
        c.fileOpen = selectedFile != nil
        c.reviewVisible = (showInspector && rightTab == .changes) || maximizedPanel == .diffReviewer
        c.focusMode = maximizedPanel != .none
    }

    func importFiles(_ urls: [URL]) {
        // Attachments are stored per session, so attaching ends the draft.
        if engine.sessionID < 0 && engine.draft != nil {
            engine.commitDraft { applyModel(model); importFiles(urls) }
            return
        }
        guard ensureSessionReady() && engine.sessionID >= 0 else {
            hint = workspace.isEmpty ? "สร้างหรือเลือกโปรเจกต์ก่อนแนบไฟล์" : "เปิด session ก่อนแนบไฟล์"
            return
        }
        let room = max(0, 5 - draftBox.attachments.count - importingFiles)
        guard urls.count <= room else { hint = "แนบได้สูงสุด 5 ไฟล์ต่อข้อความ"; return }
        importingFiles += urls.count
        for url in urls {
            engine.importFile(url) { attachment, error in
                importingFiles -= 1
                if let attachment {
                    draftBox.attachments.append(attachment)
                    AttachmentThumbnails.shared.load(id: attachment.id, url: url)
                } else if let error {
                    let size = (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize).map(Int64.init)
                    draftBox.rejected.append(RejectedAttachment(name: url.lastPathComponent, size: size, reason: error))
                }
            }
        }
    }

    func importPastedImage(_ data: Data, mime: String) {
        if engine.sessionID < 0 && engine.draft != nil {
            engine.commitDraft { applyModel(model); importPastedImage(data, mime: mime) }
            return
        }
        guard ensureSessionReady() && engine.sessionID >= 0 else {
            hint = workspace.isEmpty ? "สร้างหรือเลือกโปรเจกต์ก่อนแนบรูป" : "เปิด session ก่อนแนบรูป"
            return
        }
        guard draftBox.attachments.count + importingFiles < 5 else { hint = "แนบได้สูงสุด 5 ไฟล์ต่อข้อความ"; return }
        importingFiles += 1
        engine.importImage(data, mime: mime) { attachment, error in
            importingFiles -= 1
            if let attachment {
                draftBox.attachments.append(attachment)
                AttachmentThumbnails.shared.load(id: attachment.id, data: data)
            }
            if let error { hint = error }
        }
    }

    func importDroppedProviders(_ providers: [NSItemProvider]) -> Bool {
        var handled = false
        for provider in providers {
            if provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier)
                || provider.hasItemConformingToTypeIdentifier(UTType.url.identifier) {
                handled = true
                let type = provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier)
                    ? UTType.fileURL.identifier : UTType.url.identifier
                provider.loadItem(forTypeIdentifier: type, options: nil) { item, _ in
                    let url = (item as? URL) ?? (item as? NSURL).map { $0 as URL } ?? (item as? Data).flatMap { URL(dataRepresentation: $0, relativeTo: nil) }
                    if let url { DispatchQueue.main.async { self.importFiles([url]) } }
                }
            } else if let imageType = provider.registeredTypeIdentifiers.first(where: { UTType($0)?.conforms(to: .image) == true }) {
                handled = true
                provider.loadDataRepresentation(forTypeIdentifier: imageType) { data, _ in
                    guard let data else { return }
                    let mime = UTType(imageType)?.preferredMIMEType ?? "image/png"
                    DispatchQueue.main.async { self.importPastedImage(data, mime: mime) }
                }
            }
        }
        return handled
    }

    func submit() {
        let raw = draftBox.text.trimmingCharacters(in: .whitespacesAndNewlines)
        let (target, text) = route(raw)
        guard engine.sessionID >= 0 || engine.draft != nil,
              !raw.isEmpty || !draftBox.attachments.isEmpty else { return }
        guard importingFiles == 0 else { hint = "รอแนบไฟล์ให้เสร็จก่อนส่ง"; return }
        if text.isEmpty && draftBox.attachments.isEmpty {
            hint = "พิมพ์ข้อความต่อจาก \(raw) ด้วย"
            return
        }
        guard draftBox.attachments.allSatisfy(\.agentReadable) else {
            hint = "มีไฟล์ที่ agent ยังอ่านไม่ได้ กรุณาลบไฟล์นั้นก่อนส่ง"
            return
        }
        guard draftBox.attachments.reduce(Int64(0), { $0 + $1.size }) <= 50 * 1024 * 1024 else {
            hint = "ไฟล์แนบรวมต้องไม่เกิน 50 MB"
            return
        }
        hint = nil
        draftBox.text = ""
        composerFocus &+= 1
        let explicitTag = raw.hasPrefix("@")
            && engine.agents.contains { candidate in
                let tag = raw.dropFirst().split(separator: " ", maxSplits: 1).first.map(String.init) ?? ""
                return candidate.ready && (candidate.id.caseInsensitiveCompare(tag) == .orderedSame
                    || candidate.label.caseInsensitiveCompare(tag) == .orderedSame
                    || (candidate.id == "antigravity" && tag.caseInsensitiveCompare("agy") == .orderedSame))
            }
        let send = PendingSend(model: target,
                               text: text.isEmpty ? "ช่วยดูไฟล์ที่แนบมา" : text,
                               attachments: draftBox.attachments,
                               explicitAgent: explicitTag)
        draftBox.attachments = []
        draftBox.rejected = []
        if engine.sessionID < 0 && engine.draft != nil {
            // First message: the draft becomes a session now. The model was
            // picked with no engine to tell, so pin it before sending.
            engine.commitDraft { applyModel(model); routeOrDispatch(send) }
            return
        }
        // Busy: line it up behind the running turn (Cursor-style queue)
        // instead of dropping the keystroke on a disabled button.
        if engine.busy || routingInProgress || pendingRoute != nil { queued.append(send); return }
        routeOrDispatch(send)
    }

    /// The relay chain for a turn, as *agent names* (with accounts), never
    /// model ids: the engine picks the bridge from the name, so sending
    /// "opus" or "default" used to start the Codex bridge for a Claude pick.
    /// Order: the chosen agent's accounts (chosen one first), then every
    /// other ready agent's accounts.
    func chain(for agentID: String) -> [String] {
        let usable = engine.accounts.filter { $0.ready && !$0.limited }
        func names(_ agent: String) -> [String] {
            let own = usable.filter { $0.agent == agent }.map(\.name)
            return own.isEmpty ? [agent] : own
        }
        // An agent with no logged-in account is skipped up front: a revoked
        // Claude login used to cost ~3 minutes of retries before failing over.
        let chosenReady = engine.agents.first { $0.id == agentID }?.ready ?? true
        var out = chosenReady ? names(agentID) : []
        if let pick = UserDefaults.standard.string(forKey: "chorus.preferred.\(agentID)"),
           let i = out.firstIndex(of: pick) {
            out.insert(out.remove(at: i), at: 0)
        }
        if !failoverAccounts { out = Array(out.prefix(1)) }
        if failoverProviders {
            for a in engine.agents where a.ready && a.id != agentID { out += names(a.id) }
        }
        return out
    }

    func accounts_split(_ name: String) -> String {
        String(name.split(separator: "@", maxSplits: 1).first ?? Substring(name))
    }

    func routeOrDispatch(_ send: PendingSend) {
        if currentSessionMode == .orchestra && !send.explicitAgent {
            // Rust picks the lead and its backup, and runs it read-only.
            lastPrompt = send.text
            engine.sendOrchestra(send.text, primary: configuredOrchestrator,
                                 backup: configuredOrchestratorBackup,
                                 concurrency: orchestraConcurrency,
                                 maxChildren: orchestraMaxChildren,
                                 maxMinutes: orchestraMaxMinutes,
                                 level: permission.rawValue,
                                 attachments: send.attachments.map(\.id))
            return
        }
        guard currentSessionMode == .autoRoute, !send.explicitAgent else {
            engine.recordLearnedRoute(
                workspace: workspace,
                prompt: send.text,
                model: send.model,
                agent: agentFor(send.model)?.id ?? "agent",
                reason: send.explicitAgent ? "ผู้ใช้ระบุ agent เจาะจง" : "ผู้ใช้เลือกโมเดลเองโดยตรง"
            )
            dispatch(send.model, send.text, attachments: send.attachments.map(\.id))
            return
        }
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
            "attachments": send.attachments.map { file -> [String: Any] in
                ["media_type": file.mediaType.hasPrefix("image/") ? file.mediaType : "text/plain",
                 "size_bytes": file.size]
            },
            "enabled_integrations": integrations, "required_integrations": []
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
        let conserveEnergy = PowerManager.shared.shouldConserveEnergyForInference
        let effectiveRouterModel = conserveEnergy ? "in-process-heuristic" : routerModel
        routingInProgress = true
        routeStatus = .thinking(routerModel: effectiveRouterModel, candidates: candidates.count)
        let session = engine.sessionID
        engine.proposeRoute(taskJSON: taskJSON, candidatesJSON: candidatesJSON, routerModel: effectiveRouterModel) { json in
            routingInProgress = false
            guard session == engine.sessionID else { routeStatus = nil; return }
            let response = try? JSONDecoder().decode(RouterEnvelope.self, from: Data(json.utf8))
            if response?.ok == true, response?.route?.status == "proposal",
               let chosenID = response?.route?.result?.candidate_id,
               let chosen = engine.agents.flatMap({ agent in
                   agent.models.map { choice in ("\(agent.id):\(choice.id)", choice.id, agent.id) }
               }).first(where: { $0.0 == chosenID }) {
                var chosenReason = response?.route?.result?.reason ?? "เหมาะกับงานนี้"
                if conserveEnergy {
                    chosenReason += " [โหมดประหยัดแบตเตอรี่]"
                }
                let id = ModelID(chosen.1)
                routeStatus = .chosen(
                    agent: engine.agents.first(where: { $0.id == chosen.2 })?.label ?? chosen.2,
                    model: id.base,
                    effort: id.effort ?? "ค่าเริ่มต้น",
                    preset: engine.routePresetLabel,
                    reason: chosenReason
                )
                // The router's own pick is not learned: only user overrides are.
                dispatch(chosen.1, send.text,
                         attachments: send.attachments.map(\.id),
                         agentOverride: chosen.2)
            } else {
                routeStatus = nil
                reviewModel = send.model
                pendingRoute = PendingRoute(send: send,
                    reason: response?.route?.result?.message ?? response?.error
                        ?? "Router ในเครื่องยังเลือกไม่ได้ กรุณาเลือกโมเดลและยืนยันส่งเอง")
            }
        }
    }

    func sendReviewedRoute() {
        guard let pendingRoute else { return }
        self.pendingRoute = nil
        routeStatus = nil
        engine.recordLearnedRoute(
            workspace: workspace,
            prompt: pendingRoute.send.text,
            model: reviewModel,
            agent: agentFor(reviewModel)?.id ?? "agent",
            reason: "ผู้ใช้ยืนยันและเลือกโมเดลนี้เอง"
        )
        dispatch(reviewModel, pendingRoute.send.text,
                 attachments: pendingRoute.send.attachments.map(\.id),
                 agentOverride: nil)
    }

    func cancelReviewedRoute() {
        guard let pendingRoute else { return }
        routeStatus = nil
        draftBox.text = pendingRoute.send.text + (draftBox.text.isEmpty ? "" : "\n" + draftBox.text)
        draftBox.attachments.insert(contentsOf: pendingRoute.send.attachments, at: 0)
        self.pendingRoute = nil
        flushQueue()
    }

    func dispatch(_ target: String, _ text: String, attachments: [Int64] = [],
                          agentOverride: String? = nil) {
        lastPrompt = text
        let agentID = agentOverride ?? agentFor(target)?.id ?? target
        // `@codex …` picked another agent's default model; pin it for the turn.
        if target != model {
            model = target
            engine.setModel(agent: agentID, model: target)
        }
        engine.liveAgent = agentID
        engine.consumeHandoffPrep(for: agentID)
        var names = chain(for: agentID)
        var notice: String?
        if names.first.map({ accounts_split($0) }) != agentID {
            let label = agentFor(target)?.label ?? agentID
            if let next = names.first {
                notice = "\(label) ยังไม่ได้ login หรือติดลิมิตทุกบัญชี — ส่งให้ \(next) แทน (login ได้ที่ ตั้งค่า → บัญชี)"
            } else {
                notice = "\(label) ยังไม่ได้ login — ไปที่ ตั้งค่า → บัญชี แล้วกด Login"
                names = [agentID]
            }
        }
        // After the send: starting a turn clears the banner.
        defer { if let notice { engine.lastError = notice } }
        if planFirst {
            // Read-only whatever `permission` says; enforced in Rust.
            engine.sendPlan(text, agents: names.joined(separator: ","), attachments: attachments)
        } else if permission == .ask {
            engine.sendInteractive(text, agent: names.first ?? agentID, attachments: attachments)
        } else {
            // Always the relay path, even for one agent: a chain of one
            // behaves exactly like a single turn, and the user never has
            // to know the word "relay" to get failover.
            engine.sendRelay(text, agents: names.joined(separator: ","), level: permission.rawValue,
                             attachments: attachments)
        }
    }

    /// Shown under a finished plan: approve it with the composer's own
    /// permission, or just type to change it (any message clears the bar).
    func planApprovalBar(agents: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "list.bullet.clipboard").foregroundStyle(Color.accentColor)
            Text("แผนพร้อมแล้ว — กดทำตามแผน หรือพิมพ์บอกว่าจะแก้ตรงไหน")
                .font(.caption)
                .lineLimit(2)
            Spacer()
            Button("ทำตามแผน") { approvePlan(agents: agents) }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
                .help("ส่งให้ agent ลงมือตามแผน ด้วยสิทธิ์ \(permission.label)")
            Button { engine.pendingPlanAgents = nil } label: { Image(systemName: "xmark") }
                .buttonStyle(.chorusPlain)
                .foregroundStyle(.secondary)
                .help("ไม่ทำตามแผนนี้")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(Color.accentColor.opacity(0.08), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }

    func approvePlan(agents: String) {
        let text = engine.planApprovedPrompt
        guard !text.isEmpty else { return }
        engine.pendingPlanAgents = nil
        if permission == .ask {
            let first = agents.split(separator: ",").first.map(String.init) ?? agents
            engine.sendInteractive(text, agent: first)
        } else {
            engine.sendRelay(text, agents: agents, level: permission.rawValue)
        }
    }

    /// Right-click › ย้อน on one of the user's messages: continue from just
    /// before it in a fork, with the message back in the composer to edit.
    func rewindChat(before seq: Int64, code: Bool) {
        engine.rewind(beforeSeq: seq, code: code, conversation: true, db: dbPath) { text, error in
            if let error {
                engine.lastError = error
                return
            }
            if let text {
                draftBox.text = text
                composerFocus &+= 1
            }
        }
    }

    func flushQueue() {
        guard !engine.busy, !routingInProgress, pendingRoute == nil,
              !queued.isEmpty, engine.lastError == nil else { return }
        let next = queued.removeFirst()
        routeOrDispatch(next)
    }

    // MARK: `/` and `@` completion

    func suggest(_ trigger: Character, _ query: String) -> [Suggestion] {
        let q = query.lowercased()
        func hit(_ t: String) -> Bool { q.isEmpty || t.lowercased().contains(q) }
        if trigger == "@" {
            var out: [Suggestion] = engine.agents.filter { $0.ready && hit($0.id) }.map { a in
                Suggestion(title: "@" + a.id, detail: "ส่งข้อความนี้ให้ \(a.label)", icon: "person.crop.circle",
                           insert: "@\(a.id) ")
            }
            if engine.agents.contains(where: { $0.ready && $0.id == "antigravity" }) && hit("agy") {
                out.append(Suggestion(title: "@agy", detail: "ส่งข้อความนี้ให้ Antigravity", icon: "person.crop.circle",
                                      insert: "@agy "))
            }
            out += fileIndex.match(query, limit: 8 - min(out.count, 3)).map { f in
                Suggestion(title: (f as NSString).lastPathComponent, detail: f, icon: "doc.text", insert: "@\(f) ")
            }
            return out
        }
        var out: [Suggestion] = []
        for a in engine.agents where a.ready && a.id != currentAgentID && engine.sessionID >= 0 {
            out.append(.init(title: "/handoff \(a.id)", detail: "ส่งงานต่อให้ \(a.label) พร้อม brief", icon: "arrowshape.turn.up.right.fill") {
                engine.handoffAuto(to: a.id, level: max(permission.rawValue, 0))
            })
        }
        for a in engine.agents where a.ready {
            for m in a.baseModels {
                out.append(.init(title: "/model \(m.id)", detail: "\(a.label) · \(m.label)", icon: "cpu") {
                    let efforts = a.efforts(for: m.id)
                    model = ModelID.compose(m.id, efforts.contains("high") ? "high" : efforts.first)
                })
            }
        }
        if let a = agentFor(model) {
            for e in a.efforts(for: ModelID(model).base) {
                out.append(.init(title: "/effort \(e)", detail: "ระดับการคิด", icon: "brain") {
                    model = ModelID.compose(ModelID(model).base, e)
                })
            }
        }
        out.append(.init(title: "/new", detail: "session ใหม่ในโฟลเดอร์นี้", icon: "square.and.pencil") {
            if !workspace.isEmpty { startSession() }
        })
        out.append(.init(title: "/compare", detail: "ถามทุก agent แล้วเทียบ", icon: "square.split.2x1") { runCompare() })
        out.append(.init(title: "/snapshot", detail: "บันทึกจุดย้อนกลับตอนนี้", icon: "camera") { engine.snapshot() })
        out.append(.init(title: "/ask", detail: "ถามก่อนเขียนไฟล์", icon: "hand.raised") { permission = .ask })
        out.append(.init(title: "/auto", detail: "ให้ทำเลยไม่ต้องถาม", icon: "bolt.fill") { permission = .allowAll })
        out.append(.init(title: "/plan", detail: planFirst ? "ปิดวางแผนก่อน" : "วางแผนก่อนลงมือ (อ่านอย่างเดียว)",
                         icon: "list.bullet.clipboard") { planFirst.toggle() })
        out.append(.init(title: "/settings", detail: "ตั้งค่าและ skills", icon: "gearshape") { showSettings = true })
        // Skills go in as text: Claude loads them by name, the others are told
        // where they live by the projected AGENTS.md index.
        for sk in engine.skills {
            out.append(.init(title: "/" + sk.name, detail: sk.description, icon: "sparkles", insert: "/\(sk.name) "))
        }
        return Array(out.filter { hit($0.title) || hit($0.detail) }.prefix(9))
    }
}
