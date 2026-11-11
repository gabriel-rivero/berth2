// Bounded event replay for socket subscribers. Core events carry a process-wide
// monotonic `seq`; the ring retains the last N lines so reconnect can ask for
// `since=<seq>` (docs/03-protocol.md §3).

import Foundation

struct WireEvent {
    let seq: UInt64
    let type: String
    let line: String
}

struct EventReplay {
    /// Number of events the ring no longer has.
    let dropped: UInt64
    let lines: [String]
    /// Highest sequence considered by this replay.
    let upTo: UInt64
}

final class EventRing: @unchecked Sendable {
    private let capacity: Int
    private let lock = NSLock()
    private var events: [WireEvent] = []
    private var latest: UInt64 = 0

    init(capacity: Int = 1024) {
        self.capacity = max(1, capacity)
    }

    /// Record every event line from one drained batch. Lines without a numeric
    /// `seq` (for example a handshake) are ignored for replay but still reach
    /// live subscribers through the normal broadcast path.
    @discardableResult
    func record(_ ndjson: String) -> [WireEvent] {
        var parsed: [WireEvent] = []
        for line in ndjson.split(separator: "\n") where !line.isEmpty {
            guard
                let data = line.data(using: .utf8),
                let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                let seq = (object["seq"] as? NSNumber)?.uint64Value,
                let type = object["event"] as? String
            else { continue }
            parsed.append(WireEvent(seq: seq, type: type, line: String(line)))
        }
        guard !parsed.isEmpty else { return [] }

        lock.lock()
        for event in parsed {
            latest = max(latest, event.seq)
            events.append(event)
        }
        if events.count > capacity {
            events.removeFirst(events.count - capacity)
        }
        lock.unlock()
        return parsed
    }

    var latestSeq: UInt64 {
        lock.lock()
        defer { lock.unlock() }
        return latest
    }

    /// Events after `since`, optionally filtered by type. A ring overflow is
    /// reported as an explicit gap before the surviving events.
    func replay(since: UInt64, types: Set<String>?) -> EventReplay {
        lock.lock()
        defer { lock.unlock() }

        let oldest = events.first?.seq
        var dropped: UInt64 = 0
        if let oldest, since &+ 1 < oldest {
            dropped = oldest - since - 1
        } else if oldest == nil, latest > since {
            dropped = latest - since
        }

        let lines = events
            .filter { $0.seq > since }
            .filter { types?.contains($0.type) ?? true }
            .map(\.line)
        return EventReplay(dropped: dropped, lines: lines, upTo: latest)
    }

    /// A fresh subscriber without `since` starts at the current head.
    func head() -> UInt64 {
        latestSeq
    }
}
