// The socket's request handler: forwards protocol lines to the Rust core and
// relays whatever the core queued as events. No domain logic lives here.

import BerthCore
import Foundation

final class CoreHandler: RequestHandler {
    private let engine: Engine
    weak var server: SocketServer?
    /// The real transport mode, injected into `system.hello` responses. The
    /// core itself has no socket and cannot know whether this request arrived
    /// through a token-bearing local socket or a trusted one.
    var authMode = "local"
    /// Wired by AppDelegate: `session.save` is app policy (paths, journal),
    /// not pure core state.
    var onSaveSession: (() -> Void)?

    init(engine: Engine) {
        self.engine = engine
    }

    func handle(_ line: Data, from sink: ClientSink) -> Data? {
        guard let text = String(data: line, encoding: .utf8) else { return nil }

        // Subscription, filtering and replay are transport concerns.
        let payload = Payload.parse(line)
        if let payload {
            let thread = Thread.isMainThread ? "main" : "off-main"
            BerthLog.core.debug(
                "request method=\(payload.method, privacy: .public) thread=\(thread, privacy: .public)")
            switch payload.method {
            case "events.subscribe":
                sink.subscribe(types: payload.types, since: payload.since)
            case "events.unsubscribe":
                sink.unsubscribe()
            case "session.save":
                BerthLog.restore.info("session.save requested over socket")
                onSaveSession?()
            default:
                break
            }
        }

        let response = dispatch(text, rewriteHello: payload?.method == "system.hello")
        flushEvents()
        // Structural changes (split/close/create) land in the core first; the
        // window tree follows on the next runloop turn (coalesced).
        BerthCoreHost.shared.scheduleReconcile()
        return response.data(using: .utf8)
    }

    /// The server's greeting: protocol/app/instance facts, no event sequence.
    func hello() -> Data? {
        let response = dispatch(
            #"{"id":"hello","method":"system.hello","params":{}}"#, rewriteHello: true)
        guard let data = response.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let result = object["result"]
        else { return nil }
        let event: [String: Any] = ["event": "hello", "data": result]
        guard let encoded = try? JSONSerialization.data(withJSONObject: event) else { return nil }
        return encoded
    }

    /// Run the core on the main thread. The socket forwards requests here
    /// asynchronously, so this should always already be main.
    private func dispatch(_ request: String, rewriteHello: Bool = false) -> String {
        let raw: String
        if Thread.isMainThread {
            raw = engine.dispatch(request: request)
        } else {
            BerthLog.core.error("core dispatch off-main; hopping synchronously")
            assertionFailure("core dispatch must run on the main thread")
            let engine = self.engine
            raw = DispatchQueue.main.sync {
                engine.dispatch(request: request)
            }
        }
        guard rewriteHello else { return raw }
        return Self.injectAuthMode(raw, mode: authMode)
    }

    /// Replace the core's placeholder `auth` field with the transport's real
    /// mode. Everything else is passed through byte-for-byte.
    private static func injectAuthMode(_ response: String, mode: String) -> String {
        guard let data = response.data(using: .utf8),
              var object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              var result = object["result"] as? [String: Any]
        else { return response }
        result["auth"] = mode
        object["result"] = result
        guard let encoded = try? JSONSerialization.data(withJSONObject: object),
              let text = String(data: encoded, encoding: .utf8)
        else { return response }
        return text
    }

    /// Move queued events from the core to the server's replay ring and
    /// subscribed clients.
    func flushEvents() {
        let pending = engine.drainEvents()
        guard !pending.isEmpty else { return }
        server?.broadcast(pending)
    }
}

private enum Payload {
    struct Parsed {
        var method: String
        var types: [String]?
        var since: UInt64?
    }

    static func parse(_ line: Data) -> Parsed? {
        guard
            let obj = try? JSONSerialization.jsonObject(with: line) as? [String: Any],
            let method = obj["method"] as? String
        else { return nil }
        let params = obj["params"] as? [String: Any]
        let types = (params?["types"] as? [String])?.filter { !$0.isEmpty }
        let since = (params?["since"] as? NSNumber)?.uint64Value
        return Parsed(method: method, types: types, since: since)
    }
}
