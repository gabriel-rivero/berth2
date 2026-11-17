// Rebuild-scope tests. Building a split view posts didResize notifications;
// reporting those back to the core re-entered MainWindow.apply and overflowed
// the stack on ⌘D. These pin the guard so the feedback path cannot return.

import AppKit
import XCTest
@testable import Berth

@MainActor
final class LayoutRebuildTests: XCTestCase {
    private func makeSplit() -> NSSplitView {
        let split = NSSplitView(frame: NSRect(x: 0, y: 0, width: 400, height: 200))
        split.isVertical = true
        split.addArrangedSubview(NSView(frame: NSRect(x: 0, y: 0, width: 200, height: 200)))
        split.addArrangedSubview(NSView(frame: NSRect(x: 200, y: 0, width: 200, height: 200)))
        split.adjustSubviews()
        return split
    }

    private func makeActions(
        reported: @escaping @MainActor (String, Int, Double) -> Void
    ) -> PaneActions {
        PaneActions(
            activate: { _ in },
            close: { _ in },
            newTab: { _ in },
            resizeSplit: { reported($0, $1, $2) })
    }

    func testRebuildScopeNests() {
        XCTAssertFalse(LayoutRebuild.isActive)
        LayoutRebuild.perform {
            XCTAssertTrue(LayoutRebuild.isActive)
            LayoutRebuild.perform {
                XCTAssertTrue(LayoutRebuild.isActive)
            }
            XCTAssertTrue(LayoutRebuild.isActive)
        }
        XCTAssertFalse(LayoutRebuild.isActive)
    }

    func testResizeDuringRebuildIsNotReported() {
        var reported: [(String, Int, Double)] = []
        let coordinator = SplitCoordinator(
            splitID: "p1",
            isVertical: true,
            weights: [0.5, 0.5],
            actions: makeActions { reported.append(($0, $1, $2)) })
        let notification = Notification(
            name: NSSplitView.didResizeSubviewsNotification,
            object: makeSplit())

        LayoutRebuild.perform {
            coordinator.splitViewDidResizeSubviews(notification)
        }
        XCTAssertTrue(
            reported.isEmpty,
            "a resize reported mid-rebuild feeds half-laid-out weights back to the core")

        coordinator.splitViewDidResizeSubviews(notification)
        XCTAssertEqual(reported.count, 1, "a divider drag outside a rebuild still reaches the core")
    }
}
