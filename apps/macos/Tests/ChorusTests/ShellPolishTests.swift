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

    func testFolderExpansionIsSeededOncePerRoot() {
        let nodes = [
            Engine.FileNode(name: "src", rel_path: "src", is_dir: true, icon: nil, size: 0,
                            git_status: nil, is_focus: false, children: []),
            Engine.FileNode(name: "README.md", rel_path: "README.md", is_dir: false, icon: nil, size: 1,
                            git_status: nil, is_focus: false, children: []),
        ]
        XCTAssertEqual(WorkspaceExplorerView.seedExpansion(nodes, root: "/a", seededRoot: nil), ["src"])
        // A refresh of the same folder must leave the user's collapse alone.
        XCTAssertNil(WorkspaceExplorerView.seedExpansion(nodes, root: "/a", seededRoot: "/a"))
        XCTAssertEqual(WorkspaceExplorerView.seedExpansion(nodes, root: "/b", seededRoot: "/a"), ["src"])
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

    func testMarkdownSplitsFencesAndCaches() {
        let text = "intro **bold**\n```swift\nlet x = 1\n```\noutro"
        let parts = MarkdownText.parse(text)
        XCTAssertEqual(parts.count, 3)
        if case let .code(language, body) = parts[1] {
            XCTAssertEqual(language, "swift")
            XCTAssertEqual(body, "let x = 1")
        } else { XCTFail("middle part should be code") }
        if case let .prose(a) = parts[0] {
            XCTAssertEqual(String(a.characters), "intro bold")
        } else { XCTFail("first part should be prose") }
        XCTAssertEqual(MarkdownText.parts(for: text).count, 3)
    }
}
