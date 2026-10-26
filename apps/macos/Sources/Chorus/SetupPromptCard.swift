import SwiftUI

/// One contextual setup suggestion at a time, kept close to the composer.
struct SetupPromptCard: View {
    let title: String
    let detail: String
    let symbol: String
    let actionTitle: String
    var working = false
    let onAction: () -> Void
    let onDismiss: () -> Void
    var secondaryTitle: String? = nil
    var onSecondary: (() -> Void)? = nil

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: symbol)
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(.orange)
                .frame(width: 30, height: 30)
                .background(.orange.opacity(0.12), in: RoundedRectangle(cornerRadius: 9))

            VStack(alignment: .leading, spacing: 6) {
                Text(title).font(.callout.weight(.semibold))
                Text(detail).font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: 8) {
                    Button(action: onAction) {
                        if working { ProgressView().controlSize(.small) }
                        else { Text(actionTitle) }
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
                    .disabled(working)
                    if let secondaryTitle, let onSecondary {
                        Button(secondaryTitle, action: onSecondary)
                            .buttonStyle(.bordered)
                            .controlSize(.small)
                    }
                }
            }
            Spacer(minLength: 0)
            Button(action: onDismiss) {
                Image(systemName: "xmark")
                    .font(.caption.weight(.semibold))
            }
            .buttonStyle(.chorusPlain)
            .foregroundStyle(.secondary)
            .accessibilityLabel("ปิดคำแนะนำ: \(title)")
            .help("ปิดคำแนะนำนี้ — เปิดกลับได้ในตั้งค่า")
        }
        .padding(12)
        .floatingPanel(cornerRadius: 14)
    }
}
