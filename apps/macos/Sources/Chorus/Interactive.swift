import SwiftUI

/// One look for everything clickable: a faint fill on hover, a deeper one
/// while pressed (with a small squeeze), and the accent when selected. SwiftUI
/// gives plain buttons none of these on macOS, which is why the app felt inert.
struct InteractiveStyle: ButtonStyle {
    var cornerRadius: CGFloat = 8
    var selected = false
    var padding = EdgeInsets(top: 5, leading: 10, bottom: 5, trailing: 10)
    /// A faint resting fill and hairline, so a control reads as clickable
    /// before the pointer finds it (chips, toolbar actions with text).
    var bordered = false

    func makeBody(configuration: Configuration) -> some View {
        StyledLabel(configuration: configuration, style: self)
    }

    private struct StyledLabel: View {
        let configuration: Configuration
        let style: InteractiveStyle
        @State private var hover = false
        @Environment(\.isEnabled) private var enabled

        var body: some View {
            configuration.label
                .padding(style.padding)
                .background(fill, in: RoundedRectangle(cornerRadius: style.cornerRadius, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: style.cornerRadius, style: .continuous)
                        .strokeBorder(stroke, lineWidth: 1))
                .contentShape(RoundedRectangle(cornerRadius: style.cornerRadius, style: .continuous))
                .scaleEffect(configuration.isPressed ? 0.97 : 1)
                .opacity(enabled ? 1 : 0.45)
                .onHover { hover = $0 && enabled }
                .animation(.easeOut(duration: 0.12), value: hover)
                .animation(.easeOut(duration: 0.08), value: configuration.isPressed)
        }

        private var fill: Color {
            if style.selected { return Color.accentColor.opacity(configuration.isPressed ? 0.28 : 0.18) }
            if configuration.isPressed { return Color.primary.opacity(0.16) }
            if hover { return Color.primary.opacity(style.bordered ? 0.11 : 0.08) }
            return style.bordered ? Color.primary.opacity(0.05) : .clear
        }

        private var stroke: Color {
            if style.selected { return Color.accentColor.opacity(0.45) }
            if style.bordered { return Color.primary.opacity(hover ? 0.18 : 0.10) }
            return .clear
        }
    }
}

/// `.plain` with feedback: the same hover/pressed fill as `InteractiveStyle`,
/// drawn just outside the label so swapping it in for `.plain` moves nothing.
/// Use it for rows, icons and text links that lay themselves out.
struct PlainInteractiveStyle: ButtonStyle {
    var cornerRadius: CGFloat = 6
    /// How far the highlight reaches past the label.
    var outset: CGFloat = 3

    func makeBody(configuration: Configuration) -> some View {
        Body(configuration: configuration, style: self)
    }

    private struct Body: View {
        let configuration: Configuration
        let style: PlainInteractiveStyle
        @State private var hover = false
        @Environment(\.isEnabled) private var enabled

        var body: some View {
            configuration.label
                .contentShape(Rectangle())
                .background(
                    RoundedRectangle(cornerRadius: style.cornerRadius, style: .continuous)
                        .fill(fill)
                        .padding(-style.outset))
                .opacity(enabled ? (configuration.isPressed ? 0.75 : 1) : 0.4)
                .onHover { hover = $0 && enabled }
                .animation(.easeOut(duration: 0.12), value: hover)
                .animation(.easeOut(duration: 0.08), value: configuration.isPressed)
        }

        private var fill: Color {
            if configuration.isPressed { return Color.primary.opacity(0.14) }
            if hover { return Color.primary.opacity(0.08) }
            return .clear
        }
    }
}

extension ButtonStyle where Self == PlainInteractiveStyle {
    /// Drop-in for `.plain` that still answers hover and press.
    static var chorusPlain: PlainInteractiveStyle { PlainInteractiveStyle() }
}

/// Hover feedback for things that are not buttons (List rows, tool rows).
struct HoverHighlight: ViewModifier {
    var cornerRadius: CGFloat = 6
    @State private var hover = false

    func body(content: Content) -> some View {
        content
            .background(hover ? Color.primary.opacity(0.06) : .clear,
                        in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
            .onHover { hover = $0 }
            .animation(.easeOut(duration: 0.12), value: hover)
    }
}

extension View {
    func hoverHighlight(cornerRadius: CGFloat = 6) -> some View {
        modifier(HoverHighlight(cornerRadius: cornerRadius))
    }
}
