// Unix-socket NDJSON server. Transport only: framing, auth, subscriptions,
// replay and backpressure live here; domain decisions belong to berth-core
// (docs/03-protocol.md).

import Darwin
import Foundation

/// Per-connection event subscription, controlled by the request handler after
/// it has seen `events.subscribe`.
protocol ClientSink: AnyObject {
    func subscribe(types: [String]?, since: UInt64?)
    func unsubscribe()
}

protocol RequestHandler: AnyObject {
    /// One complete NDJSON line in, one NDJSON line out (no newline needed).
    func handle(_ line: Data, from sink: ClientSink) -> Data?
    /// Greeting written before the first request is read.
    func hello() -> Data?
}

final class SocketServer {
    private let path: String
    private let mode: SocketMode
    private let token: String?
    private let ring = EventRing()
    private var listenFD: Int32 = -1
    private var acceptSource: DispatchSourceRead?
    private let acceptQueue = DispatchQueue(label: "dev.berth.socket.accept")
    private var clients: [Int32: Client] = [:]
    private let clientsLock = NSLock()
    private let handler: RequestHandler

    init(path: String, handler: RequestHandler, mode: SocketMode = .local, token: String? = nil) {
        self.path = path
        self.handler = handler
        self.mode = mode
        self.token = token
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
        chmod(path, 0o600)
        let flags = fcntl(fd, F_GETFL, 0)
        _ = fcntl(fd, F_SETFL, flags | O_NONBLOCK)
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
        BerthLog.socket.info(
            "listening path=\(path, privacy: .public) mode=\(mode.rawValue, privacy: .public)")
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

    /// Record an event batch for replay, then fan it out. Filtering happens in
    /// each client so a reconnect can replay events it never asked to see live.
    func broadcast(_ ndjson: String) {
        let events = ring.record(ndjson)
        guard !events.isEmpty else { return }
        clientsLock.lock()
        let targets = Array(clients.values)
        clientsLock.unlock()
        for client in targets {
            client.deliver(events)
        }
    }

    private func acceptPending() {
        while true {
            let clientFD = accept(listenFD, nil, nil)
            if clientFD < 0 { return }
            let client = Client(
                fd: clientFD,
                handler: handler,
                ring: ring,
                mode: mode,
                token: token
            ) { [weak self] fd in
                self?.clientsLock.lock()
                self?.clients.removeValue(forKey: fd)
                self?.clientsLock.unlock()
            }
            clientsLock.lock()
            clients[clientFD] = client
            clientsLock.unlock()
            BerthLog.socket.debug("accepted client fd=\(clientFD, privacy: .public)")
            client.start()
        }
    }
}

private final class Client: ClientSink, @unchecked Sendable {
    private let fd: Int32
    private let queue: DispatchQueue
    private let handler: RequestHandler
    private let ring: EventRing
    private let mode: SocketMode
    private let token: String?
    private let onClose: (Int32) -> Void

    private var source: DispatchSourceRead?
    private var writeSource: DispatchSourceWrite?
    private var frames = NDJSONBuffer()
    private var outbound = Data()
    private var closed = false
    private let maxPendingBytes = 4 << 20

    private var subscribed = false
    private var filters: Set<String>?
    private var lastSeq: UInt64 = 0

    init(
        fd: Int32,
        handler: RequestHandler,
        ring: EventRing,
        mode: SocketMode,
        token: String?,
        onClose: @escaping (Int32) -> Void
    ) {
        self.fd = fd
        self.handler = handler
        self.ring = ring
        self.mode = mode
        self.token = token
        self.onClose = onClose
        self.queue = DispatchQueue(label: "dev.berth.client.\(fd)")
    }

    func start() {
        var one: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &one, socklen_t(MemoryLayout<Int32>.size))
        let flags = fcntl(fd, F_GETFL, 0)
        _ = fcntl(fd, F_SETFL, flags | O_NONBLOCK)

        let source = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
        source.setEventHandler { [weak self] in self?.readAvailable() }
        source.setCancelHandler { [fd] in Darwin.close(fd) }
        source.resume()
        self.source = source

        queue.async { [weak self] in
            guard let self else { return }
            // The core and its host callbacks live on the main thread. Forward
            // there asynchronously instead of blocking this client queue on a
            // synchronous hop: a stuck main thread must not wedge every socket.
            DispatchQueue.main.async { [weak self] in
                guard let self, let hello = self.handler.hello() else { return }
                self.queue.async { [weak self] in self?.send(hello) }
            }
        }
    }

    func close() {
        queue.async { self.closeNow() }
    }

    // MARK: - Subscriptions

    func subscribe(types: [String]?, since: UInt64?) {
        queue.async {
            guard !self.closed else { return }
            self.subscribed = true
            self.filters = types.map { Set($0) }
            if let since {
                let replay = self.ring.replay(since: since, types: self.filters)
                if replay.dropped > 0 {
                    self.sendGap(replay.dropped)
                }
                for line in replay.lines {
                    self.send(Data(line.utf8))
                }
                self.lastSeq = max(self.lastSeq, replay.upTo)
            } else {
                self.lastSeq = max(self.lastSeq, self.ring.head())
            }
        }
    }

    func unsubscribe() {
        queue.async {
            self.subscribed = false
            self.filters = nil
        }
    }

    func deliver(_ events: [WireEvent]) {
        queue.async {
            guard !self.closed, self.subscribed else { return }
            for event in events {
                guard event.seq > self.lastSeq else { continue }
                self.lastSeq = event.seq
                if self.filters?.contains(event.type) ?? true {
                    self.send(Data(event.line.utf8))
                }
            }
        }
    }

    // MARK: - I/O

    private func readAvailable() {
        var chunk = [UInt8](repeating: 0, count: 16 * 1024)
        let n = read(fd, &chunk, chunk.count)
        if n < 0 {
            if errno == EAGAIN || errno == EINTR { return }
            closeNow()
            return
        }
        if n == 0 {
            closeNow()
            return
        }

        let lines = frames.append(Data(chunk[0..<n]))
        if frames.droppedOversized {
            send(Data(Self.errorLine(
                id: NSNull(),
                code: "payload_too_large",
                message: "request exceeds 8 MiB").utf8))
            closeNow()
            return
        }

        for line in lines {
            handle(line)
        }
    }

    private func handle(_ line: Data) {
        guard let object = try? JSONSerialization.jsonObject(with: line) as? [String: Any] else {
            // Let the core produce the canonical bad_request response.
            forward(line)
            return
        }

        let id = object["id"] ?? NSNull()
        if let method = object["method"] as? String,
           !SocketAuth.authorized(
                method: method,
                presented: SocketAuth.presentedToken(in: object),
                token: token,
                mode: mode)
        {
            BerthLog.socket.error(
                "permission denied method=\(method, privacy: .public) fd=\(self.fd, privacy: .public)")
            send(Data(Self.errorLine(
                id: id,
                code: "permission_denied",
                message: "token required for \(method)").utf8))
            return
        }

        forward(line)
    }

    /// Hand the request to the main-thread core and send the reply back here.
    private func forward(_ line: Data) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            if let response = self.handler.handle(line, from: self) {
                self.queue.async { [weak self] in self?.send(response) }
            }
        }
    }

    /// Append one frame and flush non-blockingly. A client that cannot keep up
    /// is disconnected rather than allowed to grow an unbounded buffer.
    private func send(_ data: Data) {
        guard !closed else { return }
        outbound.append(data)
        outbound.append(0x0A)
        if outbound.count > maxPendingBytes {
            BerthLog.socket.error(
                "client fd=\(self.fd, privacy: .public) exceeded outbound buffer bytes=\(self.outbound.count, privacy: .public)")
            closeNow()
            return
        }
        flush()
    }

    private func sendGap(_ dropped: UInt64) {
        let line = #"{"event":"gap","data":{"dropped":\#(dropped)}}"#
        send(Data(line.utf8))
    }

    private func flush() {
        while !closed && !outbound.isEmpty {
            let written: Int = outbound.withUnsafeBytes { raw -> Int in
                guard let base = raw.baseAddress else { return 0 }
                return Darwin.write(fd, base, raw.count)
            }
            if written > 0 {
                outbound.removeFirst(written)
                continue
            }
            if written < 0 && errno == EINTR { continue }
            if written < 0 && (errno == EAGAIN || errno == EWOULDBLOCK) {
                ensureWriteSource()
                return
            }
            closeNow()
            return
        }
        if outbound.isEmpty {
            writeSource?.cancel()
            writeSource = nil
        }
    }

    private func ensureWriteSource() {
        guard writeSource == nil, !closed else { return }
        let source = DispatchSource.makeWriteSource(fileDescriptor: fd, queue: queue)
        source.setEventHandler { [weak self] in self?.flush() }
        source.resume()
        writeSource = source
    }

    private func closeNow() {
        guard !closed else { return }
        closed = true
        source?.cancel()
        source = nil
        writeSource?.cancel()
        writeSource = nil
        shutdown(fd, SHUT_RDWR)
        onClose(fd)
    }

    private static func errorLine(id: Any, code: String, message: String) -> String {
        let object: [String: Any] = [
            "id": id,
            "ok": false,
            "error": ["code": code, "message": message],
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: object) else {
            return #"{"id":null,"ok":false,"error":{"code":"internal","message":"encode"}}"#
        }
        return String(decoding: data, as: UTF8.self)
    }
}
