import SwiftUI

/// One thin line above the composer while a handoff is prepared because the
/// user picked another provider's model. There is no button that starts it:
/// sending the next message is the handoff. The strip only says what will
/// travel and lets the user read the brief (and switch back to cancel).
struct HandoffPrepStrip: View {
    let prep: Engine.HandoffPrep
    let agentLabel: (String) -> String
    @State private var showBrief = false

    var body: some View {
        switch prep {
        case .none:
            EmptyView()
        case .preparing(let to):
            row {
                ProgressView().controlSize(.mini)
                Text("กำลังเตรียมส่งต่อให้ \(agentLabel(to))…")
                    .foregroundStyle(.secondary)
            }
        case .ready(let p):
            row {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                Text("พร้อมส่งต่อให้ \(agentLabel(p.to ?? "")) — ข้อความถัดไปจะพาไปด้วย")
                    .lineLimit(1)
                Spacer(minLength: 8)
                Button("ดู brief") { showBrief.toggle() }
                    .buttonStyle(.link)
                    .popover(isPresented: $showBrief, arrowEdge: .top) { briefView(p) }
            }
        case .failed:
            row {
                Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
                Text("เตรียมส่งต่อไม่สำเร็จ — ส่งข้อความได้ตามปกติ ระบบจะใช้ประวัติจาก ledger")
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
    }

    private func row<C: View>(@ViewBuilder _ content: () -> C) -> some View {
        HStack(spacing: 8) { content() }
            .font(.caption)
            .padding(.horizontal, 10).padding(.vertical, 6)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.accentColor.opacity(0.08), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }

    private func briefView(_ p: Engine.PreparedHandoff) -> some View {
        ScrollView {
            Text(p.markdown ?? "")
                .font(.system(.caption, design: .monospaced))
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(12)
        }
        .frame(width: 460, height: 320)
    }
}
