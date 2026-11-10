// Tests that do not need a window, a GPU, or a PTY. Anything touching Ghostty
// belongs in the manual smoke checklist in docs/05-build.md until M1.

import XCTest
@testable import Berth

final class NDJSONBufferTests: XCTestCase {
    func testSplitsOnNewlinesAndKeepsRemainder() {
        var buffer = NDJSONBuffer()
        var lines = buffer.append(Data("{\"a\":1}\n{\"b\":".utf8))
        XCTAssertEqual(lines.count, 1)
        XCTAssertEqual(String(data: lines[0], encoding: .utf8), "{\"a\":1}")
        XCTAssertEqual(String(data: buffer.pending, encoding: .utf8), "{\"b\":")

        lines = buffer.append(Data("2}\n\n".utf8))
        XCTAssertEqual(lines.count, 1)
        XCTAssertEqual(String(data: lines[0], encoding: .utf8), "{\"b\":2}")
        XCTAssertTrue(buffer.pending.isEmpty)
    }

    func testMultipleLinesInOneChunk() {
        var buffer = NDJSONBuffer()
        let lines = buffer.append(Data("a\nb\nc".utf8))
        XCTAssertEqual(lines.map { String(data: $0, encoding: .utf8)! }, ["a", "b"])
    }
}
