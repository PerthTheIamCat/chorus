import SwiftUI
import AppKit

// MARK: - Scrollbar Style Customization

public enum ScrollbarStyle: String, CaseIterable, Identifiable {
    case hidden = "hidden"
    case overlay = "overlay"
    case system = "system"

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .hidden: return "ซ่อน"
        case .overlay: return "ลอยโปร่งใส"
        case .system: return "ตามระบบ"
        }
    }

    public var hint: String {
        switch self {
        case .hidden: return "ไม่แสดงแถบเลื่อน เลื่อนด้วย trackpad หรือเมาส์ได้ตามเดิม"
        case .overlay: return "แสดงแถบเลื่อนลอยเฉพาะตอนเลื่อน ไม่มีราง"
        case .system: return "ตาม System Settings ถ้าต่อเมาส์อาจมีราง"
        }
    }

    public var visibility: ScrollIndicatorVisibility {
        switch self {
        case .hidden: return .hidden
        case .overlay: return .automatic
        case .system: return .visible
        }
    }
}

public struct ChorusScrollIndicatorsModifier: ViewModifier {
    @AppStorage("chorus.scrollbarStyle") private var scrollbarStyle = "hidden"

    public func body(content: Content) -> some View {
        let style = ScrollbarStyle(rawValue: scrollbarStyle) ?? .hidden
        content
            .scrollIndicators(style.visibility)
    }
}

public extension View {
    /// Applies the user's preferred scrollbar style (defaults to hidden with no track background).
    func chorusScrollIndicators() -> some View {
        modifier(ChorusScrollIndicatorsModifier())
    }
}

// MARK: - Panel Maximizing & Layout

public enum MaximizedPanel: String, Equatable {
    case none
    case codeViewer
    case diffReviewer
    case thread
    case explorer
    case orchestrator
    case livePreview
}

public enum ResizeDirection {
    case forward   // Dragging right increases width (e.g. left sidebars)
    case reversed  // Dragging left increases width (e.g. right inspector)
}

/// An AppKit-backed native splitter that tracks mouse coordinates with 1:1 precision, zero jitter,
/// locked resize cursor, and double-click reset.
/// Layout numbers from `07-workbench-layout-v2` (variant A).
enum WorkbenchLayout {
    /// Viewer : chat ≈ 1.2 : 1 of the space the fixed columns leave.
    static let defaultViewerShare = 1.2 / 2.2
    static let chatMinWidth: CGFloat = 380
    static let viewerMinWidth: CGFloat = 360
    /// The gutter between two panels *is* the resize grip: the hit area, the
    /// highlight and the visible gap are the same strip, so the handle sits
    /// exactly on the panel edges instead of floating in dead space.
    static let gripWidth: CGFloat = 10
    static let columnSpacing: CGFloat = 0
}

public struct PanelResizeGrip: View {
    @Binding var width: Double
    let minWidth: Double
    let maxWidth: Double
    var direction: ResizeDirection
    var onReset: (() -> Void)?

    @State private var isHovering = false
    @State private var isDragging = false

    public init(
        width: Binding<Double>,
        minWidth: Double = 180,
        maxWidth: Double = 600,
        direction: ResizeDirection = .forward,
        onReset: (() -> Void)? = nil
    ) {
        self._width = width
        self.minWidth = minWidth
        self.maxWidth = maxWidth
        self.direction = direction
        self.onReset = onReset
    }

    public var body: some View {
        ZStack {
            if isHovering || isDragging {
                Rectangle()
                    .fill(Color.accentColor.opacity(isDragging ? 0.45 : 0.25))
                    .frame(width: 3)
                    .clipShape(Capsule())

                RoundedRectangle(cornerRadius: 1)
                    .fill(Color.accentColor)
                    .frame(width: 2, height: 28)
            } else {
                Color.clear
                    .frame(width: WorkbenchLayout.gripWidth)
            }

            NativeSplitterRepresentable(
                width: $width,
                minWidth: minWidth,
                maxWidth: maxWidth,
                direction: direction,
                isHovering: $isHovering,
                isDragging: $isDragging,
                onReset: onReset
            )
        }
        .frame(width: WorkbenchLayout.gripWidth)
        .contentShape(Rectangle())
        .help("ลากเพื่อปรับขนาดแผง (ดับเบิลคลิกเพื่อคืนค่าเริ่มต้น)")
    }
}

private struct NativeSplitterRepresentable: NSViewRepresentable {
    @Binding var width: Double
    let minWidth: Double
    let maxWidth: Double
    let direction: ResizeDirection
    @Binding var isHovering: Bool
    @Binding var isDragging: Bool
    var onReset: (() -> Void)?

    func makeNSView(context: Context) -> SplitterView {
        let view = SplitterView()
        updateView(view)
        return view
    }

    func updateNSView(_ nsView: SplitterView, context: Context) {
        updateView(nsView)
    }

    private func updateView(_ view: SplitterView) {
        view.widthBinding = { width }
        view.onWidthChanged = { newW in width = newW }
        view.minWidth = minWidth
        view.maxWidth = maxWidth
        view.direction = direction
        view.onHoverChanged = { h in
            DispatchQueue.main.async { isHovering = h }
        }
        view.onDragChanged = { d in
            DispatchQueue.main.async { isDragging = d }
        }
        view.onReset = onReset
    }

    class SplitterView: NSView {
        var widthBinding: (() -> Double)?
        var onWidthChanged: ((Double) -> Void)?
        var minWidth: Double = 180
        var maxWidth: Double = 700
        var direction: ResizeDirection = .forward
        var onHoverChanged: ((Bool) -> Void)?
        var onDragChanged: ((Bool) -> Void)?
        var onReset: (() -> Void)?

        private var startMouseX: CGFloat = 0
        private var initialWidth: Double = 0
        private var isDraggingInternal = false
        private var trackingArea: NSTrackingArea?

        override func updateTrackingAreas() {
            super.updateTrackingAreas()
            if let trackingArea { removeTrackingArea(trackingArea) }
            let area = NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeInActiveApp, .cursorUpdate], owner: self, userInfo: nil)
            addTrackingArea(area)
            self.trackingArea = area
        }

        override func cursorUpdate(with event: NSEvent) {
            NSCursor.resizeLeftRight.set()
        }

        override func mouseEntered(with event: NSEvent) {
            onHoverChanged?(true)
        }

        override func mouseExited(with event: NSEvent) {
            if !isDraggingInternal {
                onHoverChanged?(false)
            }
        }

        override func mouseDown(with event: NSEvent) {
            if event.clickCount == 2 {
                onReset?()
                return
            }
            isDraggingInternal = true
            onDragChanged?(true)
            startMouseX = NSEvent.mouseLocation.x
            initialWidth = widthBinding?() ?? 260
            NSCursor.resizeLeftRight.push()
        }

        override func mouseDragged(with event: NSEvent) {
            guard isDraggingInternal else { return }
            let currentX = NSEvent.mouseLocation.x
            let delta = direction == .forward ? (currentX - startMouseX) : (startMouseX - currentX)
            // Widths live in @AppStorage on ContentView: every write persists to
            // UserDefaults and re-evaluates the whole view. Whole points only,
            // and nothing when the clamped width did not change.
            let newWidth = min(max((initialWidth + Double(delta)).rounded(), minWidth), maxWidth)
            guard newWidth != widthBinding?() else { return }
            onWidthChanged?(newWidth)
        }

        override func mouseUp(with event: NSEvent) {
            if isDraggingInternal {
                isDraggingInternal = false
                onDragChanged?(false)
                onHoverChanged?(false)
                NSCursor.pop()
            }
        }
    }
}

/// One-click Maximize / Restore button for panels.
public struct PanelMaximizeButton: View {
    let isMaximized: Bool
    let onToggle: () -> Void

    public init(isMaximized: Bool, onToggle: @escaping () -> Void) {
        self.isMaximized = isMaximized
        self.onToggle = onToggle
    }

    public var body: some View {
        Button(action: onToggle) {
            Image(systemName: isMaximized ? "arrow.down.right.and.arrow.up.left" : "arrow.up.left.and.arrow.down.right")
                .font(.system(size: 11, weight: .semibold))
                .foregroundColor(isMaximized ? .accentColor : .secondary)
                .padding(4)
                .background(isMaximized ? Color.accentColor.opacity(0.15) : Color.primary.opacity(0.05))
                .clipShape(RoundedRectangle(cornerRadius: 5))
        }
        .buttonStyle(.plain)
        .help(isMaximized ? "ย่อกลับ (Esc)" : "ขยายเต็มจอ")
    }
}
