import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

// ContentView, split out of ContentView.swift: chat thread and welcome state.
// Moved verbatim; only `private` was dropped so the pieces can see each other.

extension ContentView {
    @ViewBuilder
    func rowForItem(_ item: ChatItem, snap: ChatSnapshot) -> some View {
        switch item {
        case .event(let ev):
            rowForEvent(ev, atts: snap.attachments[ev.seq] ?? [], lastAgentMsg: snap.lastAgentMsg)
        case .tools(let evs):
            ToolGroupRow(events: evs, taint: evs.first.flatMap { engine.taintLabel(forSeq: $0.seq) }).id(item.id)
        }
    }

    @ViewBuilder
    func rowForEvent(_ ev: LedgerEvent, atts: [SessionAttachment], lastAgentMsg: Int64?) -> some View {
        let isUser = ev.actor == "user"
        let isLastAgentMsg = !engine.busy
            && !isUser
            && ev.kind == "msg"
            && ev.id == lastAgentMsg
        let th = isLastAgentMsg ? engine.lastThoughts : nil
        EventRow(event: ev,
                 attachments: atts,
                 agents: engine.agents,
                 thoughts: th,
                 onDeleteAttachment: engine.deleteAttachment,
                 // Echoed rows (negative seq) aren't in the ledger yet.
                 onRewind: isUser && ev.kind == "msg" && ev.seq > 0
                     ? { code in rewindChat(before: ev.seq, code: code) } : nil)
            .equatable()
            .id(ev.seq)
    }

    var thread: some View {
        Group {
            if let toolPage {
                ToolPage(toolID: toolPage, workspace: workspace, onClose: { self.toolPage = nil })
                    .id(toolPage)
            } else if engine.sessionID < 0 && engine.draft == nil {
                emptyState
            } else if engine.events.isEmpty && !engine.busy {
                sessionWelcomeView
            } else {
                GeometryReader { viewport in
                    ScrollViewReader { proxy in
                        ScrollView {
                            LazyVStack(alignment: .leading, spacing: 16) {
                                // Grouped and indexed once per change of the ledger,
                                // not once per render.
                                let snap = chatSnapshots.snapshot(events: engine.events, attachments: engine.attachments)
                                ForEach(snap.items) { item in
                                    rowForItem(item, snap: snap)
                                        .transition(.asymmetric(
                                            insertion: .opacity.combined(with: .offset(y: 8)),
                                            removal: .opacity))
                                }
                                if engine.busy {
                                    WorkingRow(stream: engine.liveStream,
                                               agentID: engine.liveAgent ?? currentAgentID,
                                               agentLabel: engine.agents.first(where: { $0.id == (engine.liveAgent ?? currentAgentID) })?.label ?? "Agent",
                                               startedAt: engine.turnStartedAt,
                                               onStop: { engine.cancel(); queued.removeAll() },
                                               onGrow: {
                                                   if engine.busy && chatPinned {
                                                       proxy.scrollTo("chatBottom", anchor: .bottom)
                                                   }
                                               })
                                        .id("working")
                                        .transition(.opacity.combined(with: .offset(y: 8)))
                                }
                                orchestraStatusSection
                                AgentPromptCards(engine: engine, agentLabel: askingAgentLabel) {
                                    if chatPinned { proxy.scrollTo("chatBottom", anchor: .bottom) }
                                }
                                Color.clear.frame(height: 1).id("chatBottom")
                            }
                            .animation(reduceMotion ? nil : .spring(response: 0.35, dampingFraction: 0.88),
                                       value: engine.busy)
                            .animation(!reduceMotion && chatSnapshots.snapshot(events: engine.events, attachments: engine.attachments).animate
                                        ? .spring(response: 0.35, dampingFraction: 0.88) : nil,
                                       value: engine.events.count)
                            .frame(maxWidth: 820)
                            .frame(maxWidth: .infinity)
                            .padding(.horizontal, 20).padding(.vertical, 16)
                            .background(GeometryReader { g in
                                let f = g.frame(in: .named("chatScroll"))
                                Color.clear.preference(key: ChatContentFrameKey.self,
                                                       value: ChatContentFrame(minY: f.minY, maxY: f.maxY))
                            })
                        }
                        .coordinateSpace(name: "chatScroll")
                        .modifier(OpenAtBottom())
                        // A fresh scroll view per session, so it opens on the
                        // latest message instead of keeping the last one's offset.
                        .id(engine.sessionID)
                        .onPreferenceChange(ChatContentFrameKey.self) { f in
                            // Only an offset change (the reader or us scrolling)
                            // decides pinning; growth at the bottom moves maxY
                            // alone and must not unpin a reader who is following.
                            guard abs(f.minY - chatLastMinY) > 0.5 else { return }
                            chatLastMinY = f.minY
                            let atBottom = f.maxY - viewport.size.height < 80
                            if atBottom != chatPinned { chatPinned = atBottom }
                        }
                        .onChange(of: engine.sessionID) { _, _ in chatPinned = true }
                        .onChange(of: engine.events.count) { _, _ in
                            guard chatPinned else { return }
                            if reduceMotion {
                                proxy.scrollTo("chatBottom", anchor: .bottom)
                            } else {
                                withAnimation(.spring(response: 0.28, dampingFraction: 0.86)) {
                                    proxy.scrollTo("chatBottom", anchor: .bottom)
                                }
                            }
                        }
                        .onChange(of: engine.busy) { _, b in
                            // Sending is a request to see the reply: re-pin.
                            if b {
                                chatPinned = true
                                withAnimation(.spring(response: 0.3, dampingFraction: 0.85)) {
                                    proxy.scrollTo("chatBottom", anchor: .bottom)
                                }
                            }
                        }
                        .chorusScrollIndicators()
                        .overlay(alignment: .bottom) {
                            Group {
                                if !chatPinned {
                                    Button {
                                        chatPinned = true
                                        withAnimation(.spring(response: 0.3, dampingFraction: 0.85)) {
                                            proxy.scrollTo("chatBottom", anchor: .bottom)
                                        }
                                    } label: {
                                        Image(systemName: "arrow.down")
                                            .font(.system(size: 12, weight: .semibold))
                                            .padding(9)
                                            .background(.regularMaterial, in: Circle())
                                            .overlay(Circle().stroke(Color.primary.opacity(0.1), lineWidth: 1))
                                    }
                                    .buttonStyle(.chorusPlain)
                                    .help("ไปที่ข้อความล่าสุด")
                                    .padding(.bottom, 10)
                                    .transition(.scale(scale: 0.8).combined(with: .opacity))
                                }
                            }
                            .animation(.spring(response: 0.3, dampingFraction: 0.8), value: chatPinned)
                        }
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        // Floating composer: the thread scrolls underneath it instead of
        // stopping at a divider, the way Claude/Codex/ChatGPT do.
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if toolPage == nil {
                VStack(spacing: 4) {
                    composer
                    MetricsStripView(engine: engine)
                }
                    .frame(maxWidth: 820)
                    .padding(.horizontal, 16).padding(.bottom, 14).padding(.top, 4)
            }
        }
        .overlay(alignment: .top) {
            VStack(spacing: 8) {
                if isScratchChat {
                    HStack(spacing: 8) {
                        Image(systemName: "bubble.left.and.bubble.right.fill")
                            .foregroundStyle(Color.accentColor)
                        Text("แชทลอย")
                            .font(.caption.weight(.medium))
                        Spacer()
                        if let p = projects.projects.first, let firstFolder = p.folders.first {
                            Button("ผูกเข้ากับโปรเจกต์ \(p.name)") {
                                workspace = firstFolder
                                startSession()
                            }
                            .buttonStyle(.borderedProminent)
                            .controlSize(.small)
                            .help("ย้ายบทสนทนานี้เข้าไปในโปรเจกต์ \(p.name)")
                        }
                    }
                    .padding(.horizontal, 12).padding(.vertical, 7)
                    .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(Color.accentColor.opacity(0.2), lineWidth: 1))
                }
                if let err = engine.lastError {
                    HStack(spacing: 8) {
                        Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.red)
                        Text(err).font(.callout).lineLimit(3).textSelection(.enabled)
                        Spacer(minLength: 0)
                        Button { engine.lastError = nil } label: { Image(systemName: "xmark") }
                            .buttonStyle(.chorusPlain)
                    }
                    .padding(.horizontal, 14).padding(.vertical, 9)
                    .floatingPanel(cornerRadius: 14, tint: .red)
                }
                setupCard
            }
            .frame(maxWidth: 820)
            .padding(.horizontal, 16).padding(.top, 8)
            .animation(.spring(response: 0.35, dampingFraction: 0.85), value: setupNeed != nil)
            .animation(.spring(response: 0.35, dampingFraction: 0.85), value: engine.lastError != nil)
        }
        .frame(minWidth: 420)
        .clipShape(RoundedRectangle(cornerRadius: 20))
        .onDrop(of: [UTType.fileURL, UTType.url, UTType.image], isTargeted: nil, perform: importDroppedProviders)
    }

    @ViewBuilder
    var orchestraStatusSection: some View {
        if !engine.checklist.isEmpty {
            PlanChecklistView(
                items: engine.checklist,
                askBeforePlan: askBeforePlan,
                isPlanApproved: !askBeforePlan || (engine.sessionID >= 0 && planApprovedSessions.contains(engine.sessionID)),
                onStartPlan: {
                    if engine.sessionID >= 0 {
                        planApprovedSessions.insert(engine.sessionID)
                        let currentSubs = engine.subagentsByParent[engine.sessionID] ?? (engine.subagentsByDB[dbPath] ?? [])
                        let waitingSubs = currentSubs.filter { $0.status == "waiting" && $0.kind == "chorus" }
                        for sub in waitingSubs {
                            engine.resumeSubagent(db: dbPath, id: sub.id)
                        }
                    }
                },
                onStop: {
                    if engine.sessionID >= 0 {
                        engine.cancelOrchestration(db: dbPath, parentSess: engine.sessionID)
                    }
                    engine.cancel()
                    queued.removeAll()
                }
            )
            .id("planChecklist")
        }
        let currentSubs = (engine.sessionID >= 0 ? engine.subagentsByParent[engine.sessionID] : nil) ?? (engine.subagentsByDB[dbPath] ?? [])
        let waitingSubs = currentSubs.filter { $0.status == "waiting" && $0.kind == "chorus" }
        if !waitingSubs.isEmpty {
            CapPausePromptCard(
                subagents: waitingSubs,
                onContinue: { subId in
                    engine.resumeSubagent(db: dbPath, id: subId)
                },
                onStop: { subId in
                    engine.cancelSubagent(db: dbPath, id: subId)
                },
                onStopAll: {
                    if engine.sessionID >= 0 {
                        engine.cancelOrchestration(db: dbPath, parentSess: engine.sessionID)
                    }
                    engine.cancel()
                    queued.removeAll()
                }
            )
            .id("capPause")
        }
    }

    var emptyState: some View {
        VStack(spacing: 10) {
            Spacer()
            Image(systemName: "bubble.left.and.bubble.right")
                .font(.system(size: 36)).foregroundStyle(.tertiary)
            Text("ยังไม่ได้เปิด session").font(.title3.weight(.semibold))
            Text("เลือกโฟลเดอร์โปรเจกต์ทางซ้าย แล้วกดเริ่ม session")
                .font(.callout).foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    var sessionWelcomeView: some View {
        VStack(spacing: 16) {
            Spacer()
            VStack(spacing: 8) {
                ZStack {
                    Circle()
                        .fill(Color.accentColor.opacity(0.12))
                        .frame(width: 58, height: 58)
                    Image(systemName: "sparkles")
                        .font(.system(size: 26, weight: .medium))
                        .foregroundStyle(Color.accentColor)
                }
                Text("พร้อมเริ่มงานใน session นี้")
                    .font(.title3.weight(.semibold))
                Text("สั่งงาน ถามคำถามเกี่ยวกับโค้ด หรือเลือกแนวทางเริ่มต้นด้านล่าง")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }

            HStack(spacing: 8) {
                welcomePromptPill(title: "สรุปการเปลี่ยนแปลงล่าสุด", icon: "clock.arrow.circlepath") {
                    draftBox.text = "สรุปการเปลี่ยนแปลงล่าสุดในโปรเจกต์ให้หน่อยว่ามีอะไรบ้าง"
                }
                welcomePromptPill(title: "อธิบายโครงสร้างโค้ด", icon: "folder.badge.gearshape") {
                    draftBox.text = "ช่วยอธิบายโครงสร้างและการทำงานหลักของโปรเจกต์นี้ให้หน่อย"
                }
                welcomePromptPill(title: "ตรวจสอบโค้ดหาจุดบกพร่อง", icon: "checkmark.shield") {
                    draftBox.text = "ช่วยตรวจสอบโค้ดในโปรเจกต์ว่ามีจุดไหนที่ควรปรับปรุงหรือมีบั๊กซ่อนอยู่ไหม"
                }
            }
            .padding(.top, 6)
            Spacer()
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .transition(.opacity)
    }

    func welcomePromptPill(title: String, icon: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 6) {
                Image(systemName: icon).font(.caption).foregroundStyle(Color.accentColor)
                Text(title).font(.caption.weight(.medium))
            }
            .padding(.horizontal, 12).padding(.vertical, 8)
            .background(Color(nsColor: .controlBackgroundColor).opacity(0.6), in: Capsule())
            .overlay(Capsule().stroke(Color.primary.opacity(0.08), lineWidth: 1))
        }
        .buttonStyle(.chorusPlain)
    }
}
