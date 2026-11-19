import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

extension View {
    /// A clean panel: crisp continuous rounded corners with subtle native border and translucent material.
    /// Completely eliminates harsh drop-shadow artifacts and corner clipping anomalies.
    @ViewBuilder
    func floatingPanel(cornerRadius requested: CGFloat, tint: Color? = nil) -> some View {
        // One quiet radius for every panel, whatever the call site asks for.
        let cornerRadius = min(requested, 14)
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
                    .stroke(Color.primary.opacity(0.06), lineWidth: 0.5)
            )
    }
}


/// What the panels sit on: the plain window colour. (It used to carry an
/// accent-to-purple wash; a flat ground is calmer and keeps text contrast.)
struct Backdrop: View {
    var body: some View {
        Color(nsColor: .windowBackgroundColor)
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
