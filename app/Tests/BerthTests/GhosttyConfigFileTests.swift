// The generated ghostty.conf carries Berth's font and background overrides, so
// these pin what Ghostty reads without starting the terminal engine.

import XCTest
@testable import Berth

final class GhosttyConfigFileTests: XCTestCase {
    func testTextAlwaysCarriesFontSize() {
        let text = GhosttyConfigFile.text(fontSize: 16, opacity: nil, backgroundBlur: nil)
        XCTAssertEqual(text, "font-size = 16.0\n")
    }

    func testTextWithOpacityCarriesIt() {
        let text = GhosttyConfigFile.text(fontSize: 16, opacity: 0.9, backgroundBlur: nil)
        XCTAssertTrue(text.contains("background-opacity = 0.9\n"), text)
    }

    /// An untouched config must not override the user's own Ghostty settings.
    func testTextWithoutOpacityLeftTheKeyOut() {
        let text = GhosttyConfigFile.text(fontSize: 14, opacity: nil, backgroundBlur: nil)
        XCTAssertFalse(text.contains("background-opacity"), text)
    }

    func testTextWithoutBlurOmitsTheBlurKey() {
        let text = GhosttyConfigFile.text(fontSize: 14, opacity: nil, backgroundBlur: 0)
        XCTAssertFalse(text.contains("background-blur"), text)
    }

    func testTextWithBlurCarriesIt() {
        let text = GhosttyConfigFile.text(fontSize: 14, opacity: nil, backgroundBlur: 20)
        XCTAssertTrue(text.contains("background-blur = 20\n"), text)
    }

    func testTextEndsWithOneNewline() {
        let text = GhosttyConfigFile.text(fontSize: 14, opacity: 0.8, backgroundBlur: 8)
        XCTAssertTrue(text.hasSuffix("background-blur = 8\n"), text)
        XCTAssertFalse(text.hasSuffix("\n\n"), text)
    }
}
