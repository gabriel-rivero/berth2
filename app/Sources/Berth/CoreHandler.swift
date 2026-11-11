// The socket's request handler: forwards protocol lines to the Rust core and
// relays whatever the core queued as events. No domain logic lives here.

import BerthCore
import Foundation

final class CoreHandler: RequestHandler {
    private let engine: Engine
    weak var server: SocketServer?

    init(engine: Engine) {
        self.engine = engine
    }

    func handle(_ line: Data, from sink: ClientSink) -> Data? {
        guard let text = String(data: line, encoding: .utf8) else { return nil }

        // Subscription, filtering and replay are transport concerns.
        if let payload = Payload.parse(line) {
            switch payload.method {
            case "events.subscribe":
                sink.subscribe(types: payload.types, since: payload.since)
            case "events.unsubscribe":
                sink.unsubscribe()
            default:
                break
            }
        }

        let response = dispatch(text)
        flushEvents()
        // Structural changes (split/close/create) land in the core first; the
        // window tree follows on the next runloop turn.
        DispatchQueue.main.async {
            BerthCoreHost.shared.reconcileAll()
        }
        return response.data(using: .utf8)
    }

    /// The server's greeting: protocol/app/instance facts, no event sequence.
    func hello() -> Data? {
        let response = dispatch(
            #"{"id":"hello","method":"system.hello","params":{}}"#)
        guard let data = response.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let result = object["result"]
        else { return nil }
        let event: [String: Any] = ["event": "hello", "data": result]
        guard let encoded = try? JSONSerialization.data(withJSONObject: event) else { return nil }
        return encoded
    }

    /// Run the core on the main thread. Core dispatch holds its state lock while
    /// it calls back into the host, and host callbacks synchronously hop to the
    /// main thread; dispatching from a socket thread would deadlock against any
    /// main-thread dispatch (timer, reconcile, UI command) waiting on that lock.
    private func dispatch(_ request: String) -> String {
        if Thread.isMainThread {
            return engine.dispatch(request: request)
        }
        let engine = self.engine
        return DispatchQueue.main.sync {
            engine.dispatch(request: request)
        }
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
