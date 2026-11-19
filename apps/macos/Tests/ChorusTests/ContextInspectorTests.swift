import XCTest
@testable import Chorus

final class ContextInspectorTests: XCTestCase {
    /// Verbatim `chorus_context_inspect` output (text/brief shortened), so a
    /// rename on the Rust side fails here rather than as an empty card.
    private let sample = #"""
    {"budget":120000,"text":"T","tokens":136,"report":{"budget":120000,"total_tokens":124,"tiers":[{"tier":"System","tokens":30,"cap":120000,"fill_pct":0.025,"note":""},{"tier":"TaskState","tokens":7,"cap":6000,"fill_pct":0.11666666666666667,"note":"ไม่เคยถูกตัด — เข้าไปครบเสมอ"},{"tier":"Memory","tokens":6,"cap":10800,"fill_pct":0.05555555555555555,"note":""},{"tier":"Conversation","tokens":61,"cap":45600,"fill_pct":0.1337719298245614,"note":""},{"tier":"Code","tokens":20,"cap":31200,"fill_pct":0.0641025641025641,"note":""},{"tier":"Reserve","tokens":0,"cap":19200,"fill_pct":0.0,"note":"กันไว้ให้ agent ตอบ ไม่มีอะไรถูกใส่"}],"conversation":{"turns_total":4,"at_l0":4,"at_l1":0,"at_l2":0,"demoted_by_budget":0,"dropped":0,"notes":[],"notes_omitted":0},"memory":[{"text":"ใช้ pnpm","score":0.0,"pinned":false,"included":true,"tokens":6,"referenced":null}],"code":[{"label":"diff","tokens":20,"status":"kept"}]},"target":{"agent":"codex","budget":120000,"budget_known":false,"context_tokens":136,"demoted":0,"dropped":0,"brief_markdown":"B","brief_tokens":110,"instruction_file":"AGENTS.md"}}
    """#

    func testDecodesTheFFIShape() throws {
        let i = try JSONDecoder().decode(ContextInspection.self, from: Data(sample.utf8))
        XCTAssertEqual(i.report.tiers.map(\.tier), ["System", "TaskState", "Memory", "Conversation", "Code", "Reserve"])
        XCTAssertEqual(i.report.conversation.turns_total, 4)
        XCTAssertNil(i.report.memory[0].referenced, "counters do not exist yet: nothing is claimed")
        XCTAssertEqual(i.target?.agent, "codex")
        XCTAssertEqual(i.target?.budget_known, false)
    }

    func testTurnNotesDecodeWhenSomethingWasShortened() throws {
        let json = #"""
        {"turns_total":50,"at_l0":10,"at_l1":20,"at_l2":15,"demoted_by_budget":8,"dropped":5,"notes":[{"age":12,"from_rung":1,"to_rung":2,"dropped":false,"tokens_before":300,"tokens_after":90,"preview":"แก้ login…"},{"age":49,"from_rung":2,"to_rung":2,"dropped":true,"tokens_before":80,"tokens_after":0,"preview":"เริ่มงาน"}],"notes_omitted":3}
        """#
        let c = try JSONDecoder().decode(ContextConversation.self, from: Data(json.utf8))
        XCTAssertEqual(c.notes.map(\.age), [12, 49])
        XCTAssertTrue(c.notes[1].dropped)
        XCTAssertEqual(c.notes_omitted, 3)
    }

    func testTierNamesAreThai() {
        XCTAssertEqual(ContextTierStyle.title("Conversation"), "บทสนทนา")
        XCTAssertEqual(ContextTierStyle.title("Unknown"), "Unknown")
        XCTAssertEqual(ContextTierStyle.rungLabel(1), "สรุปสั้น (L1)")
        XCTAssertEqual(ContextTierStyle.rungLabel(9), "สรุปตอน (L2)")
    }

    func testTokenTextGroupsThousands() {
        XCTAssertEqual(tokenText(12_345), "12,345")
    }
}
