// Layout decoding tests: the core serializes split orientation in snake_case,
// and NSSplitView's `isVertical` has the opposite sense. These pin the mapping
// so Split Right and Split Down cannot collapse into the same geometry again.

import XCTest
@testable import Berth

final class WorkspaceLayoutTests: XCTestCase {
    private func layout(orientation: String) -> WorkspaceLayout? {
        let json = """
        {
          "workspace": {
            "id": "ws1",
            "name": "main",
            "active_surface": "s1",
            "root": {
              "type": "split",
              "id": "p1",
              "orientation": "\(orientation)",
              "children": [
                {"type": "leaf", "pane_id": "pane1"},
                {"type": "leaf", "pane_id": "pane2"}
              ],
              "weights": [0.5, 0.5]
            }
          },
          "panes": [
            {"id": "pane1", "surfaces": ["s1"], "active_surface": "s1"},
            {"id": "pane2", "surfaces": ["s2"], "active_surface": "s2"}
          ],
          "surfaces": [
            {"id": "s1", "title": "", "cwd": "/tmp"},
            {"id": "s2", "title": "", "cwd": "/tmp"}
          ]
        }
        """
        return WorkspaceLayout.parse(json)
    }

    func testHorizontalSplitIsVerticalDivider() throws {
        let root = try XCTUnwrap(layout(orientation: "horizontal")?.root)
        guard case let .split(_, isVertical, children, _) = root else {
            return XCTFail("expected a split root")
        }
        XCTAssertTrue(isVertical, "horizontal children must sit side by side")
        XCTAssertEqual(children.count, 2)
    }

    func testVerticalSplitIsHorizontalDivider() throws {
        let root = try XCTUnwrap(layout(orientation: "vertical")?.root)
        guard case let .split(_, isVertical, _, _) = root else {
            return XCTFail("expected a split root")
        }
        XCTAssertFalse(isVertical, "vertical children must stack")
    }

    func testLegacyCapitalizedOrientationStillParses() throws {
        let root = try XCTUnwrap(layout(orientation: "Horizontal")?.root)
        guard case let .split(_, isVertical, _, _) = root else {
            return XCTFail("expected a split root")
        }
        XCTAssertTrue(isVertical)
    }
}
