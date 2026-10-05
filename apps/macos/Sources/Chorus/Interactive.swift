import SwiftUI

/// One look for everything clickable: a faint fill on hover, a deeper one
/// while pressed (with a small squeeze), and the accent when selected. SwiftUI
/// gives plain buttons none of these on macOS, which is why the app felt inert.
struct InteractiveStyle: ButtonStyle {
    var cornerRadius: CGFloat = 8
    var selected = false
    var padding = EdgeInsets(top: 5, leading: 10, bottom: 5, trailing: 10)

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
                        .strokeBorder(style.selected ? Color.accentColor.opacity(0.45) : .clear, lineWidth: 1))
                .contentShape(RoundedRectangle(cornerRadius: style.cornerRadius, style: .continuous))
                .scaleEffect(configuration.isPressed ? 0.97 : 1)
                .opacity(enabled ? 1 : 0.45)
                .onHover { hover = $0 && enabled }
                .animation(.easeOut(duration: 0.12), value: hover)
                .animation(.easeOut(duration: 0.08), value: configuration.isPressed)
        }

        private var fill: Color {
            if style.selected { return Color.accentColor.opacity(configuration.isPressed ? 0.28 : 0.18) }
            if configuration.isPressed { return Color.primary.opacity(0.14) }
            if hover { return Color.primary.opacity(0.07) }
            return .clear
        }
    }
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
