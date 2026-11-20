import SwiftUI

// Design board "Session" — the inspector: five text tabs (งาน · ที่แก้ · ความจำ ·
// คอนเท็กซ์ · ย้อน, ⌥⌘1–5), each a quiet column of sections. Everything shown is
// already computed by Rust; this only lays it out.

struct InspectorView: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 2) {
                ForEach(InspectorTab.allCases) { t in
                    Button { shell.inspectorTab = t } label: {
                        HStack(spacing: 5) {
                            Text(t.title)
                            if let n = badge(t), n > 0 {
                                Text("\(n)").font(.system(size: 10.5, weight: .semibold))
                                    .foregroundStyle(t == .memory ? Color.black.opacity(0.8) : .white)
                                    .padding(.horizontal, 5).frame(height: 16)
                                    .background(t == .memory ? Palette.warn : Palette.accentButton, in: Capsule())
                            }
                        }
                        .font(.system(size: 12.5, weight: shell.inspectorTab == t ? .medium : .regular))
                        .foregroundStyle(shell.inspectorTab == t ? Palette.text : Palette.text2)
                        .padding(.horizontal, 9).frame(height: 28)
                        .background(shell.inspectorTab == t ? Palette.selected : .clear,
                                    in: RoundedRectangle(cornerRadius: 7, style: .continuous))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(shell.inspectorTab == t ? .isSelected : [])
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 10).frame(height: 44)
            Hairline()
            Group {
                switch shell.inspectorTab {
                case .task: TaskTab()
                case .changes: ChangesTab()
                case .memory: MemoryTab()
                case .context: ContextTab()
                case .rewind: RewindTab()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        }
        .background(Palette.sidebar)
    }

    private func badge(_ t: InspectorTab) -> Int? {
        switch t {
        case .changes: engine.diffFiles.count
        case .memory: engine.pending.count
        default: nil
        }
    }
}

/// "เป้าหมาย", "ความคืบหน้า · 3 / 5" — the section label of the inspector.
private struct SectionTitle: View {
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View { GroupHeader(text) }
}

// MARK: งาน

private struct TaskTab: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                let sections = TaskSection.parse(engine.taskState)
                if sections.isEmpty && engine.checklist.isEmpty {
                    VStack(spacing: 8) {
                        Image(systemName: "checklist").font(.system(size: 22)).foregroundStyle(Palette.text3)
                        Text("ยังไม่มีงาน").font(Typo.headline)
                        Text("เป้าหมายและสิ่งที่ค้างจะขึ้นหลังข้อความแรก").font(Typo.caption).foregroundStyle(Palette.text3)
                    }
                    .frame(maxWidth: .infinity).padding(.top, 40)
                }
                ForEach(Array(sections.enumerated()), id: \.offset) { _, s in
                    VStack(alignment: .leading, spacing: 8) {
                        SectionTitle(s.title)
                        ForEach(Array(s.items.enumerated()), id: \.offset) { _, item in
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                if s.kind != .plain {
                                    Image(systemName: s.symbol).font(.system(size: 11, weight: .semibold)).foregroundStyle(s.tint)
                                }
                                Text(item).font(Typo.ui).foregroundStyle(s.kind == .done ? Palette.text2 : Palette.text)
                                    .lineSpacing(2).textSelection(.enabled)
                            }
                        }
                    }
                }
                if !engine.checklist.isEmpty {
                    let done = engine.checklist.filter(\.done).count
                    VStack(alignment: .leading, spacing: 8) {
                        SectionTitle("ความคืบหน้า · \(done) / \(engine.checklist.count)")
                        ForEach(engine.checklist) { item in
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                Group {
                                    if item.done {
                                        Image(systemName: "checkmark").font(.system(size: 11, weight: .bold)).foregroundStyle(Palette.ok)
                                    } else if item.in_progress {
                                        Spinner(size: 11)
                                    } else {
                                        Circle().strokeBorder(Palette.text3, lineWidth: 1.5).frame(width: 11, height: 11)
                                    }
                                }
                                .frame(width: 13)
                                Text(item.text).font(Typo.ui).foregroundStyle(item.done ? Palette.text2 : Palette.text)
                            }
                        }
                    }
                }
                participants
                subagents
            }
            .padding(16)
        }
    }

    @ViewBuilder
    private var participants: some View {
        let msgs = engine.events.filter { $0.kind == "msg" && $0.actor != "user" }
        let agents = ProviderStack.unique(msgs.map(\.actor))
        if !agents.isEmpty {
            let max = agents.map { a in msgs.filter { ProviderTint.provider(of: $0.actor) == a }.count }.max() ?? 1
            VStack(alignment: .leading, spacing: 10) {
                SectionTitle("ใครทำอะไร")
                ForEach(agents, id: \.self) { a in
                    let n = msgs.filter { ProviderTint.provider(of: $0.actor) == a }.count
                    HStack(spacing: 10) {
                        ProviderAvatar(provider: a, size: 20)
                        Text(shell.agentLabel(a)).font(Typo.ui).frame(width: 70, alignment: .leading)
                        Meter(value: Double(n) / Double(Swift.max(max, 1)), tint: Palette.accent)
                        Text("\(n) เทิร์น").font(Typo.tiny.monospacedDigit()).foregroundStyle(Palette.text3)
                            .frame(width: 54, alignment: .trailing)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var subagents: some View {
        let subs = engine.subagentsByParent[engine.sessionID] ?? []
        if !subs.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                SectionTitle("งานย่อย")
                ForEach(subs) { sub in
                    Button { if let c = sub.child_sess { shell.open(SessionKey(folder: shell.workspace, id: c)) } } label: {
                        HStack(spacing: 8) {
                            if let m = sub.model { ProviderIcon(provider: m, size: 13) }
                            Text(sub.task.isEmpty ? sub.role : "\(sub.role) · \(sub.task)").lineLimit(1)
                            Spacer()
                            Tag(text: SubStatus.label(sub.status), tone: SubStatus.tone(sub.status))
                        }
                        .padding(.horizontal, 10).frame(height: 36)
                        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Palette.line))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }
}

enum SubStatus {
    static func label(_ s: String) -> String {
        switch s {
        case "running": "กำลังทำ"
        case "waiting": "รอ"
        case "failed": "ล้มเหลว"
        case "cancelled": "ยกเลิก"
        default: "เสร็จ"
        }
    }
    static func tone(_ s: String) -> Tag.Tone {
        switch s {
        case "running": .accent
        case "waiting": .warn
        case "failed": .bad
        case "cancelled": .neutral
        default: .ok
        }
    }
}

/// Splits the task-state text Rust renders into labelled lists.
/// Handles `done: a | b` and a `done:` header followed by `- item` lines.
struct TaskSection {
    enum Kind { case goal, done, pending, constraints, plain }
    let title: String
    let kind: Kind
    var items: [String]

    var symbol: String {
        switch kind {
        case .goal: "target"
        case .done: "checkmark"
        case .pending: "circle"
        case .constraints: "exclamationmark.shield"
        case .plain: "circle"
        }
    }
    var tint: Color {
        switch kind {
        case .goal: Palette.accent
        case .done: Palette.ok
        case .pending: Palette.text3
        case .constraints: Palette.warn
        case .plain: Palette.text3
        }
    }

    static func parse(_ text: String) -> [TaskSection] {
        var out: [TaskSection] = []
        func items(_ s: Substring) -> [String] {
            s.split(separator: "|").map { $0.trimmingCharacters(in: .whitespaces) }
                .map { $0.hasPrefix("- ") ? String($0.dropFirst(2)) : $0 }.filter { !$0.isEmpty }
        }
        for raw in text.split(whereSeparator: \.isNewline) {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line.isEmpty { continue }
            if line.hasPrefix("- ") || line.hasPrefix("• ") {
                if out.isEmpty { out.append(section("")) }
                out[out.count - 1].items.append(String(line.dropFirst(2)).trimmingCharacters(in: .whitespaces))
                continue
            }
            let known = ["goal", "done", "pending", "constraints"]
            if let colon = line.firstIndex(of: ":"),
               !line[..<colon].contains(" ") || known.contains(line[..<colon].lowercased()) {
                var s = section(line[..<colon].lowercased())
                s.items = items(line[line.index(after: colon)...])
                out.append(s)
            } else {
                if out.isEmpty { out.append(section("")) }
                out[out.count - 1].items.append(line)
            }
        }
        return out.filter { !$0.items.isEmpty }
    }

    private static func section(_ key: String) -> TaskSection {
        switch key {
        case "goal": TaskSection(title: "เป้าหมาย", kind: .goal, items: [])
        case "done": TaskSection(title: "เสร็จแล้ว", kind: .done, items: [])
        case "pending": TaskSection(title: "ค้างอยู่", kind: .pending, items: [])
        case "constraints": TaskSection(title: "ข้อจำกัด", kind: .constraints, items: [])
        default: TaskSection(title: key, kind: .plain, items: [])
        }
    }
}

// MARK: ที่แก้

private struct ChangesTab: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        if engine.diffFiles.isEmpty {
            VStack(spacing: 8) {
                Image(systemName: "doc.badge.ellipsis").font(.system(size: 22)).foregroundStyle(Palette.text3)
                Text("ยังไม่มีไฟล์ที่แก้").font(Typo.headline)
                Text("เมื่อ agent แก้ไฟล์ จะเห็นตรงนี้ทีละก้อน").font(Typo.caption).foregroundStyle(Palette.text3)
            }
            .frame(maxWidth: .infinity).padding(.top, 40)
        } else {
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    HStack {
                        Text("\(engine.diffFiles.count) ไฟล์ · \(engine.diffHunks.count) ก้อน").font(Typo.caption).foregroundStyle(Palette.text3)
                        Spacer()
                        Button("รีวิวในโต๊ะงาน") { shell.stage = .workbench; shell.workbenchTab = .changes }
                            .buttonStyle(QuietButtonStyle(small: true))
                    }
                    ForEach(engine.diffFiles, id: \.self) { file in
                        let hunks = engine.diffHunks.filter { $0.file == file }
                        Button { shell.stage = .workbench; shell.workbenchTab = .changes } label: {
                            HStack(spacing: 8) {
                                Text(file).font(Typo.monoSmall).foregroundStyle(Palette.text).lineLimit(1).truncationMode(.head)
                                if engine.testFiles.contains(file) { Tag(text: "แก้เทสต์", tone: .warn) }
                                Spacer()
                                Text("+\(hunks.reduce(0) { $0 + $1.addCount })").foregroundStyle(Palette.ok)
                                Text("−\(hunks.reduce(0) { $0 + $1.delCount })").foregroundStyle(Palette.bad)
                            }
                            .font(Typo.monoSmall)
                            .padding(.horizontal, 10).frame(height: 34)
                            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Palette.line))
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(16)
            }
        }
    }
}

// MARK: ความจำ

private struct MemoryTab: View {
    @EnvironmentObject private var engine: Engine

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                if !engine.pending.isEmpty {
                    SectionTitle("agent เสนอให้จำ · \(engine.pending.count)")
                    ForEach(engine.pending) { p in
                        VStack(alignment: .leading, spacing: 8) {
                            Text(p.text).font(Typo.ui).textSelection(.enabled)
                            if p.replaces != nil { Tag(text: "แทนข้อเดิม", tone: .warn) }
                            if p.suspicious == true {
                                Label("มาจากของภายนอก ตรวจก่อนรับ", systemImage: "exclamationmark.shield")
                                    .font(Typo.caption).foregroundStyle(Palette.warn)
                            }
                            if let e = p.evidence, !e.isEmpty {
                                Text(e).font(Typo.caption).foregroundStyle(Palette.text3).lineLimit(3)
                            }
                            HStack {
                                Spacer()
                                Button("ทิ้ง") { engine.review(p.id, approve: false) }.buttonStyle(QuietButtonStyle(small: true))
                                Button(p.kind == "retire" ? "เลิกจำ" : "จำ") { engine.review(p.id, approve: true) }
                                    .buttonStyle(PrimaryButtonStyle(small: true))
                            }
                        }
                        .padding(12)
                        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Palette.line2))
                    }
                }
                SectionTitle("จำไว้แล้ว · \(engine.facts.count)")
                if engine.facts.isEmpty {
                    Text("ยังไม่มี agent จะเสนอสิ่งที่ควรจำเมื่อเจอ").font(Typo.caption).foregroundStyle(Palette.text3)
                }
                ForEach(Array(engine.facts.enumerated()), id: \.offset) { _, f in
                    Text(f).font(Typo.ui).foregroundStyle(Palette.text2).textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                Text("ฉายลง CLAUDE.md · AGENTS.md ให้อัตโนมัติ\(engine.memoryScope.map { " · โปรเจกต์ \($0)" } ?? "")")
                    .font(Typo.tiny).foregroundStyle(Palette.text3)
            }
            .padding(16)
        }
        .onAppear { engine.refreshMemory() }
    }
}

// MARK: คอนเท็กซ์

private struct ContextTab: View {
    @EnvironmentObject private var engine: Engine
    @State private var inspection: ContextInspection?
    @State private var showRaw = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                if let r = inspection?.report {
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text(r.total_tokens.formatted()).font(.system(size: 22, weight: .medium, design: .monospaced))
                        Text("/ \(compact(r.budget)) token ที่ agent จะเห็น").font(Typo.caption).foregroundStyle(Palette.text3)
                    }
                    let slices = Self.slices(r)
                    GeometryReader { g in
                        HStack(spacing: 2) {
                            ForEach(slices.filter { $0.tokens > 0 }, id: \.title) { s in
                                s.color.frame(width: max(2, g.size.width * CGFloat(s.tokens) / CGFloat(max(r.budget, 1))))
                            }
                            Spacer(minLength: 0)
                        }
                        .background(Palette.line2)
                        .clipShape(RoundedRectangle(cornerRadius: 5))
                    }
                    .frame(height: 10)
                    VStack(spacing: 8) {
                        ForEach(slices, id: \.title) { s in
                            HStack(spacing: 8) {
                                RoundedRectangle(cornerRadius: 2).fill(s.color).frame(width: 9, height: 9)
                                Text(s.title).font(Typo.small).foregroundStyle(Palette.text2)
                                Spacer()
                                Text(s.tokens.formatted()).font(Typo.monoSmall).foregroundStyle(Palette.text2)
                            }
                        }
                    }
                    let c = r.conversation
                    Text("บทสนทนา \(c.turns_total) เทิร์น · เต็ม \(c.at_l0) · ย่อ \(c.at_l1) · บรรทัดเดียว \(c.at_l2)")
                        .font(Typo.caption).foregroundStyle(Palette.text2)
                        .padding(.horizontal, 12).padding(.vertical, 10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Palette.hover, in: RoundedRectangle(cornerRadius: 8))
                    Button("ดูสิ่งที่ agent เห็นจริง ๆ") { showRaw = true }
                        .buttonStyle(SecondaryButtonStyle()).frame(maxWidth: .infinity)
                } else {
                    Text(engine.sessionID < 0 ? "เปิดงานแล้วจะเห็นตรงนี้" : "กำลังคำนวณ…")
                        .font(Typo.caption).foregroundStyle(Palette.text3)
                }
                TurnFacts()
            }
            .padding(16)
        }
        .onAppear(perform: reload)
        .onChange(of: engine.sessionID) { _, _ in inspection = nil; reload() }
        .onChange(of: engine.busy) { _, busy in if !busy { reload() } }
        .sheet(isPresented: $showRaw) { ContextRawSheet(inspection: inspection) }
    }

    struct Slice { let title: String; let tokens: Int64; let color: Color }

    static func slices(_ r: ContextReportData) -> [Slice] {
        func sum(_ n: [String]) -> Int64 { r.tiers.filter { n.contains($0.tier) }.reduce(0) { $0 + $1.tokens } }
        return [
            Slice(title: "สถานะงาน · ไม่มีวันตัด", tokens: sum(["System", "TaskState"]), color: Palette.text),
            Slice(title: "ความจำ", tokens: sum(["Memory"]), color: Palette.text2),
            Slice(title: "บทสนทนา", tokens: sum(["Conversation"]), color: Palette.accent),
            Slice(title: "โค้ด · repo map + diff", tokens: sum(["Code"]), color: Palette.text3),
        ]
    }

    private func compact(_ n: Int64) -> String { n >= 1000 ? "\(n / 1000)k" : "\(n)" }

    private func reload() {
        guard engine.sessionID >= 0 else { inspection = nil; return }
        engine.contextInspection { result in inspection = result ?? inspection }
    }
}

/// Speed, tokens, cache and cost of the last turn.
private struct TurnFacts: View {
    @EnvironmentObject private var engine: Engine
    @AppStorage(DisplayKey.currency) private var currency = "auto"
    @AppStorage(SettingsKey.thbRate) private var thbRate = ""

    var body: some View {
        let m = engine.metrics
        VStack(alignment: .leading, spacing: 6) {
            GroupHeader("เทิร์นล่าสุด")
            if let v = m?.ttft_ms { row("รอคำแรก", String(format: "%.1f วิ", Double(v) / 1000)) }
            if let v = m?.tok_per_s { row("ความเร็ว", "\(m?.tok_per_s_estimated == true ? "≈" : "")\(Int(v.rounded())) tok/s") }
            row("token", "\(engine.sessionTokensIn.formatted()) เข้า · \(engine.sessionTokensOut.formatted()) ออก")
            if let v = m?.cache_hit_pct { row("cache", "\(Int(v.rounded()))%") }
            if let v = m?.cost_usd {
                row("ค่าใช้จ่าย", Money.format(usd: v, currency: currency, thbRate: Money.rate(thbRate),
                                               usdDecimals: v < 1 ? 4 : 2, thbDecimals: 2))
            }
        }
    }

    private func row(_ k: String, _ v: String) -> some View {
        HStack {
            Text(k).font(Typo.caption).foregroundStyle(Palette.text3)
            Spacer()
            Text(v).font(Typo.monoSmall).foregroundStyle(Palette.text2)
        }
    }
}

/// Design board "สิ่งที่ agent เห็นจริง": tiers on the left, the raw text on the right.
struct ContextRawSheet: View {
    let inspection: ContextInspection?
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            SheetHeader(title: "สิ่งที่ agent จะเห็นในเทิร์นถัดไป",
                        subtitle: "ประกอบใหม่ทุกเทิร์นจาก ledger · ไม่เขียนกลับลงดิสก์") { dismiss() }
            Hairline()
            HStack(spacing: 0) {
                VStack(alignment: .leading, spacing: 10) {
                    if let r = inspection?.report {
                        Text(r.total_tokens.formatted()).font(.system(size: 28, weight: .medium, design: .monospaced))
                        Text("/ \(r.budget.formatted())").font(Typo.caption).foregroundStyle(Palette.text3)
                        ForEach(r.tiers) { t in
                            VStack(alignment: .leading, spacing: 2) {
                                HStack { Text(t.tier).font(Typo.ui); Spacer(); Text(t.tokens.formatted()).font(Typo.monoSmall) }
                                if !t.note.isEmpty { Text(t.note).font(Typo.caption).foregroundStyle(Palette.text3) }
                            }
                            .padding(10)
                            .background(Palette.hover, in: RoundedRectangle(cornerRadius: 8))
                        }
                    }
                    Spacer()
                    Button("คัดลอกทั้งหมด") {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(inspection?.text ?? "", forType: .string)
                    }
                    .buttonStyle(SecondaryButtonStyle())
                }
                .padding(18)
                .frame(width: 300)
                .background(Palette.sidebar)
                Hairline(vertical: true)
                ScrollView {
                    Text(inspection?.text ?? "").font(Typo.mono).foregroundStyle(Palette.text).lineSpacing(3)
                        .textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading).padding(20)
                }
            }
        }
        .frame(minWidth: 900, minHeight: 600)
        .background(Palette.window)
    }
}

// MARK: ย้อน

/// Design board "ย้อนเวลา", compact: one row per checkpoint, the point picked
/// opens what will be undone and what to undo (chat, files, both).
private struct RewindTab: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @State private var items: [Engine.CheckpointRecord] = []
    @State private var picked: Engine.CheckpointRecord?
    @State private var status: String?
    @State private var chat = true
    @State private var files = true

    var body: some View {
        VStack(spacing: 0) {
            if engine.sessionID < 0 || items.isEmpty {
                VStack(spacing: 8) {
                    Image(systemName: "clock.arrow.circlepath").font(.system(size: 22)).foregroundStyle(Palette.text3)
                    Text(engine.sessionID < 0 ? "เปิด session ก่อน" : "ยังไม่มีจุดบันทึก").font(Typo.headline)
                    Text("ทุกเทิร์นที่แก้ไฟล์จะมี snapshot ให้ย้อน").font(Typo.caption).foregroundStyle(Palette.text3)
                }
                .frame(maxWidth: .infinity).padding(.top, 40)
                Spacer()
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(items.reversed()) { c in
                            Button { picked = c } label: {
                                HStack(alignment: .top, spacing: 12) {
                                    VStack(spacing: 4) {
                                        Image(systemName: "circle.fill").font(.system(size: 7))
                                            .foregroundStyle(picked?.id == c.id ? Palette.accent : Palette.text3)
                                            .padding(.top, 5)
                                        Rectangle().fill(Palette.line2).frame(width: 1)
                                    }
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(c.message.isEmpty ? "เทิร์น #\(c.seq)" : c.message).lineLimit(2)
                                            .foregroundStyle(Palette.text)
                                        Text("\(c.shortSha) · \(c.formattedDate)").font(Typo.monoSmall).foregroundStyle(Palette.text3)
                                    }
                                    .padding(.bottom, 10)
                                    Spacer(minLength: 0)
                                }
                            }
                            .buttonStyle(RowStyle(selected: picked?.id == c.id, height: nil))
                        }
                    }
                    .padding(10)
                }
                if let p = picked {
                    Hairline()
                    VStack(alignment: .leading, spacing: 10) {
                        GroupHeader("ย้อนไปก่อน #\(p.seq)")
                        Toggle("แชท — แยกเป็น session ใหม่ ของเดิมยังอยู่", isOn: $chat)
                        Toggle("ไฟล์ — เก็บ snapshot ปัจจุบันไว้ก่อนเสมอ", isOn: $files)
                        if let status { Text(status).font(Typo.caption).foregroundStyle(Palette.text3) }
                        HStack {
                            Button("ยกเลิก") { picked = nil }.buttonStyle(SecondaryButtonStyle())
                            Spacer()
                            Button("ย้อนไปจุดนี้") { rewind(p) }
                                .buttonStyle(PrimaryButtonStyle())
                                .disabled(engine.busy || (!chat && !files))
                        }
                    }
                    .toggleStyle(.checkbox)
                    .font(Typo.small)
                    .padding(16)
                }
            }
        }
        .onAppear(perform: load)
        .onChange(of: engine.sessionID) { _, _ in picked = nil; load() }
    }

    private func load() {
        guard engine.sessionID >= 0 else { items = []; return }
        engine.listCheckpoints(db: shell.currentDB, sessionID: engine.sessionID) { items = $0 }
    }

    private func rewind(_ c: Engine.CheckpointRecord) {
        let mode = chat && files ? "code_and_ledger" : (files ? "code_only" : "ledger_only")
        status = "กำลังย้อน…"
        engine.rewindCheckpoint(db: shell.currentDB, ws: shell.workspace, sessionID: engine.sessionID,
                                targetSeq: c.seq, mode: mode) { ok, err in
            status = ok ? "ย้อนแล้ว" : "ย้อนไม่สำเร็จ: \(err ?? "เกิดข้อผิดพลาด")"
            if ok { picked = nil }
            load()
        }
    }
}
