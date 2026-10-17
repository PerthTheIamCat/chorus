import SwiftUI
import AppKit

/// Multi-Agent Orchestrator View (Ticket 02, 12 & 16).
/// Real-time supervision of parallel agent worktrees, subagent hierarchies,
/// dynamic roles, and dispatch steering.
struct OrchestratorView: View {
    @ObservedObject var engine: Engine
    let workspace: String
    var onSelectWorktree: (Engine.WorktreeInfo) -> Void
    var onDispatchParallelPrompt: (String) -> Void

    @State private var worktrees: [Engine.WorktreeInfo] = []
    @State private var roles: [Engine.AgentRole] = []
    @State private var isLoading = false
    @State private var expandedTranscript: String? = nil
    @State private var isShowingRoleEditor = false
    @State private var editingRole: Engine.AgentRole? = nil
    @State private var startWithAIMode = false
    @State private var roleToDelete: Engine.AgentRole? = nil

    private struct SubagentLedgerPayload: Decodable {
        let event: String
        let sessionId: String
        let parentSessionId: String?
        let name: String?
        let task: String?
        let state: String?
        let message: String?
    }

    private struct SubagentItem: Identifiable {
        let id: String
        let parentSessionId: String
        var name: String
        var task: String
        let actor: String
        let seq: Int64
        var state: String?
        var message: String
    }

    private var subagentItems: [SubagentItem] {
        var items: [String: SubagentItem] = [:]
        for ev in engine.events where ev.kind == "subagent" || ev.kind == "tool_call" || ev.kind == "tool_result" {
            guard let data = ev.body.data(using: .utf8),
                  let payload = try? JSONDecoder().decode(SubagentLedgerPayload.self, from: data) else {
                continue
            }
            if var existing = items[payload.sessionId] {
                if let name = payload.name { existing.name = name }
                if let task = payload.task { existing.task = task }
                if let state = payload.state { existing.state = state }
                if let msg = payload.message { existing.message = msg }
                items[payload.sessionId] = existing
            } else {
                items[payload.sessionId] = SubagentItem(
                    id: payload.sessionId,
                    parentSessionId: payload.parentSessionId ?? "",
                    name: payload.name ?? payload.sessionId,
                    task: payload.task ?? "",
                    actor: ev.actor,
                    seq: ev.seq,
                    state: payload.state,
                    message: payload.message ?? ""
                )
            }
        }
        return items.values.sorted { $0.seq < $1.seq }
    }

    var body: some View {
        VStack(spacing: 0) {
            // Header
            HStack(spacing: 10) {
                Image(systemName: "point.3.connected.trianglepath.dotted")
                    .foregroundColor(.accentColor)
                    .font(.system(size: 16, weight: .semibold))

                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Text("ORCHESTRATOR")
                            .font(.system(size: 13, weight: .bold))
                        Text("MULTI-AGENT")
                            .font(.system(size: 9, weight: .heavy))
                            .padding(.horizontal, 5)
                            .padding(.vertical, 1.5)
                            .background(Color.accentColor.opacity(0.15))
                            .foregroundColor(.accentColor)
                            .clipShape(Capsule())
                    }
                    Text("การกระจายงานขนาน & กิ่งแยก Git Worktree")
                        .font(.system(size: 10))
                        .foregroundColor(.secondary)
                }

                Spacer()

                Button(action: reloadWorktrees) {
                    Image(systemName: "arrow.clockwise")
                        .font(.system(size: 11))
                }
                .buttonStyle(.plain)
                .help("รีเฟรชข้อมูล Worktree")
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            .background(Color(NSColor.controlBackgroundColor).opacity(0.5))

            Divider()

            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    // Status Overview Card
                    statusOverviewCard

                    // Parallel Worktrees Section
                    worktreesSection

                    // Subagents Hierarchy Section
                    subagentsHierarchySection

                    // Available Roles Section
                    agentRolesSection

                    // Quick Actions
                    quickActionsSection
                }
                .padding(12)
            }
            .chorusScrollIndicators()
        }
        .onAppear {
            reloadWorktrees()
            reloadRoles()
        }
        .sheet(isPresented: $isShowingRoleEditor) {
            RoleEditorSheet(
                engine: engine,
                workspace: workspace,
                initialRole: editingRole,
                initialAIMode: startWithAIMode,
                onSaved: {
                    reloadRoles()
                    isShowingRoleEditor = false
                },
                onCancel: {
                    isShowingRoleEditor = false
                }
            )
            .frame(minWidth: 540, minHeight: 520)
        }
        .confirmationDialog(
            "ต้องการลบ Role '\(roleToDelete?.name ?? "")' หรือไม่?",
            isPresented: Binding(
                get: { roleToDelete != nil },
                set: { if !$0 { roleToDelete = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button("ลบ Role", role: .destructive) {
                if let r = roleToDelete {
                    engine.deleteRole(cwd: workspace, name: r.name) { ok, _ in
                        if ok { reloadRoles() }
                    }
                }
                roleToDelete = nil
            }
            Button("ยกเลิก", role: .cancel) {
                roleToDelete = nil
            }
        } message: {
            Text("ระบบจะลบไฟล์ .chorus/agents/\(roleToDelete?.name ?? "").md ออกจากโปรเจกต์")
        }
    }

    // MARK: - Status Overview Card
    private var statusOverviewCard: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: "crown.fill")
                    .foregroundColor(.yellow)
                    .font(.system(size: 13))
                Text("หลักการทำงาน ORCHESTRATION & ROLES")
                    .font(.system(size: 11, weight: .bold))
                Spacer()
                HStack(spacing: 6) {
                    Circle()
                        .fill(worktrees.isEmpty ? Color.green : Color.orange)
                        .frame(width: 8, height: 8)
                    Text(worktrees.isEmpty ? "สแตนด์บาย (พร้อมรับงานขนาน)" : "กำลังทำงานขนาน (\(worktrees.count) งาน)")
                        .font(.system(size: 10, weight: .semibold))
                }
            }

            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .top, spacing: 8) {
                    Text("👑")
                        .font(.system(size: 13))
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Lead Orchestrator (ตัวคุมงาน)")
                            .font(.system(size: 11, weight: .bold))
                        Text("ตามหลักการควรเลือกโมเดลที่ฉลาดและมี Reasoning สูงที่สุด ที่มีอยู่ เพื่อทำหน้าที่วิเคราะห์เป้าหมาย แตกงานย่อย ควบคุมลำดับงาน และสังเคราะห์ข้อสรุปรวม")
                            .font(.system(size: 10))
                            .foregroundColor(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }

                Divider().opacity(0.4)

                HStack(alignment: .top, spacing: 8) {
                    Text("👷")
                        .font(.system(size: 13))
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Specialist Subagents (ลูกทีมทำงานขนาน)")
                            .font(.system(size: 11, weight: .bold))
                        Text("สามารถกำหนดโมเดลที่เร็วและประหยัด (เช่น Gemini Flash, Haiku, Codex) แยกทำหน้าที่ค้นหาโค้ด (investigator) หรือเขียนโค้ด (builder) ใน Git Worktrees (.chorus/worktrees/) โดยไม่กระทบกิ่งหลัก")
                            .font(.system(size: 10))
                            .foregroundColor(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            .padding(10)
            .background(Color(NSColor.controlBackgroundColor).opacity(0.6))
            .cornerRadius(7)
        }
        .padding(12)
        .background(Color.primary.opacity(0.04))
        .cornerRadius(9)
    }

    // MARK: - Worktrees Section
    private var worktreesSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Image(systemName: "arrow.triangle.branch")
                    .foregroundColor(.accentColor)
                    .font(.system(size: 11))
                Text("ACTIVE AGENT WORKTREES")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundColor(.secondary)
                Spacer()
                Text("\(worktrees.count) ACTIVE")
                    .font(.system(size: 9, weight: .bold))
                    .foregroundColor(worktrees.isEmpty ? .secondary : .orange)
            }

            if worktrees.isEmpty {
                HStack(spacing: 8) {
                    Image(systemName: "checkmark.shield")
                        .foregroundColor(.secondary)
                        .font(.system(size: 14))
                    Text("ไม่มี Worktree ที่ทำงานค้างอยู่ (ระบบอยู่ในสถานะ Clean)")
                        .font(.system(size: 10.5))
                        .foregroundColor(.secondary)
                }
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.primary.opacity(0.02))
                .cornerRadius(6)
            } else {
                ForEach(worktrees, id: \.id) { wt in
                    Button(action: { onSelectWorktree(wt) }) {
                        VStack(alignment: .leading, spacing: 4) {
                            HStack(spacing: 6) {
                                Image(systemName: "cpu")
                                    .foregroundColor(.orange)
                                    .font(.system(size: 11))
                                Text(wt.role.uppercased())
                                    .font(.system(size: 10, weight: .bold))
                                    .foregroundColor(.primary)

                                Spacer()

                                Text("PORT :\(wt.allocated_port)")
                                    .font(.system(size: 9, weight: .bold, design: .monospaced))
                                    .padding(.horizontal, 5)
                                    .padding(.vertical, 1)
                                    .background(Color.blue.opacity(0.15))
                                    .foregroundColor(.blue)
                                    .cornerRadius(4)
                            }

                            HStack(spacing: 4) {
                                Image(systemName: "arrow.triangle.branch")
                                    .foregroundColor(.secondary)
                                    .font(.system(size: 9))
                                Text(wt.branch)
                                    .font(.system(size: 10, design: .monospaced))
                                    .foregroundColor(.secondary)
                                    .lineLimit(1)
                            }
                        }
                        .padding(8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Color.primary.opacity(0.05))
                        .cornerRadius(6)
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }

    // MARK: - Subagents Hierarchy Section
    private var subagentsHierarchySection: some View {
        let items = subagentItems
        let calls: [(call: LedgerEvent, result: LedgerEvent?)] = engine.events.enumerated().compactMap { i, e in
            guard e.kind == "tool_call", isSubagentCall(e.body) else { return nil }
            let r = engine.events[(i + 1)...].first { $0.kind == "tool_result" }
            return (e, r)
        }

        return VStack(alignment: .leading, spacing: 8) {
            HStack {
                Image(systemName: "network")
                    .foregroundColor(.accentColor)
                    .font(.system(size: 11))
                Text("SUBAGENT HIERARCHY")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundColor(.secondary)
                Spacer()
                Text("\(items.isEmpty ? calls.count : items.count) NODES")
                    .font(.system(size: 9, weight: .bold))
                    .foregroundColor(.secondary)
            }

            if items.isEmpty && calls.isEmpty {
                VStack(spacing: 6) {
                    Image(systemName: "point.3.connected.trianglepath.dotted")
                        .foregroundColor(.secondary)
                        .font(.system(size: 20))
                    Text("ยังไม่มีการเรียกงานย่อยในเทิร์นปัจจุบัน")
                        .font(.system(size: 11))
                        .foregroundColor(.secondary)
                    Text("เมื่อ agent เรียกใช้ Task หรือ subagent ผลลัพธ์และการประสานงานจะปรากฏที่นี่")
                        .font(.system(size: 9.5))
                        .foregroundColor(.secondary.opacity(0.8))
                        .multilineTextAlignment(.center)
                }
                .padding(14)
                .frame(maxWidth: .infinity)
                .background(Color.primary.opacity(0.02))
                .cornerRadius(6)
            } else if !items.isEmpty {
                ForEach(items) { item in
                    VStack(alignment: .leading, spacing: 6) {
                        HStack(spacing: 6) {
                            Image(systemName: item.state == "completed" ? "checkmark.circle.fill" : item.state == "failed" ? "xmark.circle.fill" : "hourglass")
                                .foregroundColor(item.state == "completed" ? .green : item.state == "failed" ? .red : .orange)
                                .font(.system(size: 11))

                            Text(item.name)
                                .font(.system(size: 11, weight: .bold))

                            Spacer()

                            Text(item.state ?? "กำลังทำงาน")
                                .font(.system(size: 9, weight: .semibold))
                                .padding(.horizontal, 4)
                                .padding(.vertical, 1)
                                .background(Color.primary.opacity(0.08))
                                .cornerRadius(3)
                        }

                        if !item.task.isEmpty {
                            Text(item.task)
                                .font(.system(size: 10.5))
                                .foregroundColor(.secondary)
                        }

                        if !item.message.isEmpty {
                            DisclosureGroup("ดูผลลัพธ์และ Brief สรุป") {
                                Text(item.message)
                                    .font(.system(size: 10, design: .monospaced))
                                    .foregroundColor(.primary)
                                    .textSelection(.enabled)
                                    .padding(.top, 4)
                            }
                            .font(.system(size: 10))
                        }
                    }
                    .padding(8)
                    .background(Color.primary.opacity(0.04))
                    .cornerRadius(6)
                }
            } else {
                ForEach(calls, id: \.call.seq) { c in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(spacing: 6) {
                            Image(systemName: c.result == nil ? "hourglass" : "checkmark.circle.fill")
                                .foregroundColor(c.result == nil ? .orange : .green)
                                .font(.system(size: 11))
                            Text(c.call.actor)
                                .font(.system(size: 11, weight: .bold))
                            Spacer()
                            Text("#\(c.call.seq)")
                                .font(.system(size: 9, design: .monospaced))
                                .foregroundColor(.secondary)
                        }
                        Text(c.call.body)
                            .font(.system(size: 10))
                            .lineLimit(2)
                    }
                    .padding(8)
                    .background(Color.primary.opacity(0.04))
                    .cornerRadius(6)
                }
            }
        }
    }

    // MARK: - Available Roles
    private var agentRolesSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: "person.3.sequence.fill")
                    .foregroundColor(.accentColor)
                    .font(.system(size: 11))
                Text("ORCHESTRATOR ROLES")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundColor(.secondary)

                Text("\(roles.count) ROLES")
                    .font(.system(size: 9, weight: .bold))
                    .padding(.horizontal, 5)
                    .padding(.vertical, 1)
                    .background(Color.secondary.opacity(0.12))
                    .foregroundColor(.secondary)
                    .cornerRadius(4)

                Spacer()

                Button(action: {
                    editingRole = nil
                    startWithAIMode = true
                    isShowingRoleEditor = true
                }) {
                    HStack(spacing: 4) {
                        Image(systemName: "sparkles")
                            .font(.system(size: 10))
                        Text("AI สร้าง Role")
                            .font(.system(size: 10, weight: .semibold))
                    }
                    .padding(.horizontal, 7)
                    .padding(.vertical, 3.5)
                    .background(Color.accentColor.opacity(0.12))
                    .foregroundColor(.accentColor)
                    .cornerRadius(5)
                }
                .buttonStyle(.plain)
                .help("ให้ AI ออกแบบ Role ใหม่ตามคำสั่งภารกิจ")

                Button(action: {
                    editingRole = nil
                    startWithAIMode = false
                    isShowingRoleEditor = true
                }) {
                    HStack(spacing: 3) {
                        Image(systemName: "plus")
                            .font(.system(size: 10))
                        Text("เพิ่ม Role")
                            .font(.system(size: 10, weight: .semibold))
                    }
                    .padding(.horizontal, 7)
                    .padding(.vertical, 3.5)
                    .background(Color.primary.opacity(0.06))
                    .foregroundColor(.primary)
                    .cornerRadius(5)
                }
                .buttonStyle(.plain)
                .help("สร้าง Custom Role กำหนดเอง")
            }

            if roles.isEmpty {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    Text("กำลังโหลด Roles…")
                        .font(.system(size: 10))
                        .foregroundColor(.secondary)
                }
                .padding(8)
            } else {
                VStack(spacing: 6) {
                    ForEach(roles) { r in
                        roleCard(role: r)
                    }
                }
            }
        }
    }

    private func roleCard(role: Engine.AgentRole) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: role.iconName)
                .foregroundColor(.accentColor)
                .font(.system(size: 13))
                .frame(width: 22, height: 22)
                .background(Color.accentColor.opacity(0.1))
                .clipShape(RoundedRectangle(cornerRadius: 5))

            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(role.name)
                        .font(.system(size: 11, weight: .bold))

                    if role.isBuiltin {
                        Text("BUILT-IN")
                            .font(.system(size: 8, weight: .bold))
                            .padding(.horizontal, 4)
                            .padding(.vertical, 1)
                            .background(Color.secondary.opacity(0.15))
                            .foregroundColor(.secondary)
                            .cornerRadius(3)
                    } else {
                        Text("CUSTOM")
                            .font(.system(size: 8, weight: .bold))
                            .padding(.horizontal, 4)
                            .padding(.vertical, 1)
                            .background(Color.purple.opacity(0.15))
                            .foregroundColor(.purple)
                            .cornerRadius(3)
                    }

                    // Model quick assignment dropdown
                    Menu {
                        Button("ค่าเริ่มต้น") {
                            quickAssignModel(role: role, model: nil)
                        }
                        Divider()
                        ForEach(modelSuggestions, id: \.self) { m in
                            Button(m) {
                                quickAssignModel(role: role, model: m)
                            }
                        }
                    } label: {
                        HStack(spacing: 3) {
                            Image(systemName: "cpu")
                                .font(.system(size: 8))
                            Text(role.model ?? "ตั้งโมเดล")
                                .font(.system(size: 8, design: .monospaced))
                            Image(systemName: "chevron.down")
                                .font(.system(size: 6))
                        }
                        .padding(.horizontal, 4)
                        .padding(.vertical, 1.5)
                        .background(role.model != nil ? Color.blue.opacity(0.15) : Color.secondary.opacity(0.1))
                        .foregroundColor(role.model != nil ? .blue : .secondary)
                        .cornerRadius(3)
                    }
                    .menuStyle(.borderlessButton)
                }

                Text(role.description)
                    .font(.system(size: 9.5))
                    .foregroundColor(.secondary)
                    .fixedSize(horizontal: false, vertical: true)

                // Tools tags
                HStack(spacing: 4) {
                    ForEach(role.tools.prefix(6), id: \.self) { t in
                        Text(t)
                            .font(.system(size: 8, design: .monospaced))
                            .padding(.horizontal, 4)
                            .padding(.vertical, 1)
                            .background(Color.primary.opacity(0.04))
                            .foregroundColor(.secondary)
                            .cornerRadius(3)
                    }
                    if role.tools.count > 6 {
                        Text("+\(role.tools.count - 6)")
                            .font(.system(size: 8))
                            .foregroundColor(.secondary)
                    }
                }
                .padding(.top, 1)
            }

            Spacer(minLength: 4)

            HStack(spacing: 4) {
                Button(action: {
                    onDispatchParallelPrompt("แตกงานย่อยให้ subagent ในบทบาท \(role.name) (ขอบเขต: \(role.description))")
                }) {
                    Image(systemName: "bolt.fill")
                        .font(.system(size: 10))
                        .foregroundColor(.accentColor)
                        .frame(width: 22, height: 22)
                        .background(Color.accentColor.opacity(0.12))
                        .clipShape(RoundedRectangle(cornerRadius: 4))
                }
                .buttonStyle(.plain)
                .help("สั่งกระจายงานด้วย Role นี้")

                Button(action: {
                    editingRole = role
                    startWithAIMode = false
                    isShowingRoleEditor = true
                }) {
                    Image(systemName: "pencil")
                        .font(.system(size: 10))
                        .foregroundColor(.secondary)
                        .frame(width: 22, height: 22)
                        .background(Color.primary.opacity(0.05))
                        .clipShape(RoundedRectangle(cornerRadius: 4))
                }
                .buttonStyle(.plain)
                .help(role.isBuiltin ? "ปรับแต่ง Prompt หรือตั้ง Model ของ Role นี้" : "แก้ไข Role นี้")

                if !role.isBuiltin {
                    Button(action: {
                        roleToDelete = role
                    }) {
                        Image(systemName: "trash")
                            .font(.system(size: 10))
                            .foregroundColor(.red.opacity(0.8))
                            .frame(width: 22, height: 22)
                            .background(Color.red.opacity(0.08))
                            .clipShape(RoundedRectangle(cornerRadius: 4))
                    }
                    .buttonStyle(.plain)
                    .help("ลบ Custom Role")
                }
            }
        }
        .padding(8)
        .background(Color.primary.opacity(0.03))
        .cornerRadius(6)
    }

    private var modelSuggestions: [String] {
        engine.knownBaseModels().map(\.id)
    }

    private func quickAssignModel(role: Engine.AgentRole, model: String?) {
        guard !workspace.isEmpty else { return }
        var updated = role
        updated.model = model
        engine.saveRole(cwd: workspace, role: updated) { ok, _ in
            if ok { reloadRoles() }
        }
    }

    // MARK: - Quick Actions
    private var quickActionsSection: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("QUICK DISPATCH")
                .font(.system(size: 10, weight: .bold))
                .foregroundColor(.secondary)

            Button(action: {
                onDispatchParallelPrompt("กรุณาวิเคราะห์ปัญหานี้ แล้วแตกงานให้ subagent ขนานกันใน Git worktree แยก เพื่อดำเนินการพร้อมกัน")
            }) {
                HStack(spacing: 6) {
                    Image(systemName: "bolt.badge.automatic.fill")
                        .foregroundColor(.accentColor)
                    Text("สั่งงานขนาน")
                        .font(.system(size: 11, weight: .semibold))
                    Spacer()
                    Image(systemName: "arrow.up.right")
                        .font(.system(size: 10))
                        .foregroundColor(.secondary)
                }
                .padding(8)
                .background(Color.accentColor.opacity(0.1))
                .cornerRadius(6)
            }
            .buttonStyle(.plain)

            if !workspace.isEmpty {
                Button(action: openWorktreesFolder) {
                    HStack(spacing: 6) {
                        Image(systemName: "folder")
                            .foregroundColor(.secondary)
                        Text("เปิดโฟลเดอร์ .chorus/worktrees")
                            .font(.system(size: 10.5))
                            .foregroundColor(.secondary)
                        Spacer()
                    }
                    .padding(6)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.top, 4)
    }

    private func reloadWorktrees() {
        guard !workspace.isEmpty else { return }
        isLoading = true
        engine.fetchWorktrees(cwd: workspace) { list in
            self.worktrees = list
            self.isLoading = false
        }
    }

    private func reloadRoles() {
        engine.fetchRoles(cwd: workspace) { list in
            self.roles = list
        }
    }

    private func openWorktreesFolder() {
        let path = (workspace as NSString).appendingPathComponent(".chorus/worktrees")
        let url = URL(fileURLWithPath: path)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        NSWorkspace.shared.open(url)
    }

    private func isSubagentCall(_ body: String) -> Bool {
        body.contains("\"subagent\"") || body.contains("\"Task\"") || body.contains("chorus_spawn_agent")
    }
}

// MARK: - Role Editor & AI Role Generator Sheet
struct RoleEditorSheet: View {
    @ObservedObject var engine: Engine
    let workspace: String
    let initialRole: Engine.AgentRole?
    let initialAIMode: Bool
    var onSaved: () -> Void
    var onCancel: () -> Void

    enum Mode: String, CaseIterable, Identifiable {
        case ai = "AI Role Generator"
        case manual = "กำหนดเอง (Manual)"
        var id: String { rawValue }
    }

    @State private var mode: Mode = .ai
    @State private var aiTaskPrompt: String = ""
    @State private var aiWriteMode: Bool = true
    @State private var aiNameHint: String = ""
    @State private var isSynthesizing: Bool = false
    @State private var aiSuccessNotice: String? = nil

    @State private var name: String = ""
    @State private var description: String = ""
    @State private var selectedTools: Set<String> = ["read", "write", "edit", "bash", "diff"]
    @State private var customToolInput: String = ""
    @State private var modelPreference: String = ""
    @State private var systemPrompt: String = ""
    @State private var errorMessage: String? = nil
    @State private var isSaving: Bool = false

    private let standardTools = ["read", "write", "edit", "bash", "diff", "grep", "find"]

    init(
        engine: Engine,
        workspace: String,
        initialRole: Engine.AgentRole?,
        initialAIMode: Bool,
        onSaved: @escaping () -> Void,
        onCancel: @escaping () -> Void
    ) {
        self.engine = engine
        self.workspace = workspace
        self.initialRole = initialRole
        self.initialAIMode = initialAIMode
        self.onSaved = onSaved
        self.onCancel = onCancel

        _mode = State(initialValue: (initialRole != nil || !initialAIMode) ? .manual : .ai)
        if let r = initialRole {
            _name = State(initialValue: r.name)
            _description = State(initialValue: r.description)
            _selectedTools = State(initialValue: Set(r.tools))
            _modelPreference = State(initialValue: r.model ?? "")
            _systemPrompt = State(initialValue: r.system_prompt)
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            // Header
            HStack(spacing: 10) {
                Image(systemName: mode == .ai ? "sparkles" : "person.badge.shield.checkmark")
                    .foregroundColor(.accentColor)
                    .font(.system(size: 16, weight: .bold))

                VStack(alignment: .leading, spacing: 2) {
                    Text(initialRole == nil ? (mode == .ai ? "AI ROLE GENERATOR" : "สร้าง AGENT ROLE ใหม่") : "แก้ไข ROLE: \(name)")
                        .font(.system(size: 13, weight: .bold))
                    Text("บันทึกใน .chorus/agents/*.md เพื่อให้ Orchestrator และ Subagent เรียกใช้งานได้")
                        .font(.system(size: 10))
                        .foregroundColor(.secondary)
                }

                Spacer()

                Button(action: onCancel) {
                    Image(systemName: "xmark.circle.fill")
                        .foregroundColor(.secondary)
                        .font(.system(size: 16))
                }
                .buttonStyle(.plain)
            }
            .padding(14)
            .background(Color(NSColor.controlBackgroundColor).opacity(0.6))

            Divider()

            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    // Mode Picker (only when creating new)
                    if initialRole == nil {
                        Picker("", selection: $mode) {
                            ForEach(Mode.allCases) { m in
                                Text(m.rawValue).tag(m)
                            }
                        }
                        .pickerStyle(.segmented)
                    }

                    // AI Generator Box
                    if mode == .ai {
                        aiGeneratorCard
                    }

                    // Role Details Form
                    roleDetailsForm
                }
                .padding(16)
            }
            .chorusScrollIndicators()

            Divider()

            // Footer
            HStack {
                if let err = errorMessage {
                    Text(err)
                        .font(.system(size: 11))
                        .foregroundColor(.red)
                        .lineLimit(1)
                }

                Spacer()

                Button("ยกเลิก", action: onCancel)
                    .buttonStyle(.plain)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                    .background(Color.primary.opacity(0.06))
                    .cornerRadius(6)

                Button(action: saveRole) {
                    HStack(spacing: 5) {
                        if isSaving {
                            ProgressView().controlSize(.small)
                        } else {
                            Image(systemName: "square.and.arrow.down")
                        }
                        Text("บันทึก Role (.chorus/agents)")
                            .font(.system(size: 11, weight: .semibold))
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 6)
                    .background(Color.accentColor)
                    .foregroundColor(.white)
                    .cornerRadius(6)
                }
                .buttonStyle(.plain)
                .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || isSaving)
            }
            .padding(12)
            .background(Color(NSColor.controlBackgroundColor).opacity(0.4))
        }
    }

    private var aiGeneratorCard: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Image(systemName: "sparkles")
                    .foregroundColor(.accentColor)
                Text("อธิบายบทบาทหรือภารกิจที่ต้องการให้ AI ออกแบบ")
                    .font(.system(size: 11, weight: .bold))
            }

            TextField("เช่น: วิเคราะห์ performance และหา memory leak ใน Rust หรือ ตรวจสอบความปลอดภัย API authentication", text: $aiTaskPrompt)
                .textFieldStyle(.roundedBorder)
                .font(.system(size: 11))

            HStack(spacing: 14) {
                Toggle("ให้แก้ไฟล์ได้", isOn: $aiWriteMode)
                    .font(.system(size: 10.5))
                    .toggleStyle(.switch)

                TextField("ชื่อ Role ที่ต้องการ (ไม่บังคับ เช่น leak-hunter)", text: $aiNameHint)
                    .textFieldStyle(.roundedBorder)
                    .font(.system(size: 10.5))
                    .frame(maxWidth: 240)
            }

            HStack {
                Spacer()
                Button(action: runAISynthesis) {
                    HStack(spacing: 6) {
                        if isSynthesizing {
                            ProgressView().controlSize(.small)
                        } else {
                            Image(systemName: "sparkles")
                        }
                        Text("ให้ AI ออกแบบ Role (Synthesize)")
                            .font(.system(size: 11, weight: .semibold))
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 5)
                    .background(Color.accentColor.opacity(0.15))
                    .foregroundColor(.accentColor)
                    .cornerRadius(6)
                }
                .buttonStyle(.plain)
                .disabled(aiTaskPrompt.trimmingCharacters(in: .whitespaces).isEmpty || isSynthesizing)
            }

            if let notice = aiSuccessNotice {
                HStack(spacing: 6) {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundColor(.green)
                    Text(notice)
                        .font(.system(size: 10.5))
                        .foregroundColor(.green)
                }
                .padding(6)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.green.opacity(0.08))
                .cornerRadius(5)
            }
        }
        .padding(12)
        .background(Color.accentColor.opacity(0.04))
        .cornerRadius(8)
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .stroke(Color.accentColor.opacity(0.2), lineWidth: 1)
        )
    }

    private var roleDetailsForm: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("รายละเอียด ROLE")
                .font(.system(size: 10, weight: .bold))
                .foregroundColor(.secondary)

            // Name
            VStack(alignment: .leading, spacing: 4) {
                Text("ชื่อ Role (Slug / ID)").font(.system(size: 10.5, weight: .medium))
                TextField("เช่น: security-auditor, test-engineer", text: $name)
                    .textFieldStyle(.roundedBorder)
                    .font(.system(size: 11, design: .monospaced))
                    .disabled(initialRole?.isBuiltin == true)
                Text("ใช้เป็นชื่อไฟล์ .chorus/agents/<name>.md และรหัสอ้างอิงตอนสั่งงาน").font(.system(size: 9)).foregroundColor(.secondary)
            }

            // Description
            VStack(alignment: .leading, spacing: 4) {
                Text("คำอธิบาย").font(.system(size: 10.5, weight: .medium))
                TextField("สรุปขอบเขตภารกิจสั้นๆ", text: $description)
                    .textFieldStyle(.roundedBorder)
                    .font(.system(size: 11))
            }

            // Tools Selection
            VStack(alignment: .leading, spacing: 6) {
                Text("เครื่องมือที่ใช้ได้").font(.system(size: 10.5, weight: .medium))
                HStack(spacing: 6) {
                    ForEach(standardTools, id: \.self) { tool in
                        let isSelected = selectedTools.contains(tool)
                        Button(action: {
                            if isSelected { selectedTools.remove(tool) }
                            else { selectedTools.insert(tool) }
                        }) {
                            HStack(spacing: 4) {
                                Image(systemName: isSelected ? "checkmark.circle.fill" : "circle")
                                    .font(.system(size: 10))
                                Text(tool)
                                    .font(.system(size: 10, design: .monospaced))
                            }
                            .padding(.horizontal, 7)
                            .padding(.vertical, 3.5)
                            .background(isSelected ? Color.accentColor.opacity(0.15) : Color.primary.opacity(0.04))
                            .foregroundColor(isSelected ? .accentColor : .primary)
                            .cornerRadius(5)
                        }
                        .buttonStyle(.plain)
                    }
                }

                // Add Custom Tool
                HStack(spacing: 6) {
                    TextField("เพิ่มเครื่องมืออื่น (เช่น chorus_request_review)", text: $customToolInput)
                        .textFieldStyle(.roundedBorder)
                        .font(.system(size: 10, design: .monospaced))
                    Button("เพิ่ม") {
                        let t = customToolInput.trimmingCharacters(in: .whitespaces)
                        if !t.isEmpty {
                            selectedTools.insert(t)
                            customToolInput = ""
                        }
                    }
                    .buttonStyle(.plain)
                    .font(.system(size: 10))
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(Color.primary.opacity(0.06))
                    .cornerRadius(4)
                }
            }

            // Model preference
            VStack(alignment: .leading, spacing: 4) {
                Text("โมเดลที่แนะนำ (Model Preference - ไม่บังคับ)").font(.system(size: 10.5, weight: .medium))
                TextField("เว้นว่างไว้เพื่อใช้ค่าเริ่มต้น หรือระบุ model id จาก `chorus models`", text: $modelPreference)
                    .textFieldStyle(.roundedBorder)
                    .font(.system(size: 10.5, design: .monospaced))
            }

            // System prompt
            VStack(alignment: .leading, spacing: 4) {
                Text("System Prompt (คำสั่งหลักสำหรับบทบาทนี้)").font(.system(size: 10.5, weight: .medium))
                TextEditor(text: $systemPrompt)
                    .font(.system(size: 10.5, design: .monospaced))
                    .frame(minHeight: 120, maxHeight: 180)
                    .padding(4)
                    .background(Color.primary.opacity(0.03))
                    .cornerRadius(6)
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.primary.opacity(0.1), lineWidth: 1))
            }
        }
        .padding(12)
        .background(Color.primary.opacity(0.02))
        .cornerRadius(8)
    }

    private func runAISynthesis() {
        let task = aiTaskPrompt.trimmingCharacters(in: .whitespaces)
        guard !task.isEmpty else { return }
        isSynthesizing = true
        errorMessage = nil
        aiSuccessNotice = nil

        let hint = aiNameHint.trimmingCharacters(in: .whitespaces)
        engine.synthesizeRole(task: task, writeMode: aiWriteMode, nameHint: hint.isEmpty ? nil : hint) { synthesized in
            self.isSynthesizing = false
            if let r = synthesized {
                self.name = r.name
                self.description = r.description
                self.selectedTools = Set(r.tools)
                self.systemPrompt = r.system_prompt
                self.aiSuccessNotice = "AI ออกแบบ Role '\(r.name)' สำเร็จแล้ว! คุณสามารถปรับแต่งรายละเอียดก่อนบันทึกได้"
            } else {
                self.errorMessage = "ไม่สามารถสร้าง Role ได้ โปรดลองใหม่อีกครั้ง"
            }
        }
    }

    private func saveRole() {
        let trimmedName = name.trimmingCharacters(in: .whitespaces).lowercased()
        guard !trimmedName.isEmpty else {
            errorMessage = "กรุณาระบุชื่อ Role"
            return
        }

        let trimmedPrompt = systemPrompt.trimmingCharacters(in: .whitespaces)
        guard !trimmedPrompt.isEmpty else {
            errorMessage = "กรุณาระบุ System Prompt"
            return
        }

        isSaving = true
        errorMessage = nil

        var toolsList = Array(selectedTools).sorted()
        if toolsList.isEmpty { toolsList = ["read"] }

        let roleToSave = Engine.AgentRole(
            name: trimmedName,
            description: description.trimmingCharacters(in: .whitespaces).isEmpty ? "Custom agent role \(trimmedName)" : description.trimmingCharacters(in: .whitespaces),
            system_prompt: trimmedPrompt,
            tools: toolsList,
            model: modelPreference.trimmingCharacters(in: .whitespaces).isEmpty ? nil : modelPreference.trimmingCharacters(in: .whitespaces)
        )

        engine.saveRole(cwd: workspace, role: roleToSave) { success, err in
            self.isSaving = false
            if success {
                self.onSaved()
            } else {
                self.errorMessage = err ?? "บันทึก Role ไม่สำเร็จ"
            }
        }
    }
}
