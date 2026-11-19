import SwiftUI

// What the agent saw this turn. Everything here is a number or a sentence
// Rust already computed (`chorus_context_inspect`); this file only decodes
// and draws it (CLAUDE.md 2.3).

struct ContextInspection: Codable, Equatable {
    var budget: Int64
    var text: String
    var tokens: Int64
    var report: ContextReportData
    var target: ContextTargetPreview?
}

struct ContextReportData: Codable, Equatable {
    var budget: Int64
    var total_tokens: Int64
    var tiers: [ContextTier]
    var conversation: ContextConversation
    var memory: [ContextMemoryNote]
    var code: [ContextCodeNote]
}

struct ContextTier: Codable, Equatable, Identifiable {
    var tier: String
    var tokens: Int64
    var cap: Int64
    var fill_pct: Double
    var note: String
    var id: String { tier }
}

struct ContextConversation: Codable, Equatable {
    var turns_total: Int
    var at_l0: Int
    var at_l1: Int
    var at_l2: Int
    var demoted_by_budget: Int
    var dropped: Int
    var notes: [ContextTurnNote]
    var notes_omitted: Int
}

struct ContextTurnNote: Codable, Equatable, Identifiable {
    var age: Int
    var from_rung: Int
    var to_rung: Int
    var dropped: Bool
    var tokens_before: Int64
    var tokens_after: Int64
    var preview: String
    var id: Int { age }
}

struct ContextMemoryNote: Codable, Equatable, Identifiable {
    var text: String
    var score: Double?
    var pinned: Bool
    var included: Bool
    var tokens: Int64
    /// nil until Chorus counts references (memory counters); the column is
    /// hidden while every fact has nil.
    var referenced: Bool?
    var id: String { text }
}

struct ContextCodeNote: Codable, Equatable, Identifiable {
    var label: String
    var tokens: Int64
    var status: String
    var id: String { label + status + String(tokens) }
}

struct ContextTargetPreview: Codable, Equatable {
    var agent: String
    var budget: Int64
    var budget_known: Bool
    var context_tokens: Int64
    var demoted: Int
    var dropped: Int
    var brief_markdown: String
    var brief_tokens: Int64
    var instruction_file: String?
}

/// Names and colours for the tiers, in the order Rust sends them.
enum ContextTierStyle {
    static func title(_ tier: String) -> String {
        switch tier {
        case "System": "ระบบ"
        case "TaskState": "สถานะงาน"
        case "Memory": "ความจำ"
        case "Conversation": "บทสนทนา"
        case "Code": "โค้ดและ diff"
        case "Reserve": "กันไว้ให้ตอบ"
        default: tier
        }
    }

    static func color(_ tier: String) -> Color {
        switch tier {
        case "System": .gray
        case "TaskState": .blue
        case "Memory": .orange
        case "Conversation": .green
        case "Code": .purple
        default: Color.primary.opacity(0.18)
        }
    }

    static func rungLabel(_ rung: Int) -> String {
        ["เต็ม", "สรุปสั้น (L1)", "สรุปตอน (L2)"][min(max(rung, 0), 2)]
    }
}

/// "12,345" — the numbers here are token counts a person compares by eye.
func tokenText(_ n: Int64) -> String {
    let f = NumberFormatter()
    f.numberStyle = .decimal
    return f.string(from: NSNumber(value: n)) ?? String(n)
}

/// The "what the agent saw" card of the Context pane.
struct ContextInspectorSection: View {
    @ObservedObject var engine: Engine
    /// Opens the existing text sheet with exactly what was sent.
    var onViewText: (String, Int64) -> Void

    @State private var inspection: ContextInspection?
    @State private var loading = false
    @State private var target = ""
    @State private var showTurns = false
    @State private var showBrief = false

    private var report: ContextReportData? { inspection?.report }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            header
            if let r = report {
                tierBar(r)
                VStack(spacing: 6) {
                    ForEach(r.tiers) { tierRow($0) }
                }
                conversation(r.conversation)
                memory(r.memory)
                code(r.code)
                actions
                if let t = inspection?.target { targetPreview(t) }
            } else if loading {
                ProgressView().controlSize(.small)
            } else {
                Text("ยังไม่มีข้อมูล — เปิด session แล้วกดรีเฟรช")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .onAppear { reload() }
        .onChange(of: engine.sessionID) { _, _ in inspection = nil; reload() }
        .onChange(of: engine.busy) { _, busy in if !busy { reload() } }
        .onChange(of: target) { _, _ in reload() }
    }

    private var header: some View {
        HStack(spacing: 6) {
            Image(systemName: "eye").foregroundStyle(Color.accentColor)
            Text("สิ่งที่ agent เห็น").font(.caption.weight(.semibold))
            Spacer()
            if let i = inspection {
                Text("\(tokenText(i.tokens)) / \(tokenText(i.budget)) token")
                    .font(.caption2.monospacedDigit()).foregroundStyle(.secondary)
                    .help("ข้อความที่ส่งจริงรวมหัวข้อของแต่ละชั้น ผลรวมของชั้นอยู่ที่ \(tokenText(i.report.total_tokens)) token")
            }
            Button { reload() } label: { Image(systemName: "arrow.clockwise") }
                .buttonStyle(.chorusPlain)
                .help("ประกอบใหม่จาก ledger ตอนนี้ (อ่านอย่างเดียว ไม่นับเป็นการใช้ความจำ)")
                .disabled(engine.sessionID < 0 || loading)
        }
    }

    private func tierBar(_ r: ContextReportData) -> some View {
        GeometryReader { geo in
            HStack(spacing: 1) {
                ForEach(r.tiers.filter { $0.tier != "Reserve" && $0.tokens > 0 }) { t in
                    ContextTierStyle.color(t.tier)
                        .frame(width: max(2, geo.size.width * CGFloat(t.tokens) / CGFloat(max(r.budget, 1))))
                }
                Spacer(minLength: 0)
            }
            .background(Color.primary.opacity(0.08))
            .clipShape(RoundedRectangle(cornerRadius: 4))
        }
        .frame(height: 8)
        .accessibilityLabel("ใช้ \(tokenText(r.total_tokens)) จาก \(tokenText(r.budget)) token")
    }

    private func tierRow(_ t: ContextTier) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Circle().fill(ContextTierStyle.color(t.tier)).frame(width: 7, height: 7)
                Text(ContextTierStyle.title(t.tier)).font(.caption)
                Spacer()
                Text(t.tier == "Reserve" || t.tier == "System"
                     ? tokenText(t.tokens)
                     : "\(tokenText(t.tokens)) / \(tokenText(t.cap))")
                    .font(.caption2.monospacedDigit()).foregroundStyle(.secondary)
            }
            if !t.note.isEmpty {
                Text(t.note).font(.caption2).foregroundStyle(.secondary)
                    .padding(.leading, 13)
            }
        }
    }

    @ViewBuilder
    private func conversation(_ c: ContextConversation) -> some View {
        if c.turns_total > 0 {
            Divider()
            VStack(alignment: .leading, spacing: 4) {
                Text("บทสนทนา \(c.turns_total) เทิร์น").font(.caption.weight(.semibold))
                Text("ส่งเต็ม \(c.at_l0) · สรุปสั้น \(c.at_l1) · สรุปตอน \(c.at_l2)"
                     + (c.dropped > 0 ? " · ไม่ได้ส่ง \(c.dropped)" : ""))
                    .font(.caption2).foregroundStyle(.secondary)
                if c.demoted_by_budget + c.dropped > 0 {
                    Button {
                        withAnimation(.snappy) { showTurns.toggle() }
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "chevron.right").font(.caption2)
                                .rotationEffect(.degrees(showTurns ? 90 : 0))
                            Text("ถูกย่อ \(c.demoted_by_budget) · ถูกทิ้ง \(c.dropped) เพราะเกินเพดาน")
                                .font(.caption2)
                        }
                        .foregroundStyle(Color.orange)
                    }
                    .buttonStyle(.chorusPlain)
                    if showTurns {
                        ForEach(c.notes) { n in
                            VStack(alignment: .leading, spacing: 1) {
                                Text(n.preview).font(.caption2).lineLimit(1)
                                Text(turnDetail(n)).font(.caption2).foregroundStyle(.tertiary)
                            }
                        }
                        if c.notes_omitted > 0 {
                            Text("…และอีก \(c.notes_omitted) เทิร์น").font(.caption2).foregroundStyle(.tertiary)
                        }
                    }
                }
            }
        }
    }

    private func turnDetail(_ n: ContextTurnNote) -> String {
        let age = n.age == 0 ? "เทิร์นล่าสุด" : "ย้อนหลัง \(n.age) เทิร์น"
        if n.dropped { return "\(age) · ไม่ได้ส่ง (เดิม \(tokenText(n.tokens_before)) token)" }
        return "\(age) · \(ContextTierStyle.rungLabel(n.from_rung)) → \(ContextTierStyle.rungLabel(n.to_rung)) · "
            + "\(tokenText(n.tokens_before)) → \(tokenText(n.tokens_after)) token"
    }

    @ViewBuilder
    private func memory(_ facts: [ContextMemoryNote]) -> some View {
        if !facts.isEmpty {
            Divider()
            let showRef = facts.contains { $0.referenced != nil }
            VStack(alignment: .leading, spacing: 4) {
                Text("ความจำ \(facts.filter(\.included).count) จาก \(facts.count) ข้อ")
                    .font(.caption.weight(.semibold))
                ForEach(facts) { f in
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Image(systemName: f.pinned ? "pin.fill" : (f.included ? "checkmark" : "xmark"))
                            .font(.system(size: 8))
                            .foregroundStyle(f.included ? Color.secondary : Color.orange)
                            .frame(width: 10)
                        Text(f.text)
                            .font(.caption2)
                            .strikethrough(!f.included)
                            .foregroundStyle(f.included ? Color.primary : Color.secondary)
                            .lineLimit(2)
                        Spacer(minLength: 4)
                        if let s = f.score {
                            Text(String(format: "%.2f", s))
                                .font(.caption2.monospacedDigit()).foregroundStyle(.tertiary)
                                .help("ความคล้ายกับสิ่งที่คุณพิมพ์ล่าสุด")
                        }
                        if showRef, let r = f.referenced {
                            Image(systemName: r ? "quote.bubble.fill" : "quote.bubble")
                                .font(.caption2).foregroundStyle(.tertiary)
                                .help(r ? "agent พูดถึงข้อนี้" : "agent ไม่เคยพูดถึงข้อนี้")
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func code(_ items: [ContextCodeNote]) -> some View {
        if !items.isEmpty {
            Divider()
            VStack(alignment: .leading, spacing: 3) {
                Text("โค้ดและ diff").font(.caption.weight(.semibold))
                ForEach(items) { c in
                    HStack {
                        Text(c.label).font(.caption2.monospaced())
                        Spacer()
                        Text(c.status == "kept" ? "\(tokenText(c.tokens)) token"
                             : c.status == "repeat" ? "ซ้ำกับก่อนหน้า (ส่งแค่ตัวอ้างอิง)"
                             : "ใหญ่เกิน ส่งแค่รายการ")
                            .font(.caption2).foregroundStyle(c.status == "kept" ? Color.secondary : Color.orange)
                    }
                }
            }
        }
    }

    private var actions: some View {
        VStack(alignment: .leading, spacing: 6) {
            Divider()
            HStack {
                Button {
                    if let i = inspection { onViewText(i.text, i.tokens) }
                } label: {
                    Label("ดูข้อความที่ส่งจริง", systemImage: "doc.text.magnifyingglass")
                }
                .controlSize(.small)
                .disabled(inspection == nil)
                Spacer()
                Picker("ถ้าส่งให้", selection: $target) {
                    Text("ถ้าส่งให้ agent อื่นตอนนี้…").tag("")
                    ForEach(engine.agents.filter(\.ready)) { a in Text("ถ้าส่งให้ \(a.label) ตอนนี้").tag(a.id) }
                }
                .labelsHidden().fixedSize().controlSize(.small)
            }
        }
    }

    private func targetPreview(_ t: ContextTargetPreview) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("ถ้าส่งให้ \(t.agent) ตอนนี้").font(.caption.weight(.semibold))
            Text("เทิร์นแรกของ \(t.agent) คือ brief ยืนยันความเข้าใจ \(tokenText(t.brief_tokens)) token "
                 + "แล้วเทิร์นถัดไปได้บริบท \(tokenText(t.context_tokens)) token")
                .font(.caption2).foregroundStyle(.secondary)
            if t.dropped + t.demoted > 0 {
                Text("ที่หน้าต่าง \(tokenText(t.budget)) ถูกย่อ \(t.demoted) · ถูกทิ้ง \(t.dropped) เทิร์น")
                    .font(.caption2).foregroundStyle(.orange)
            }
            if !t.budget_known {
                Text("ไม่ทราบขนาดหน้าต่างของ \(t.agent) จึงใช้ \(tokenText(t.budget)) token เท่าเดิม"
                     + "\n(ตั้ง CHORUS_CONTEXT_WINDOW_\(t.agent.uppercased()) เพื่อให้ตรง)")
                    .font(.caption2).foregroundStyle(.tertiary)
            }
            if let f = t.instruction_file {
                Text("\(t.agent) อ่าน \(f) ของโปรเจกต์เองเพิ่มจากนี้").font(.caption2).foregroundStyle(.tertiary)
            }
            Button(showBrief ? "ซ่อน brief" : "ดู brief ที่จะส่ง") { showBrief.toggle() }
                .buttonStyle(.link).font(.caption2)
            if showBrief {
                Text(t.brief_markdown)
                    .font(.caption2.monospaced()).textSelection(.enabled)
                    .padding(6).frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 6))
            }
        }
        .padding(8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.accentColor.opacity(0.06), in: RoundedRectangle(cornerRadius: 8))
    }

    private func reload() {
        guard engine.sessionID >= 0 else { inspection = nil; return }
        loading = true
        engine.contextInspection(target: target) { result in
            loading = false
            inspection = result ?? inspection
        }
    }
}
