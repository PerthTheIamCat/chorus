import XCTest
@testable import Chorus

final class MetricsStripTests: XCTestCase {
    func testDecodesTheFFIShapeWithNulls() throws {
        let json = #"{"tok_per_s":42.4,"tok_per_s_estimated":true,"cache_hit_pct":null,"cost_usd":null,"context_pct":25.0,"limit_pct":46.0,"limit_window":"5h"}"#
        let m = try JSONDecoder().decode(SessionMetrics.self, from: Data(json.utf8))
        XCTAssertEqual(m.tok_per_s, 42.4)
        XCTAssertNil(m.cache_hit_pct)
        XCTAssertNil(m.cost_usd)
    }

    /// No number, no chip: nothing the provider did not report is drawn.
    func testMissingNumbersDrawNothing() {
        XCTAssertTrue(MetricsChips.make(nil, .init()).isEmpty)
        XCTAssertTrue(MetricsChips.make(SessionMetrics(), .init()).isEmpty)
    }

    func testEstimatesAreLabelledAndTogglesHideChips() {
        let m = SessionMetrics(tok_per_s: 42.4, tok_per_s_estimated: true, cache_hit_pct: 87.0,
                               cost_usd: 0.0125, context_pct: 80.0, limit_pct: 18.0, limit_window: "5h")
        let all = MetricsChips.make(m, .init())
        XCTAssertEqual(all.map(\.label), ["≈42 tok/s", "cache 87%", "$0.0125", "บริบท 80%", "5h 18%"])
        XCTAssertEqual(all.map(\.warn), [false, false, false, true, false])
        var t = MetricsToggles()
        t.tokPerS = false
        t.cost = false
        XCTAssertEqual(MetricsChips.make(m, t).map(\.label), ["cache 87%", "บริบท 80%", "5h 18%"])
    }

    func testCostRoundsByMagnitude() {
        let big = SessionMetrics(cost_usd: 3.456)
        XCTAssertEqual(MetricsChips.make(big, .init()).first?.label, "$3.46")
    }
}
