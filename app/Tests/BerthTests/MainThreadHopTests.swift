// Lock-order tests. `Engine.dispatch` holds the core's state mutex for the
// whole request, so a host callback must never wait on the main thread: the
// main thread can be waiting for that same mutex. That inversion wedged the app
// seconds after launch (see Diagnostics watchdog breadcrumbs).

import XCTest
@testable import Berth

final class MainThreadHopTests: XCTestCase {
    /// The background detection pass asks the core for the foreground pid while
    /// the core holds its lock. If `pidInfo` waits on the main thread, and the
    /// main thread is inside a core dispatch, neither can proceed.
    func testPidInfoFromBackgroundDoesNotWaitForMain() {
        let returned = Flag()
        DispatchQueue.global(qos: .utility).async {
            _ = try? BerthCoreHost.shared.pidInfo(surfaceId: "srf_not_a_surface")
            returned.value = true
        }

        // Main deliberately services nothing while the lookup runs: a main-queue
        // hop cannot complete, which is exactly the deadlock.
        Thread.sleep(forTimeInterval: 0.5)
        XCTAssertTrue(
            returned.value,
            "pidInfo blocked on the main thread; the core holds its lock while calling it")
    }
}

private final class Flag: @unchecked Sendable {
    var value = false
}
