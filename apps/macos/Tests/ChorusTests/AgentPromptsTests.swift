import XCTest
@testable import Chorus

final class AgentPromptsTests: XCTestCase {
    func testBadgeAppearsOnlyWhileSomethingWaits() {
        XCTAssertNil(AgentPromptNotice.badge(0))
        XCTAssertEqual(AgentPromptNotice.badge(1), "รอคุณ 1 คำถาม")
    }

    func testNoticesNameTheAgentWithoutItsAccountSuffix() {
        let q = AgentPromptNotice.question(agent: "claude@work", question: "\nใช้ schema ไหน?\nรายละเอียด")
        XCTAssertEqual(q.title, "Chorus: claude ถามคุณ")
        XCTAssertEqual(q.body, "ใช้ schema ไหน?")
        let s = AgentPromptNotice.secret(agent: "codex", name: "STRIPE_API_KEY")
        XCTAssertEqual(s.title, "Chorus: codex ขอ STRIPE_API_KEY")
        XCTAssertFalse(s.body.isEmpty)
    }

    func testUtf8PrefixNeverSplitsACharacter() {
        // Each Thai letter is 3 bytes; a cap of 8 leaves room for 7 bytes + NUL.
        let bytes = AgentPromptCallbacks.utf8Prefix("กขค", cap: 8)
        XCTAssertEqual(bytes.count, 6)
        XCTAssertEqual(String(decoding: bytes, as: UTF8.self), "กข")
        XCTAssertEqual(AgentPromptCallbacks.utf8Prefix("abc", cap: 100).count, 3)
    }

    func testGateHandsTheAnswerBackAndLetsADismissalThrough() {
        let gate = PromptGate<String>()
        let answered = gate.ask {
            DispatchQueue.global().asyncAfter(deadline: .now() + 0.05) { gate.resolve("สอง") }
        }
        XCTAssertEqual(answered, "สอง")
        let dismissed = gate.ask {
            DispatchQueue.global().asyncAfter(deadline: .now() + 0.05) { gate.resolve(nil) }
        }
        XCTAssertNil(dismissed, "a dismissal must not return the previous answer")
    }

    func testOfferDecodesWhatRustSends() throws {
        let json = """
        {"agent":"claude","title":"Pick a login","recommended":1,
         "variants":[{"label":"Dense","kind":"image","target":"/p/a.png","note":""},
                     {"label":"Airy","kind":"url","target":"http://localhost:5173","note":"wide"}]}
        """
        let offer = try JSONDecoder().decode(VariantOffer.self, from: Data(json.utf8))
        XCTAssertEqual(offer.variants.map(\.kind), ["image", "url"])
        XCTAssertEqual(offer.recommended, 1)
    }

    func testChoiceEncodesTheShapeRustReads() throws {
        let choice = VariantChoice(
            chosen: 1,
            comments: [VariantPin(variant: 0, x: 0.4, y: 0.25, text: "ปุ่มเล็ก")],
            note: "warmer")
        let data = try JSONEncoder().encode(choice)
        let obj = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(obj["chosen"] as? Int, 1)
        XCTAssertEqual(obj["note"] as? String, "warmer")
        let first = try XCTUnwrap((obj["comments"] as? [[String: Any]])?.first)
        XCTAssertEqual(Set(first.keys), ["variant", "x", "y", "text"], "no local id leaks to Rust")

        let none = try JSONEncoder().encode(VariantChoice(chosen: nil, comments: [], note: ""))
        let noneObj = try XCTUnwrap(JSONSerialization.jsonObject(with: none) as? [String: Any])
        XCTAssertNil(noneObj["chosen"], "no pick = key absent, which Rust reads as None")
    }

    func testRememberCodesMatchTheCHeader() {
        XCTAssertEqual(SecretRemember.once.rawValue, 1)
        XCTAssertEqual(SecretRemember.project.rawValue, 2)
        XCTAssertEqual(SecretRemember.all.rawValue, 3)
    }
}
