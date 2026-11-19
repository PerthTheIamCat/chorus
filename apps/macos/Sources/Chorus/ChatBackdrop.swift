import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

extension View {
    /// A clean panel: crisp continuous rounded corners with subtle native border and translucent material.
    /// Completely eliminates harsh drop-shadow artifacts and corner clipping anomalies.
    @ViewBuilder
    func floatingPanel(cornerRadius: CGFloat, tint: Color? = nil) -> some View {
        self
            .clipShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
            .background(
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .fill(.regularMaterial)
            )
            .overlay(
                Group {
                    if let tint {
                        RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                            .fill(tint.opacity(0.12))
                    }
                }
            )
            .overlay(
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .stroke(Color.primary.opacity(0.08), lineWidth: 1)
            )
    }
}


/// What the glass panels float over: the window colour with a faint wash of
/// the accent, so the panels read as separate layers.
struct Backdrop: View {
    var body: some View {
        ZStack {
            Color(nsColor: .windowBackgroundColor)
            LinearGradient(colors: [Color.accentColor.opacity(0.10), .clear, Color.purple.opacity(0.06)],
                           startPoint: .topLeading, endPoint: .bottomTrailing)
        }
        .ignoresSafeArea()
    }
}

/// The thread's content frame in its scroll view's coordinate space:
/// `minY` tracks the scroll offset, `maxY` the bottom of the content.
struct ChatContentFrame: Equatable {
    var minY: CGFloat
    var maxY: CGFloat
}

struct ChatContentFrameKey: PreferenceKey {
    static let defaultValue = ChatContentFrame(minY: 0, maxY: 0)
    static func reduce(value: inout ChatContentFrame, nextValue: () -> ChatContentFrame) { value = nextValue() }
}

/// Open on the newest message. macOS 15 can limit the anchor to the first
/// layout; on 14 the plain anchor also keeps the bottom on content growth.
struct OpenAtBottom: ViewModifier {
    func body(content: Content) -> some View {
        if #available(macOS 15.0, *) {
            content.defaultScrollAnchor(.bottom, for: .initialOffset)
        } else {
            content.defaultScrollAnchor(.bottom)
        }
    }
}
