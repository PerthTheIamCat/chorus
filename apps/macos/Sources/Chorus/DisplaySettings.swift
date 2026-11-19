import AppKit
import SwiftUI

// Settings › การแสดงผล: how much of a turn is drawn, which numbers show, the
// currency, the theme and the accent colour. These are presentation choices
// only — every number they affect was computed in Rust (CLAUDE.md 2.3).

enum DisplayKey {
    static let detail = "chorus.display.detail"     // "compact" | "normal" | "detailed"
    static let currency = "chorus.display.currency" // "auto" | "usd" | "thb"
    static let theme = "chorus.display.theme"       // "system" | "light" | "dark"
    static let accent = "chorus.display.accent"     // "" = system accent, else "#RRGGBB"
}

/// How much of a turn the chat draws. One place decides, so the thread, the
/// tool rows and the metrics strip cannot disagree about what "compact" means.
enum DetailLevel: String, CaseIterable, Identifiable {
    case compact, normal, detailed
    var id: String { rawValue }

    static func from(_ raw: String) -> DetailLevel { DetailLevel(rawValue: raw) ?? .normal }

    var label: String {
        switch self {
        case .compact: "กะทัดรัด"
        case .normal: "ปกติ"
        case .detailed: "ละเอียด"
        }
    }

    var summary: String {
        switch self {
        case .compact: "พับกล่องความคิดและรายการเครื่องมือเหลือบรรทัดเดียว ไม่มีตัวอย่างข้อความ และซ่อนแถบตัวชี้วัด"
        case .normal: "แบบเดิม: กล่องความคิดตามสวิตช์ในหน้าทั่วไป รายการเครื่องมือพับพร้อมตัวอย่างหนึ่งบรรทัด"
        case .detailed: "กางกล่องความคิดและรายการเครื่องมือไว้เลย และแสดงตัวชี้วัดทุกตัวที่เปิดไว้"
        }
    }

    var showsMetrics: Bool { self != .compact }
    var showsPreviews: Bool { self != .compact }
    var toolsOpenByDefault: Bool { self == .detailed }

    /// `userPref` is the older "เปิดกล่องความคิดไว้" switch, which still rules
    /// at the normal level.
    func thinkingOpenByDefault(userPref: Bool) -> Bool {
        switch self {
        case .compact: false
        case .normal: userPref
        case .detailed: true
        }
    }
}

enum AppTheme: String, CaseIterable, Identifiable {
    case system, light, dark
    var id: String { rawValue }

    var label: String {
        switch self {
        case .system: "ตามระบบ"
        case .light: "สว่าง"
        case .dark: "มืด"
        }
    }

    /// nil = follow macOS.
    var scheme: ColorScheme? {
        switch self {
        case .system: nil
        case .light: .light
        case .dark: .dark
        }
    }
}

/// Money in the person's currency. USD is always the source; baht needs a
/// rate the person typed (Chorus never fetches one).
enum Money {
    /// The rate as a number, or nil when empty / not positive.
    static func rate(_ raw: String) -> Double? {
        guard let r = Double(raw.trimmingCharacters(in: .whitespaces)), r > 0 else { return nil }
        return r
    }

    /// "usd" or "thb". "auto" keeps the older behaviour: baht once a rate is set.
    static func resolve(_ currency: String, thbRate: Double?) -> String {
        switch currency {
        case "usd": return "usd"
        case "thb": return thbRate == nil ? "usd" : "thb"
        default: return thbRate == nil ? "usd" : "thb"
        }
    }

    static func format(usd: Double, currency: String, thbRate: Double?,
                       usdDecimals: Int = 3, thbDecimals: Int = 1) -> String {
        if resolve(currency, thbRate: thbRate) == "thb", let r = thbRate {
            return String(format: "฿%.\(thbDecimals)f", usd * r)
        }
        return String(format: "$%.\(usdDecimals)f", usd)
    }
}

struct AccentPreset: Identifiable {
    let name: String
    let hex: String
    var id: String { hex }
}

enum ChorusPalette {
    static let accent = Color(nsColor: NSColor(name: nil) { a in
        a.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            ? NSColor(srgbRed: 0.42, green: 0.49, blue: 0.94, alpha: 1)
            : NSColor(srgbRed: 0.20, green: 0.27, blue: 0.78, alpha: 1)
    })
}

enum AccentHex {
    static let presets: [AccentPreset] = [
        AccentPreset(name: "น้ำเงิน", hex: "#0A84FF"), AccentPreset(name: "ม่วง", hex: "#BF5AF2"),
        AccentPreset(name: "ชมพู", hex: "#FF375F"), AccentPreset(name: "ส้ม", hex: "#FF9F0A"),
        AccentPreset(name: "เขียว", hex: "#30D158"), AccentPreset(name: "เขียวน้ำทะเล", hex: "#40C8E0"),
    ]

    /// "#RRGGBB" (the # is optional) → colour; anything else → nil.
    static func color(from hex: String) -> Color? {
        var h = hex.trimmingCharacters(in: .whitespaces)
        if h.hasPrefix("#") { h.removeFirst() }
        guard h.count == 6, let v = UInt32(h, radix: 16) else { return nil }
        return Color(.sRGB,
                     red: Double((v >> 16) & 0xFF) / 255,
                     green: Double((v >> 8) & 0xFF) / 255,
                     blue: Double(v & 0xFF) / 255)
    }

    static func hex(from color: Color) -> String {
        let c = NSColor(color).usingColorSpace(.sRGB) ?? .controlAccentColor
        return String(format: "#%02X%02X%02X",
                      Int((c.redComponent * 255).rounded()),
                      Int((c.greenComponent * 255).rounded()),
                      Int((c.blueComponent * 255).rounded()))
    }
}

/// Applies the chosen theme and accent to the whole window.
struct AppearanceModifier: ViewModifier {
    @AppStorage(DisplayKey.theme) private var theme = AppTheme.system.rawValue
    @AppStorage(DisplayKey.accent) private var accent = ""

    func body(content: Content) -> some View {
        let scheme = AppTheme(rawValue: theme)?.scheme
        // Board 13: one accent, #3346C8 (lifted in dark so it still reads).
        let color = AccentHex.color(from: accent) ?? ChorusPalette.accent
        content.preferredColorScheme(scheme).tint(color)
    }
}

extension View {
    func chorusAppearance() -> some View { modifier(AppearanceModifier()) }
}

struct DisplaySettingsView: View {
    @AppStorage(DisplayKey.detail) private var detail = DetailLevel.normal.rawValue
    @AppStorage(DisplayKey.currency) private var currency = "auto"
    @AppStorage(DisplayKey.theme) private var theme = AppTheme.system.rawValue
    @AppStorage(DisplayKey.accent) private var accent = ""
    @AppStorage(SettingsKey.thbRate) private var thbRate = ""
    @AppStorage(MetricsKey.tokPerS) private var showTokPerS = true
    @AppStorage(MetricsKey.ttft) private var showTtft = true
    @AppStorage(MetricsKey.cache) private var showCache = true
    @AppStorage(MetricsKey.cost) private var showCost = true
    @AppStorage(MetricsKey.context) private var showContext = true
    @AppStorage(MetricsKey.limit) private var showLimit = true

    private var level: DetailLevel { DetailLevel.from(detail) }
    private var rate: Double? { Money.rate(thbRate) }
    private var resolvedCurrency: String { Money.resolve(currency, thbRate: rate) }

    private var customAccent: Binding<Color> {
        Binding(
            get: { AccentHex.color(from: accent) ?? Color.accentColor },
            set: { accent = AccentHex.hex(from: $0) }
        )
    }

    var body: some View {
        SettingsGroup(title: "ระดับรายละเอียดของเทิร์น", footer: level.summary) {
            SettingsRow(title: "ระดับ", info: "ใช้กับกล่องความคิด รายการเครื่องมือ และแถบตัวชี้วัดใต้ช่องพิมพ์ กดที่แถวไหนในแชทก็ยังกางดูทีละอันได้เสมอ") {
                Picker("ระดับ", selection: $detail) {
                    ForEach(DetailLevel.allCases) { Text($0.label).tag($0.rawValue) }
                }
                .pickerStyle(.segmented).labelsHidden().frame(width: 230)
            }
        }
        SettingsGroup(title: "แถบตัวชี้วัด",
                      footer: "ตัวเลขที่ provider ไม่รายงานจะไม่ขึ้น ไม่เดาแทน · บัญชี subscription ขึ้น % ของโควตาแทนราคา") {
            SettingsRow(title: "ความเร็ว (tok/s)",
                        info: "ประมาณจากข้อความที่ agent ส่งออกมา ไม่ใช่ตัวเลขจาก provider") {
                Toggle("ความเร็ว", isOn: $showTokPerS).labelsHidden().toggleStyle(.switch)
            }
            RowDivider()
            SettingsRow(title: "เวลารอคำแรก (TTFT)",
                        info: "นับจากส่งคำสั่งถึงตัวอักษรแรกที่ agent ตอบ วัดที่ฝั่ง Chorus รวมเวลาที่ bridge เริ่มเทิร์น") {
                Toggle("TTFT", isOn: $showTtft).labelsHidden().toggleStyle(.switch)
            }
            RowDivider()
            SettingsRow(title: "Cache hit",
                        info: "สัดส่วน prompt ที่มาจาก cache ของ provider ตามตัวเลขที่ bridge ส่งมาตอนจบเทิร์น (Claude และ Codex ส่ง) ไม่มีตัวเลข = ไม่ขึ้น") {
                Toggle("Cache hit", isOn: $showCache).labelsHidden().toggleStyle(.switch)
            }
            RowDivider()
            SettingsRow(title: "ค่าใช้จ่าย",
                        info: "ตามที่ provider รายงาน เจ้าที่ไม่รายงาน (เช่น Codex) จะไม่ขึ้น ไม่เดาราคาเอง · บัญชี subscription ไม่แสดงราคา") {
                Toggle("ค่าใช้จ่าย", isOn: $showCost).labelsHidden().toggleStyle(.switch)
            }
            RowDivider()
            SettingsRow(title: "หน้าต่างบริบท", info: "สัดส่วนบริบทที่ใช้ไปของ session นี้") {
                Toggle("หน้าต่างบริบท", isOn: $showContext).labelsHidden().toggleStyle(.switch)
            }
            RowDivider()
            SettingsRow(title: "% ของโควตา",
                        info: "โควตา 5 ชั่วโมงหรือรายสัปดาห์ อันที่ใกล้เต็มกว่า · ขึ้นเมื่ออ่านโควตาของบัญชีนั้นได้") {
                Toggle("% ของโควตา", isOn: $showLimit).labelsHidden().toggleStyle(.switch)
            }
        }
        SettingsGroup(title: "สกุลเงิน") {
            SettingsRow(title: "แสดงราคาเป็น",
                        detail: currency == "thb" && rate == nil ? "ยังไม่ได้ใส่อัตรา จึงแสดงเป็น USD ไปก่อน" : nil,
                        info: "USD คือค่าที่ provider รายงาน บาทคือ USD × อัตราที่คุณใส่เอง Chorus ไม่ดึงอัตราจากเน็ต") {
                Picker("แสดงราคาเป็น", selection: Binding(get: { resolvedCurrency }, set: { currency = $0 })) {
                    Text("USD").tag("usd")
                    Text("บาท").tag("thb")
                }
                .pickerStyle(.segmented).labelsHidden().frame(width: 130)
            }
            RowDivider()
            SettingsRow(title: "อัตราบาท") {
                HStack(spacing: 6) {
                    Text("1 USD =").foregroundStyle(.secondary)
                    TextField("ยังไม่ตั้ง", text: $thbRate)
                        .textFieldStyle(.roundedBorder)
                        .multilineTextAlignment(.trailing)
                        .frame(width: 80)
                        .accessibilityLabel("อัตราบาทต่อ 1 USD")
                    Text("บาท").foregroundStyle(.secondary)
                }
            }
        }
        SettingsGroup(title: "ธีมและสีเน้น") {
            SettingsRow(title: "ธีม") {
                Picker("ธีม", selection: $theme) {
                    ForEach(AppTheme.allCases) { Text($0.label).tag($0.rawValue) }
                }
                .pickerStyle(.segmented).labelsHidden().frame(width: 230)
            }
            RowDivider()
            SettingsRow(title: "สีเน้น",
                        info: "สีของปุ่ม ตัวเลือกที่เลือกอยู่ และไฮไลต์ในแอป · ค่าเริ่มต้นตามสีเน้นของ macOS") {
                HStack(spacing: 8) {
                    Button {
                        accent = ""
                    } label: {
                        Text("ค่าเริ่มต้น").font(.caption)
                    }
                    .buttonStyle(.bordered).controlSize(.small)
                    .disabled(accent.isEmpty)
                    ForEach(AccentHex.presets) { p in
                        Button {
                            accent = p.hex
                        } label: {
                            Circle()
                                .fill(AccentHex.color(from: p.hex) ?? .gray)
                                .frame(width: 18, height: 18)
                                .overlay(Circle().strokeBorder(Color.primary.opacity(accent == p.hex ? 0.9 : 0), lineWidth: 2))
                        }
                        .buttonStyle(.plain)
                        .help(p.name)
                        .accessibilityLabel(p.name)
                    }
                    ColorPicker("สีเอง", selection: customAccent, supportsOpacity: false)
                        .labelsHidden()
                }
            }
        }
    }
}
