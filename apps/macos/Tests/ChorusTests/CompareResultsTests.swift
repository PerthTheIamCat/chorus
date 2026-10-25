import XCTest
@testable import Chorus

final class CompareResultsTests: XCTestCase {
    private func branch(_ extra: String = "") throws -> CompareResultsSheet.Branch {
        let json = """
        {"agent":"codex","session_id":7,"message":"m","stop_reason":"end_turn","error":null\(extra)}
        """
        return try JSONDecoder().decode(CompareResultsSheet.Branch.self, from: Data(json.utf8))
    }

    func testStatsLineShowsTimeTokensAndCost() throws {
        let b = try branch(#","elapsed_ms":65000,"context_used":12400,"cost_usd":0.0421"#)
        XCTAssertEqual(CompareResultsSheet.stats(b), "1m 05s · 12.4k tokens · $0.0421")
        XCTAssertEqual(CompareResultsSheet.duration(12_300), "12.3s")
    }

    func testPayloadFromAnOlderEngineStillDecodes() throws {
        let b = try branch()
        XCTAssertNil(b.diff)
        XCTAssertEqual(CompareResultsSheet.stats(b), "")
    }

    func testMarkdownLinksBecomeLinks() {
        let text = CompareResultsSheet.markdown("see [hello.txt](file:///tmp/qa/hello.txt) now")
        XCTAssertEqual(String(text.characters), "see hello.txt now")
        XCTAssertEqual(text.runs.compactMap(\.link).map(\.absoluteString), ["file:///tmp/qa/hello.txt"])
    }
}
