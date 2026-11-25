import SwiftUI
import XCTest
@testable import Chorus

@MainActor
final class ShellPolishTests: XCTestCase {
    /// Every menu shortcut is bound to exactly one command (⇧⌘E, Esc and ⌘↩
    /// each used to mean two things).
    func testEveryShortcutHasOneMeaning() {
        var seen: [String: AppCommand] = [:]
        for command in AppCommand.all {
            let s = command.shortcut
            let key = "\(s.key.character.unicodeScalars.map(\.value))-\(s.modifiers.rawValue)"
            if let other = seen[key] {
                XCTFail("\(command) and \(other) share one shortcut")
            }
            seen[key] = command
        }
        XCTAssertEqual(seen.count, AppCommand.all.count)
    }

    func testEscapeIsNotAMenuShortcut() {
        // A menu Esc would swallow it from sheets and the completion popup.
        XCTAssertFalse(AppCommand.all.contains { $0.shortcut.key == .escape })
    }

    func testOversizedAttachmentIsRejectedBeforeReading() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }

        let big = dir.appendingPathComponent("big.bin")
        FileManager.default.createFile(atPath: big.path, contents: nil)
        let fh = try FileHandle(forWritingTo: big)
        try fh.truncate(atOffset: UInt64(Engine.maxAttachmentBytes + 1)) // sparse: no bytes written
        try fh.close()
        XCTAssertNotNil(Engine.attachmentRejection(big))

        let small = dir.appendingPathComponent("small.txt")
        try Data("hi".utf8).write(to: small)
        XCTAssertNil(Engine.attachmentRejection(small))

        XCTAssertNotNil(Engine.attachmentRejection(dir)) // folders are not attachments
    }

    func testMarkdownSplitsFences() {
        let text = "intro **bold**\n```swift\nlet x = 1\n```\noutro"
        let parts = MarkdownBody.blocks(text)
        XCTAssertEqual(parts.count, 3)
        if case let .code(language, body) = parts[1] {
            XCTAssertEqual(language, "swift")
            XCTAssertEqual(body, "let x = 1")
        } else { XCTFail("middle part should be code") }
        if case let .prose(a) = parts[0] {
            XCTAssertTrue(a.contains("intro"))
        } else { XCTFail("first part should be prose") }
    }

    /// A streaming reply re-parses only its last paragraph, so each paragraph
    /// must be its own block (and a fence inside stays one block).
    func testMarkdownSplitsParagraphs() {
        let parts = MarkdownBody.blocks("one\n\ntwo\nstill two\n\n\n```\na\n\nb\n```\nthree")
        XCTAssertEqual(parts.count, 4)
        if case let .prose(a) = parts[1] { XCTAssertEqual(a, "two\nstill two") } else { XCTFail("second part should be prose") }
        if case let .code(_, body) = parts[2] { XCTAssertEqual(body, "a\n\nb") } else { XCTFail("third part should be code") }
    }
}
