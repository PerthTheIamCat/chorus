import XCTest
@testable import Chorus

final class ChatComponentsTests: XCTestCase {
    private func ev(_ seq: Int64, _ kind: String, actor: String = "codex") -> LedgerEvent {
        LedgerEvent(seq: seq, kind: kind, actor: actor, body: "b\(seq)", model_id: nil)
    }

    /// A run of tool events folds into one row; a lone tool call stays a plain row.
    func testConsecutiveToolEventsFoldIntoOneGroup() {
        let events = [
            ev(1, "msg", actor: "user"),
            ev(2, "tool_call"), ev(3, "tool_result"), ev(4, "tool_call"),
            ev(5, "msg"),
            ev(6, "tool_call"),
        ]
        let snap = ChatSnapshotCache().snapshot(events: events, attachments: [])
        XCTAssertEqual(snap.items.count, 4)
        guard case .tools(let group) = snap.items[1] else { return XCTFail("expected a tool group") }
        XCTAssertEqual(group.map(\.seq), [2, 3, 4])
        XCTAssertEqual(snap.items[1].id, 2)
        guard case .event(let lone) = snap.items[3] else { return XCTFail("expected a plain row") }
        XCTAssertEqual(lone.seq, 6)
        XCTAssertEqual(snap.lastAgentMsg, 5)
    }

    /// The same ledger gives the same snapshot without regrouping.
    func testSnapshotIsReusedUntilTheLedgerChanges() {
        let cache = ChatSnapshotCache()
        var events = [ev(1, "msg", actor: "user"), ev(2, "msg")]
        let a = cache.snapshot(events: events, attachments: [])
        events.append(ev(3, "msg", actor: "user"))
        let b = cache.snapshot(events: events, attachments: [])
        XCTAssertEqual(a.items.count, 2)
        XCTAssertEqual(b.items.count, 3)
        XCTAssertTrue(b.animate)
    }

    /// The finished paragraphs split off; an open code fence is never cut in two.
    func testStreamingSplitKeepsOpenFencesWhole() {
        let plain = StreamingMarkdown.split("one\n\ntwo\n\nthr")
        XCTAssertEqual(plain.stable, "one\n\ntwo")
        XCTAssertEqual(plain.tail, "thr")

        let open = "intro\n\n```swift\nlet a = 1\n\nlet b = 2"
        let split = StreamingMarkdown.split(open)
        XCTAssertEqual(split.stable, "")
        XCTAssertEqual(split.tail, open)
    }
}
