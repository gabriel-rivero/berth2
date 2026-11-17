// Process-wide diagnostics: unified logging categories, a rolling breadcrumb
// trail, a file log, a crash log for uncaught exceptions, and a watchdog for a
// wedged main thread. The unified log answers "what happened" after the fact
// (`log show --predicate 'subsystem == "dev.berth.app"'`); the breadcrumbs
// answer "what was in flight" when the app stops responding or dies.

import AppKit
import Foundation
import os

enum BerthLog {
    static let subsystem = "dev.berth.app"

    static let app = Logger(subsystem: subsystem, category: "app")
    static let core = Logger(subsystem: subsystem, category: "core")
    static let socket = Logger(subsystem: subsystem, category: "socket")
    static let agent = Logger(subsystem: subsystem, category: "agent")
    static let restore = Logger(subsystem: subsystem, category: "restore")
    static let ui = Logger(subsystem: subsystem, category: "ui")
    static let layout = Logger(subsystem: subsystem, category: "layout")

    /// Main-thread violations are logged rather than trapped: a hitch is
    /// easier to diagnose than a crash in the middle of a render pass.
    static func expectMain(_ label: String) {
        if !Thread.isMainThread {
            core.error("off-main: \(label, privacy: .public)")
        }
    }
}

// NSSetUncaughtExceptionHandler takes a C function pointer, so this must be a
// global function with no captured context. It only touches static state.
private func berthUncaughtExceptionHandler(_ exception: NSException) {
    let stack = exception.callStackSymbols.joined(separator: "\n")
    let trail = Diagnostics.breadcrumbs(40).joined(separator: "\n")
    let message = """
    \(Date()) \(exception.name.rawValue)
    \(exception.reason ?? "no reason")
    --- breadcrumbs ---
    \(trail)
    --- stack ---
    \(stack)
    """
    Diagnostics.recordProblem(message)
    BerthLog.app.fault(
        "uncaught exception \(exception.name.rawValue, privacy: .public): \(exception.reason ?? "", privacy: .public)")
}

/// Rolling in-memory breadcrumbs plus the file log. Everything is written off
/// the main thread; `record` only takes a short lock.
final class TraceStore: @unchecked Sendable {
    private let lock = NSLock()
    private var recent: [String] = []
    private let recentLimit = 256
    private let fileURL: URL
    private let rotatedURL: URL
    private let fileLimit = 2 * 1024 * 1024
    private let queue = DispatchQueue(label: "dev.berth.app.trace", qos: .utility)
    /// Only touched on `queue`.
    private var fileBytes = 0
    /// Only touched under `lock`.
    private let stamp: DateFormatter

    init(fileURL: URL) {
        self.fileURL = fileURL
        rotatedURL = fileURL.appendingPathExtension("1")
        stamp = DateFormatter()
        stamp.dateFormat = "HH:mm:ss.SSS"
    }

    func record(_ category: String, _ message: String) {
        lock.lock()
        let line = "\(stamp.string(from: Date())) \(category): \(message)"
        recent.append(line)
        if recent.count > recentLimit { recent.removeFirst(recent.count - recentLimit) }
        lock.unlock()
        queue.async { [self] in append(line) }
    }

    func snapshot(_ count: Int) -> [String] {
        lock.lock()
        defer { lock.unlock() }
        return Array(recent.suffix(count))
    }

    private func append(_ line: String) {
        let manager = FileManager.default
        if fileBytes == 0 {
            try? manager.createDirectory(
                at: fileURL.deletingLastPathComponent(), withIntermediateDirectories: true)
            if let size = try? manager.attributesOfItem(atPath: fileURL.path)[.size] as? Int {
                fileBytes = size
            }
        }
        if fileBytes > fileLimit {
            try? manager.removeItem(at: rotatedURL)
            try? manager.moveItem(at: fileURL, to: rotatedURL)
            fileBytes = 0
        }
        let data = Data((line + "\n").utf8)
        guard let handle = try? FileHandle(forWritingTo: fileURL) else {
            try? data.write(to: fileURL)
            fileBytes = data.count
            return
        }
        defer { try? handle.close() }
        try? handle.seekToEnd()
        try? handle.write(contentsOf: data)
        fileBytes += data.count
    }
}

/// Detects a wedged main thread. One ping per second is queued on the main
/// thread; a ping that stays unserviced past the threshold means the runloop is
/// blocked, and the breadcrumbs then name whatever was in flight. A lock shows
/// up here as a stall with a trail, not as a frozen window with no evidence.
final class MainThreadWatchdog: @unchecked Sendable {
    private let queue = DispatchQueue(label: "dev.berth.app.watchdog", qos: .utility)
    private let lock = NSLock()
    private var pingSentAt: Date?
    private var hasReported = false
    private var timer: DispatchSourceTimer?
    private let threshold: TimeInterval
    private let trace: TraceStore

    init(trace: TraceStore, threshold: TimeInterval = 2) {
        self.trace = trace
        self.threshold = threshold
    }

    func start() {
        queue.async { [self] in
            let timer = DispatchSource.makeTimerSource(queue: queue)
            timer.schedule(deadline: .now() + 1, repeating: 1, leeway: .milliseconds(200))
            timer.setEventHandler { [weak self] in self?.tick() }
            timer.resume()
            self.timer = timer
        }
    }

    private func tick() {
        lock.lock()
        let pending = pingSentAt
        lock.unlock()

        if let pending {
            let waited = Date().timeIntervalSince(pending)
            if waited > threshold { reportStall(waited) }
            return
        }

        let sentAt = Date()
        lock.lock()
        pingSentAt = sentAt
        lock.unlock()
        DispatchQueue.main.async { [self] in
            let latency = Date().timeIntervalSince(sentAt)
            lock.lock()
            pingSentAt = nil
            let wasReported = hasReported
            hasReported = false
            lock.unlock()
            if latency > threshold || wasReported {
                Diagnostics.trace(
                    "watchdog", "main thread responsive again after \(Self.seconds(latency))s")
            }
        }
    }

    private func reportStall(_ waited: TimeInterval) {
        lock.lock()
        let alreadyReported = hasReported
        hasReported = true
        lock.unlock()
        guard !alreadyReported else { return }

        let trail = trace.snapshot(40).joined(separator: "\n")
        Diagnostics.trace("watchdog", "STALL main thread blocked for \(Self.seconds(waited))s")
        BerthLog.app.fault("main thread blocked \(Self.seconds(waited), privacy: .public)s")
        Diagnostics.recordProblem("""
        \(Date()) STALL main thread blocked for \(Self.seconds(waited))s
        --- breadcrumbs ---
        \(trail)
        """)
    }

    private static func seconds(_ value: TimeInterval) -> String {
        String(format: "%.1f", value)
    }
}

enum Diagnostics {
    static var directory: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/Berth")
    }

    static var crashLogURL: URL { directory.appendingPathComponent("crash.log") }
    static var traceLogURL: URL { directory.appendingPathComponent("berth.log") }

    private static let store = TraceStore(fileURL: traceLogURL)
    private static let watchdog = MainThreadWatchdog(trace: store)

    static func install() {
        let info = ProcessInfo.processInfo
        BerthLog.app.info(
            "launch pid=\(info.processIdentifier, privacy: .public) os=\(info.operatingSystemVersionString, privacy: .public)")
        NSSetUncaughtExceptionHandler(berthUncaughtExceptionHandler)
        watchdog.start()
        trace("app", "launched pid=\(info.processIdentifier) log=\(traceLogURL.path)")
    }

    /// Breadcrumb: cheap, thread-safe, and written to `~/Library/Logs/Berth/berth.log`.
    static func trace(_ category: String, _ message: String) {
        store.record(category, message)
    }

    static func breadcrumbs(_ count: Int = 40) -> [String] {
        store.snapshot(count)
    }

    static func millis(since start: Date) -> Int {
        Int(Date().timeIntervalSince(start) * 1000)
    }

    /// Problems worth keeping past the log rotation: uncaught exceptions and
    /// main-thread stalls.
    static func recordProblem(_ message: String) {
        do {
            let url = crashLogURL
            try FileManager.default.createDirectory(
                at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            let data = Data((message + "\n\n").utf8)
            if FileManager.default.fileExists(atPath: url.path) {
                let handle = try FileHandle(forWritingTo: url)
                defer { try? handle.close() }
                try handle.seekToEnd()
                try handle.write(contentsOf: data)
            } else {
                try data.write(to: url, options: .atomic)
            }
        } catch {
            // Diagnostics must never throw or recurse.
        }
    }
}
