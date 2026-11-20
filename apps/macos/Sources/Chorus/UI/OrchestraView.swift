import SwiftUI

// Design board "Orchestra — เส้นเวลางานย่อย": the lead's plan on the left, one
// lane per sub-agent on a shared time axis in the middle (solid = done, striped
// = working, dashed = waiting), and the picked lane's details on the right.
// Times and states come from the ledger via Engine; nothing is computed here
// beyond placing bars on the axis.

struct OrchestraView: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @State private var picked: Int64?
    @State private var messaging: Engine.SubagentInfo?
    @State private var steer = ""

    private var subs: [Engine.SubagentInfo] {
        let mine = engine.subagentsByParent[engine.sessionID] ?? []
        let all = mine.isEmpty ? (engine.subagentsByDB[shell.currentDB] ?? []) : mine
        return all.sorted { $0.created_ms < $1.created_ms }
    }

    var body: some View {
        HStack(spacing: 0) {
            plan.frame(width: 290)
            Hairline(vertical: true)
            lanes
            if let s = subs.first(where: { $0.id == picked }) ?? subs.first(where: { $0.status == "running" }) {
                Hairline(vertical: true)
                detail(s).frame(width: 310)
            }
        }
        .background(Palette.window)
        .onAppear { engine.loadSubagents(db: shell.currentDB) }
        .alert("ส่งข้อความถึง agent ย่อย", isPresented: Binding(get: { messaging != nil }, set: { if !$0 { messaging = nil } }),
               presenting: messaging) { sub in
            TextField("ข้อความ", text: $steer)
            Button("ส่ง") {
                let t = steer.trimmingCharacters(in: .whitespacesAndNewlines)
                if !t.isEmpty { engine.messageSubagent(db: shell.currentDB, id: sub.id, message: t) }
                messaging = nil
            }
            Button("ยกเลิก", role: .cancel) { messaging = nil }
        } message: { sub in Text(sub.task) }
    }

    // MARK: plan

    private var plan: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                if let lead = engine.events.last(where: { $0.kind == "msg" && $0.actor != "user" })?.actor {
                    ProviderAvatar(provider: lead, size: 26)
                }
                VStack(alignment: .leading, spacing: 1) {
                    Text("แผนของหัวหน้าวง").font(Typo.headline)
                    Text("อ่านอย่างเดียว · แจกงานให้ agent ย่อย").font(Typo.tiny).foregroundStyle(Palette.text3)
                }
            }
            if engine.checklist.isEmpty {
                Text("ยังไม่มีแผน เปลี่ยนช่อง Auto ในช่องพิมพ์เป็น Orchestra แล้วสั่งงาน หัวหน้าวงจะแตกงานเมื่อคุ้ม")
                    .font(Typo.caption).foregroundStyle(Palette.text3).lineSpacing(2)
            }
            ForEach(engine.checklist) { item in
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Group {
                        if item.done { RoundedRectangle(cornerRadius: 4).fill(Palette.ok).frame(width: 13, height: 13) }
                        else if item.in_progress { Spinner(size: 13) }
                        else { RoundedRectangle(cornerRadius: 4).strokeBorder(Palette.text3, style: StrokeStyle(lineWidth: 1.5, dash: [3, 2])).frame(width: 13, height: 13) }
                    }
                    .frame(width: 14)
                    Text(item.text).font(Typo.ui).foregroundStyle(item.done ? Palette.text2 : Palette.text).lineSpacing(2)
                }
            }
            Spacer()
            Text("งานที่เขียนไฟล์แยก worktree ของตัวเอง แล้วรวมหลังคุณดู diff")
                .font(Typo.caption).foregroundStyle(Palette.text2)
                .padding(12).frame(maxWidth: .infinity, alignment: .leading)
                .background(Palette.hover, in: RoundedRectangle(cornerRadius: 8))
        }
        .padding(18)
        .background(Palette.sidebar)
    }

    // MARK: lanes

    private var lanes: some View {
        let start = subs.map(\.created_ms).min() ?? 0
        let end = max((subs.map(\.updated_ms).max() ?? start) + 1, Int64(Date().timeIntervalSince1970 * 1000))
        let span = Double(max(end - start, 1))
        return VStack(spacing: 0) {
            HStack {
                Text("งานย่อย \(subs.count)").font(Typo.headline)
                Spacer()
                if subs.contains(where: { $0.status == "running" || $0.status == "waiting" }) {
                    Button("หยุดทั้งวง") {
                        for s in subs where s.status == "running" || s.status == "waiting" {
                            engine.cancelSubagent(db: shell.currentDB, id: s.id)
                        }
                    }
                    .buttonStyle(QuietButtonStyle(small: true, danger: true))
                }
            }
            .padding(.horizontal, 20).frame(height: 44)
            Hairline()
            if subs.isEmpty {
                ContentUnavailableView {
                    Label("ยังไม่มีงานย่อย", systemImage: "point.3.connected.trianglepath.dotted")
                } description: {
                    Text("เมื่อหัวหน้าวงแตกงาน แต่ละ agent จะได้เส้นเวลาของตัวเองที่นี่")
                }
            } else {
                HStack {
                    Spacer().frame(width: 236)
                    ForEach(0..<4) { i in
                        Text(Self.clock(start + Int64(span * Double(i) / 4))).frame(maxWidth: .infinity, alignment: .leading)
                    }
                    Text("ตอนนี้")
                }
                .font(Typo.monoSmall).foregroundStyle(Palette.text3)
                .padding(.horizontal, 20).padding(.top, 12)
                ScrollView {
                    VStack(spacing: 4) {
                        ForEach(subs) { s in lane(s, start: start, span: span) }
                    }
                    .padding(.horizontal, 10).padding(.vertical, 8)
                    HStack(spacing: 16) {
                        Text("■ เสร็จ"); Text("▤ กำลังทำ").foregroundStyle(Palette.accentText); Text("▢ รอคิว")
                    }
                    .font(Typo.caption).foregroundStyle(Palette.text3)
                    .padding(.leading, 246).frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
    }

    private func lane(_ s: Engine.SubagentInfo, start: Int64, span: Double) -> some View {
        let x0 = Double(s.created_ms - start) / span
        let x1 = Double(s.updated_ms - start) / span
        let selected = picked == s.id
        return Button { picked = s.id } label: {
            HStack(alignment: .top, spacing: 0) {
                HStack(alignment: .top, spacing: 10) {
                    ProviderAvatar(provider: s.model ?? s.role, size: 30)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(s.role).font(.system(size: 13, weight: .medium)).foregroundStyle(Palette.text)
                        Text(s.model ?? s.kind).font(Typo.tiny).foregroundStyle(Palette.text3).lineLimit(1)
                        Text(SubStatus.label(s.status)).font(Typo.tiny).foregroundStyle(Palette.text2)
                    }
                }
                .frame(width: 226, alignment: .leading)
                VStack(alignment: .leading, spacing: 8) {
                    GeometryReader { g in
                        ZStack(alignment: .leading) {
                            HStack(spacing: 0) {
                                ForEach(0..<4) { _ in
                                    Rectangle().fill(Palette.line).frame(width: 1)
                                    Spacer(minLength: 0)
                                }
                            }
                            bar(s).frame(width: max(40, g.size.width * CGFloat(max(x1 - x0, 0.04))), height: 28)
                                .offset(x: g.size.width * CGFloat(min(x0, 0.95)))
                        }
                    }
                    .frame(height: 40)
                    Text("ล่าสุด  \(s.summary ?? s.task)").font(Typo.monoSmall).foregroundStyle(Palette.text2).lineLimit(1)
                }
            }
            .padding(12)
            .background(selected ? Palette.selected : .clear, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    @ViewBuilder
    private func bar(_ s: Engine.SubagentInfo) -> some View {
        let label = Text(s.diff_stat ?? s.task).font(Typo.tiny).lineLimit(1).padding(.horizontal, 8)
        switch s.status {
        case "running":
            label.foregroundStyle(Palette.accentText)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
                .background(Palette.accentWash, in: RoundedRectangle(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Palette.accent, lineWidth: 1.5))
        case "waiting", "queued":
            label.foregroundStyle(Palette.text3)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
                .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Palette.line2, style: StrokeStyle(lineWidth: 1.5, dash: [4, 3])))
        case "failed":
            label.foregroundStyle(Palette.bad)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
                .background(Palette.badWash, in: RoundedRectangle(cornerRadius: 6))
        default:
            label.foregroundStyle(Palette.text)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
                .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Palette.line2))
        }
    }

    // MARK: detail

    private func detail(_ s: Engine.SubagentInfo) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 10) {
                ProviderAvatar(provider: s.model ?? s.role, size: 32)
                VStack(alignment: .leading, spacing: 1) {
                    Text(s.role).font(.system(size: 15, weight: .semibold))
                    Text(s.model ?? s.kind).font(Typo.tiny).foregroundStyle(Palette.text3)
                }
                Spacer()
                Tag(text: SubStatus.label(s.status), tone: SubStatus.tone(s.status))
            }
            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 8) {
                GridRow { Text("สิทธิ์").foregroundStyle(Palette.text3); Text(s.worktree == nil ? "อ่านอย่างเดียว" : "เขียนใน worktree") }
                if let w = s.worktree {
                    GridRow { Text("worktree").foregroundStyle(Palette.text3); Text(w).font(Typo.monoSmall).lineLimit(1).truncationMode(.head) }
                }
                if let d = s.diff_stat { GridRow { Text("ที่แก้").foregroundStyle(Palette.text3); Text(d).font(Typo.monoSmall) } }
            }
            .font(Typo.small)
            VStack(alignment: .leading, spacing: 6) {
                GroupHeader("ได้รับมอบหมาย")
                Text(s.task).font(Typo.ui).foregroundStyle(Palette.text2).lineSpacing(2)
            }
            if let sum = s.summary, !sum.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    GroupHeader("ล่าสุด")
                    Text(sum).font(Typo.ui).foregroundStyle(Palette.text2).lineSpacing(2)
                }
            }
            if !s.files_touched.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    GroupHeader("ไฟล์")
                    ForEach(s.files_touched.prefix(6), id: \.self) { Text($0).font(Typo.monoSmall).foregroundStyle(Palette.text2) }
                }
            }
            Spacer()
            VStack(spacing: 8) {
                if s.status == "running" || s.status == "waiting" {
                    Button("ส่งข้อความถึง \(s.role)") { steer = ""; messaging = s }
                        .buttonStyle(SecondaryButtonStyle()).frame(maxWidth: .infinity)
                }
                HStack(spacing: 8) {
                    if let c = s.child_sess {
                        Button("เปิดแชท") { shell.open(SessionKey(folder: shell.workspace, id: c)) }
                            .buttonStyle(SecondaryButtonStyle()).frame(maxWidth: .infinity)
                    }
                    if let w = s.worktree {
                        Button("เปิดใน Finder") { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: w)]) }
                            .buttonStyle(SecondaryButtonStyle()).frame(maxWidth: .infinity)
                    }
                }
                if s.status == "running" || s.status == "waiting" {
                    Button("หยุดงานนี้") { engine.cancelSubagent(db: shell.currentDB, id: s.id) }
                        .buttonStyle(QuietButtonStyle(danger: true))
                }
            }
        }
        .padding(18)
        .background(Palette.sidebar)
    }

    private static func clock(_ ms: Int64) -> String {
        let f = DateFormatter()
        f.dateFormat = "HH:mm"
        return f.string(from: Date(timeIntervalSince1970: Double(ms) / 1000))
    }
}
