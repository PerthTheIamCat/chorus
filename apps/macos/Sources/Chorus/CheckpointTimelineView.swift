import SwiftUI
import AppKit

/// Visual Time-Travel Checkpoint & Selective Rewind Engine (Ticket 10 & 21).
/// Allows developers to visually inspect turn checkpoints and scrub/rewind
/// the codebase and ledger across shadow git commits.
struct CheckpointTimelineView: View {
    @ObservedObject var engine: Engine
    let dbPath: String
    let workspace: String
    var onClose: (() -> Void)?

    @State private var checkpoints: [Engine.CheckpointRecord] = []
    @State private var isLoading: Bool = false
    @State private var selectedSeq: Int64? = nil
    @State private var statusMessage: String? = nil
    @State private var isRewinding: Bool = false
    @State private var showingConfirmAlert: Bool = false
    @State private var pendingRewindMode: String = "code_and_ledger"

    init(
        engine: Engine,
        dbPath: String,
        workspace: String,
        onClose: (() -> Void)? = nil
    ) {
        self.engine = engine
        self.dbPath = dbPath
        self.workspace = workspace
        self.onClose = onClose
    }

    public var body: some View {
        VStack(spacing: 0) {
            // Header Bar
            HStack(spacing: 8) {
                Image(systemName: "clock.arrow.circlepath")
                    .font(.headline)
                    .foregroundStyle(.purple)

                VStack(alignment: .leading, spacing: 1) {
                    Text("Time-Travel Checkpoints")
                        .font(.headline)
                    Text("ย้อนเวลาโค้ดและประวัติการสนทนาตามจุดบันทึกของเทิร์น")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }

                Spacer()

                Button {
                    loadCheckpoints()
                } label: {
                    Image(systemName: "arrow.clockwise")
                        .font(.caption)
                }
                .buttonStyle(.chorusPlain)
                .help("รีเฟรชจุดบันทึก")

                if let onClose {
                    Button {
                        onClose()
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.chorusPlain)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(Color(NSColor.windowBackgroundColor))
            .overlay(Divider(), alignment: .bottom)

            if let msg = statusMessage {
                HStack(spacing: 6) {
                    Image(systemName: "info.circle.fill")
                        .font(.caption)
                        .foregroundStyle(.blue)
                    Text(msg)
                        .font(.caption)
                    Spacer()
                    Button("ตกลง") { statusMessage = nil }
                        .font(.caption2)
                        .buttonStyle(.chorusPlain)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .background(Color.blue.opacity(0.1))
            }

            // Timeline List
            if isLoading {
                VStack(spacing: 8) {
                    ProgressView()
                    Text("กำลังโหลดจุดบันทึก...")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if checkpoints.isEmpty {
                VStack(spacing: 10) {
                    Image(systemName: "clock.badge.questionmark")
                        .font(.system(size: 32))
                        .foregroundStyle(.tertiary)
                    Text("ยังไม่มีจุดบันทึก Checkpoint ใน session นี้")
                        .font(.callout.weight(.medium))
                        .foregroundStyle(.secondary)
                    Text("เมื่อ agent เขียนโค้ด ระบบจะสร้างสแน็ปช็อต Git อัตโนมัติในทุกเทิร์น")
                        .font(.caption)
                        .foregroundStyle(.tertiary)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 20)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 10) {
                        ForEach(checkpoints.reversed()) { ckpt in
                            checkpointRow(ckpt)
                        }
                    }
                    .padding(14)
                }
            }
        }
        .frame(minWidth: 320, minHeight: 300)
        .onAppear {
            loadCheckpoints()
        }
    }

    @ViewBuilder
    private func checkpointRow(_ ckpt: Engine.CheckpointRecord) -> some View {
        let isSelected = selectedSeq == ckpt.seq
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                // Turn Sequence Badge
                Text("#\(ckpt.seq)")
                    .font(.system(.caption, design: .monospaced).weight(.bold))
                    .foregroundStyle(.white)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Color.purple, in: Capsule())

                // Commit SHA Pill
                HStack(spacing: 4) {
                    Image(systemName: "number")
                        .font(.system(size: 9))
                    Text(ckpt.shortSha)
                        .font(.system(size: 11, design: .monospaced))
                }
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 4))

                Spacer()

                Text(ckpt.formattedDate)
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
            }

            if !ckpt.message.isEmpty {
                Text(ckpt.message)
                    .font(.caption)
                    .foregroundStyle(.primary)
                    .lineLimit(2)
            }

            // Rewind action controls
            HStack(spacing: 6) {
                Button {
                    triggerRewind(seq: ckpt.seq, mode: "code_and_ledger")
                } label: {
                    Label("ย้อนทั้งโค้ดและแชท", systemImage: "arrow.counterclockwise.circle.fill")
                        .font(.caption2)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
                .tint(.purple)
                .disabled(isRewinding)

                Menu {
                    Button("ย้อนเฉพาะไฟล์โค้ด (เก็บแชทไว้)") {
                        triggerRewind(seq: ckpt.seq, mode: "code_only")
                    }
                    Button("ย้อนเฉพาะแชท (เก็บไฟล์โค้ดไว้)") {
                        triggerRewind(seq: ckpt.seq, mode: "ledger_only")
                    }
                } label: {
                    Image(systemName: "ellipsis.circle")
                        .font(.caption)
                }
                .menuStyle(.borderlessButton)
                .hoverHighlight()
                .disabled(isRewinding)
            }
            .padding(.top, 2)
        }
        .padding(10)
        .background(Color.primary.opacity(isSelected ? 0.08 : 0.03), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(isSelected ? Color.purple.opacity(0.5) : Color.primary.opacity(0.06), lineWidth: 1))
    }

    private func loadCheckpoints() {
        guard engine.sessionID >= 0, !dbPath.isEmpty else { return }
        isLoading = true
        engine.listCheckpoints(db: dbPath, sessionID: engine.sessionID) { list in
            self.isLoading = false
            self.checkpoints = list
        }
    }

    private func triggerRewind(seq: Int64, mode: String) {
        guard !workspace.isEmpty, engine.sessionID >= 0 else { return }
        isRewinding = true
        statusMessage = "กำลังย้อนเวลากลับไปยังเทิร์น #\(seq)..."

        engine.rewindCheckpoint(db: dbPath, ws: workspace, sessionID: engine.sessionID, targetSeq: seq, mode: mode) { success, err in
            self.isRewinding = false
            if success {
                self.statusMessage = "ย้อนเวลากลับไปยังเทิร์น #\(seq) สำเร็จเรียบร้อยแล้ว"
                self.loadCheckpoints()
            } else {
                self.statusMessage = "ย้อนเวลาไม่สำเร็จ: \(err ?? "เกิดข้อผิดพลาด")"
            }
        }
    }
}
