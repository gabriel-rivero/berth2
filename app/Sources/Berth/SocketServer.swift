// Unix-socket JSON-RPC server. Transport only: bytes in, bytes out. All domain
// decisions belong to berth-core (docs/03-protocol.md).
//
// TODO(M2): wire `handler` to the UniFFI bridge. Today the default handler
// answers `system.ping` and reports `internal` for everything else, which is
// enough to exercise framing and the CLI end to end.

import Darwin
import Foundation

protocol RequestHandler: AnyObject {
    /// One complete NDJSON line in, one NDJSON line out (no newline needed).
    func handle(_ line: Data) -> Data?
}

final class StubHandler: RequestHandler {
    func handle(_ line: Data) -> Data? {
        guard
            let obj = try? JSONSerialization.jsonObject(with: line) as? [String: Any],
            let id = obj["id"],
            let method = obj["method"] as? String
        else {
            return Self.encode(["id": NSNull(), "ok": false,
                                "error": ["code": "bad_request", "message": "malformed request"]])
        }
        if method == "system.ping" {
            return Self.encode(["id": id, "ok": true, "result": ["pong": true, "uptime_ms": 0]])
        }
        return Self.encode([
            "id": id, "ok": false,
            "error": ["code": "internal", "message": "berth-core bridge not wired yet"],
        ])
    }

    private static func encode(_ obj: [String: Any]) -> Data? {
        try? JSONSerialization.data(withJSONObject: obj, options: [])
    }
}

final class SocketServer {
    private let path: String
    private var listenFD: Int32 = -1
    private var acceptSource: DispatchSourceRead?
    private let acceptQueue = DispatchQueue(label: "dev.berth.socket.accept")
    private var clients: [Int32: Client] = [:]
    private let clientsLock = NSLock()
    private let handler: RequestHandler

    init(path: String, handler: RequestHandler) {
        self.path = path
        self.handler = handler
    }

    deinit { stop() }

    func start() throws {
        unlink(path)

        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw POSIXError(.ENOTSOCK) }

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let pathBytes = Array(path.utf8)
        guard pathBytes.count < MemoryLayout.size(ofValue: addr.sun_path) else {
            close(fd)
            throw POSIXError(.ENAMETOOLONG)
        }
        withUnsafeMutableBytes(of: &addr.sun_path) { raw in
            raw.copyBytes(from: pathBytes)
        }

        let bound = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard bound == 0 else {
            let err = POSIXError(rawValue: errno) ?? .EIO
            close(fd)
            throw err
        }
        chmod(path, 0o600)
        guard listen(fd, 16) == 0 else {
            let err = POSIXError(rawValue: errno) ?? .EIO
            close(fd)
            throw err
        }

        listenFD = fd
        let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: acceptQueue)
        source.setEventHandler { [weak self] in self?.acceptPending() }
        source.resume()
        acceptSource = source
        NSLog("berth: listening on \(path)")
    }

    func stop() {
        acceptSource?.cancel()
        acceptSource = nil
        if listenFD >= 0 {
            close(listenFD)
            listenFD = -1
        }
        unlink(path)
    }

    private func acceptPending() {
        while true {
            let clientFD = accept(listenFD, nil, nil)
            if clientFD < 0 { return }
            let client = Client(fd: clientFD, handler: handler) { [weak self] fd in
                self?.clientsLock.lock()
                self?.clients.removeValue(forKey: fd)
                self?.clientsLock.unlock()
            }
            clientsLock.lock()
            clients[clientFD] = client
            clientsLock.unlock()
            client.start()
        }
    }
}

private final class Client {
    private let fd: Int32
    private let queue: DispatchQueue
    private var source: DispatchSourceRead?
    private var frames = NDJSONBuffer()
    private let handler: RequestHandler
    private let onClose: (Int32) -> Void

    init(fd: Int32, handler: RequestHandler, onClose: @escaping (Int32) -> Void) {
        self.fd = fd
        self.handler = handler
        self.onClose = onClose
        // One serial queue per connection: ordering per client, no global lock.
        self.queue = DispatchQueue(label: "dev.berth.client.\(fd)")
    }

    func start() {
        let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
        source.setEventHandler { [weak self] in self?.readAvailable() }
        source.setCancelHandler { [fd] in close(fd) }
        source.resume()
        self.source = source
    }

    private func readAvailable() {
        var chunk = [UInt8](repeating: 0, count: 16 * 1024)
        let n = read(fd, &chunk, chunk.count)
        if n <= 0 {
            source?.cancel()
            onClose(fd)
            return
        }
        for line in frames.append(Data(chunk[0..<n])) {
            guard let response = handler.handle(line) else { continue }
            var out = response
            out.append(0x0A)
            out.withUnsafeBytes { raw in
                var written = 0
                while written < raw.count {
                    let w = write(fd, raw.baseAddress!.advanced(by: written), raw.count - written)
                    if w <= 0 { return }
                    written += w
                }
            }
        }
    }
}
