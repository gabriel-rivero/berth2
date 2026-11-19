// Config round-trip tests: the portable window reads the same config.json, so a
// save from this shell must not delete keys it does not own (fontFamily landed
// that way and a save here silently reverted the portable window's face).

import XCTest
@testable import Berth

@MainActor
final class AppConfigTests: XCTestCase {
    private func tempURL() -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("berth-config-\(UUID().uuidString).json")
    }

    private func read(_ url: URL) throws -> [String: Any] {
        let data = try Data(contentsOf: url)
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    /// The tests write through the shared singleton, so the font size it
    /// started with is restored and the suite stays order-independent.
    private func withRestoredFontSize(_ body: (AppConfig) throws -> Void) rethrows {
        let original = AppConfig.shared.fontSize
        defer { AppConfig.shared.setFontSize(original) }
        try body(AppConfig.shared)
    }

    func testSaveKeepsKeysTheShellDoesNotOwn() throws {
        let url = tempURL()
        defer { try? FileManager.default.removeItem(at: url) }
        let existing = """
        {"fontFamily": ["VictorMono NFM"], "futureKey": {"a": 1}, "fontSize": 10}
        """
        try existing.write(to: url, atomically: true, encoding: .utf8)

        try withRestoredFontSize { config in
            config.setFontSize(18)
            try config.save(to: url)
        }

        let object = try read(url)
        XCTAssertEqual(object["fontFamily"] as? [String], ["VictorMono NFM"])
        XCTAssertEqual((object["futureKey"] as? [String: Any])?["a"] as? Int, 1)
        XCTAssertEqual(object["fontSize"] as? Double, 18)
    }

    func testSaveWritesItsOwnKeysWhenTheFileIsMissing() throws {
        let url = tempURL()
        defer { try? FileManager.default.removeItem(at: url) }

        try withRestoredFontSize { config in
            config.setFontSize(20)
            try config.save(to: url)
        }

        let object = try read(url)
        XCTAssertEqual(object["fontSize"] as? Double, 20)
        XCTAssertNotNil(object["shortcuts"] as? [String: String])
        XCTAssertNotNil(object["socket"] as? String)
    }

    func testSaveRefusesToReplaceADocumentItCannotMerge() throws {
        let url = tempURL()
        defer { try? FileManager.default.removeItem(at: url) }
        let original = "[1, 2, 3]"
        try original.write(to: url, atomically: true, encoding: .utf8)

        XCTAssertThrowsError(try AppConfig.shared.save(to: url)) { error in
            XCTAssertTrue(
                error.localizedDescription.contains("not a JSON object"),
                error.localizedDescription)
        }
        let left = try String(contentsOf: url, encoding: .utf8)
        XCTAssertEqual(left, original)
    }
}
