import SwiftUI

/// What the Auto router is doing, shown above the composer.
/// `thinking` covers the local-model call; `chosen` stays after the turn is
/// sent so the user can see which agent/model took the work and why.
enum RouteStatus: Equatable {
    case thinking(routerModel: String, candidates: Int)
    case chosen(agent: String, model: String, effort: String, reason: String)
}

struct RouteStatusCard: View {
    let status: RouteStatus
    let onDismiss: () -> Void

    var body: some View {
        switch status {
        case let .thinking(routerModel, candidates):
            HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                VStack(alignment: .leading, spacing: 1) {
                    Text("Auto กำลังเลือกโมเดล…").font(.callout.weight(.medium))
                    Text("\(routerModel) กำลังเทียบ \(candidates) ตัวเลือกกับงานนี้")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.accentColor.opacity(0.08), in: RoundedRectangle(cornerRadius: 10))
            .accessibilityElement(children: .combine)
            .accessibilityLabel("Auto กำลังเลือกโมเดล จาก \(candidates) ตัวเลือก")
            .accessibilityAddTraits(.updatesFrequently)

        case let .chosen(agent, model, effort, reason):
            HStack(alignment: .top, spacing: 8) {
                Image(systemName: "arrow.triangle.branch")
                    .foregroundStyle(Color.accentColor)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text("Auto เลือก").font(.caption).foregroundStyle(.secondary)
                        Text(agent).font(.callout.weight(.semibold))
                        Text("/").foregroundStyle(.tertiary)
                        Text(model).font(.callout)
                        Text("· คิด \(effort)").font(.caption).foregroundStyle(.secondary)
                    }
                    Text(reason)
                        .font(.caption).foregroundStyle(.secondary)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 0)
                Button(action: onDismiss) {
                    Image(systemName: "xmark.circle.fill")
                }
                .buttonStyle(.plain).foregroundStyle(.tertiary)
                .accessibilityLabel("ปิดสรุปการเลือก")
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.secondary.opacity(0.06), in: RoundedRectangle(cornerRadius: 10))
            .accessibilityElement(children: .combine)
            .accessibilityLabel("Auto เลือก \(agent) \(model) ระดับคิด \(effort). เหตุผล: \(reason)")
        }
    }
}
