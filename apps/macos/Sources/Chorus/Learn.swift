import AppKit
import CChorus
import SwiftUI

// MARK: - Learn (Settings › เรียนรู้)
//
// What the app has learned from past work, and how much it may learn without
// asking. Swift only draws and forwards taps: scoring, drafting, the
// autonomy rules and the minimum sample sizes all live in Rust
// (chorus-core `outcomes` / `skill_drafts` / `project_notes` / `learn`,
// reached through `chorus_learn_*`). CLAUDE.md 2.3.

struct LearnDraft: Codable, Identifiable, Hashable {
    let name: String
    let description: String
    let signature: String
    let steps: [String]
    let sessions: [Int64]
    let body: String
    var id: String { name }
}

struct LearnCell: Codable, Identifiable, Hashable {
    let task: String
    let task_label: String
    let agent: String
    let model: String
    let n: Int
    let passed: Int
    let lower_bound: Double
    let steers: Bool
    var id: String { "\(task)|\(agent)|\(model)" }
}

struct LearnOutcomes: Codable, Hashable {
    let cells: [LearnCell]
    let counted_turns: Int
    let min_samples: Int
    let enabled: Bool
}

struct LearnState: Codable {
    let ok: Bool
    let error: String?
    let autonomy: String?
    let drafts: [LearnDraft]?
    let outcomes: LearnOutcomes?
    let notes: String?
    let notes_enabled: Bool?
}

private struct LearnReply: Codable {
    let ok: Bool
    let error: String?
}

/// Calls into Rust for the Learn screen. Everything runs on one utility queue
/// and opens its own database connections, so it never waits behind a turn.
@MainActor
final class LearnModel: ObservableObject {
    @Published private(set) var state: LearnState?
    @Published private(set) var working = false
    @Published private(set) var message: String?

    private let queue = DispatchQueue(label: "chorus.learn", qos: .utility)

    nonisolated static func db(for workspace: String) -> String {
        (workspace as NSString).appendingPathComponent(".chorus/store.db")
    }

    nonisolated private static func string(from p: UnsafeMutablePointer<CChar>?) -> String {
        guard let p else { return "" }
        defer { chorus_string_free(p) }
        return String(cString: p)
    }

    nonisolated private static func decode<T: Decodable>(_ type: T.Type, _ json: String) -> T? {
        guard let data = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(T.self, from: data)
    }

    func load(workspace: String) {
        guard !workspace.isEmpty else { return }
        let db = Self.db(for: workspace)
        queue.async { [weak self] in
            let json = db.withCString { d in
                workspace.withCString { c in Self.string(from: chorus_learn_state_json(d, c)) }
            }
            let s = Self.decode(LearnState.self, json)
            DispatchQueue.main.async {
                guard let self else { return }
                if let s, s.ok {
                    self.state = s
                } else {
                    self.message = s?.error ?? "โหลดสิ่งที่เรียนรู้ไม่สำเร็จ"
                }
            }
        }
    }

    /// Run the idle pass now (scores turns, looks for recipes) and reload.
    func learnNow(workspace: String) {
        guard !workspace.isEmpty, !working else { return }
        working = true
        message = nil
        let db = Self.db(for: workspace)
        queue.async { [weak self] in
            let json = db.withCString { d in
                workspace.withCString { c in Self.string(from: chorus_learn_idle_json(d, c)) }
            }
            let r = Self.decode(LearnReply.self, json)
            DispatchQueue.main.async {
                guard let self else { return }
                self.working = false
                if r?.ok != true { self.message = r?.error ?? "เรียนรู้ไม่สำเร็จ" }
                self.load(workspace: workspace)
            }
        }
    }

    func setAutonomy(_ mode: String, workspace: String) {
        queue.async { [weak self] in
            let json = mode.withCString { Self.string(from: chorus_learn_set_autonomy($0)) }
            let r = Self.decode(LearnReply.self, json)
            DispatchQueue.main.async {
                guard let self else { return }
                if r?.ok != true { self.message = r?.error ?? "บันทึกไม่สำเร็จ" }
                self.load(workspace: workspace)
            }
        }
    }

    func decide(_ draft: LearnDraft, accept: Bool, workspace: String) {
        queue.async { [weak self] in
            let json = workspace.withCString { c in
                draft.name.withCString { n in
                    (accept ? "accept" : "reject").withCString { a in
                        Self.string(from: chorus_learn_draft_json(c, n, a))
                    }
                }
            }
            let r = Self.decode(LearnReply.self, json)
            DispatchQueue.main.async {
                guard let self else { return }
                if r?.ok != true {
                    self.message = r?.error ?? "ทำรายการไม่สำเร็จ"
                } else if accept {
                    self.message = "เก็บเป็น skill แล้ว — จะถูกใช้กับทุก agent ในโปรเจกต์นี้ตั้งแต่เทิร์นถัดไป"
                }
                self.load(workspace: workspace)
            }
        }
    }
}

/// Runs the idle pass in the background at most every few minutes. Called
/// after the warm-tier compaction pass, so it follows the same "user went
/// quiet" signal. It has its own queue and connections: it never holds the
/// engine's queue (CLAUDE.md 7.7).
enum LearnIdle {
    private static var lastRun: [String: Date] = [:]
    private static let queue = DispatchQueue(label: "chorus.learnIdle", qos: .background)
    private static let every: TimeInterval = 5 * 60

    @MainActor
    static func runIfDue(db: String) {
        guard !db.isEmpty else { return }
        if let last = lastRun[db], Date().timeIntervalSince(last) < every { return }
        lastRun[db] = Date()
        let cwd = ((db as NSString).deletingLastPathComponent as NSString).deletingLastPathComponent
        queue.async {
            let p = db.withCString { d in cwd.withCString { c in chorus_learn_idle_json(d, c) } }
            if let p { chorus_string_free(p) }
        }
    }
}

struct LearnSettings: View {
    let workspace: String
    @StateObject private var model = LearnModel()
    @State private var openDraft: String?

    private var autonomy: Binding<String> {
        Binding(
            get: { model.state?.autonomy ?? "ask" },
            set: { model.setAutonomy($0, workspace: workspace) })
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            if workspace.isEmpty {
                Text("เปิดโปรเจกต์ก่อน แล้วจะเห็นสิ่งที่ Chorus เรียนรู้จากงานในโปรเจกต์นั้น")
                    .foregroundStyle(.secondary)
            } else {
                autonomyGroup
                draftsGroup
                outcomesGroup
                notesGroup
                if let m = model.message {
                    Text(m).font(.callout).foregroundStyle(.secondary)
                        .accessibilityLabel(m)
                }
            }
        }
        .onAppear { model.load(workspace: workspace) }
    }

    private var autonomyGroup: some View {
        SettingsGroup(title: "ให้เรียนรู้เองได้แค่ไหน",
                      footer: "Skill ที่ร่างจากงานเก่าต้องให้คุณอ่านก่อนเสมอ ไม่ว่าเลือกโหมดไหน — skill ที่ไม่มีคนรีวิวไม่ช่วยให้ agent เก่งขึ้น") {
            SettingsRow(title: "โหมด",
                        info: "ถามทุกครั้ง = ทุกอย่างที่เรียนรู้ได้รอให้คุณกดรับ · รับเองเมื่อมีผลเทสต์ผ่าน = บทเรียนที่มีหลักฐานเป็นเทสต์ผ่านถูกรับเลย แต่ของที่มาจากเว็บหรือ session ที่นำเข้ายังต้องถามเสมอ · ปิด = ไม่ร่างอะไรเลย") {
                Picker("โหมด", selection: autonomy) {
                    Text("ถามทุกครั้ง").tag("ask")
                    Text("รับเองเมื่อเทสต์ผ่าน").tag("auto_with_tests")
                    Text("ปิด").tag("off")
                }
                .pickerStyle(.segmented).labelsHidden().frame(width: 330)
            }
            RowDivider()
            SettingsRow(title: "เรียนรู้จากงานล่าสุดตอนนี้",
                        info: "ปกติ Chorus ทำเองเบื้องหลังตอนคุณไม่ได้ใช้งาน ไม่กระทบเทิร์นที่กำลังรัน") {
                HStack(spacing: 8) {
                    if model.working { ProgressView().controlSize(.small) }
                    Button("เรียนรู้ตอนนี้") { model.learnNow(workspace: workspace) }
                        .disabled(model.working)
                }
            }
        }
    }

    private var draftsGroup: some View {
        let drafts = model.state?.drafts ?? []
        return SettingsGroup(title: drafts.isEmpty ? "ร่าง Skill" : "ร่าง Skill รอคุณดู \(drafts.count) ข้อ",
                             footer: "ร่างขึ้นเมื่อลำดับคำสั่งเดียวกันเกิดขึ้นในอย่างน้อย 3 session ของโปรเจกต์นี้ และทุก session จบด้วยเทสต์ผ่าน · เก็บแล้วจะอยู่ใน .chorus/skills และฉายให้ทุก agent") {
            if drafts.isEmpty {
                SettingsRow(title: "ยังไม่มีร่าง", detail: "เมื่อทำงานซ้ำรูปแบบเดิมหลายรอบ จะมีร่างมาให้ตรงนี้") { EmptyView() }
            }
            ForEach(Array(drafts.enumerated()), id: \.element.id) { i, d in
                if i > 0 { RowDivider() }
                draftRow(d)
            }
        }
    }

    @ViewBuilder
    private func draftRow(_ d: LearnDraft) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text(d.name).font(.callout.weight(.semibold))
                Text("\(d.sessions.count) session").font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button(openDraft == d.id ? "ซ่อน" : "อ่าน") { openDraft = openDraft == d.id ? nil : d.id }
                    .controlSize(.small)
                Button("ไม่เอา") { model.decide(d, accept: false, workspace: workspace) }
                    .controlSize(.small)
                Button("เก็บเป็น Skill") { model.decide(d, accept: true, workspace: workspace) }
                    .controlSize(.small).buttonStyle(.borderedProminent)
            }
            Text(d.steps.joined(separator: "  →  "))
                .font(.caption.monospaced()).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if openDraft == d.id {
                Text(d.body)
                    .font(.caption.monospaced())
                    .textSelection(.enabled)
                    .padding(8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 6))
            }
        }
        .padding(.horizontal, 12).padding(.vertical, 8)
        .accessibilityElement(children: .contain)
    }

    private var outcomesGroup: some View {
        let o = model.state?.outcomes
        return SettingsGroup(title: "ผลจริงที่ใช้เลือกโมเดล",
                             footer: "นับเฉพาะเทิร์นที่มีหลักฐาน: รันเทสต์แล้วผ่าน/ไม่ผ่าน, ถูกย้อนกลับ, หรือต้องส่งต่อเพราะ context เต็ม/ผิดพลาด · ไม่นับตอนส่งต่อเพราะโควตาหมด · ต้องมีอย่างน้อย \(o?.min_samples ?? 5) เทิร์นต่อโมเดลต่อประเภทงานก่อนจะมีผลต่อการเลือก และเลือกได้เฉพาะภายในระดับราคาเดียวกัน") {
            if let o, o.enabled == false {
                SettingsRow(title: "ปิดอยู่", detail: "ตั้ง CHORUS_ROUTER_OUTCOMES=off ไว้") { EmptyView() }
            } else if let o, !o.cells.isEmpty {
                ForEach(Array(o.cells.enumerated()), id: \.element.id) { i, c in
                    if i > 0 { RowDivider() }
                    SettingsRow(title: "\(c.task_label) · \(c.agent)",
                                detail: c.model) {
                        HStack(spacing: 8) {
                            Text("ผ่าน \(c.passed)/\(c.n)").monospacedDigit()
                            Text(c.steers ? "ใช้ตัดสินใจแล้ว" : "ยังน้อยไป")
                                .font(.caption)
                                .padding(.horizontal, 6).padding(.vertical, 2)
                                .background((c.steers ? Color.green : Color.gray).opacity(0.15), in: Capsule())
                        }
                    }
                }
            } else {
                SettingsRow(title: "ยังไม่มีข้อมูล", detail: "จะเริ่มสะสมเมื่อมีเทิร์นที่รันเทสต์ ถูกย้อนกลับ หรือส่งต่อ") { EmptyView() }
            }
        }
    }

    private var notesGroup: some View {
        SettingsGroup(title: "โน้ตโปรเจกต์ (ทดลอง)",
                      footer: "โมเดลในเครื่อง (Ollama) สรุปคำสั่ง build/test ที่เคยใช้ได้ ไฟล์ที่แก้บ่อย และคำสั่งที่ล้มซ้ำ ๆ จากประวัติ ไม่ใช่จากบทสนทนา · ปิดอยู่เป็นค่าเริ่มต้น เปิดโดยตั้ง CHORUS_SUMMARY_MODEL") {
            if model.state?.notes_enabled != true {
                SettingsRow(title: "ปิดอยู่", detail: "ยังไม่ได้ตั้ง CHORUS_SUMMARY_MODEL") { EmptyView() }
            } else if let notes = model.state?.notes {
                Text(notes)
                    .font(.callout)
                    .textSelection(.enabled)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                SettingsRow(title: "ยังไม่มีโน้ต", detail: "จะเขียนให้เมื่อมีประวัติพอและคุณไม่ได้ใช้งาน") { EmptyView() }
            }
        }
    }
}
