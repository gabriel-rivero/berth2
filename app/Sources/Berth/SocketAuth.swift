// Socket authentication policy and token storage. Transport-level, so it is
// unit-testable without a running app or a socket.
//
// Modes mirror docs/03-protocol.md §2: `off`, `local` (token required for
// mutating methods) and `trusted` (same-uid only, no token).

import Foundation

enum SocketMode: String, CaseIterable {
    case off
    case local
    case trusted
}

enum SocketAuth {
    /// Methods that never require a token in `local` mode. Reading terminal
    /// contents is deliberately *not* here: screen text is sensitive.
    static func isReadOnly(method: String) -> Bool {
        if method.hasPrefix("system.") { return true }
        switch method {
        case "workspace.list", "workspace.get", "workspace.layout",
             "group.list",
             "pane.list",
             "surface.list",
             "agent.get", "agent.list",
             "notification.list",
             "events.subscribe", "events.unsubscribe",
             "session.dump":
            return true
        default:
            return false
        }
    }

    static func requiresToken(method: String) -> Bool {
        !isReadOnly(method: method)
    }

    /// Only `local` mode authenticates mutations. `trusted` relies on the
    /// socket's 0600 permissions; `off` never reaches this code.
    static func authorized(
        method: String,
        presented: String?,
        token: String?,
        mode: SocketMode
    ) -> Bool {
        guard mode == .local else { return mode == .trusted }
        guard requiresToken(method: method) else { return true }
        guard let token, let presented else { return false }
        return constantTimeEquals(token, presented)
    }

    static func constantTimeEquals(_ lhs: String, _ rhs: String) -> Bool {
        let a = Array(lhs.utf8)
        let b = Array(rhs.utf8)
        guard a.count == b.count else { return false }
        var difference: UInt8 = 0
        for index in a.indices {
            difference |= a[index] ^ b[index]
        }
        return difference == 0
    }

    /// Read an existing token, creating a 0600 token file when absent.
    @discardableResult
    static func loadOrCreateToken(at url: URL) throws -> String {
        if let existing = try? String(contentsOf: url, encoding: .utf8)
            .trimmingCharacters(in: .whitespacesAndNewlines),
            !existing.isEmpty
        {
            try? FileManager.default.setAttributes(
                [.posixPermissions: 0o600], ofItemAtPath: url.path)
            return existing
        }

        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true)
        var bytes = [UInt8](repeating: 0, count: 32)
        for index in bytes.indices {
            bytes[index] = UInt8.random(in: 0...255)
        }
        let token = bytes.map { String(format: "%02x", $0) }.joined()
        try Data(token.utf8).write(to: url, options: .atomic)
        try FileManager.default.setAttributes(
            [.posixPermissions: 0o600], ofItemAtPath: url.path)
        return token
    }

    static func readToken(at url: URL) -> String? {
        guard let value = try? String(contentsOf: url, encoding: .utf8)
            .trimmingCharacters(in: .whitespacesAndNewlines),
            !value.isEmpty
        else { return nil }
        return value
    }

    /// Top-level `token`, with `params.token` accepted for clients that put
    /// every argument in the params object.
    static func presentedToken(in object: [String: Any]) -> String? {
        if let token = object["token"] as? String { return token }
        if let params = object["params"] as? [String: Any],
           let token = params["token"] as? String
        {
            return token
        }
        return nil
    }

    static func tokenURL(supportDirectory: URL) -> URL {
        supportDirectory.appendingPathComponent("token")
    }
}
