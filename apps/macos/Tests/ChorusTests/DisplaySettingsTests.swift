import XCTest
@testable import Chorus

final class DisplaySettingsTests: XCTestCase {
    func testDetailLevelsDecideWhatIsDrawn() {
        XCTAssertEqual(DetailLevel.from("nonsense"), .normal)
        XCTAssertFalse(DetailLevel.compact.showsMetrics)
        XCTAssertFalse(DetailLevel.compact.showsPreviews)
        XCTAssertTrue(DetailLevel.normal.showsMetrics && DetailLevel.detailed.showsMetrics)
        // The old thinking switch still rules at the normal level only.
        XCTAssertTrue(DetailLevel.normal.thinkingOpenByDefault(userPref: true))
        XCTAssertFalse(DetailLevel.normal.thinkingOpenByDefault(userPref: false))
        XCTAssertFalse(DetailLevel.compact.thinkingOpenByDefault(userPref: true))
        XCTAssertTrue(DetailLevel.detailed.thinkingOpenByDefault(userPref: false))
        XCTAssertTrue(DetailLevel.detailed.toolsOpenByDefault)
        XCTAssertFalse(DetailLevel.normal.toolsOpenByDefault)
    }

    func testMoneyFollowsTheChosenCurrency() {
        XCTAssertNil(Money.rate(""))
        XCTAssertNil(Money.rate("0"))
        XCTAssertNil(Money.rate("abc"))
        XCTAssertEqual(Money.rate(" 36.5 "), 36.5)
        XCTAssertEqual(Money.format(usd: 2, currency: "usd", thbRate: 36), "$2.000")
        XCTAssertEqual(Money.format(usd: 2, currency: "thb", thbRate: 36), "฿72.0")
        // Baht without a rate cannot be shown: USD, never a made-up rate.
        XCTAssertEqual(Money.format(usd: 2, currency: "thb", thbRate: nil), "$2.000")
        // "auto" is the older behaviour: baht once a rate exists.
        XCTAssertEqual(Money.resolve("auto", thbRate: 36), "thb")
        XCTAssertEqual(Money.resolve("auto", thbRate: nil), "usd")
        XCTAssertEqual(Money.format(usd: 0.0125, currency: "usd", thbRate: nil, usdDecimals: 4), "$0.0125")
    }

    func testAccentHexRoundTrips() {
        XCTAssertNil(AccentHex.color(from: ""))
        XCTAssertNil(AccentHex.color(from: "#12345"))
        XCTAssertNil(AccentHex.color(from: "zzzzzz"))
        XCTAssertNotNil(AccentHex.color(from: "#0A84FF"))
        XCTAssertNotNil(AccentHex.color(from: "0A84FF"))
        let c = AccentHex.color(from: "#0A84FF")!
        XCTAssertEqual(AccentHex.hex(from: c), "#0A84FF")
        XCTAssertEqual(Set(AccentHex.presets.map(\.hex)).count, AccentHex.presets.count)
    }

    func testThemeMapsToSchemes() {
        XCTAssertNil(AppTheme.system.scheme)
        XCTAssertEqual(AppTheme.light.scheme, .light)
        XCTAssertEqual(AppTheme.dark.scheme, .dark)
    }
}
