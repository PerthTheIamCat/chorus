import XCTest
@testable import Chorus

final class TurnNoticeTests: XCTestCase {
    func testSuccessShowsTheFirstLineOfTheReply() {
        let n = TurnNotice.content(lastError: nil, lastReply: "\n  Done: added the tests.\nMore detail", agent: "codex")
        XCTAssertEqual(n.title, "Chorus: เทิร์นเสร็จแล้ว · codex")
        XCTAssertEqual(n.body, "Done: added the tests.")
    }

    func testFailureWinsOverTheReply() {
        let n = TurnNotice.content(lastError: "agent turn: rate limit", lastReply: "old reply", agent: nil)
        XCTAssertEqual(n.title, "Chorus: เทิร์นล้มเหลว")
        XCTAssertEqual(n.body, "agent turn: rate limit")
    }

    func testEmptyReplyAndLongLinesAreHandled() {
        XCTAssertEqual(TurnNotice.content(lastError: "", lastReply: nil, agent: nil).body, "agent ตอบเสร็จแล้ว")
        let long = String(repeating: "ก", count: 300)
        XCTAssertEqual(TurnNotice.firstLine(long).count, 140)
        XCTAssertTrue(TurnNotice.firstLine(long).hasSuffix("…"))
    }
}
