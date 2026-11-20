// The generated ghostty.conf carries Berth's font and background overrides, so
// these pin what Ghostty reads without starting the terminal engine.

import XCTest
@testable import Berth

final class GhosttyConfigFileTests: XCTestCase {
    func testTextAlwaysCarriesFontSizeAndOpacity() {
        let text = GhosttyConfigFile.text(fontSize: 16, opacity: 0.9, backgroundBlur: 0)
        XCTAssertTrue(text.contains("font-size = 16.0\n"), text)
        XCTAssertTrue(text.contains("background-opacity = 0.9\n"), text)
    }

    func testTextWithoutBlurOmitsTheBlurKey() {
        let text = GhosttyConfigFile.text(fontSize: 14, opacity: 1.0, backgroundBlur: 0)
        XCTAssertFalse(text.contains("background-blur"), text)
    }

    func testTextWithBlurCarriesIt() {
        let text = GhosttyConfigFile.text(fontSize: 14, opacity: 1.0, backgroundBlur: 20)
        XCTAssertTrue(text.contains("background-blur = 20\n"), text)
    }

    func testTextEndsWithOneNewline() {
        let text = GhosttyConfigFile.text(fontSize: 14, opacity: 1.0, backgroundBlur: 8)
        XCTAssertTrue(text.hasSuffix("background-blur = 8\n"), text)
        XCTAssertFalse(text.hasSuffix("\n\n"), text)
    }
}
