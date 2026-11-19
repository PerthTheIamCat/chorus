import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

/// Right-click menu on a user message: edit-and-resend (chat only), or chat
/// and files, which asks first because it changes the workspace.
private struct RewindMenu: ViewModifier {
    let onRewind: ((Bool) -> Void)?
    @Binding var confirmFiles: Bool

    func body(content: Content) -> some View {
        if let onRewind {
            content
                .contextMenu {
                    Button {
                        onRewind(false)
                    } label: {
                        Label("แก้ข้อความนี้แล้วส่งใหม่", systemImage: "arrow.uturn.backward")
                    }
                    Button {
                        confirmFiles = true
                    } label: {
                        Label("ย้อนแชทและไฟล์กลับไปก่อนข้อความนี้…", systemImage: "clock.arrow.circlepath")
                    }
                }
                .confirmationDialog("ย้อนไฟล์กลับไปก่อนข้อความนี้?", isPresented: $confirmFiles) {
                    Button("ย้อนแชทและไฟล์", role: .destructive) { onRewind(true) }
                } message: {
                    Text("ไฟล์ใน workspace จะกลับไปเหมือนตอนส่งข้อความนี้ Chorus เก็บสภาพปัจจุบันไว้ใน shadow repo ก่อน และแชทเดิมยังอยู่ครบ — คุยต่อใน session ที่แตกออกมา")
                }
        } else {
            content
        }
    }
}

struct EventRow: View, Equatable {
    let event: LedgerEvent
    var attachments: [SessionAttachment] = []
    var agents: [Engine.AgentInfo] = []
    var thoughts: String? = nil
    var onDeleteAttachment: (Int64) -> Void = { _ in }
    /// Set on the user's own messages: rewind to just before it
    /// (`true` = files too). Rust does the work; this only asks.
    var onRewind: ((Bool) -> Void)? = nil

    /// Rows re-render far more often than their content changes (every engine
    /// publish); closures are rebuilt each time and not compared.
    static func == (l: EventRow, r: EventRow) -> Bool {
        l.event == r.event && l.attachments == r.attachments
            && l.thoughts == r.thoughts && l.agents == r.agents
    }

    @State private var isHovered = false
    @State private var confirmFileRewind = false
    @State private var copied = false

    private var isUser: Bool { event.actor == "user" }
    private var cleanActor: String {
        let base = String(event.actor.split(separator: ":", maxSplits: 1).first ?? Substring(event.actor))
        return String(base.split(separator: "@", maxSplits: 1).first ?? Substring(base))
    }
    private var agent: Engine.AgentInfo? {
        agents.first { cleanActor == $0.id }
    }
    private var providerID: String {
        agent?.id ?? cleanActor
    }
    private var modelLabel: String? {
        guard let modelID = event.model_id else { return nil }
        let model = ModelID(modelID)
        let name = agent?.baseModels.first(where: { $0.id == model.base })?.label ?? model.base
        return [name, model.effort].compactMap { $0 }.joined(separator: " · ")
    }

    var body: some View {
        switch event.kind {
        case "msg":
            message
        case "handoff":
            if let b = HandoffBrief(json: event.body) {
                card(tint: .purple) { handoffBody(b) }
            } else {
                note(event.body)
            }
        case "tool_call", "tool_result", "diff":
            toolRow
        default:
            note(event.body)
        }
    }

    // Chat layout: the user's words sit on the right in an accent bubble; an
    // agent turn sits on the left behind its avatar, with who answered (and
    // after a handoff, who took over) in the header above the bubble.
    private var message: some View {
        Group {
            if isUser { userMessage } else { agentMessage }
        }
        .frame(maxWidth: .infinity, alignment: isUser ? .trailing : .leading)
        .onHover { h in withAnimation(.easeOut(duration: 0.15)) { isHovered = h } }
    }

    private var userMessage: some View {
        HStack(alignment: .bottom, spacing: 6) {
            Spacer(minLength: 56)
            copyButton
                .opacity(isHovered ? 1 : 0)
                .allowsHitTesting(isHovered)
            VStack(alignment: .trailing, spacing: 4) {
                bubble
                attachmentChips
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("คุณ")
    }

    private var agentMessage: some View {
        HStack(alignment: .top, spacing: 10) {
            AgentAvatar(provider: providerID)
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 6) {
                    Text(agent?.label ?? event.actor).foregroundStyle(.primary.opacity(0.88))
                    if let modelLabel {
                        Text(modelLabel)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 6).padding(.vertical, 1.5)
                            .background(Color.primary.opacity(0.05), in: Capsule())
                    }
                    if let cost = event.cost_usd {
                        Text(String(format: "$%.4f", cost))
                            .font(.caption2.monospacedDigit())
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 5).padding(.vertical, 1)
                            .background(Color.primary.opacity(0.05), in: Capsule())
                            .help("มูลค่าเทียบ API ของเทิร์นนี้: $\(String(format: "%.4f", cost)) USD")
                    }
                    Spacer(minLength: 0)
                    copyButton
                        .opacity(isHovered ? 1 : 0)
                        .allowsHitTesting(isHovered)
                }
                .font(.caption.weight(.semibold))

                if let thoughts, !thoughts.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    ThinkingBlock(text: thoughts, running: false)
                }

                bubble
                attachmentChips
            }
            Spacer(minLength: 24)
        }
    }

    private var bubble: some View {
        let shape = chatBubbleShape(isUser: isUser)
        return MarkdownText(text: event.body, hug: true)
            .padding(.horizontal, 14).padding(.vertical, 10)
            .background(
                isUser
                    ? AnyShapeStyle(LinearGradient(colors: [Color.accentColor.opacity(0.20), Color.accentColor.opacity(0.13)],
                                                  startPoint: .topLeading, endPoint: .bottomTrailing))
                    : AnyShapeStyle(Color(nsColor: .controlBackgroundColor).opacity(0.55)),
                in: shape
            )
            .overlay(shape.stroke(isUser ? Color.accentColor.opacity(0.24) : Color.primary.opacity(0.07), lineWidth: 1))
            .frame(maxWidth: isUser ? 560 : 720, alignment: isUser ? .trailing : .leading)
            .modifier(RewindMenu(onRewind: onRewind, confirmFiles: $confirmFileRewind))
    }

    @ViewBuilder
    private var attachmentChips: some View {
        ForEach(attachments) { file in
            HStack(spacing: 6) {
                Image(systemName: file.status == "deleted" ? "doc.badge.xmark" : "paperclip")
                Text(file.name).lineLimit(1)
                if file.status == "deleted" {
                    Text("ลบแล้ว").foregroundStyle(.secondary)
                } else {
                    Button("ลบ", role: .destructive) { onDeleteAttachment(file.id) }
                        .buttonStyle(.link)
                        .accessibilityLabel("ลบไฟล์ \(file.name) จาก session")
                }
            }
            .font(.caption)
            .padding(.horizontal, 8).padding(.vertical, 4)
            .background(Color.primary.opacity(0.06), in: Capsule())
        }
    }

    private var copyButton: some View {
        Button {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(event.body, forType: .string)
            withAnimation(.spring(response: 0.2, dampingFraction: 0.7)) { copied = true }
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.8) {
                withAnimation { copied = false }
            }
        } label: {
            HStack(spacing: 3) {
                Image(systemName: copied ? "checkmark" : "doc.on.doc")
                    .font(.caption2)
                Text(copied ? "คัดลอกแล้ว" : "คัดลอก")
                    .font(.caption2)
            }
            .foregroundStyle(copied ? .green : .secondary)
            .padding(.horizontal, 6).padding(.vertical, 2)
            .background(Color.primary.opacity(0.06), in: Capsule())
        }
        .buttonStyle(.chorusPlain)
    }

    /// One collapsed line per tool call; click to see the payload.
    private var toolRow: some View {
        DisclosureGroup {
            if let payload = subagentPayload {
                VStack(alignment: .leading, spacing: 4) {
                    if let task = payload.task, !task.isEmpty { Text(task) }
                    if let message = payload.message, !message.isEmpty { Text(message) }
                    if let state = payload.state { Text(Self.subagentStateLabel(state)).foregroundStyle(.secondary) }
                }
                .font(.caption2)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, 4)
            } else {
                Text(event.body)
                    .font(.caption2.monospaced())
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.top, 4)
            }
        } label: {
            HStack(spacing: 6) {
                Image(systemName: subagentPayload == nil ? (event.kind == "diff" ? "plusminus" : "wrench.and.screwdriver") : "point.3.connected.trianglepath.dotted")
                    .font(.caption2).foregroundStyle(.secondary)
                Text(toolLabel).font(.caption).lineLimit(1).foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, 8).padding(.vertical, 5)
        .background(.secondary.opacity(0.07), in: RoundedRectangle(cornerRadius: 8))
        .hoverHighlight(cornerRadius: 8)
    }

    private var firstLine: String {
        let l = event.body.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: true).first.map(String.init) ?? event.kind
        return l.count > 110 ? String(l.prefix(110)) + "…" : l
    }

    private var subagentPayload: SubagentLedgerPayload? {
        guard let data = event.body.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(SubagentLedgerPayload.self, from: data)
    }

    private var toolLabel: String {
        guard let payload = subagentPayload else { return firstLine }
        if payload.event == "subagent_spawned" { return "งานย่อย · \(payload.name ?? "Agent")" }
        return "สถานะงานย่อย · \(payload.state.map(Self.subagentStateLabel) ?? "อัปเดต")"
    }

    private static func subagentStateLabel(_ state: String) -> String {
        switch state {
        case "completed": "เสร็จแล้ว"
        case "failed": "ล้มเหลว"
        case "cancelled": "ยกเลิกแล้ว"
        case "disconnected": "ขาดการเชื่อมต่อ"
        default: state
        }
    }

    private func note(_ t: String) -> some View {
        Text(t).font(.caption).foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func card<C: View>(tint: Color, @ViewBuilder _ c: () -> C) -> some View {
        c().padding(10)
            .background(tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 10))
    }

    /// The handoff is the product's whole argument, so it gets read as a
    /// moment — who fell over, why, and what the next agent was told — rather
    /// than as the brief's raw JSON.
    @ViewBuilder
    private func handoffBody(_ b: HandoffBrief) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 6) {
                Text(b.from).font(.caption.bold())
                Image(systemName: "arrow.right").font(.caption2)
                Text(b.to).font(.caption.bold())
                Text(b.reason)
                    .font(.caption2)
                    .padding(.horizontal, 5).padding(.vertical, 1)
                    .background(.orange.opacity(0.25), in: Capsule())
            }
            if !b.goal.isEmpty {
                Text("goal · \(b.goal)").font(.caption)
            }
            HStack(spacing: 10) {
                if !b.done.isEmpty { Text("done \(b.done.count)") }
                if !b.pending.isEmpty { Text("pending \(b.pending.count)") }
                if !b.files.isEmpty { Text("files \(b.files.count)") }
                if b.diffBytes > 0 { Text("diff \(b.diffBytes)B") }
            }
            .font(.caption2).foregroundStyle(.secondary)
            DisclosureGroup("ดู brief ที่ส่งต่อ") {
                Text(event.body)
                    .font(.caption2.monospaced())
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .font(.caption2)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

}
