import AppKit
import SwiftUI

// The Chorus look (design v3, canvas "Chorus UI Redesign"): a navy window taken
// from the app icon, flush panes split by hairlines, one blue primary button per
// screen. Colour says state only (blue working · amber waiting on you · red
// stuck · green done); who did something is always the provider's own mark.
//
// Every colour is a dynamic NSColor so light and dark are designed separately
// (not inverted) and follow the system appearance live. Increase Contrast swaps
// hairlines for stronger ones. Nothing in this file decides anything.

enum Palette {
    private static func dyn(_ light: UInt32, _ dark: UInt32, alpha la: CGFloat = 1, _ da: CGFloat = 1) -> Color {
        Color(nsColor: NSColor(name: nil) { ap in
            let isDark = ap.bestMatch(from: [.darkAqua, .vibrantDark, .accessibilityHighContrastDarkAqua]) != nil
            return NSColor(rgb: isDark ? dark : light, alpha: isDark ? da : la)
        })
    }

    // surfaces
    static let window = dyn(0xF7F8FA, 0x0F1117)
    static let sidebar = dyn(0xEEF0F4, 0x13161E)
    static let surface = dyn(0xFFFFFF, 0x191C25)
    static let elevated = dyn(0xFFFFFF, 0x20242F)
    static let hover = dyn(0x0F1428, 0xFFFFFF, alpha: 0.04, 0.045)
    static let selected = dyn(0x0F1428, 0xFFFFFF, alpha: 0.065, 0.08)
    static let line = dyn(0x0F1428, 0xFFFFFF, alpha: 0.08, 0.07)
    static let line2 = dyn(0x0F1428, 0xFFFFFF, alpha: 0.14, 0.12)

    // text
    static let text = dyn(0x151821, 0xECEEF3)
    static let text2 = dyn(0x545A68, 0xA4A9B6)
    static let text3 = dyn(0x676D7B, 0x868C9A)

    // state
    static let accent = dyn(0x2F6FEB, 0x5B9BFF)
    static let accentButton = dyn(0x2F6FEB, 0x3D7FF0)
    static let accentText = dyn(0x2459C7, 0x8DBBFF)
    static let accentWash = dyn(0x2F6FEB, 0x5B9BFF, alpha: 0.10, 0.14)
    static let ok = dyn(0x0E8A62, 0x4FD1A5)
    static let okWash = dyn(0x0E8A62, 0x4FD1A5, alpha: 0.09, 0.12)
    static let warn = dyn(0x9A6100, 0xF5B94E)
    static let warnWash = dyn(0x9A6100, 0xF5B94E, alpha: 0.07, 0.09)
    static let bad = dyn(0xC83444, 0xFF7A85)
    static let badWash = dyn(0xC83444, 0xFF7A85, alpha: 0.07, 0.10)
    static let addWash = dyn(0x0E8A62, 0x4FD1A5, alpha: 0.07, 0.09)
    static let delWash = dyn(0xC83444, 0xFF7A85, alpha: 0.06, 0.09)

    // syntax (diff/code)
    static let synKeyword = dyn(0x8E3FC7, 0xC792EA)
    static let synString = dyn(0x2E7D32, 0xA5D6A7)
    static let synFunc = dyn(0x1F5FD1, 0x82AAFF)
    static let synType = dyn(0xA35D00, 0xFFCB6B)
}

extension NSColor {
    convenience init(rgb: UInt32, alpha: CGFloat = 1) {
        self.init(srgbRed: CGFloat((rgb >> 16) & 0xFF) / 255, green: CGFloat((rgb >> 8) & 0xFF) / 255,
                  blue: CGFloat(rgb & 0xFF) / 255, alpha: alpha)
    }
}

/// Type scale from the design, on the system font (SF Pro + Sukhumvit Set for
/// Thai). Body is 14 with a tall line for Thai vowels; nothing goes below 11.5.
enum Typo {
    static let title = Font.system(size: 22, weight: .semibold)
    static let headline = Font.system(size: 13.5, weight: .semibold)
    static let body = Font.system(size: 14)
    static let ui = Font.system(size: 13)
    static let small = Font.system(size: 12.5)
    static let caption = Font.system(size: 12)
    static let tiny = Font.system(size: 11.5)
    static let groupHeader = Font.system(size: 11.5, weight: .semibold)
    static let mono = Font.system(size: 12, design: .monospaced)
    static let monoSmall = Font.system(size: 11.5, design: .monospaced)
}

// MARK: provider identity

enum ProviderTint {
    /// `claude@work:opus` → `claude`.
    static func provider(of actor: String) -> String {
        let base = String(actor.split(separator: ":", maxSplits: 1).first ?? Substring(actor))
        return String(base.split(separator: "@", maxSplits: 1).first ?? Substring(base))
    }
}

/// The provider's mark on a rounded tile — the only way the app says "who".
struct ProviderAvatar: View {
    let provider: String
    var size: CGFloat = 24

    var body: some View {
        let r = size * 0.28
        ProviderIcon(provider: ProviderTint.provider(of: provider), size: size * 0.62)
            .frame(width: size, height: size)
            .background(Palette.elevated, in: RoundedRectangle(cornerRadius: r, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: r, style: .continuous).strokeBorder(Palette.line, lineWidth: 1))
            .accessibilityHidden(true)
    }
}

/// Overlapping tiles for every agent that took part.
struct ProviderStack: View {
    let providers: [String]
    var size: CGFloat = 18
    var limit = 3

    var body: some View {
        let shown = Array(Self.unique(providers).prefix(limit))
        HStack(spacing: -size * 0.28) {
            ForEach(shown, id: \.self) { ProviderAvatar(provider: $0, size: size) }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(shown.joined(separator: ", "))
        .help(shown.joined(separator: ", "))
    }

    static func unique(_ ids: [String]) -> [String] {
        var seen = Set<String>()
        return ids.map(ProviderTint.provider(of:)).filter { seen.insert($0).inserted }
    }
}

// MARK: controls

/// The one blue button per screen.
struct PrimaryButtonStyle: ButtonStyle {
    var small = false
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: small ? 12 : 13, weight: .semibold))
            .foregroundStyle(.white)
            .padding(.horizontal, small ? 10 : 13)
            .frame(height: small ? 24 : 30)
            .background(Palette.accentButton.opacity(enabled ? (configuration.isPressed ? 0.8 : 1) : 0.4),
                        in: RoundedRectangle(cornerRadius: small ? 6 : 8, style: .continuous))
            .contentShape(Rectangle())
    }
}

/// Outlined secondary button.
struct SecondaryButtonStyle: ButtonStyle {
    var small = false
    @Environment(\.isEnabled) private var enabled
    @State private var hover = false
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: small ? 12 : 13))
            .foregroundStyle(Palette.text.opacity(enabled ? 1 : 0.4))
            .padding(.horizontal, small ? 9 : 12)
            .frame(height: small ? 24 : 30)
            .background(configuration.isPressed ? Palette.selected : (hover ? Palette.hover : .clear),
                        in: RoundedRectangle(cornerRadius: small ? 6 : 8, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: small ? 6 : 8, style: .continuous).strokeBorder(Palette.line2))
            .contentShape(Rectangle())
            .onHover { hover = $0 }
    }
}

/// Text-only, quiet. `danger` turns it red.
struct QuietButtonStyle: ButtonStyle {
    var small = false
    var danger = false
    @State private var hover = false
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: small ? 12 : 13))
            .foregroundStyle(danger ? Palette.bad : Palette.text2)
            .padding(.horizontal, small ? 8 : 10)
            .frame(height: small ? 24 : 30)
            .background(configuration.isPressed ? Palette.selected : (hover ? Palette.hover : .clear),
                        in: RoundedRectangle(cornerRadius: 7, style: .continuous))
            .contentShape(Rectangle())
            .onHover { hover = $0 }
    }
}

/// Square icon button (28 pt) used in toolbars and footers.
struct IconButtonStyle: ButtonStyle {
    var on = false
    @State private var hover = false
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 13.5))
            .foregroundStyle(on || hover ? Palette.text : Palette.text2)
            .frame(width: 28, height: 28)
            .background(on || configuration.isPressed ? Palette.selected : (hover ? Palette.hover : .clear),
                        in: RoundedRectangle(cornerRadius: 7, style: .continuous))
            .contentShape(Rectangle())
            .onHover { hover = $0 }
    }
}

/// Pill chip, used in the composer.
struct ChipStyle: ButtonStyle {
    var on = false
    @State private var hover = false
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 12.5))
            .foregroundStyle(on ? Palette.accentText : Palette.text2)
            .padding(.horizontal, 10)
            .frame(height: 28)
            .background(on ? Palette.accentWash : (configuration.isPressed ? Palette.selected : (hover ? Palette.hover : .clear)),
                        in: Capsule())
            .overlay(Capsule().strokeBorder(on ? .clear : Palette.line))
            .contentShape(Capsule())
            .onHover { hover = $0 }
    }
}

/// A row in a list: sidebar, palette, settings nav. Selected = filled.
struct RowStyle: ButtonStyle {
    var selected = false
    var height: CGFloat? = 30
    @State private var hover = false
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 13))
            .foregroundStyle(selected ? Palette.text : Palette.text2)
            .padding(.horizontal, 8)
            .frame(minHeight: height)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(selected || configuration.isPressed ? Palette.selected : (hover ? Palette.hover : .clear),
                        in: RoundedRectangle(cornerRadius: 7, style: .continuous))
            .contentShape(Rectangle())
            .onHover { hover = $0 }
    }
}

/// Small status pill.
struct Tag: View {
    enum Tone { case neutral, accent, ok, warn, bad }
    let text: String
    var tone: Tone = .neutral
    var symbol: String?

    var body: some View {
        HStack(spacing: 4) {
            if let symbol { Image(systemName: symbol).font(.system(size: 9.5, weight: .semibold)) }
            Text(text)
        }
        .font(.system(size: 11.5))
        .lineLimit(1)
        .foregroundStyle(fg)
        .padding(.horizontal, 8).frame(height: 22)
        .background(bg, in: Capsule())
    }

    private var fg: Color {
        switch tone {
        case .neutral: Palette.text2
        case .accent: Palette.accentText
        case .ok: Palette.ok
        case .warn: Palette.warn
        case .bad: Palette.bad
        }
    }
    private var bg: Color {
        switch tone {
        case .neutral: Palette.selected
        case .accent: Palette.accentWash
        case .ok: Palette.okWash
        case .warn: Palette.warnWash
        case .bad: Palette.badWash
        }
    }
}

/// Keyboard hint, e.g. `⌘K`.
struct Kbd: View {
    let keys: String
    init(_ keys: String) { self.keys = keys }
    var body: some View {
        Text(keys)
            .font(.system(size: 10.5, design: .monospaced))
            .foregroundStyle(Palette.text3)
            .padding(.horizontal, 4).frame(height: 16)
            .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(Palette.line2))
    }
}

/// Small caps-ish group header ("โปรเจกต์", "วงของคุณ").
struct GroupHeader: View {
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View {
        Text(text).font(Typo.groupHeader).foregroundStyle(Palette.text3)
    }
}

/// Flat card on a pane.
struct Card<Content: View>: View {
    var padding: CGFloat = 12
    var stroke: Color = Palette.line
    var fill: Color = Palette.surface
    @ViewBuilder var content: Content
    var body: some View {
        VStack(alignment: .leading, spacing: 8) { content }
            .padding(padding)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(fill, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(stroke))
    }
}

/// Hairline that separates flush panes.
struct Hairline: View {
    var vertical = false
    var body: some View {
        Rectangle().fill(Palette.line)
            .frame(width: vertical ? 1 : nil, height: vertical ? nil : 1)
    }
}

/// Partial-arc spinner used for "working" (sidebar rows, checklist).
struct Spinner: View {
    var size: CGFloat = 12
    @State private var turn = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        Circle()
            .trim(from: 0, to: 0.72)
            .stroke(Palette.accent, style: StrokeStyle(lineWidth: max(1.5, size / 6), lineCap: .round))
            .frame(width: size, height: size)
            .rotationEffect(.degrees(turn ? 360 : 0))
            .animation(reduceMotion ? nil : .linear(duration: 0.9).repeatForever(autoreverses: false), value: turn)
            .onAppear { turn = true }
            .accessibilityLabel("กำลังทำงาน")
    }
}

/// A thin horizontal meter (quota, budget).
struct Meter: View {
    let value: Double
    var tint: Color = Palette.text2
    var body: some View {
        GeometryReader { g in
            ZStack(alignment: .leading) {
                Capsule().fill(Palette.line2)
                Capsule().fill(tint).frame(width: max(0, min(1, value)) * g.size.width)
            }
        }
        .frame(height: 5)
    }
}

/// Memoises pure, expensive rendering work (Markdown parse, JSON) by key.
final class RenderCache<Value> {
    private let cache = NSCache<NSString, Box>()
    private final class Box { let value: Value; init(_ v: Value) { value = v } }
    init(limit: Int) { cache.countLimit = limit }
    func value(for key: String, make: () -> Value) -> Value {
        if let hit = cache.object(forKey: key as NSString) { return hit.value }
        let v = make()
        cache.setObject(Box(v), forKey: key as NSString)
        return v
    }
}

extension View {
    /// Solid pane background (no material): panes never need to see through.
    func pane(_ color: Color) -> some View { background(color) }
}
