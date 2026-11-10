// Unix-socket JSON-RPC server. Transport only: bytes in, bytes out. All domain
// decisions belong to berth-core (docs/03-protocol.md).

import Darwin
import Foundation

/// A place to send unsolicited lines (events) back to one connection.
protocol ClientSink: AnyObject {
    func setSubscribed(_ value: Bool)
}

protocol RequestHandler: AnyObject {
    /// One complete NDJSON line in, one NDJSON line out (no newline needed).
    func handle(_ line: Data, from sink: ClientSink) -> Data?
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
        try FileManager.default.createDirectory(
            at: URL(fileURLWithPath: path).deletingLastPathComponent(),
            withIntermediateDirectories: true)
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
            let err = POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
            close(fd)
            throw err
        }
        // 0600 is the whole of the v1 auth model (docs/03-protocol.md §2);
        // the token handshake lands with the client library.
        chmod(path, 0o600)
        guard listen(fd, 16) == 0 else {
            let err = POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
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
        clientsLock.lock()
        let all = Array(clients.values)
        clients.removeAll()
        clientsLock.unlock()
        for client in all {
            client.close()
        }
        if listenFD >= 0 {
            close(listenFD)
            listenFD = -1
        }
        unlink(path)
    }

    /// Deliver event lines to every subscribed client. `ndjson` may hold several
    /// newline-separated events; each is framed independently.
    func broadcast(_ ndjson: String) {
        clientsLock.lock()
        let targets = Array(clients.values)
        clientsLock.unlock()
        for client in targets {
            client.sendIfSubscribed(ndjson)
        }
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

private final class Client: ClientSink {
    private let fd: Int32
    private let queue: DispatchQueue
    private var source: DispatchSourceRead?
    private var frames = NDJSONBuffer()
    private var subscribed = false
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
        source.setCancelHandler { [fd] in Darwin.close(fd) }
        source.resume()
        self.source = source
    }

    func close() {
        queue.async { self.source?.cancel() }
    }

    func setSubscribed(_ value: Bool) {
        queue.async { self.subscribed = value }
    }

    func sendIfSubscribed(_ ndjson: String) {
        queue.async {
            guard self.subscribed else { return }
            for line in ndjson.split(separator: "\n") where !line.isEmpty {
                self.write(Data(line.utf8))
            }
        }
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
            guard let response = handler.handle(line, from: self) else { continue }
            write(response)
        }
    }

    private func write(_ data: Data) {
        var out = data
        out.append(0x0A)
        out.withUnsafeBytes { raw in
            var written = 0
            while written < raw.count {
                let w = Darwin.write(fd, raw.baseAddress!.advanced(by: written), raw.count - written)
                if w <= 0 { return }
                written += w
            }
        }
    }
}
