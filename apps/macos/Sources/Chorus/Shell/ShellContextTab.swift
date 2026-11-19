import SwiftUI

// The Context tab of the inspector: three cards and nothing else.
//   1. what the agent sees this turn (one bar, four rows)
//   2. what a handoff would carry right now
//   3. this turn's numbers
// Every number is something Rust already computed; this file only draws it
// (CLAUDE.md 2.3).

struct ContextTab: View {
    @EnvironmentObject private var engine: Engine
    @AppStorage(DisplayKey.currency) private var currency = "auto"
    @AppStorage(SettingsKey.thbRate) private var thbRate = ""

    @State private var inspection: ContextInspection?
    @State private var target = ""
    @State private var showBrief = false
    @State private var sent: SentText?

    private struct SentText: Identifiable {
        let id = UUID()
        let text: String
        let tokens: Int64
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                seenCard
                handoffCard
                turnCard
            }
            .padding(12)
        }
        .onAppear { reload() }
        .onChange(of: engine.sessionID) { _, _ in inspection = nil; reload() }
        .onChange(of: engine.busy) { _, busy in if !busy { reload() } }
        .onChange(of: target) { _, _ in reload() }
        .sheet(item: $sent) { s in
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text("ข้อความที่ส่งจริง · \(tokenText(s.tokens)) token").font(.headline)
                    Spacer()
                    Button("ปิด") { sent = nil }.keyboardShortcut(.cancelAction)
                }
                ScrollView {
                    Text(s.text).font(.caption.monospaced()).textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding(16)
            .frame(minWidth: 560, minHeight: 420)
        }
    }

    // MARK: 1 · what the agent sees

    private var seenCard: some View {
        InspectorCard {
            if let r = inspection?.report {
                let total = max(r.total_tokens, 0)
                let pct = r.budget > 0 ? Int((Double(total) / Double(r.budget) * 100).rounded()) : 0
                HStack {
                    Text("ที่ agent เห็นในเทิร์นนี้").font(.callout.weight(.semibold))
                    Spacer()
                    Text("\(pct)% ของ \(compact(r.budget))")
                        .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                }
                let rows = slices(r)
                GeometryReader { geo in
                    HStack(spacing: 1) {
                        ForEach(rows.filter { $0.tokens > 0 && !$0.isFree }) { g in
                            g.color.frame(width: max(2, geo.size.width * CGFloat(g.tokens) / CGFloat(max(r.budget, 1))))
                        }
                        Spacer(minLength: 0)
                    }
                    .background(Color.primary.opacity(0.08))
                    .clipShape(Capsule())
                }
                .frame(height: 8)
                .accessibilityLabel("ใช้ \(tokenText(total)) จาก \(tokenText(r.budget)) token")
                VStack(spacing: 5) {
                    ForEach(rows) { g in
                        HStack(spacing: 6) {
                            Circle().fill(g.color).frame(width: 7, height: 7)
                            Text(g.title).font(.caption)
                            Spacer()
                            Text(compact(g.tokens)).font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                        }
                    }
                }
                if let i = inspection {
                    Button("ดูข้อความที่ส่งจริง") { sent = SentText(text: i.text, tokens: i.tokens) }
                        .buttonStyle(.link).font(.caption)
                }
            } else {
                Text("ที่ agent เห็นในเทิร์นนี้").font(.callout.weight(.semibold))
                Text(engine.sessionID < 0 ? "เปิดงานแล้วจะเห็นตรงนี้" : "กำลังคำนวณ…")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private struct Slice: Identifiable {
        let title: String
        let tokens: Int64
        let color: Color
        var isFree = false
        var id: String { title }
    }

    private func slices(_ r: ContextReportData) -> [Slice] {
        func sum(_ names: [String]) -> Int64 { r.tiers.filter { names.contains($0.tier) }.reduce(0) { $0 + $1.tokens } }
        return [
            Slice(title: "งานและความจำ", tokens: sum(["System", "TaskState", "Memory"]), color: Color.accentColor),
            Slice(title: "บทสนทนา", tokens: sum(["Conversation"]), color: Color.indigo.opacity(0.6)),
            Slice(title: "โค้ดและ diff", tokens: sum(["Code"]), color: Color.orange),
            Slice(title: "ที่ว่าง", tokens: max(r.budget - r.total_tokens, 0), color: Color.primary.opacity(0.15), isFree: true),
        ]
    }

    // MARK: 2 · if handed over now

    private var handoffCard: some View {
        InspectorCard {
            Text("ถ้าส่งต่อตอนนี้").font(.callout.weight(.semibold))
            if case .ready(let p) = engine.handoffPrep {
                row("ปลายทาง", engine.agents.first { $0.id == p.to }?.label ?? (p.to ?? "—"))
                row("ทำแล้ว", "\((p.done ?? []).count) ข้อ")
                row("ยังค้าง", "\((p.pending ?? []).count) ข้อ")
                Text("เตรียมไว้แล้ว ข้อความถัดไปจะพาไปด้วย").font(.caption).foregroundStyle(.secondary)
                if let md = p.markdown, !md.isEmpty { briefToggle(md) }
            } else if let t = inspection?.target {
                row("ปลายทาง", engine.agents.first { $0.id == t.agent }?.label ?? t.agent)
                row("brief", "\(tokenText(t.brief_tokens)) token")
                if t.dropped + t.demoted > 0 { row("ถูกย่อ/ทิ้ง", "\(t.demoted)/\(t.dropped) เทิร์น", warn: true) }
                briefToggle(t.brief_markdown)
            } else {
                Text("เลือกปลายทางเพื่อดูว่าจะส่งอะไรไปบ้าง").font(.caption).foregroundStyle(.secondary)
            }
            Picker("ปลายทาง", selection: $target) {
                Text("เลือก agent…").tag("")
                ForEach(engine.agents.filter(\.ready)) { a in Text(a.label).tag(a.id) }
            }
            .labelsHidden().controlSize(.small)
        }
    }

    private func briefToggle(_ md: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Button(showBrief ? "ซ่อน brief" : "ดู brief เต็ม") { showBrief.toggle() }
                .buttonStyle(.link).font(.caption)
            if showBrief {
                Text(md).font(.caption2.monospaced()).textSelection(.enabled)
                    .padding(6).frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 6))
            }
        }
    }

    // MARK: 3 · this turn

    private var turnCard: some View {
        InspectorCard {
            Text("เทิร์นนี้").font(.callout.weight(.semibold))
            let m = engine.metrics
            if let v = m?.ttft_ms { row("รอคำแรก", String(format: "%.1f วินาที", Double(v) / 1000)) }
            if let v = m?.tok_per_s {
                row("ความเร็ว", "\(m?.tok_per_s_estimated == true ? "≈" : "")\(Int(v.rounded())) tok/s")
            }
            row("token", "\(engine.sessionTokensIn.formatted()) เข้า · \(engine.sessionTokensOut.formatted()) ออก")
            if let v = m?.cache_hit_pct { row("cache", "\(Int(v.rounded()))%") }
            if let v = m?.cost_usd {
                row("ค่าใช้จ่าย", Money.format(usd: v, currency: currency, thbRate: Money.rate(thbRate),
                                               usdDecimals: v < 1 ? 4 : 2, thbDecimals: 2))
            } else if m?.subscription == true, let l = m?.limit_pct {
                row("โควตา \(m?.limit_window ?? "")", "\(Int(l.rounded()))%", warn: l > 75)
            }
        }
    }

    // MARK: bits

    private func row(_ k: String, _ v: String, warn: Bool = false) -> some View {
        HStack {
            Text(k).font(.caption).foregroundStyle(.secondary)
            Spacer()
            Text(v).font(.caption.monospacedDigit()).foregroundStyle(warn ? Color.orange : Color.primary)
        }
    }

    private func compact(_ n: Int64) -> String {
        n >= 1000 ? String(format: n >= 10_000 ? "%.0fk" : "%.1fk", Double(n) / 1000) : String(n)
    }

    private func reload() {
        guard engine.sessionID >= 0 else { inspection = nil; return }
        engine.contextInspection(target: target) { result in inspection = result ?? inspection }
    }
}

/// One flat card. No border: the inspector is already a surface.
struct InspectorCard<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 8) { content }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}
