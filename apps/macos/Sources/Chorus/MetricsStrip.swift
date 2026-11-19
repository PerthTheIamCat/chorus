import SwiftUI

/// The numbers Rust computed for the strip under the chat
/// (`chorus_session_metrics`). Every field can be absent: the provider did not
/// report it, and the strip hides it rather than show a guess.
struct SessionMetrics: Codable, Equatable {
    var tok_per_s: Double?
    var tok_per_s_estimated: Bool?
    var ttft_ms: Int?
    var cache_hit_pct: Double?
    var cost_usd: Double?
    /// A plan-limit account: it shows the share of its limit, not a price.
    var subscription: Bool?
    var context_pct: Double?
    var limit_pct: Double?
    var limit_window: String?
}

/// Which chips the user wants (Settings › การแสดงผล). One key per chip.
enum MetricsKey {
    static let tokPerS = "chorus.metrics.tokPerS"
    static let ttft = "chorus.metrics.ttft"
    static let cache = "chorus.metrics.cache"
    static let cost = "chorus.metrics.cost"
    static let limit = "chorus.metrics.limit"
    static let context = "chorus.metrics.context"
}

struct MetricsToggles: Equatable {
    var tokPerS = true
    var ttft = true
    var cache = true
    var cost = true
    var limit = true
    var context = true
    /// "auto" | "usd" | "thb" and the baht-per-USD rate (see `Money`).
    var currency = "usd"
    var thbRate: Double?
}

/// One chip of the strip. `warn` tints it when the number is near a ceiling.
struct MetricsChip: Equatable, Identifiable {
    var id: String { label }
    let icon: String
    let label: String
    let help: String
    var warn = false
}

/// Pure: which chips to draw for `m` under `t`. No number, no chip.
enum MetricsChips {
    static func make(_ m: SessionMetrics?, _ t: MetricsToggles) -> [MetricsChip] {
        guard let m else { return [] }
        var out: [MetricsChip] = []
        if t.tokPerS, let v = m.tok_per_s {
            let est = m.tok_per_s_estimated ?? false
            out.append(.init(icon: "speedometer", label: "\(est ? "≈" : "")\(Int(v.rounded())) tok/s",
                             help: est ? "ประมาณ: นับจากข้อความที่ agent ส่งออกมาในช่วงที่กำลังพิมพ์" : "ความเร็วในการตอบ"))
        }
        if t.ttft, let v = m.ttft_ms {
            out.append(.init(icon: "timer", label: "TTFT " + String(format: "%.1f", Double(v) / 1000) + "s",
                             help: "เวลารอคำแรก: นับจากส่งคำสั่งถึงตัวอักษรแรกที่ agent ตอบ"))
        }
        if t.cache, let v = m.cache_hit_pct {
            out.append(.init(icon: "internaldrive", label: "cache \(Int(v.rounded()))%",
                             help: "สัดส่วน prompt ที่มาจาก cache ของ provider"))
        }
        if t.cost, let v = m.cost_usd {
            out.append(.init(icon: "dollarsign.circle",
                             label: Money.format(usd: v, currency: t.currency, thbRate: t.thbRate,
                                                 usdDecimals: v < 1 ? 4 : 2, thbDecimals: 2),
                             help: "ค่าใช้จ่ายของ session ตามที่ provider รายงาน"))
        }
        if t.context, let v = m.context_pct {
            out.append(.init(icon: "cpu", label: "บริบท \(Int(v.rounded()))%",
                             help: "หน้าต่างบริบทที่ใช้ไป", warn: v > 75))
        }
        if t.limit, let v = m.limit_pct {
            out.append(.init(icon: "gauge.with.dots.needle.67percent",
                             label: "\(m.limit_window ?? "โควตา") \(Int(v.rounded()))%",
                             help: "โควตาของบัญชีที่ใช้อยู่ (หน้าต่างที่ใกล้เต็มกว่า)", warn: v > 75))
        }
        return out
    }
}

/// A thin row under the composer. Draws what Rust gave it; empty = invisible.
struct MetricsStripView: View {
    @ObservedObject var engine: Engine
    @AppStorage(MetricsKey.tokPerS) private var tokPerS = true
    @AppStorage(MetricsKey.ttft) private var ttft = true
    @AppStorage(DisplayKey.detail) private var detail = DetailLevel.normal.rawValue
    @AppStorage(DisplayKey.currency) private var currency = "auto"
    @AppStorage(SettingsKey.thbRate) private var thbRate = ""
    @AppStorage(MetricsKey.cache) private var cache = true
    @AppStorage(MetricsKey.cost) private var cost = true
    @AppStorage(MetricsKey.limit) private var limit = true
    @AppStorage(MetricsKey.context) private var context = true

    var body: some View {
        let chips = MetricsChips.make(engine.metrics,
            .init(tokPerS: tokPerS, ttft: ttft, cache: cache, cost: cost, limit: limit, context: context,
                  currency: currency, thbRate: Money.rate(thbRate)))
        if DetailLevel.from(detail).showsMetrics, !chips.isEmpty {
            HStack(spacing: 12) {
                ForEach(chips) { c in
                    Label(c.label, systemImage: c.icon)
                        .labelStyle(.titleAndIcon)
                        .font(.caption2.monospacedDigit())
                        .foregroundStyle(c.warn ? Color.orange : Color.secondary)
                        .help(c.help)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 6)
            .accessibilityElement(children: .combine)
        }
    }
}
