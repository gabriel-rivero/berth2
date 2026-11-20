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

    // MARK: - Appearance

    /// A fresh instance reads only the file it is given, so these tests never
    /// depend on the user's real ~/.config/berth/config.json.
    func testConfigWithoutAFileUsesTheAppearanceDefaults() {
        let config = AppConfig(url: tempURL())
        XCTAssertEqual(config.theme, "")
        XCTAssertEqual(config.opacity, 1.0)
        XCTAssertEqual(config.backgroundBlur, 0)
        XCTAssertEqual(config.fontSize, AppConfig.defaultFontSize)
    }

    func testOpacityBelowRangeClampsToTheMinimum() {
        let config = AppConfig(url: tempURL())
        config.setOpacity(0.1)
        XCTAssertEqual(config.opacity, 0.3, accuracy: 0.0001)
    }

    func testOpacityAboveRangeClampsToTheMaximum() {
        let config = AppConfig(url: tempURL())
        config.setOpacity(1.5)
        XCTAssertEqual(config.opacity, 1.0, accuracy: 0.0001)
    }

    func testBackgroundBlurOutsideRangeClampsToItsBounds() {
        let config = AppConfig(url: tempURL())
        config.setBackgroundBlur(-4)
        XCTAssertEqual(config.backgroundBlur, 0)
        config.setBackgroundBlur(200)
        XCTAssertEqual(config.backgroundBlur, 64)
    }

    func testThemeIsStoredTrimmed() {
        let config = AppConfig(url: tempURL())
        config.setTheme("  Dracula\n")
        XCTAssertEqual(config.theme, "Dracula")
    }

    func testSaveAndReloadRoundTripsAppearanceAndKeepsUnrelatedKey() throws {
        let url = tempURL()
        defer { try? FileManager.default.removeItem(at: url) }
        let existing = """
        {"theme": "old", "futureKey": {"a": 1}}
        """
        try existing.write(to: url, atomically: true, encoding: .utf8)

        let config = AppConfig(url: url)
        config.setTheme("Dracula")
        config.setOpacity(0.85)
        config.setBackgroundBlur(20)
        try config.save(to: url)

        let object = try read(url)
        XCTAssertEqual(object["theme"] as? String, "Dracula")
        XCTAssertEqual(object["opacity"] as? Double, 0.85)
        XCTAssertEqual(object["backgroundBlur"] as? Int, 20)
        XCTAssertEqual((object["futureKey"] as? [String: Any])?["a"] as? Int, 1)

        let reloaded = AppConfig(url: url)
        XCTAssertEqual(reloaded.theme, "Dracula")
        XCTAssertEqual(reloaded.opacity, 0.85, accuracy: 0.0001)
        XCTAssertEqual(reloaded.backgroundBlur, 20)
    }
}
