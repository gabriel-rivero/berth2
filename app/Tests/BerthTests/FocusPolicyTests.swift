// Focus ownership. A click reports focus to the core, but the core's active
// surface can lag by one pass, and a reconcile pass that trusted it handed the
// keyboard back to the other pane.

import XCTest
@testable import Berth

final class FocusPolicyTests: XCTestCase {
    func testFocusIsNotTakenFromATerminal() {
        XCTAssertFalse(
            FocusPolicy.shouldPlaceFocus(isKeyWindow: true, firstResponderIsTerminal: true),
            "a reconcile pass overrode the pane the user had clicked into")
    }

    func testFocusIsPlacedWhenNothingTerminalHasIt() {
        XCTAssertTrue(
            FocusPolicy.shouldPlaceFocus(isKeyWindow: true, firstResponderIsTerminal: false))
    }

    func testFocusIsNotPlacedInABackgroundWindow() {
        XCTAssertFalse(
            FocusPolicy.shouldPlaceFocus(isKeyWindow: false, firstResponderIsTerminal: false))
    }
}
