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

        // Subscription is a transport concern, not a core one.
        if let method = Payload.method(in: line) {
            switch method {
            case "events.subscribe": sink.setSubscribed(true)
            case "events.unsubscribe": sink.setSubscribed(false)
            default: break
            }
        }

        let response = engine.dispatch(request: text)
        flushEvents()
        return response.data(using: .utf8)
    }

    /// Move queued events from the core to subscribed clients.
    func flushEvents() {
        let pending = engine.drainEvents()
        guard !pending.isEmpty else { return }
        server?.broadcast(pending)
    }
}

private enum Payload {
    static func method(in line: Data) -> String? {
        guard
            let obj = try? JSONSerialization.jsonObject(with: line) as? [String: Any],
            let method = obj["method"] as? String
        else { return nil }
        return method
    }
}
