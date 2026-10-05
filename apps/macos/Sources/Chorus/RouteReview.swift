import SwiftUI

/// Lets the user review an automatic route before Chorus dispatches a turn.
/// The five-second countdown pauses as soon as a review control receives
/// keyboard or accessibility focus, or if the proposed choice changes.
struct RouteReview: View {
    let agent: String
    let model: String
    let effort: String
    let reason: String
    let onSendNow: () -> Void
    let onChange: () -> Void
    let onCancel: () -> Void

    @State private var deadline: Date?
    @State private var secondsWhenPaused: TimeInterval = 5
    @State private var isPaused = false
    @State private var didFinish = false
    @FocusState private var focusedAction: ReviewAction?
    @AccessibilityFocusState private var accessibilityFocusedAction: ReviewAction?

    private enum ReviewAction: Hashable {
        case heading
        case countdown
        case summary
        case send
        case change
        case cancel
    }

    init(
        agent: String,
        model: String,
        effort: String,
        reason: String,
        onSendNow: @escaping () -> Void,
        onChange: @escaping () -> Void,
        onCancel: @escaping () -> Void
    ) {
        self.agent = agent
        self.model = model
        self.effort = effort
        self.reason = reason
        self.onSendNow = onSendNow
        self.onChange = onChange
        self.onCancel = onCancel
    }

    private var proposalKey: String {
        [agent, model, effort, reason].joined(separator: "\u{0}")
    }

    var body: some View {
        TimelineView(.periodic(from: .now, by: 0.25)) { timeline in
            let secondsRemaining = remainingSeconds(at: timeline.date)

            VStack(alignment: .leading, spacing: 16) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Image(systemName: "arrow.triangle.branch")
                        .foregroundStyle(Color.accentColor)
                        .accessibilityHidden(true)
                    Text("ตรวจตัวเลือกก่อนส่ง")
                        .font(.headline)
                        .accessibilityAddTraits(.isHeader)
                        .accessibilityFocused($accessibilityFocusedAction, equals: .heading)
                    Spacer(minLength: 12)
                    Label(
                        isPaused ? "หยุดนับ · เหลือ \(secondsRemaining) วิ" : "ส่งใน \(secondsRemaining) วิ",
                        systemImage: isPaused ? "pause.circle" : "clock"
                    )
                    .font(.callout.monospacedDigit())
                    .foregroundStyle(isPaused ? Color.secondary : Color.accentColor)
                    .accessibilityLabel(
                        isPaused
                            ? "หยุดส่งอัตโนมัติชั่วคราว เหลือ \(secondsRemaining) วินาที"
                            : "จะส่งอัตโนมัติใน \(secondsRemaining) วินาที"
                    )
                    .accessibilityAddTraits(.updatesFrequently)
                    .accessibilityFocused($accessibilityFocusedAction, equals: .countdown)
                }

                VStack(alignment: .leading, spacing: 7) {
                    HStack(spacing: 8) {
                        Image(systemName: "person.crop.square")
                            .foregroundStyle(.secondary)
                            .accessibilityHidden(true)
                        Text(agent).font(.body.weight(.semibold))
                        Text("/").foregroundStyle(.tertiary)
                        Text(model).font(.body)
                    }

                    Label("ระดับคิด: \(effort)", systemImage: "brain")
                        .font(.callout)
                        .foregroundStyle(.secondary)

                    Text(reason)
                        .font(.callout)
                        .foregroundStyle(.primary)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.top, 2)
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.quaternary.opacity(0.55), in: RoundedRectangle(cornerRadius: 10))
                .accessibilityElement(children: .combine)
                .accessibilityLabel("ตัวเลือก: \(agent), \(model), ระดับคิด \(effort). เหตุผล: \(reason)")
                .accessibilityFocused($accessibilityFocusedAction, equals: .summary)

                HStack(spacing: 8) {
                    Spacer(minLength: 0)
                    Button {
                        pauseCountdown()
                        finish(onCancel)
                    } label: {
                        Label("ยกเลิก", systemImage: "xmark")
                    }
                    .buttonStyle(.bordered)
                    .keyboardShortcut(.cancelAction)
                    .focused($focusedAction, equals: .cancel)
                    .accessibilityFocused($accessibilityFocusedAction, equals: .cancel)

                    Button {
                        pauseCountdown()
                        finish(onChange)
                    } label: {
                        Label("เปลี่ยนตัวเลือก", systemImage: "slider.horizontal.3")
                    }
                    .buttonStyle(.bordered)
                    .focused($focusedAction, equals: .change)
                    .accessibilityFocused($accessibilityFocusedAction, equals: .change)

                    Button {
                        sendNow()
                    } label: {
                        Label("ส่งตอนนี้", systemImage: "arrow.up")
                    }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
                    .focused($focusedAction, equals: .send)
                    .accessibilityFocused($accessibilityFocusedAction, equals: .send)
                }
            }
            .padding(18)
            .frame(minWidth: 360, maxWidth: 560, alignment: .leading)
            .onChange(of: secondsRemaining) { _, remaining in
                if remaining == 0 && !isPaused {
                    sendNow()
                }
            }
        }
        .onAppear {
            if deadline == nil && !isPaused {
                deadline = Date().addingTimeInterval(5)
            }
        }
        .onChange(of: focusedAction) { _, action in
            if action != nil { pauseCountdown() }
        }
        .onChange(of: accessibilityFocusedAction) { _, action in
            if action != nil { pauseCountdown() }
        }
        .onChange(of: proposalKey) { _, _ in
            pauseCountdown()
        }
    }

    private func remainingSeconds(at date: Date) -> Int {
        let remaining = isPaused
            ? secondsWhenPaused
            : max(0, (deadline ?? date.addingTimeInterval(5)).timeIntervalSince(date))
        return Int(ceil(remaining))
    }

    private func pauseCountdown() {
        guard !isPaused, !didFinish else { return }
        if let deadline {
            secondsWhenPaused = max(0, deadline.timeIntervalSinceNow)
        }
        isPaused = true
    }

    private func sendNow() {
        finish(onSendNow)
    }

    private func finish(_ action: () -> Void) {
        guard !didFinish else { return }
        didFinish = true
        action()
    }
}
