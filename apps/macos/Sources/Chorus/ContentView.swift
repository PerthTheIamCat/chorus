import AppKit
import SwiftUI

/// Mirrors `Policy::from_level` in chorus-cli/session.rs — the FFI takes
/// this as a plain int, not a bool, since read-only sits between deny
/// and allow (auto-approves reads, still denies writes/commands/network).
/// What the agent may do. Two answers, not four.
///
/// There were four: ask / deny / read-only / allow. Two of them stopped being
/// distinguishable once the engine learned that agents do not reliably ask
/// permission at all — deny and read-only now produce the same thing (the
/// restricted ACP mode, plus a revert of anything written anyway), and
/// deny-everything was never usable: an agent that may not read cannot answer.
/// A control whose options differ only in theory is a control that makes people
/// guess, so it is down to the two answers that behave differently.
enum PermissionLevel: Int32, CaseIterable {
    /// Routes to the interactive path: each request pops a dialog, and if the
    /// agent writes without asking, the turn is reverted.
    case ask = -1
    /// Everything allowed, nothing reverted.
    case allowAll = 2

    var label: String {
        switch self {
        case .ask: "ถามก่อน"
        case .allowAll: "ให้ทำเลย"
        }
    }

    var hint: String {
        switch self {
        case .ask: "เด้งถามก่อนเขียนไฟล์หรือรันคำสั่ง — ถ้า agent เขียนโดยไม่ถาม Chorus ย้อนกลับให้"
        case .allowAll: "ให้ทำได้ทุกอย่างโดยไม่ถาม — ใช้กับโฟลเดอร์ที่ลบทิ้งได้เท่านั้น"
        }
    }
}

/// sessions (left) · thread (centre) · context (right) — the layout from
/// design §7, at its smallest. Every action goes straight to `Engine`,
/// which forwards to Rust.
struct ContentView: View {
    @EnvironmentObject var engine: Engine

    // Empty on purpose: defaulting to the home directory made "Start
    // session" look hung — it snapshots the whole tree (checkpoint.rs does
    // `git add -A`) before the first turn, which is minutes-to-forever over
    // a home folder (worse still if it's iCloud-synced). Force a real
    // project path instead of silently picking something enormous.
    @State private var workspace = ""
    @State private var recents: [String] =
        UserDefaults.standard.stringArray(forKey: "chorus.recentWorkspaces") ?? []
    /// Holds the composer text outside `ContentView`'s own state: a `@State`
    /// string here re-evaluated this 1000-line body on every keystroke, which
    /// is what made typing lag. Only `ComposerField` observes the box.
    @StateObject private var draftBox = DraftBox()
    @State private var agent = "codex"
    @State private var permission = PermissionLevel.ask
    @State private var model = ""
    @State private var showSettings = false
    @State private var compareOutput: String?
    @State private var inspected: (text: String, tokens: Int64)?
    @State private var lastPrompt = ""

    @State private var rightTab = RightTab.context
    @State private var showPalette = false
    @State private var expanded: Set<String> = []
    @StateObject private var projects = ProjectStore()
    @StateObject private var fileIndex = FileIndex()
    @State private var editing: ChorusProject?
    @State private var editingIsNew = false
    @State private var sidebarSel: SidebarSel?
    @State private var sessionQuery = ""
    @State private var renaming: (folder: String, s: Engine.SessionSummary)?
    @State private var renameText = ""
    @State private var deleting: (folder: String, s: Engine.SessionSummary)?
    @State private var queued: [(model: String, text: String)] = []
    @State private var hint: String?
    @State private var confirmRevert = false
    @State private var rejected: Set<UUID> = []

    enum RightTab: String, CaseIterable {
        case context = "คอนเท็กซ์", subagents = "งานย่อย", changes = "ที่แก้ไป", memory = "ความจำ"
    }

    var body: some View {
        NavigationSplitView {
            sidebar
        } detail: {
            HStack(spacing: 0) {
                thread
                Divider()
                VStack(spacing: 0) {
                    Picker("", selection: $rightTab) {
                        ForEach(RightTab.allCases, id: \.self) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    .padding(8)
                    Divider()
                    switch rightTab {
                    case .context: contextPane
                    case .subagents: subagentsPane
                    case .changes: changesPane
                    case .memory: memoryPane
                    }
                }
                .frame(width: 300)
            }
        }
        .toolbar {
            ToolbarItem(placement: .principal) {
                Text(engine.sessionID < 0 ? "ยังไม่ได้เปิดเซสชัน"
                     : (engine.sessionsByDB[dbPath]?.first { $0.id == engine.sessionID }).map(displayTitle)
                        ?? "เซสชัน \(engine.sessionID)")
                    .lineLimit(1)
                    .font(.headline)
            }
            ToolbarItemGroup(placement: .primaryAction) {
                // Handing over is one click per agent. Everything the next
                // agent needs is already in the ledger, so asking the user to
                // fill in a form would be asking them to retype what we know.
                if engine.sessionID >= 0 {
                    ForEach(engine.agents.filter { $0.ready && $0.id != currentAgentID }) { a in
                        Button {
                            engine.handoffAuto(to: a.defaultModel, level: max(permission.rawValue, 0))
                        } label: {
                            Label("ส่งต่อให้ \(a.label)", systemImage: "arrow.left.arrow.right")
                        }
                        .disabled(engine.busy)
                        .help("ส่งงานต่อให้ \(a.label) พร้อมเป้าหมาย สิ่งที่ทำไปแล้ว และ diff — ไม่ต้องพิมพ์อะไร")
                    }
                }
                Button { showSettings = true } label: {
                    Label("ตั้งค่า", systemImage: "gearshape")
                }
            }
        }
        .onAppear { engine.loadAgents(); engine.registerProgress() }
        .onChange(of: workspace) { _, w in fileIndex.load(w); engine.loadSkills(cwd: w) }
        .onChange(of: engine.sessionID) { _, id in
            guard !workspace.isEmpty else { return }
            engine.loadSessions(db: dbPath)
            if id >= 0, let p = projects.project(containing: workspace) {
                engine.setMemoryDB(ProjectStore.memoryPath(p), projectName: p.name)
            }
        }
        .onChange(of: engine.busy) { _, _ in flushQueue() }
        .onChange(of: engine.agents) { _, _ in syncModelSelection(); engine.refreshModels() }
        .onChange(of: model) { _, m in applyModel(m) }
        .sheet(isPresented: $showPalette) { palette }
        .alert("เปลี่ยนชื่อเซสชัน", isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
            TextField("ชื่อ", text: $renameText)
            Button("บันทึก") {
                if let r = renaming {
                    engine.renameSession(db: dbPath(for: r.folder), id: r.s.id,
                                         title: renameText.trimmingCharacters(in: .whitespaces))
                }
                renaming = nil
            }
            Button("ยกเลิก", role: .cancel) { renaming = nil }
        }
        .confirmationDialog(
            "ลบเซสชันนี้?",
            isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
            presenting: deleting
        ) { d in
            Button("ลบถาวร", role: .destructive) {
                let open = d.folder == workspace && d.s.id == engine.sessionID
                engine.deleteSession(db: dbPath(for: d.folder), id: d.s.id, isOpen: open)
                deleting = nil
            }
            Button("ยกเลิก", role: .cancel) { deleting = nil }
        } message: { d in
            Text("“\(displayTitle(d.s))” และประวัติทั้งหมดจะหายถาวร กู้คืนไม่ได้ ไฟล์ในโฟลเดอร์ไม่ถูกแตะ")
        }
        .sheet(item: $editing) { p in
            ProjectEditor(project: p, isNew: editingIsNew,
                          onSave: { saved in
                              projects.upsert(saved)
                              expanded.insert(saved.id.uuidString)
                              loadSessions(of: saved)
                          },
                          onDelete: editingIsNew ? nil : { projects.remove(p) })
        }
        .background(
            ZStack {
                Button("") { showPalette = true }
                    .keyboardShortcut("k", modifiers: .command)
                Button("") {
                    if workspace.isEmpty || engine.busy {
                        editing = ChorusProject(name: "", folders: []); editingIsNew = true
                    } else { startSession() }
                }
                .keyboardShortcut("n", modifiers: .command)
            }
            .frame(width: 0, height: 0)
            .opacity(0)
            .allowsHitTesting(false)
        )
        .sheet(isPresented: $showSettings) {
            SettingsSheet(workspace: workspace).environmentObject(engine)
        }
        .sheet(isPresented: Binding(get: { compareOutput != nil }, set: { if !$0 { compareOutput = nil } })) {
            CompareResultsSheet(json: compareOutput ?? "")
        }
        .sheet(isPresented: Binding(get: { inspected != nil }, set: { if !$0 { inspected = nil } })) {
            contextInspector
        }
        .sheet(item: $engine.pendingAsk) { ask in
            VStack(alignment: .leading, spacing: 14) {
                Text("ขออนุญาต").font(.title3.bold())
                Text("\(agent) wants to: **\(ask.tool)**")
                if !ask.kind.isEmpty {
                    Text("kind: \(ask.kind)").font(.caption).foregroundStyle(.secondary)
                }
                if !ask.detail.isEmpty {
                    Text(ask.detail).font(.callout).textSelection(.enabled)
                }
                HStack {
                    Spacer()
                    Button("Deny", role: .destructive) { engine.answerPermission(false) }
                    Button("Allow") { engine.answerPermission(true) }
                        .keyboardShortcut(.defaultAction)
                }
            }
            .padding(20)
            .frame(width: 420)
        }
    }

    // MARK: sidebar — open / start a session

    // A plain leading-aligned VStack instead of `Form`: Form's two-column
    // label|control layout was the cause of the earlier truncated labels —
    // there just isn't enough width in a sidebar for a label column plus a
    // control column plus a text field. Every label sits above its control
    // instead, and gets the sidebar's full width to wrap in.
    /// A native `List`: hand-rolled rows in a ScrollView drew in one place
    /// and took clicks in another, and a sidebar is exactly what `List` exists
    /// for — system selection, hover, keyboard navigation, hit-testing.
    private var sidebar: some View {
        List(selection: $sidebarSel) {
            TextField("ค้นหาเซสชัน", text: $sessionQuery)
                .textFieldStyle(.roundedBorder)
                .padding(.bottom, 4)
            Section {
                if projects.projects.isEmpty {
                    Button { editing = ChorusProject(name: "", folders: []); editingIsNew = true } label: {
                        Label("สร้างโปรเจกต์แรก", systemImage: "square.stack.3d.up.badge.plus")
                    }
                    .buttonStyle(.borderless)
                }
                ForEach(projects.projects) { p in projectRow(p) }
            } header: {
                HStack {
                    Text("โปรเจกต์")
                    Spacer()
                    Button { editing = ChorusProject(name: "", folders: []); editingIsNew = true } label: {
                        Image(systemName: "plus")
                    }
                    .buttonStyle(.borderless)
                    .help("สร้างโปรเจกต์ใหม่ (⌘N เมื่อยังไม่ได้เลือกโฟลเดอร์)")
                }
            }
        }
        .listStyle(.sidebar)
        .onChange(of: sidebarSel) { _, sel in select(sel) }
        .onChange(of: engine.sessionID) { _, id in
            sidebarSel = id >= 0 && !workspace.isEmpty ? .session(workspace, id) : nil
        }
        .safeAreaInset(edge: .bottom) { agentFooter }
        .navigationTitle("Chorus")
        .frame(minWidth: 240, idealWidth: 280)
    }

    enum SidebarSel: Hashable { case session(String, Int64) }

    private func select(_ sel: SidebarSel?) {
        guard case let .session(folder, id)? = sel else { return }
        if folder == workspace && id == engine.sessionID { return }
        // Switching frees the engine handle; never under a running turn.
        guard !engine.busy, !engine.switching else {
            sidebarSel = engine.sessionID >= 0 ? .session(workspace, engine.sessionID) : nil
            return
        }
        workspace = folder
        engine.reopen(db: dbPath(for: folder), sessionID: id)
    }

    /// Which agents can answer right now — the relay depends on it, so it
    /// stays visible instead of hiding in a settings form.
    private var agentFooter: some View {
        VStack(spacing: 0) {
            Divider()
            HStack(spacing: 10) {
                if engine.agents.isEmpty {
                    ProgressView().controlSize(.mini)
                    Text("กำลังตรวจเอเจนต์…").font(.caption2).foregroundStyle(.secondary)
                }
                ForEach(engine.agents) { a in
                    HStack(spacing: 4) {
                        Circle().fill(a.ready ? Color.green : .orange).frame(width: 6, height: 6)
                        Text(a.label).font(.caption2)
                    }
                    .help(a.ready ? "\(a.label) พร้อมใช้" : a.detail)
                }
                Spacer()
                Button { showSettings = true } label: { Image(systemName: "gearshape") }
                    .buttonStyle(.borderless)
                    .keyboardShortcut(",", modifiers: .command)
                    .help("ตั้งค่า (⌘,)")
            }
            .padding(.horizontal, 14).padding(.vertical, 9)
        }
        .background(.bar)
    }

    /// The agent whose model id is currently selected.
    private func agentFor(_ modelID: String) -> Engine.AgentInfo? {
        engine.agents.first { $0.models.contains(where: { $0.id == modelID }) || $0.id == modelID }
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
    private func applyModel(_ modelID: String) {
        guard let a = agentFor(modelID) else { return }
        engine.setModel(agent: a.id, model: modelID)
    }

    private func syncModelSelection() {
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


    private var currentAgentID: String { agentFor(model)?.id ?? "" }

    /// The chosen agent first, then every other agent that could actually take
    /// over. Built here rather than typed by the user — that text field was the
    /// single most confusing control in the old window.
    private var relayChain: String {
        var chain = [model]
        for a in engine.agents where a.ready && a.id != currentAgentID {
            chain.append(a.defaultModel)
        }
        return chain.joined(separator: ",")
    }

    private func pickFolder() {
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

    private func addRecent(_ path: String) {
        var list = recents.filter { $0 != path }
        list.insert(path, at: 0)
        if list.count > 30 { list = Array(list.prefix(30)) }
        recents = list
        persistRecents()
    }

    private func persistRecents() {
        UserDefaults.standard.set(recents, forKey: "chorus.recentWorkspaces")
    }

    private func startSession() {
        let db = (workspace as NSString).appendingPathComponent(".chorus/store.db")
        try? FileManager.default.createDirectory(
            atPath: (db as NSString).deletingLastPathComponent,
            withIntermediateDirectories: true)
        engine.open(db: db, cwd: workspace, title: "session")
    }

    /// A section header plus its content, spaced and left-aligned — the
    /// sidebar's substitute for `Form`'s `Section`.
    @ViewBuilder
    private func sidebarSection<Content: View>(
        _ title: String, @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).font(.subheadline.bold())
            content()
        }
    }

    // MARK: thread — the ledger, plus the compose box

    private var thread: some View {
        VStack(spacing: 0) {
            if let err = engine.lastError {
                Text(err)
                    .font(.callout).foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(8)
                    .background(.red.opacity(0.08))
            }
            if engine.sessionID < 0 {
                emptyState
            } else {
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 10) {
                            ForEach(engine.events) { ev in
                                EventRow(event: ev).id(ev.seq)
                            }
                            if engine.busy { workingRow.id("working") }
                            else if !engine.lastThoughts.isEmpty {
                                ThinkingBlock(text: engine.lastThoughts, running: false)
                            }
                        }
                        .padding(12)
                    }
                    .onChange(of: engine.events.count) { _, _ in
                        if let last = engine.events.last { proxy.scrollTo(last.seq, anchor: .bottom) }
                    }
                    .onChange(of: engine.live.message.count + engine.live.thoughts.count / 200 + engine.live.tools.count) { _, _ in
                        if engine.busy { proxy.scrollTo("working", anchor: .bottom) }
                    }
                    .onChange(of: engine.busy) { _, b in
                        if b { withAnimation { proxy.scrollTo("working", anchor: .bottom) } }
                    }
                }
            }
            Divider()
            composer
        }
        .frame(minWidth: 380)
    }

    /// The running turn, live: reasoning (collapsible, like the thinking
    /// block in Claude/Codex), tool calls as they start and finish, and the
    /// answer as it streams — then Stop.
    private var workingRow: some View {
        let live = engine.live
        return VStack(alignment: .leading, spacing: 8) {
            if !live.thoughts.isEmpty {
                ThinkingBlock(text: live.thoughts, running: live.message.isEmpty)
            }
            ForEach(live.tools) { t in
                HStack(spacing: 6) {
                    switch t.state {
                    case "running": ProgressView().controlSize(.mini)
                    case "failed": Image(systemName: "xmark.circle.fill").foregroundStyle(.red)
                    default: Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                    }
                    Text(t.title).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            if !live.message.isEmpty {
                MarkdownText(text: live.message)
            }
            HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                if let t = engine.turnStartedAt {
                    TimelineView(.periodic(from: t, by: 1)) { ctx in
                        Text(live.isEmpty ? "กำลังเริ่ม agent… \(Int(ctx.date.timeIntervalSince(t))) วิ"
                                          : "กำลังทำงาน… \(Int(ctx.date.timeIntervalSince(t))) วิ")
                    }
                } else {
                    Text("กำลังทำงาน…")
                }
                Spacer()
                Button { engine.cancel(); queued.removeAll() } label: {
                    Label("หยุด", systemImage: "stop.fill")
                }
                .controlSize(.small)
                .keyboardShortcut(".", modifiers: .command)
                .help("หยุดเทิร์นนี้ (⌘.) — ล้างคิวด้วย")
            }
            .font(.callout).foregroundStyle(.secondary)
        }
        .padding(10)
        .background(.secondary.opacity(0.06), in: RoundedRectangle(cornerRadius: 10))
    }

    private var emptyState: some View {
        VStack(spacing: 10) {
            Spacer()
            Image(systemName: "bubble.left.and.bubble.right")
                .font(.system(size: 36)).foregroundStyle(.tertiary)
            Text("ยังไม่ได้เปิดเซสชัน").font(.title3.weight(.semibold))
            Text("เลือกโฟลเดอร์โปรเจกต์ทางซ้าย แล้วกดเริ่มเซสชัน")
                .font(.callout).foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var composer: some View {
        VStack(spacing: 6) {
            if !queued.isEmpty {
                VStack(alignment: .leading, spacing: 3) {
                    ForEach(Array(queued.enumerated()), id: \.offset) { i, q in
                        HStack(spacing: 6) {
                            Image(systemName: "clock").font(.caption2)
                            Text(q.text).font(.caption).lineLimit(1)
                            Spacer()
                            Button { queued.remove(at: i) } label: { Image(systemName: "xmark.circle.fill") }
                                .buttonStyle(.plain).foregroundStyle(.secondary)
                        }
                    }
                    Text("รอคิว — จะส่งต่อเมื่อเทิร์นนี้จบ").font(.caption2).foregroundStyle(.tertiary)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            if let hint { Text(hint).font(.caption).foregroundStyle(.orange).frame(maxWidth: .infinity, alignment: .leading) }
            ComposerField(box: draftBox, suggestions: suggest, onSubmit: submit)
            HStack(spacing: 8) {
                ModelPicker(model: $model, onNeedsLogin: { showSettings = true })
                Menu {
                    ForEach(PermissionLevel.allCases, id: \.self) { p in
                        Button { permission = p } label: {
                            if p == permission { Label(p.label, systemImage: "checkmark") } else { Text(p.label) }
                        }
                    }
                } label: {
                    Label(permission.label, systemImage: permission == .ask ? "hand.raised" : "bolt.fill")
                        .font(.caption)
                }
                .menuStyle(.borderlessButton).fixedSize()
                .help(permission.hint)
                Spacer()
                Button(action: submit) {
                    if engine.busy { Image(systemName: "text.badge.plus").font(.title2) }
                    else { Image(systemName: "arrow.up.circle.fill").font(.title2) }
                }
                .buttonStyle(.plain)
                .keyboardShortcut(.return, modifiers: .command)
                .disabled(engine.sessionID < 0)
                .help(engine.busy ? "เข้าคิว (↩)" : "ส่ง (↩ · ⇧↩ ขึ้นบรรทัดใหม่)")
            }
        }
        .padding(10)
        .background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 10))
        .padding(10)
    }

    /// `@codex fix this` → ("codex's default model", "fix this"). Matches agent
    /// id or label, case-insensitively; anything else is sent untouched.
    private func route(_ text: String) -> (model: String, text: String) {
        guard text.hasPrefix("@") else { return (model, text) }
        let parts = text.dropFirst().split(separator: " ", maxSplits: 1, omittingEmptySubsequences: true)
        guard let tag = parts.first?.lowercased(),
              let a = engine.agents.first(where: { $0.ready && ($0.id.lowercased() == tag || $0.label.lowercased() == tag) })
        else { return (model, text) }
        return (a.defaultModel, parts.count > 1 ? String(parts[1]) : "")
    }

    private func submit() {
        let raw = draftBox.text.trimmingCharacters(in: .whitespacesAndNewlines)
        let (target, text) = route(raw)
        guard engine.sessionID >= 0, !raw.isEmpty else { return }
        guard !text.isEmpty else {
            hint = "พิมพ์ข้อความต่อจาก \(raw) ด้วย"
            return
        }
        hint = nil
        draftBox.text = ""
        // Busy: line it up behind the running turn (Cursor-style queue)
        // instead of dropping the keystroke on a disabled button.
        if engine.busy { queued.append((target, text)); return }
        dispatch(target, text)
    }

    private func dispatch(_ target: String, _ text: String) {
        lastPrompt = text
        if permission == .ask {
            engine.sendInteractive(text, agent: target)
        } else {
            // Always the relay path, even for one agent: a chain of one
            // behaves exactly like a single turn, and the user never has
            // to know the word "relay" to get failover.
            var chain = [target]
            for a in engine.agents where a.ready && a.defaultModel != target && a.id != agentFor(target)?.id {
                chain.append(a.defaultModel)
            }
            engine.sendRelay(text, agents: chain.joined(separator: ","), level: permission.rawValue)
        }
    }

    private func flushQueue() {
        guard !engine.busy, !queued.isEmpty, engine.lastError == nil else { return }
        let next = queued.removeFirst()
        dispatch(next.model, next.text)
    }

    // MARK: `/` and `@` completion

    private func suggest(_ trigger: Character, _ query: String) -> [Suggestion] {
        let q = query.lowercased()
        func hit(_ t: String) -> Bool { q.isEmpty || t.lowercased().contains(q) }
        if trigger == "@" {
            var out: [Suggestion] = engine.agents.filter { $0.ready && hit($0.id) }.map { a in
                Suggestion(title: "@" + a.id, detail: "ส่งข้อความนี้ให้ \(a.label)", icon: "person.crop.circle",
                           insert: "@\(a.id) ")
            }
            out += fileIndex.match(query, limit: 8 - min(out.count, 3)).map { f in
                Suggestion(title: (f as NSString).lastPathComponent, detail: f, icon: "doc.text", insert: "@\(f) ")
            }
            return out
        }
        var out: [Suggestion] = []
        for a in engine.agents where a.ready && a.id != currentAgentID && engine.sessionID >= 0 {
            out.append(.init(title: "/handoff \(a.id)", detail: "ส่งงานต่อให้ \(a.label) พร้อม brief", icon: "arrow.left.arrow.right") {
                engine.handoffAuto(to: a.defaultModel, level: max(permission.rawValue, 0))
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
        out.append(.init(title: "/new", detail: "เซสชันใหม่ในโฟลเดอร์นี้", icon: "square.and.pencil") {
            if !workspace.isEmpty { startSession() }
        })
        out.append(.init(title: "/compare", detail: "ถามทุกเอเจนต์แล้วเทียบ", icon: "square.split.2x1") { runCompare() })
        out.append(.init(title: "/snapshot", detail: "บันทึกจุดย้อนกลับตอนนี้", icon: "camera") { engine.snapshot() })
        out.append(.init(title: "/ask", detail: "ถามก่อนเขียนไฟล์", icon: "hand.raised") { permission = .ask })
        out.append(.init(title: "/auto", detail: "ให้ทำเลยไม่ต้องถาม", icon: "bolt.fill") { permission = .allowAll })
        out.append(.init(title: "/settings", detail: "ตั้งค่าและ skills", icon: "gearshape") { showSettings = true })
        // Skills go in as text: Claude loads them by name, the others are told
        // where they live by the projected AGENTS.md index.
        for sk in engine.skills {
            out.append(.init(title: "/" + sk.name, detail: sk.description, icon: "sparkles", insert: "/\(sk.name) "))
        }
        return Array(out.filter { hit($0.title) || hit($0.detail) }.prefix(9))
    }

    private func runCompare() {
        let others = engine.agents.filter(\.ready).map(\.defaultModel)
        guard others.count > 1, engine.sessionID >= 0 else { return }
        engine.compare(
            lastPrompt.isEmpty ? "ทบทวนงานที่ค้างอยู่แล้วเสนอขั้นต่อไป" : lastPrompt,
            agents: others.joined(separator: ","),
            level: max(permission.rawValue, 0)
        ) { compareOutput = $0 }
    }

    // MARK: command palette (⌘K)

    private struct PaletteItem: Identifiable {
        let id = UUID()
        let title: String
        let icon: String
        let run: () -> Void
    }

    private var paletteItems: [PaletteItem] {
        var items: [PaletteItem] = []
        for a in engine.agents where a.ready {
            items.append(.init(title: "สลับไปใช้ \(a.label)", icon: "person.crop.circle") { model = a.defaultModel })
            if engine.sessionID >= 0 && a.id != currentAgentID {
                items.append(.init(title: "ส่งต่องานให้ \(a.label)", icon: "arrow.left.arrow.right") {
                    engine.handoffAuto(to: a.defaultModel, level: max(permission.rawValue, 0))
                })
            }
        }
        items.append(.init(title: "เลือกโฟลเดอร์โปรเจกต์…", icon: "folder") { pickFolder() })
        if engine.sessionID >= 0 {
            items.append(.init(title: "Snapshot ตอนนี้", icon: "camera") { engine.snapshot() })
        }
        items.append(.init(title: "ตั้งค่า", icon: "gearshape") { showSettings = true })
        return items
    }

    private var palette: some View {
        PaletteSheet(items: paletteItems.map { ($0.title, $0.icon, $0.run) }) { showPalette = false }
    }

    // MARK: context pane — budget + last agent

    private var changesPane: some View {
        let hunks = parseHunks(engine.workingDiff)
        return VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Since session start").font(.headline)
                Spacer()
                Button("Snapshot") { engine.snapshot() }
                    .controlSize(.small).disabled(engine.sessionID < 0)
            }
            if hunks.isEmpty {
                Text("no changes yet").font(.caption).foregroundStyle(.secondary)
                Spacer()
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 10) {
                        ForEach(hunks) { hunk in
                            HunkView(hunk: hunk, rejected: rejected.contains(hunk.id)) {
                                if rejected.contains(hunk.id) { rejected.remove(hunk.id) }
                                else { rejected.insert(hunk.id) }
                            }
                        }
                    }
                }
                if !rejected.isEmpty {
                    Button(role: .destructive) {
                        let patch = hunks.filter { rejected.contains($0.id) }
                            .map(\.patch).joined()
                        engine.rejectHunks(patch)
                        rejected.removeAll()
                    } label: {
                        Label("Reject \(rejected.count) hunk(s)", systemImage: "xmark.circle")
                    }
                    .controlSize(.small)
                }
                Button(role: .destructive) { confirmRevert = true } label: {
                    Label("Revert everything", systemImage: "arrow.uturn.backward")
                }
                .controlSize(.small)
                .confirmationDialog(
                    "Discard every change agents made this session?",
                    isPresented: $confirmRevert, titleVisibility: .visible
                ) {
                    Button("Revert everything", role: .destructive) { engine.revertToStart() }
                    Button("Cancel", role: .cancel) {}
                }
            }
        }
        .padding(12)
        .frame(maxHeight: .infinity, alignment: .top)
    }

    private var memoryPane: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 6) {
                Image(systemName: engine.memoryScope == nil ? "folder" : "square.stack.3d.up.fill")
                    .foregroundStyle(Color.accentColor)
                Text(engine.memoryScope.map { "ความจำร่วมของโปรเจกต์ \($0)" } ?? "ความจำของโฟลเดอร์นี้")
                    .font(.caption.weight(.semibold))
            }
            .help("ทุกเซสชันและทุกเอเจนต์ในโปรเจกต์นี้อ่านความจำชุดเดียวกัน")
            if !engine.pending.isEmpty {
                Text("Proposed (\(engine.pending.count))").font(.headline)
                ForEach(engine.pending) { p in
                    VStack(alignment: .leading, spacing: 4) {
                        Text(p.text).font(.callout)
                        HStack {
                            Button("Approve") { engine.review(p.id, approve: true) }
                                .controlSize(.small)
                            Button("Reject", role: .destructive) { engine.review(p.id, approve: false) }
                                .controlSize(.small)
                        }
                    }
                    .padding(8)
                    .background(.yellow.opacity(0.1), in: RoundedRectangle(cornerRadius: 6))
                }
                Divider()
            }
            Text("Facts (\(engine.facts.count))").font(.headline)
            if engine.facts.isEmpty {
                Text("none yet").font(.caption).foregroundStyle(.secondary)
            }
            ForEach(Array(engine.facts.enumerated()), id: \.offset) { _, f in
                Text("• \(f)").font(.callout).textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            Spacer()
        }
        .padding(12)
        .frame(maxHeight: .infinity, alignment: .top)
    }

    /// The app's honesty surface. Everything else in this window *describes*
    /// what gets carried between agents; this shows the bytes.
    private var contextInspector: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text("สิ่งที่ agent ตัวถัดไปจะได้เห็น").font(.headline)
                    Text("ประกอบสดจาก ledger ไม่ใช่คำอธิบาย — นี่คือข้อความจริงที่จะถูกส่ง")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                if let n = inspected?.tokens, n > 0 {
                    Text("\(n) token").font(.caption.monospaced()).foregroundStyle(.secondary)
                }
                Button("Done") { inspected = nil }.keyboardShortcut(.defaultAction)
            }
            ScrollView {
                Text(inspected?.text ?? "")
                    .font(.caption.monospaced())
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(16)
        .frame(minWidth: 640, minHeight: 460)
    }

    // MARK: projects — several folders at once, each with its own sessions

    private func dbPath(for project: String) -> String {
        (project as NSString).appendingPathComponent(".chorus/store.db")
    }

    private func sessions(of p: ChorusProject) -> [(folder: String, s: Engine.SessionSummary, key: String)] {
        p.folders.flatMap { f in (engine.sessionsByDB[dbPath(for: f)] ?? []).map { (f, $0, "\(f)#\($0.id)") } }
            .sorted { ($0.s.started_ms + $0.s.last_ms) > ($1.s.started_ms + $1.s.last_ms) }
    }

    private func loadSessions(of p: ChorusProject) {
        for f in p.folders { engine.loadSessions(db: dbPath(for: f)) }
    }

    private func expandedBinding(_ p: ChorusProject) -> Binding<Bool> {
        Binding(
            get: { expanded.contains(p.id.uuidString) },
            set: { open in
                if open { expanded.insert(p.id.uuidString); loadSessions(of: p) }
                else { expanded.remove(p.id.uuidString) }
            })
    }

    private func projectRow(_ p: ChorusProject) -> some View {
        let rows = sessions(of: p)
        return DisclosureGroup(isExpanded: expandedBinding(p)) {
            newSessionRow(p)
            if p.folders.isEmpty {
                Button("เพิ่มโฟลเดอร์ให้โปรเจกต์นี้…") { editing = p; editingIsNew = false }
                    .buttonStyle(.link).font(.caption)
            } else if rows.isEmpty {
                Text("ยังไม่มีเซสชัน").font(.caption).foregroundStyle(.tertiary)
            }
            ForEach(rows.filter { matches($0.s) }, id: \.key) { r in
                sessionLabel(r.s, folder: r.folder, showFolder: p.folders.count > 1)
                    .tag(SidebarSel.session(r.folder, r.s.id))
                    .contextMenu {
                        Button("เปลี่ยนชื่อ…") { renaming = (r.folder, r.s); renameText = displayTitle(r.s) }
                        Button("เปิดโฟลเดอร์ใน Finder") {
                            NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: r.folder)])
                        }
                        Divider()
                        Button("ลบเซสชัน…", role: .destructive) { deleting = (r.folder, r.s) }
                    }
            }
        } label: {
            HStack(spacing: 6) {
                Image(systemName: "square.stack.3d.up.fill").foregroundStyle(Color.accentColor)
                Text(p.name).lineLimit(1)
                Spacer(minLength: 0)
                if p.folders.contains(workspace) && engine.sessionID >= 0 {
                    Circle().fill(.green).frame(width: 6, height: 6)
                }
            }
            .contextMenu {
                Button("แก้ไขโปรเจกต์…") { editing = p; editingIsNew = false }
                Menu("เปิดใน Finder") {
                    ForEach(p.folders, id: \.self) { f in
                        Button((f as NSString).lastPathComponent) {
                            NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: f)])
                        }
                    }
                }
                Divider()
                Button("ลบโปรเจกต์ (ไฟล์ไม่หาย)", role: .destructive) { projects.remove(p) }
            }
        }
        .onAppear { if expanded.contains(p.id.uuidString) { loadSessions(of: p) } }
    }

    @ViewBuilder
    private func newSessionRow(_ p: ChorusProject) -> some View {
        let title = engine.switching && p.folders.contains(workspace) ? "กำลังเตรียมโฟลเดอร์…" : "เซสชันใหม่"
        if p.folders.count > 1 {
            Menu {
                ForEach(p.folders, id: \.self) { f in
                    Button((f as NSString).lastPathComponent) { workspace = f; startSession() }
                }
            } label: { Label(title, systemImage: "square.and.pencil") }
            .menuStyle(.borderlessButton)
            .disabled(engine.busy || engine.switching)
        } else if let f = p.folders.first {
            Button { workspace = f; startSession() } label: {
                Label(title, systemImage: "square.and.pencil")
            }
            .buttonStyle(.borderless)
            .disabled(engine.busy || engine.switching)
        }
    }

    /// A title the user set wins; otherwise the first thing they asked.
    private func displayTitle(_ s: Engine.SessionSummary) -> String {
        let t = s.title.trimmingCharacters(in: .whitespaces)
        if !t.isEmpty && t != "session" { return t }
        return s.goal.isEmpty ? "เซสชัน #\(s.id)" : s.goal
    }

    private func matches(_ s: Engine.SessionSummary) -> Bool {
        let q = sessionQuery.trimmingCharacters(in: .whitespaces)
        return q.isEmpty || displayTitle(s).localizedCaseInsensitiveContains(q)
            || s.goal.localizedCaseInsensitiveContains(q)
            || s.agents.contains { $0.localizedCaseInsensitiveContains(q) }
    }

    private func sessionLabel(_ s: Engine.SessionSummary, folder: String, showFolder: Bool) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(displayTitle(s)).lineLimit(2)
            HStack(spacing: 4) {
                if showFolder {
                    Text((folder as NSString).lastPathComponent).foregroundStyle(.secondary)
                }
                Text(s.agents.joined(separator: " · ")).foregroundStyle(.secondary)
                if s.forked_from != nil { Image(systemName: "arrow.triangle.branch") }
            }
            .font(.caption2)
        }
        .padding(.vertical, 2)
    }

    /// Delegated work, read back from the ledger: every tool call that looks
    /// like a spawned subagent (Task / Agent / delegate), the result that
    /// followed it, and who ran it. This is the ledger's view — it fills in
    /// when a turn ends, not while it streams.
    private var subagentsPane: some View {
        let evs = engine.events
        let calls: [(call: LedgerEvent, result: LedgerEvent?)] = evs.enumerated().compactMap { i, e in
            guard e.kind == "tool_call", Self.isSubagentCall(e.body) else { return nil }
            let r = evs[(i + 1)...].first { $0.kind == "tool_result" }
            return (e, r)
        }
        let byAgent = Dictionary(grouping: evs.filter { $0.actor != "user" && $0.kind == "tool_call" }, by: \.actor)
        return ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                Text("งานที่ agent แตกออกไปทำ").font(.headline)
                if calls.isEmpty {
                    Text("ยังไม่มี subagent ในเซสชันนี้ — เมื่อ agent เรียก Task/Agent จะขึ้นที่นี่หลังเทิร์นจบ")
                        .font(.caption).foregroundStyle(.secondary)
                }
                ForEach(calls, id: \.call.seq) { c in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(spacing: 6) {
                            Image(systemName: c.result == nil ? "hourglass" : "checkmark.circle.fill")
                                .foregroundStyle(c.result == nil ? Color.orange : Color.green)
                            Text(c.call.actor).font(.caption.bold())
                            Spacer()
                            Text("#\(c.call.seq)").font(.caption2).foregroundStyle(.tertiary)
                        }
                        Text(c.call.body).font(.caption).lineLimit(3)
                        if let r = c.result {
                            DisclosureGroup("ผลลัพธ์") {
                                Text(r.body).font(.caption2.monospaced()).textSelection(.enabled)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }.font(.caption2)
                        }
                    }
                    .padding(8)
                    .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 8))
                }
                Divider()
                Text("กิจกรรมต่อ agent").font(.subheadline.bold())
                ForEach(byAgent.keys.sorted(), id: \.self) { a in
                    HStack {
                        Text(a).font(.caption)
                        Spacer()
                        Text("\(byAgent[a]?.count ?? 0) tool calls").font(.caption2).foregroundStyle(.secondary)
                    }
                }
            }
            .padding(12)
        }
    }

    private static func isSubagentCall(_ body: String) -> Bool {
        let t = body.lowercased()
        return ["task:", "agent:", "subagent", "delegate", "spawn"].contains { t.contains($0) }
    }

    private var dbPath: String {
        (workspace as NSString).appendingPathComponent(".chorus/store.db")
    }

    private var contextPane: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("คอนเท็กซ์").font(.headline)
            if let used = engine.contextUsed, let win = engine.contextWindow, win > 0 {
                let frac = Double(used) / Double(win)
                ProgressView(value: frac) {
                    Text("\(used) / \(win)").font(.caption).monospacedDigit()
                }
                if frac > 0.8 {
                    Label("near the window — consider a handoff", systemImage: "exclamationmark.triangle")
                        .font(.caption).foregroundStyle(.orange)
                }
            } else {
                Text("ยังไม่มีเทิร์น").font(.caption).foregroundStyle(.secondary)
            }

            Button {
                engine.assembledContext { text, tokens in
                    inspected = (text, tokens)
                }
            } label: {
                Label("ดูสิ่งที่ agent เห็นจริง ๆ", systemImage: "doc.text.magnifyingglass")
            }
            .buttonStyle(.bordered)
            .controlSize(.small)
            .disabled(engine.sessionID < 0)

            // The tier the design never trims. Showing it is also the cheapest
            // way to notice if it ever goes empty again.
            if !engine.taskState.isEmpty {
                Divider()
                Text("Task state").font(.caption.weight(.semibold))
                ScrollView {
                    Text(engine.taskState)
                        .font(.caption.monospaced())
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 160)
            }

            Spacer()

            // The warm tier catches up a little after every turn; this is just
            // so a backlog is visible rather than mysterious.
            HStack(spacing: 6) {
                Text("\(engine.events.count) events")
                if engine.uncompacted > 0 {
                    Text("·")
                    Text("\(engine.uncompacted) to summarise")
                    Button("Now") { engine.compactNow(limit: 500) }
                        .buttonStyle(.link)
                        .disabled(engine.busy)
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        }
        .padding(12)
        .frame(maxHeight: .infinity, alignment: .top)
    }
}

struct EventRow: View {
    let event: LedgerEvent

    private var isUser: Bool { event.actor == "user" }

    var body: some View {
        switch event.kind {
        case "msg":
            message
        case "handoff":
            if let b = HandoffBrief(json: event.body) {
                card(tint: .purple) { handoffBody(b) }
            } else {
                note(event.body)
            }
        case "tool_call", "tool_result", "diff":
            toolRow
        default:
            note(event.body)
        }
    }

    // User: right-aligned bubble. Agent: plain text under its name, like the
    // chat panes people already know from Claude / Codex / Cursor.
    private var message: some View {
        VStack(alignment: isUser ? .trailing : .leading, spacing: 4) {
            if !isUser {
                HStack(spacing: 5) {
                    Circle().fill(agentColor).frame(width: 7, height: 7)
                    Text(event.actor).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                }
            }
            MarkdownText(text: event.body)
                .padding(.horizontal, isUser ? 12 : 0).padding(.vertical, isUser ? 8 : 0)
                .background(isUser ? Color.accentColor.opacity(0.14) : .clear,
                            in: RoundedRectangle(cornerRadius: 12))
        }
        .frame(maxWidth: .infinity, alignment: isUser ? .trailing : .leading)
    }

    /// One collapsed line per tool call; click to see the payload.
    private var toolRow: some View {
        DisclosureGroup {
            Text(event.body)
                .font(.caption2.monospaced())
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, 4)
        } label: {
            HStack(spacing: 6) {
                Image(systemName: event.kind == "diff" ? "plusminus" : "wrench.and.screwdriver")
                    .font(.caption2).foregroundStyle(.secondary)
                Text(firstLine).font(.caption).lineLimit(1).foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, 8).padding(.vertical, 5)
        .background(.secondary.opacity(0.07), in: RoundedRectangle(cornerRadius: 8))
    }

    private var firstLine: String {
        let l = event.body.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: true).first.map(String.init) ?? event.kind
        return l.count > 110 ? String(l.prefix(110)) + "…" : l
    }

    private func note(_ t: String) -> some View {
        Text(t).font(.caption).foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func card<C: View>(tint: Color, @ViewBuilder _ c: () -> C) -> some View {
        c().padding(10)
            .background(tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 10))
    }

    private var agentColor: Color {
        let palette: [Color] = [.green, .orange, .teal, .pink, .indigo]
        return palette[abs(event.actor.hashValue) % palette.count]
    }

    /// The handoff is the product's whole argument, so it gets read as a
    /// moment — who fell over, why, and what the next agent was told — rather
    /// than as the brief's raw JSON.
    @ViewBuilder
    private func handoffBody(_ b: HandoffBrief) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 6) {
                Text(b.from).font(.caption.bold())
                Image(systemName: "arrow.right").font(.caption2)
                Text(b.to).font(.caption.bold())
                Text(b.reason)
                    .font(.caption2)
                    .padding(.horizontal, 5).padding(.vertical, 1)
                    .background(.orange.opacity(0.25), in: Capsule())
            }
            if !b.goal.isEmpty {
                Text("goal · \(b.goal)").font(.caption)
            }
            HStack(spacing: 10) {
                if !b.done.isEmpty { Text("done \(b.done.count)") }
                if !b.pending.isEmpty { Text("pending \(b.pending.count)") }
                if !b.files.isEmpty { Text("files \(b.files.count)") }
                if b.diffBytes > 0 { Text("diff \(b.diffBytes)B") }
            }
            .font(.caption2).foregroundStyle(.secondary)
            DisclosureGroup("ดู brief ที่ส่งต่อ") {
                Text(event.body)
                    .font(.caption2.monospaced())
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .font(.caption2)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

}

struct CompareResultsSheet: View {
    let json: String
    @Environment(\.dismiss) private var dismiss

    struct Branch: Codable, Identifiable {
        let agent: String
        let session_id: Int64
        let message: String
        let stop_reason: String
        let error: String?
        var id: Int64 { session_id }
    }
    struct Payload: Codable { let ok: Bool?; let branches: [Branch]? ; let error: String? }

    private var payload: Payload? {
        guard let d = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(Payload.self, from: d)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Compare").font(.title3.bold())
            if let err = payload?.error {
                Text(err).foregroundStyle(.red)
            }
            ScrollView {
                HStack(alignment: .top, spacing: 12) {
                    ForEach(payload?.branches ?? []) { b in
                        VStack(alignment: .leading, spacing: 6) {
                            Text(b.agent).font(.headline)
                            Text("resume: --session \(b.session_id)")
                                .font(.caption2).foregroundStyle(.secondary)
                            Divider()
                            if let e = b.error {
                                Text(e).font(.caption).foregroundStyle(.red)
                            } else {
                                Text(b.message).font(.callout).textSelection(.enabled)
                                Text("[\(b.stop_reason)]").font(.caption2).foregroundStyle(.tertiary)
                            }
                        }
                        .frame(width: 260, alignment: .leading)
                        .padding(10)
                        .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 8))
                    }
                }
            }
            HStack { Spacer(); Button("Done") { dismiss() }.keyboardShortcut(.defaultAction) }
        }
        .padding(20)
        .frame(minWidth: 560, minHeight: 320)
    }
}

/// A minimal unified-diff model: each hunk carries the file preamble it
/// needs to be a valid standalone patch.
struct DiffHunk: Identifiable {
    let id = UUID()
    let file: String
    let preamble: String   // "diff --git … / --- … / +++ …"
    let header: String     // "@@ -a,b +c,d @@ …"
    let lines: [String]    // body lines (context / +/-)

    var patch: String { preamble + header + "\n" + lines.joined(separator: "\n") + "\n" }
    var addCount: Int { lines.filter { $0.hasPrefix("+") }.count }
    var delCount: Int { lines.filter { $0.hasPrefix("-") }.count }
}

func parseHunks(_ diff: String) -> [DiffHunk] {
    var hunks: [DiffHunk] = []
    var file = ""
    var preamble = ""
    var inPreamble = false
    var curHeader: String?
    var curLines: [String] = []

    func flush() {
        if let h = curHeader {
            hunks.append(DiffHunk(file: file, preamble: preamble, header: h, lines: curLines))
        }
        curHeader = nil
        curLines = []
    }

    for line in diff.split(separator: "\n", omittingEmptySubsequences: false).map(String.init) {
        if line.hasPrefix("diff --git") {
            flush()
            preamble = line + "\n"
            inPreamble = true
            if let r = line.range(of: " b/") { file = String(line[r.upperBound...]) }
        } else if inPreamble && (line.hasPrefix("index ") || line.hasPrefix("--- ") || line.hasPrefix("+++ ")
                                 || line.hasPrefix("new file") || line.hasPrefix("deleted file")
                                 || line.hasPrefix("similarity") || line.hasPrefix("rename ")) {
            preamble += line + "\n"
        } else if line.hasPrefix("@@") {
            flush()
            inPreamble = false
            curHeader = line
        } else if curHeader != nil {
            curLines.append(line)
        }
    }
    flush()
    return hunks
}

struct HunkView: View {
    let hunk: DiffHunk
    let rejected: Bool
    let toggle: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text(hunk.file).font(.caption.bold())
                Text("+\(hunk.addCount) −\(hunk.delCount)").font(.caption2).foregroundStyle(.secondary)
                Spacer()
                Button(rejected ? "rejected" : "keep") { toggle() }
                    .controlSize(.mini)
                    .buttonStyle(.bordered)
                    .tint(rejected ? .red : .green)
            }
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(hunk.lines.prefix(40).enumerated()), id: \.offset) { _, l in
                    Text(l).font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(l.hasPrefix("+") ? .green : l.hasPrefix("-") ? .red : .secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding(4)
            .background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 4))
            .opacity(rejected ? 0.45 : 1)
        }
    }
}

/// Just enough of the handoff brief to render the moment it describes.
struct HandoffBrief {
    let goal: String
    let done: [String]
    let pending: [String]
    let files: [String]
    let diffBytes: Int
    let from: String
    let to: String
    let reason: String

    init?(json: String) {
        guard let data = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        let meta = obj["handoff"] as? [String: Any] ?? [:]
        goal = obj["goal"] as? String ?? ""
        done = obj["done"] as? [String] ?? []
        pending = obj["pending"] as? [String] ?? []
        files = obj["files"] as? [String] ?? []
        diffBytes = (obj["diff"] as? String)?.utf8.count ?? 0
        from = meta["from"] as? String ?? "?"
        to = meta["to"] as? String ?? "?"
        reason = (meta["reason"] as? String ?? "handoff").replacingOccurrences(of: "_", with: " ")
    }
}

/// Everything that decides *whether Chorus can run at all* in one place:
/// which agents are installed, which are logged in, and the one command that
/// fixes each. The app cannot complete an OAuth flow for you, so it does the
/// next most useful thing — opens Terminal on the exact command and then lets
/// you re-check without restarting.
struct SettingsSheet: View {
    @EnvironmentObject var engine: Engine
    @Environment(\.dismiss) private var dismiss
    let workspace: String
    @State private var tab = Tab.agents
    @State private var skillQuery = ""

    enum Tab: String, CaseIterable { case agents = "เอเจนต์", skills = "Skills" }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text("ตั้งค่า").font(.title3.bold())
                    Text("Chorus เรียก CLI ของแต่ละเจ้าผ่าน ACP — ต้อง login ที่ CLI นั้น ไม่ใช่ที่นี่")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button("เสร็จ") { dismiss() }.keyboardShortcut(.defaultAction)
            }

            Picker("", selection: $tab) {
                ForEach(Tab.allCases, id: \.self) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented).labelsHidden()

            if tab == .skills { skillsTab } else { agentsTab }
        }
        .padding(18)
        .frame(width: 560, height: 560)
        .onAppear { engine.loadAgents(); engine.loadSkills(cwd: workspace) }
    }

    private var skillsTab: some View {
        let q = skillQuery.trimmingCharacters(in: .whitespaces)
        let shown = engine.skills.filter {
            q.isEmpty || $0.name.localizedCaseInsensitiveContains(q) || $0.description.localizedCaseInsensitiveContains(q)
        }
        let titles = ["chorus": "Chorus — ส่งให้ทุกเอเจนต์", "project": "โปรเจกต์นี้ (.claude/skills)", "user": "ของคุณ (~/.claude/skills)"]
        return VStack(alignment: .leading, spacing: 8) {
            TextField("ค้นหา skill…", text: $skillQuery).textFieldStyle(.roundedBorder)
            Text("\(engine.skills.count) skill ที่เอเจนต์ในโฟลเดอร์นี้มองเห็น")
                .font(.caption).foregroundStyle(.secondary)
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    ForEach(["chorus", "project", "user"], id: \.self) { src in
                        let group = shown.filter { $0.source == src }
                        if !group.isEmpty {
                            Text(titles[src] ?? src).font(.subheadline.bold())
                            ForEach(group) { sk in
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(sk.name).font(.callout.weight(.medium))
                                    Text(sk.description).font(.caption).foregroundStyle(.secondary)
                                        .lineLimit(3)
                                }
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(8)
                                .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 6))
                                .help(sk.path)
                            }
                        }
                    }
                    if shown.isEmpty {
                        Text("ไม่พบ skill — วางไว้ที่ .chorus/skills/<ชื่อ>/SKILL.md เพื่อให้ทุกเอเจนต์ใช้ร่วมกัน")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
        }
    }

    private var agentsTab: some View {
        VStack(alignment: .leading, spacing: 14) {
            ForEach(engine.agents) { a in
                VStack(alignment: .leading, spacing: 7) {
                    HStack(spacing: 8) {
                        Circle()
                            .fill(a.ready ? Color.green : (a.auth == "needs_login" ? .orange : .red))
                            .frame(width: 8, height: 8)
                        Text(a.label).font(.headline)
                        Spacer()
                        Text(statusLabel(a.auth))
                            .font(.caption)
                            .foregroundStyle(a.ready ? .secondary : .primary)
                    }
                    Text(a.detail).font(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)

                    if !a.models.isEmpty {
                        Text("โมเดลที่เลือกได้").font(.caption2).foregroundStyle(.tertiary)
                        Text(a.models.map(\.label).joined(separator: " · "))
                            .font(.caption.monospaced())
                            .fixedSize(horizontal: false, vertical: true)
                    }

                    if !a.ready {
                        HStack(spacing: 8) {
                            Text(a.hint)
                                .font(.caption.monospaced())
                                .padding(.horizontal, 6).padding(.vertical, 3)
                                .background(.quaternary, in: RoundedRectangle(cornerRadius: 4))
                                .textSelection(.enabled)
                            Button("เปิด Terminal แล้วรันให้") { engine.runLoginCommand(a.hint) }
                                .controlSize(.small)
                            Button("ตรวจใหม่") { engine.loadAgents() }
                                .controlSize(.small)
                            Spacer(minLength: 0)
                        }
                    }
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 8))
            }

            Text("เปลี่ยนรายการโมเดลเองได้ด้วย CHORUS_CLAUDE_MODELS / CHORUS_CODEX_MODELS "
                 + "(รูปแบบ id=ชื่อ คั่นด้วยจุลภาค) — ACP ไม่มีวิธีถามรายการโมเดลจาก agent "
                 + "รายการนี้เลยเป็นของที่เราดูแลเอง ไม่ใช่ของที่ provider บอกมา")
                .font(.caption2).foregroundStyle(.tertiary)
                .fixedSize(horizontal: false, vertical: true)

            Spacer(minLength: 0)
        }
    }

    private func statusLabel(_ auth: String) -> String {
        switch auth {
        case "ok": "พร้อมใช้"
        case "needs_login": "ยังไม่ได้ login"
        default: "ยังไม่ได้ติดตั้ง CLI"
        }
    }
}


/// ⌘K: type to filter, ↩ runs the first match.
struct PaletteSheet: View {
    let items: [(title: String, icon: String, run: () -> Void)]
    let dismiss: () -> Void
    @State private var query = ""

    private var shown: [(title: String, icon: String, run: () -> Void)] {
        query.isEmpty ? items : items.filter { $0.title.localizedCaseInsensitiveContains(query) }
    }

    var body: some View {
        VStack(spacing: 0) {
            TextField("พิมพ์คำสั่ง…", text: $query)
                .textFieldStyle(.plain).font(.title3).padding(14)
                .onSubmit { if let f = shown.first { dismiss(); f.run() } }
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(Array(shown.enumerated()), id: \.offset) { _, item in
                        Button { dismiss(); item.run() } label: {
                            Label(item.title, systemImage: item.icon)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(.horizontal, 12).padding(.vertical, 7)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }.padding(6)
            }
        }
        .frame(width: 460, height: 320)
    }
}


/// Markdown for agent output: fenced code becomes a monospaced block, the rest
/// goes through `AttributedString` (bold, italic, inline code, links) with
/// whitespace preserved so lists and paragraphs keep their shape.
struct MarkdownText: View {
    let text: String

    private var parts: [(code: Bool, body: String)] {
        var out: [(Bool, String)] = []
        for (i, chunk) in text.components(separatedBy: "```").enumerated() {
            guard !chunk.isEmpty else { continue }
            if i % 2 == 1 {
                // Drop the language tag on the first line.
                let lines = chunk.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: false)
                out.append((true, lines.count > 1 ? String(lines[1]) : chunk))
            } else {
                out.append((false, chunk.trimmingCharacters(in: .newlines)))
            }
        }
        return out
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(parts.enumerated()), id: \.offset) { _, p in
                if p.code {
                    ScrollView(.horizontal) {
                        Text(p.body.trimmingCharacters(in: .newlines))
                            .font(.callout.monospaced()).textSelection(.enabled)
                            .padding(10)
                    }
                    .background(.secondary.opacity(0.12), in: RoundedRectangle(cornerRadius: 8))
                } else if let a = try? AttributedString(
                    markdown: p.body,
                    options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)) {
                    Text(a).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                } else {
                    Text(p.body).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
    }
}



/// Collapsible reasoning. Open while the agent is still thinking, collapsed
/// to one line once it starts answering.
struct ThinkingBlock: View {
    let text: String
    let running: Bool
    @State private var open: Bool?

    var body: some View {
        let isOpen = open ?? running
        VStack(alignment: .leading, spacing: 6) {
            Button { open = !isOpen } label: {
                HStack(spacing: 6) {
                    Image(systemName: "brain").foregroundStyle(.purple)
                    Text(running ? "กำลังคิด…" : "ความคิด").font(.caption.weight(.semibold))
                    Text("\(text.count) ตัวอักษร").font(.caption2).foregroundStyle(.tertiary)
                    Image(systemName: isOpen ? "chevron.up" : "chevron.down").font(.caption2)
                    Spacer()
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if isOpen {
                ScrollViewReader { proxy in
                    ScrollView {
                        Text(text)
                            .font(.caption).foregroundStyle(.secondary)
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Color.clear.frame(height: 1).id("end")
                    }
                    .frame(maxHeight: 160)
                    .onChange(of: text.count) { _, _ in if running { proxy.scrollTo("end") } }
                }
            }
        }
        .padding(8)
        .background(Color.purple.opacity(0.06), in: RoundedRectangle(cornerRadius: 8))
    }
}
