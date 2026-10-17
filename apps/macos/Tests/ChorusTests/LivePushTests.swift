import XCTest
@testable import Chorus

final class LivePushTests: XCTestCase {
    private func push(_ sid: Int64, _ seq: Int64, _ kind: String, _ text: String = "") -> LivePush {
        LivePush(session_id: sid, seq: seq, kind: kind, text: text)
    }

    func testDecodesTheFFIShape() throws {
        let json = #"{"session_id":7,"seq":42,"kind":"usage","text":"12000\t200000"}"#
        let p = try JSONDecoder().decode(LivePush.self, from: Data(json.utf8))
        XCTAssertEqual(p, push(7, 42, "usage", "12000\t200000"))
    }

    /// Two sessions streaming at once: only the open one reaches the thread,
    /// both reach the sidebar status.
    func testRoutesBySessionID() {
        let batch = [
            push(1, 3, "status", "running"), push(2, 9, "status", "running"),
            push(1, 3, "message", "hello "), push(2, 9, "message", "WRONG"),
            push(1, 3, "message", "there"), push(2, 9, "usage", "999\t1000"),
            push(1, 3, "usage", "12000\t200000"), push(2, 10, "events"),
            push(1, 3, "tool", "Read a"), push(1, 3, "tool_done", "completed\tRead a"),
        ]
        let f = LiveFold.apply(batch, session: 1, lastSeq: 3, turn: .init(), contextUsed: nil, contextWindow: nil)
        XCTAssertEqual(f.turn.message, "hello there")
        XCTAssertEqual(f.contextUsed, 12000)
        XCTAssertEqual(f.contextWindow, 200000)
        XCTAssertEqual(f.turn.tools.map(\.state), ["completed"])
        XCTAssertTrue(f.diffChanged)
        XCTAssertFalse(f.pull, "session 2's new rows must not trigger a pull of session 1")
        XCTAssertEqual(f.statuses, [1: "running", 2: "running"])
    }

    /// A push for rows already on screen asks for nothing; a newer one asks
    /// for a pull even if the pushes before it were lost.
    func testPullsOnlyWhenTheLedgerIsAhead() {
        let seen = LiveFold.apply([push(1, 5, "events")], session: 1, lastSeq: 5,
                                  turn: .init(), contextUsed: nil, contextWindow: nil)
        XCTAssertFalse(seen.pull)
        let ahead = LiveFold.apply([push(1, 8, "events")], session: 1, lastSeq: 5,
                                   turn: .init(), contextUsed: nil, contextWindow: nil)
        XCTAssertTrue(ahead.pull)
    }

    @MainActor
    func testCoalescesABurstIntoOneDelivery() {
        let done = expectation(description: "delivered")
        var deliveries: [[LivePush]] = []
        let c = PushCoalescer(frame: 0.05) { batch in
            deliveries.append(batch)
            done.fulfill()
        }
        DispatchQueue.global().sync {
            for i in 0..<200 { c.enqueue(push(1, 1, "message", "\(i)")) }
        }
        wait(for: [done], timeout: 2)
        XCTAssertEqual(deliveries.count, 1)
        XCTAssertEqual(deliveries.first?.count, 200)
    }
}
