// Config round-trip tests: the portable window reads the same config.json, so a
// save from this shell must not delete keys it does not own (fontFamily landed
// that way and a save here silently reverted the portable window's face).

import XCTest
@testable import Berth

@MainActor
final class AppConfigTests: XCTestCase {
    private var url: URL!

    override func setUp() {
        super.setUp()
        url = FileManager.default.temporaryDirectory
            .appendingPathComponent("berth-config-\(UUID().uuidString).json")
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: url)
        super.tearDown()
    }

    private func read() throws -> [String: Any] {
        let data = try Data(contentsOf: url)
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    func testSaveKeepsKeysTheShellDoesNotOwn() throws {
        let existing = """
        {"fontFamily": ["VictorMono NFM"], "futureKey": {"a": 1}, "fontSize": 10}
        """
        try existing.write(to: url, atomically: true, encoding: .utf8)

        let config = AppConfig.shared
        config.setFontSize(18)
        try config.save(to: url)

        let object = try read()
        XCTAssertEqual(object["fontFamily"] as? [String], ["VictorMono NFM"])
        XCTAssertEqual((object["futureKey"] as? [String: Any])?["a"] as? Int, 1)
        XCTAssertEqual(object["fontSize"] as? Double, 18)
    }

    func testSaveWritesItsOwnKeysWhenTheFileIsMissing() throws {
        let config = AppConfig.shared
        config.setFontSize(20)
        try config.save(to: url)

        let object = try read()
        XCTAssertEqual(object["fontSize"] as? Double, 20)
        XCTAssertNotNil(object["shortcuts"] as? [String: String])
        XCTAssertNotNil(object["socket"] as? String)
    }
}
