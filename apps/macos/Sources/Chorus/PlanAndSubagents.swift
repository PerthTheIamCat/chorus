import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

struct PlanChecklistView: View {
    let items: [Engine.ChecklistItem]
    let askBeforePlan: Bool
    let isPlanApproved: Bool
    let onStartPlan: () -> Void
    let onStop: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: "checklist")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(.purple)

                Text("แผนงาน Orchestra")
                    .font(.caption.weight(.semibold))

                Spacer()

                if askBeforePlan && !isPlanApproved {
                    Button(action: onStartPlan) {
                        Label("เริ่มทำตามแผน", systemImage: "play.fill")
                            .font(.caption.weight(.medium))
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
                }

                Button(role: .destructive, action: onStop) {
                    Label("หยุด", systemImage: "stop.fill")
                        .font(.caption.weight(.medium))
                }
                .buttonStyle(.bordered)
                .controlSize(.small)
                .help("หยุดการทำงานของแผนงาน Orchestra ทั้งหมด")
            }

            VStack(alignment: .leading, spacing: 6) {
                ForEach(items) { item in
                    HStack(alignment: .top, spacing: 8) {
                        if item.done {
                            Image(systemName: "checkmark.circle.fill")
                                .font(.system(size: 12))
                                .foregroundStyle(.green)
                        } else if item.in_progress {
                            ProgressView()
                                .controlSize(.mini)
                        } else {
                            Image(systemName: "circle")
                                .font(.system(size: 12))
                                .foregroundStyle(.secondary)
                        }

                        Text(item.text)
                            .font(.caption)
                            .foregroundStyle(item.done ? .secondary : .primary)
                            .strikethrough(item.done)
                    }
                }
            }
            .padding(.top, 2)
        }
        .padding(12)
        .background(Color(nsColor: .controlBackgroundColor).opacity(0.8), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(Color.primary.opacity(0.08), lineWidth: 1))
    }
}

struct CapPausePromptCard: View {
    let subagents: [Engine.SubagentInfo]
    let onContinue: (Int64) -> Void
    let onStop: (Int64) -> Void
    let onStopAll: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Text("รอ")
                    .font(.system(size: 9, weight: .bold))
                    .foregroundStyle(.white)
                    .padding(.horizontal, 5)
                    .padding(.vertical, 2)
                    .background(Color.orange, in: Capsule())

                Text("งานย่อยหยุดรอ (ติด Concurrency Cap หรือรอการยืนยัน)")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.orange)

                Spacer()

                Button(role: .destructive, action: onStopAll) {
                    Text("หยุดทั้งหมด")
                        .font(.caption2)
                }
                .buttonStyle(.bordered)
                .controlSize(.mini)
            }

            VStack(alignment: .leading, spacing: 6) {
                ForEach(subagents) { sub in
                    HStack(spacing: 8) {
                        Text(sub.role)
                            .font(.caption.weight(.semibold))
                        Text(sub.task)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)

                        Spacer()

                        Button {
                            onContinue(sub.id)
                        } label: {
                            Label("ทำต่อ", systemImage: "play.fill")
                                .font(.caption2.weight(.medium))
                        }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.mini)

                        Button(role: .destructive) {
                            onStop(sub.id)
                        } label: {
                            Image(systemName: "xmark")
                                .font(.system(size: 9, weight: .bold))
                        }
                        .buttonStyle(.bordered)
                        .controlSize(.mini)
                        .help("ยกเลิกงานย่อยนี้")
                    }
                }
            }
        }
        .padding(12)
        .background(Color.orange.opacity(0.08), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(Color.orange.opacity(0.25), lineWidth: 1))
    }
}

struct SubagentTreeRow: View {
    let sub: Engine.SubagentInfo
    let siblings: [Engine.SubagentInfo]
    let level: Int
    let folder: String
    let currentWorkspace: String
    let selectedSessionID: Int64?
    let subagentsByParent: [Int64: [Engine.SubagentInfo]]
    let onSelect: (Int64) -> Void
    let onMessage: (Engine.SubagentInfo) -> Void
    let onCancel: (Int64) -> Void
    let onCancelParent: () -> Void
    var onKeepRole: ((String, Engine.SubagentInfo) -> Void)? = nil

    var body: some View {
        let isSelected = folder == currentWorkspace && sub.child_sess != nil && sub.child_sess == selectedSessionID
        let isBuiltin = ["orchestrator", "investigator", "builder", "reviewer"].contains(sub.role.lowercased())
        let roleSiblings = siblings.filter { $0.role == sub.role }.sorted { $0.created_ms < $1.created_ms }
        let repeatIndex = roleSiblings.firstIndex(where: { $0.id == sub.id }).map { $0 + 1 } ?? 1
        let roleTitle = roleSiblings.count > 1 ? "\(sub.role) \(repeatIndex)" : sub.role
        let displayName = sub.kind == "provider" ? (sub.task.isEmpty ? sub.role : sub.task) : "\(roleTitle) · \(sub.task)"
        let nestedSubs = sub.child_sess.flatMap { subagentsByParent[$0] } ?? []

        VStack(alignment: .leading, spacing: 1) {
            Button {
                guard let childId = sub.child_sess, !isSelected else { return }
                onSelect(childId)
            } label: {
                HStack(spacing: 5) {
                    // Tree branch indicator
                    Image(systemName: "arrow.turn.down.right")
                        .font(.system(size: 8, weight: .bold))
                        .foregroundStyle(.tertiary)

                    // Status dot
                    switch sub.status {
                    case "running":
                        ProgressView().controlSize(.mini)
                            .help("กำลังทำ")
                    case "waiting":
                        Text("รอ")
                            .font(.system(size: 8.5, weight: .bold))
                            .foregroundStyle(.white)
                            .padding(.horizontal, 4)
                            .padding(.vertical, 1)
                            .background(Color.orange, in: Capsule())
                            .help("รอดำเนินการหรือติด Cap")
                    case "completed":
                        Image(systemName: "checkmark.circle.fill")
                            .font(.system(size: 10))
                            .foregroundStyle(.green)
                            .help("เสร็จแล้ว")
                    case "failed":
                        Image(systemName: "xmark.circle.fill")
                            .font(.system(size: 10))
                            .foregroundStyle(.red)
                            .help("ล้มเหลว: \(sub.summary ?? "")")
                    case "cancelled":
                        Image(systemName: "nosign")
                            .font(.system(size: 10))
                            .foregroundStyle(.secondary)
                            .help("ยกเลิก")
                    default:
                        Circle().fill(.secondary).frame(width: 5, height: 5)
                    }

                    Text(displayName)
                        .font(.caption2)
                        .fontWeight(isSelected ? .semibold : .regular)
                        .lineLimit(1)

                    Spacer(minLength: 4)

                    // New role badge & Keep action
                    if !isBuiltin && sub.kind == "chorus" {
                        HStack(spacing: 3) {
                            Text("new role")
                                .font(.system(size: 8, weight: .semibold))
                                .foregroundStyle(.purple)
                                .padding(.horizontal, 4)
                                .padding(.vertical, 0.5)
                                .background(Color.purple.opacity(0.12), in: Capsule())

                            if let onKeepRole {
                                Button {
                                    onKeepRole(sub.role, sub)
                                } label: {
                                    Text("Keep")
                                        .font(.system(size: 8, weight: .bold))
                                        .foregroundStyle(.white)
                                        .padding(.horizontal, 4)
                                        .padding(.vertical, 0.5)
                                        .background(Color.purple, in: Capsule())
                                }
                                .buttonStyle(.chorusPlain)
                                .help("บันทึก Role '\(sub.role)' ลง .chorus/agents/\(sub.role).md")
                            }
                        }
                    }

                    // Model chip
                    if let m = sub.model, !m.isEmpty {
                        Text(ModelID(m).base)
                            .font(.system(size: 8.5, weight: .medium, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 3)
                            .padding(.vertical, 0.5)
                            .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 3))
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(InteractiveStyle(cornerRadius: 6, selected: isSelected,
                                          padding: .init(top: 3, leading: 6, bottom: 3, trailing: 6)))
            .padding(.leading, CGFloat(24 + level * 10))
            .help(displayName)
            .contextMenu {
                if sub.kind == "chorus" {
                    if !isBuiltin {
                        Button("Keep Role '\(sub.role)' (.chorus/agents/)") {
                            onKeepRole?(sub.role, sub)
                        }
                    }
                    if sub.status == "running" || sub.status == "waiting" {
                        Button("ส่งข้อความถึงงานย่อย…") {
                            onMessage(sub)
                        }
                        Button("หยุดงานย่อยนี้", role: .destructive) {
                            onCancel(sub.id)
                        }
                    }
                    if let wt = sub.worktree, !wt.isEmpty {
                        Button("เปิด Git Worktree ใน Finder") {
                            NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: wt)])
                        }
                    }
                } else {
                    Text("Provider subagent (อ่านอย่างเดียว)").font(.caption).foregroundStyle(.secondary)
                    Button("หยุดเทิร์นของ agent หลัก", role: .destructive) {
                        onCancelParent()
                    }
                }
            }

            // Nesting up to 2 levels; deeper levels shown as …+N
            if !nestedSubs.isEmpty {
                if level < 2 {
                    ForEach(nestedSubs) { nested in
                        SubagentTreeRow(
                            sub: nested,
                            siblings: nestedSubs,
                            level: level + 1,
                            folder: folder,
                            currentWorkspace: currentWorkspace,
                            selectedSessionID: selectedSessionID,
                            subagentsByParent: subagentsByParent,
                            onSelect: onSelect,
                            onMessage: onMessage,
                            onCancel: onCancel,
                            onCancelParent: onCancelParent,
                            onKeepRole: onKeepRole
                        )
                    }
                } else {
                    Text("…+\(nestedSubs.count)")
                        .font(.system(size: 9, weight: .medium))
                        .foregroundStyle(.tertiary)
                        .padding(.leading, CGFloat(24 + (level + 1) * 10))
                        .help("มีงานย่อยระดับลึกอีก \(nestedSubs.count) งาน")
                }
            }
        }
    }
}


/// A pending "ลบ session ที่ว่าง" confirmation.
struct EmptyCleanup: Identifiable {
    let project: String
    let dbs: [String]
    let count: Int
    var id: String { project }
}
