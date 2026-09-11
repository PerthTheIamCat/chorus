import SwiftUI

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
    @State private var draft = ""
    @State private var agent = "codex"
    @State private var allow = false
    @State private var sendMode = SendMode.single
    @State private var chainAgents = "codex,claude-opus"
    @State private var compareOutput: String?

    enum SendMode: String, CaseIterable { case single = "Single", relay = "Relay", compare = "Compare", ask = "Ask each" }
    @State private var showHandoff = false
    @State private var rightTab = RightTab.context
    @State private var confirmRevert = false
    @State private var rejected: Set<UUID> = []

    enum RightTab: String, CaseIterable { case context = "Context", changes = "Changes", memory = "Memory" }

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
                    case .changes: changesPane
                    case .memory: memoryPane
                    }
                }
                .frame(width: 300)
            }
        }
        .toolbar {
            ToolbarItem(placement: .principal) {
                Text(engine.sessionID >= 0 ? "session \(engine.sessionID)" : "no session")
                    .font(.headline)
            }
        }
        .sheet(isPresented: $showHandoff) {
            HandoffSheet().environmentObject(engine)
        }
        .sheet(isPresented: Binding(get: { compareOutput != nil }, set: { if !$0 { compareOutput = nil } })) {
            CompareResultsSheet(json: compareOutput ?? "")
        }
        .sheet(item: $engine.pendingAsk) { ask in
            VStack(alignment: .leading, spacing: 14) {
                Text("Permission").font(.title3.bold())
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

    private var sidebar: some View {
        Form {
            Section("Workspace") {
                TextField("/path/to/a/small/project", text: $workspace)
                    .textFieldStyle(.roundedBorder)
                Text("A real project folder — the first turn snapshots the whole tree, "
                     + "so avoid your home directory or anything iCloud-synced.")
                    .font(.caption2).foregroundStyle(.secondary)
                Button {
                    let db = (workspace as NSString)
                        .appendingPathComponent(".chorus/store.db")
                    try? FileManager.default.createDirectory(
                        atPath: (db as NSString).deletingLastPathComponent,
                        withIntermediateDirectories: true)
                    engine.open(db: db, cwd: workspace, title: "session")
                } label: {
                    if engine.busy {
                        HStack(spacing: 6) {
                            ProgressView().controlSize(.small)
                            Text("Snapshotting workspace…")
                        }
                    } else {
                        Text("Start session here")
                    }
                }
                .disabled(engine.busy || workspace.trimmingCharacters(in: .whitespaces).isEmpty)
            }
            Section("Agent") {
                Picker("send as", selection: $sendMode) {
                    ForEach(SendMode.allCases, id: \.self) { Text($0.rawValue).tag($0) }
                }
                if sendMode == .single {
                    Picker("current", selection: $agent) {
                        Text("codex").tag("codex")
                        Text("claude-opus").tag("claude-opus")
                        Text("gemini").tag("gemini")
                    }
                } else {
                    TextField("chain (comma-separated)", text: $chainAgents)
                        .textFieldStyle(.roundedBorder)
                    Text(sendMode == .relay
                         ? "tries each in order; hands off on rate limit / quota"
                         : "forks per agent, runs the same prompt, compare the diffs")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Toggle("Allow file writes / commands", isOn: $allow)
            }
            if engine.sessionID >= 0 {
                Section {
                    Button("Hand off…") { showHandoff = true }
                        .disabled(engine.busy)
                }
            }
        }
        .navigationTitle("Chorus")
        .frame(minWidth: 240)
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
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 10) {
                        ForEach(engine.events) { ev in
                            EventRow(event: ev).id(ev.seq)
                        }
                    }
                    .padding(12)
                }
                .onChange(of: engine.events.count) { _ in
                    if let last = engine.events.last { proxy.scrollTo(last.seq, anchor: .bottom) }
                }
            }
            Divider()
            composer
        }
        .frame(minWidth: 380)
    }

    private var composer: some View {
        HStack(alignment: .bottom, spacing: 8) {
            TextEditor(text: $draft)
                .frame(minHeight: 44, maxHeight: 120)
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(.quaternary))
            Button {
                let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !text.isEmpty else { return }
                draft = ""
                switch sendMode {
                case .single:
                    engine.send(text, agent: agent, allow: allow)
                case .ask:
                    engine.sendInteractive(text, agent: agent)
                case .relay:
                    engine.sendRelay(text, agents: chainAgents, allow: allow)
                case .compare:
                    engine.compare(text, agents: chainAgents, allow: allow) { compareOutput = $0 }
                }
            } label: {
                if engine.busy { ProgressView().controlSize(.small) }
                else { Image(systemName: "arrow.up.circle.fill").font(.title2) }
            }
            .buttonStyle(.plain)
            .disabled(engine.busy || engine.sessionID < 0)
        }
        .padding(10)
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

    private var contextPane: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Context").font(.headline)
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
                Text("no turn yet").font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            Text("\(engine.events.count) events").font(.caption).foregroundStyle(.secondary)
        }
        .padding(12)
        .frame(maxHeight: .infinity, alignment: .top)
    }
}

struct EventRow: View {
    let event: LedgerEvent

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                Text(event.actor).font(.caption.bold())
                Text(event.kind).font(.caption2)
                    .padding(.horizontal, 5).padding(.vertical, 1)
                    .background(.quaternary, in: Capsule())
                Spacer()
                Text("#\(event.seq)").font(.caption2).foregroundStyle(.tertiary)
            }
            Text(event.body)
                .font(event.kind == "msg" ? .body : .caption.monospaced())
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(8)
        .background(rowColor, in: RoundedRectangle(cornerRadius: 6))
    }

    private var rowColor: Color {
        switch event.kind {
        case "handoff": return .purple.opacity(0.12)
        case "diff": return .green.opacity(0.10)
        case "tool_call", "tool_result": return .secondary.opacity(0.08)
        default: return event.actor == "user" ? .blue.opacity(0.08) : .clear
        }
    }
}

struct HandoffSheet: View {
    @EnvironmentObject var engine: Engine
    @Environment(\.dismiss) private var dismiss

    @State private var to = "codex"
    @State private var reason = "rate_limit"
    @State private var goal = ""
    @State private var pending = ""
    @State private var echo: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Hand off the session").font(.title3.bold())
            HStack {
                Picker("to", selection: $to) {
                    Text("codex").tag("codex")
                    Text("claude-opus").tag("claude-opus")
                    Text("gemini").tag("gemini")
                }
                Picker("reason", selection: $reason) {
                    Text("rate limit").tag("rate_limit")
                    Text("quota").tag("quota")
                    Text("context full").tag("context")
                    Text("my choice").tag("manual")
                }
            }
            TextField("goal for the next agent", text: $goal)
                .textFieldStyle(.roundedBorder)
            TextField("pending (one per line)", text: $pending, axis: .vertical)
                .lineLimit(2...5)
                .textFieldStyle(.roundedBorder)

            if let echo {
                GroupBox("Incoming agent understood:") {
                    Text(echo).font(.callout).frame(maxWidth: .infinity, alignment: .leading)
                }
            }

            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                Button(engine.busy ? "Handing off…" : "Hand off") {
                    engine.handoff(
                        to: to, reason: reason, goal: goal,
                        done: [],
                        pending: pending.split(separator: "\n").map(String.init),
                        allow: false
                    ) { result in
                        echo = result?.echo_back
                    }
                }
                .keyboardShortcut(.defaultAction)
                .disabled(goal.isEmpty || engine.busy)
            }
        }
        .padding(20)
        .frame(width: 480)
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
