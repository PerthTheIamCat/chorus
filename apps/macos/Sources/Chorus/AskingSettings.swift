import SwiftUI

// Settings › การถามและอัตโนมัติ — how often the app may interrupt with a
// question. Swift draws the screen; every rule lives in Rust
// (`chorus_core::ask_policy`, `learn::Autonomy`, CLAUDE.md 2.3).

private struct AskPolicyState: Codable {
    var ok: Bool?
    var error: String?
    var children: String?
    var lead: String?
}

@MainActor
final class AskPolicyModel: ObservableObject {
    @Published private(set) var children = "lead_first"
    @Published private(set) var lead = "when_stuck"
    @Published private(set) var message: String?

    private let queue = DispatchQueue(label: "chorus.askpolicy", qos: .utility)

    nonisolated private static func string(from p: UnsafeMutablePointer<CChar>?) -> String {
        guard let p else { return "" }
        defer { chorus_string_free(p) }
        return String(cString: p)
    }

    nonisolated private static func decode(_ json: String) -> AskPolicyState? {
        try? JSONDecoder().decode(AskPolicyState.self, from: Data(json.utf8))
    }

    func load() {
        queue.async { [weak self] in
            let s = Self.decode(Self.string(from: chorus_ask_policy_json()))
            DispatchQueue.main.async {
                guard let self, let s, s.ok == true else { return }
                self.children = s.children ?? "lead_first"
                self.lead = s.lead ?? "when_stuck"
            }
        }
    }

    func save(children: String? = nil, lead: String? = nil) {
        let c = children ?? self.children
        let l = lead ?? self.lead
        queue.async { [weak self] in
            let body = "{\"children\":\"\(c)\",\"lead\":\"\(l)\"}"
            let s = Self.decode(body.withCString { Self.string(from: chorus_set_ask_policy($0)) })
            DispatchQueue.main.async {
                guard let self else { return }
                if s?.ok == true {
                    self.children = s?.children ?? c
                    self.lead = s?.lead ?? l
                    self.message = nil
                } else {
                    self.message = s?.error ?? "บันทึกไม่สำเร็จ"
                }
            }
        }
    }
}

struct AskingSettings: View {
    let workspace: String
    @StateObject private var policy = AskPolicyModel()
    @StateObject private var learn = LearnModel()

    private var autonomy: Binding<String> {
        Binding(
            get: { learn.state?.autonomy ?? "auto" },
            set: { learn.setAutonomy($0, workspace: workspace) })
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("ค่าเริ่มต้นคือถามน้อยที่สุด — agent ตัดสินใจเรื่องทั่วไปเอง บอกสิ่งที่สมมติไว้ และมาถามคุณเฉพาะเรื่องที่เดาผิดแล้วเสียหนักหรือย้อนยาก")
                .font(.callout)
                .foregroundStyle(.secondary)

            SettingsGroup(title: "ความจำและบทเรียน",
                          footer: "ข้อที่มาจากเว็บหรือ session ที่นำเข้ายังรอให้คุณดูเสมอ เพราะอาจมีคำสั่งแอบแฝง") {
                SettingsRow(title: "ความจำที่ agent เสนอ และบทเรียนจากงานที่ผ่านมา",
                            info: "อัตโนมัติ = จำเลยโดยไม่ต้องกดรับ (มีปุ่มเลิกจำภายหลัง) · ถามก่อน = ทุกอย่างรอในคิว · รับเองเมื่อเทสต์ผ่าน · ปิด = ไม่ร่างอะไรเลย") {
                    Picker("ความจำ", selection: autonomy) {
                        Text("อัตโนมัติ").tag("auto")
                        Text("ถามก่อน").tag("ask")
                        Text("เมื่อเทสต์ผ่าน").tag("auto_with_tests")
                        Text("ปิด").tag("off")
                    }
                    .pickerStyle(.segmented).labelsHidden().frame(width: 360)
                }
            }

            SettingsGroup(title: "เมื่อมีหลาย agent (Orchestra)",
                          footer: "ลูกที่ทำงานให้ lead จะถาม lead ก่อนเสมอ และ lead ตอบเอง · ถ้า lead ตัดสินใจไม่ได้จริงๆ ถึงจะมาถามคุณ") {
                SettingsRow(title: "ลูกที่ทำงานให้ lead ถามใคร",
                            info: "ถาม lead ก่อน = ลูกไม่ได้รับเครื่องมือถามผู้ใช้ มีแค่ถาม lead · ถามฉันได้เลย = แบบเดิม") {
                    Picker("ลูกถามใคร", selection: Binding(
                        get: { policy.children },
                        set: { policy.save(children: $0) })) {
                        Text("ถาม lead ก่อน").tag("lead_first")
                        Text("ถามฉันได้เลย").tag("user")
                    }
                    .pickerStyle(.segmented).labelsHidden().frame(width: 270)
                }
                RowDivider()
                SettingsRow(title: "lead และ agent เดี่ยวถามฉันเมื่อไหร่",
                            info: "เมื่อติดจริงๆ = ตัดสินใจเอง บอกสมมติฐาน ถามเฉพาะเมื่อเดาผิดแล้วเสียหนัก/ย้อนยาก · เมื่อไม่แน่ใจ = ถามทุกครั้งที่ข้อมูลไม่พอ (แบบเดิม)") {
                    Picker("ถามเมื่อไหร่", selection: Binding(
                        get: { policy.lead },
                        set: { policy.save(lead: $0) })) {
                        Text("เมื่อติดจริงๆ").tag("when_stuck")
                        Text("เมื่อไม่แน่ใจ").tag("often")
                    }
                    .pickerStyle(.segmented).labelsHidden().frame(width: 270)
                }
            }

            SettingsGroup(title: "สิ่งที่ถามเสมอ (ปิดไม่ได้)",
                          footer: "เรื่องความปลอดภัยไม่ขึ้นกับการตั้งค่านี้") {
                SettingsRow(title: "คำสั่งที่ออกนอกเครื่องหลังอ่านของจากภายนอก · ขอ secret · ความจำที่น่าสงสัย") { EmptyView() }
            }

            if let m = policy.message ?? learn.message {
                Text(m).font(.callout).foregroundStyle(.secondary)
            }
        }
        .onAppear {
            policy.load()
            if !workspace.isEmpty { learn.load(workspace: workspace) }
        }
    }
}
