import XCTest
@testable import Chorus

final class ChangelogTests: XCTestCase {
    /// The repo's real CHANGELOG.md parses into a version with sections and
    /// bullets, and its heading carries the bare version Settings matches on.
    func testRepoChangelogParses() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().appendingPathComponent("../../../../CHANGELOG.md").standardized
        let lines = ChangelogSettings.parse(try String(contentsOf: url, encoding: .utf8))
        let versions = lines.compactMap { if case .version(let v) = $0 { v } else { nil } }
        XCTAssertTrue(versions.contains { $0.hasPrefix("0.1.3 ") }, "\(versions)")
        XCTAssertTrue(lines.contains { if case .bullet = $0 { true } else { false } })
    }

    /// An empty Unreleased section (right after cutting a release) is dropped,
    /// as is the file's title and intro.
    func testEmptyUnreleasedIsHidden() {
        let lines = ChangelogSettings.parse("""
            # Changelog
            intro

            ## [Unreleased]

            ## [1.0.0] - 2026-01-01

            ### Fixed

            - **thing:** fixed
              wrapped
            """)
        guard lines.count == 3, case .version("1.0.0 - 2026-01-01") = lines[0],
              case .section("Fixed") = lines[1], case .bullet("**thing:** fixed wrapped") = lines[2]
        else { return XCTFail("\(lines)") }
    }
}
