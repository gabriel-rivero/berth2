// Line framing for the NDJSON protocol. Kept separate so it can be unit tested
// without a socket, a window, or Ghostty.

import Foundation

public struct NDJSONBuffer {
    private var buffer = Data()
    private let maxLineBytes: Int

    public init(maxLineBytes: Int = 8 << 20) {
        self.maxLineBytes = maxLineBytes
    }

    /// Append bytes, return every complete line (without the trailing newline).
    public mutating func append(_ data: Data) -> [Data] {
        buffer.append(data)
        var lines: [Data] = []
        while let idx = buffer.firstIndex(of: 0x0A) {
            let line = buffer[buffer.startIndex..<idx]
            if !line.isEmpty {
                lines.append(Data(line))
            }
            buffer.removeSubrange(buffer.startIndex...idx)
        }
        if buffer.count > maxLineBytes {
            // Caller decides what to do; we drop the oversized partial line.
            buffer.removeAll(keepingCapacity: true)
        }
        return lines
    }

    /// Remaining bytes that do not yet form a line.
    public var pending: Data { buffer }
}
