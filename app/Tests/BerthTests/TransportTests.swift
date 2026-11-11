// Transport-level tests: event replay, socket auth, and oversized framing.
// None of these need a window, a GPU, or a running app.

import XCTest
@testable import Berth

final class EventRingTests: XCTestCase {
    func testReplayFiltersBySequenceAndType() {
        let ring = EventRing(capacity: 8)
        XCTAssertEqual(ring.record("""
        {"seq":1,"event":"surface.title_changed"}
        {"seq":2,"event":"surface.pwd_changed"}
        {"seq":3,"event":"surface.title_changed"}
        """).count, 3)

        let replay = ring.replay(since: 1, types: ["surface.title_changed"])
        XCTAssertEqual(replay.dropped, 0)
        XCTAssertEqual(replay.lines.count, 1)
        XCTAssertTrue(replay.lines[0].contains("\"seq\":3"))
        XCTAssertEqual(replay.upTo, 3)
    }

    func testOverflowReportsAGap() {
        let ring = EventRing(capacity: 2)
        ring.record("""
        {"seq":1,"event":"a"}
        {"seq":2,"event":"b"}
        {"seq":3,"event":"c"}
        """)
        let replay = ring.replay(since: 0, types: nil)
        XCTAssertEqual(replay.dropped, 1)
        XCTAssertEqual(replay.lines.count, 2)
        XCTAssertEqual(replay.upTo, 3)
    }

    func testLinesWithoutSeqAreIgnored() {
        let ring = EventRing()
        XCTAssertTrue(ring.record("{\"event\":\"hello\"}").isEmpty)
        XCTAssertEqual(ring.latestSeq, 0)
    }
}

final class SocketAuthTests: XCTestCase {
    func testReadOnlyClassification() {
        XCTAssertTrue(SocketAuth.isReadOnly(method: "system.hello"))
        XCTAssertTrue(SocketAuth.isReadOnly(method: "workspace.list"))
        XCTAssertTrue(SocketAuth.isReadOnly(method: "session.dump"))
        XCTAssertFalse(SocketAuth.isReadOnly(method: "surface.read_text"))
        XCTAssertFalse(SocketAuth.isReadOnly(method: "surface.send_text"))
        XCTAssertFalse(SocketAuth.isReadOnly(method: "workspace.create"))
    }

    func testLocalModeRequiresTokenForMutations() {
        XCTAssertTrue(
            SocketAuth.authorized(
                method: "workspace.list", presented: nil, token: "secret", mode: .local))
        XCTAssertFalse(
            SocketAuth.authorized(
                method: "workspace.create", presented: nil, token: "secret", mode: .local))
        XCTAssertTrue(
            SocketAuth.authorized(
                method: "workspace.create", presented: "secret", token: "secret", mode: .local))
        XCTAssertFalse(
            SocketAuth.authorized(
                method: "workspace.create", presented: "wrong", token: "secret", mode: .local))
        XCTAssertTrue(
            SocketAuth.authorized(
                method: "workspace.create", presented: nil, token: nil, mode: .trusted))
        XCTAssertFalse(
            SocketAuth.authorized(
                method: "workspace.create", presented: "secret", token: "secret", mode: .off))
    }

    func testPresentedTokenAcceptsTopLevelAndParams() {
        XCTAssertEqual(
            SocketAuth.presentedToken(in: ["method": "x", "token": "top"]), "top")
        XCTAssertEqual(
            SocketAuth.presentedToken(in: ["method": "x", "params": ["token": "inner"]]),
            "inner")
        XCTAssertNil(SocketAuth.presentedToken(in: ["method": "x"]))
    }

    func testTokenIsCreatedOnceWithOwnerOnlyPermissions() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("berth-auth-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = SocketAuth.tokenURL(supportDirectory: directory)

        let first = try SocketAuth.loadOrCreateToken(at: url)
        let second = try SocketAuth.loadOrCreateToken(at: url)
        XCTAssertEqual(first, second)
        XCTAssertEqual(first.count, 64)
        XCTAssertEqual(SocketAuth.readToken(at: url), first)

        let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
        let permissions = (attributes[.posixPermissions] as? NSNumber)?.intValue
        XCTAssertEqual(permissions, 0o600)
    }
}

final class NDJSONLimitTests: XCTestCase {
    func testOversizedLinesAreDroppedAndReported() {
        var buffer = NDJSONBuffer(maxLineBytes: 16)
        let lines = buffer.append(Data("{\"too\":\"long for the limit\"}\n{\"ok\":1}\n".utf8))
        XCTAssertEqual(lines.compactMap { String(data: $0, encoding: .utf8) }, ["{\"ok\":1}"])
        XCTAssertTrue(buffer.droppedOversized)
    }

    func testOversizedPartialLineIsDiscarded() {
        var buffer = NDJSONBuffer(maxLineBytes: 8)
        XCTAssertTrue(buffer.append(Data("{\"long\":true".utf8)).isEmpty)
        XCTAssertTrue(buffer.droppedOversized)
        let lines = buffer.append(Data("}\n{\"a\":1}\n".utf8))
        XCTAssertEqual(lines.compactMap { String(data: $0, encoding: .utf8) }, ["{\"a\":1}"])
    }
}
