// Line framing for the NDJSON protocol. Kept separate so it can be unit tested
// without a socket, a window, or Ghostty.

import Foundation

public struct NDJSONBuffer {
    private var buffer = Data()
    private let maxLineBytes: Int
    /// True after this buffer rejected an oversized line. The connection layer
    /// turns that into `payload_too_large` and closes the desynchronised client.
    public private(set) var droppedOversized = false
    private var discardingOversized = false

    public init(maxLineBytes: Int = 8 << 20) {
        self.maxLineBytes = maxLineBytes
    }

    /// Append bytes, return every complete line (without the trailing newline).
    public mutating func append(_ data: Data) -> [Data] {
        var rejected = false

        if discardingOversized {
            // The rest of an overlong line is noise; resume at the next newline.
            guard let idx = data.firstIndex(of: 0x0A) else { return [] }
            discardingOversized = false
            rejected = true
            buffer = Data(data[data.index(after: idx)...])
        } else {
            buffer.append(data)
        }

        var lines: [Data] = []
        while let idx = buffer.firstIndex(of: 0x0A) {
            let line = buffer[buffer.startIndex..<idx]
            if line.count > maxLineBytes {
                rejected = true
            } else if !line.isEmpty {
                lines.append(Data(line))
            }
            buffer.removeSubrange(buffer.startIndex...idx)
        }
        if buffer.count > maxLineBytes {
            // No newline yet: drop what we have and ignore until one arrives.
            buffer.removeAll(keepingCapacity: true)
            discardingOversized = true
            rejected = true
        }
        droppedOversized = rejected
        return lines
    }

    /// Remaining bytes that do not yet form a line.
    public var pending: Data { buffer }
}
