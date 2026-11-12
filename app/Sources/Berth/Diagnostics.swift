// Process-wide diagnostics: unified logging categories plus a small crash log
// for uncaught exceptions. Logs are intentionally structured so Console.app
// and `log stream --predicate 'subsystem == "dev.berth.app"'` can filter by
// category when chasing a race or a restore failure.

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
    let message = """
    \(Date()) \(exception.name.rawValue)
    \(exception.reason ?? "no reason")
    \(stack)
    """
    Diagnostics.appendCrash(message)
    BerthLog.app.fault(
        "uncaught exception \(exception.name.rawValue, privacy: .public): \(exception.reason ?? "", privacy: .public)")
}

enum Diagnostics {
    static var crashLogURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/Berth/crash.log")
    }

    static func install() {
        let info = ProcessInfo.processInfo
        BerthLog.app.info(
            "launch pid=\(info.processIdentifier, privacy: .public) os=\(info.operatingSystemVersionString, privacy: .public)")
        NSSetUncaughtExceptionHandler(berthUncaughtExceptionHandler)
    }

    fileprivate static func appendCrash(_ message: String) {
        do {
            let url = crashLogURL
            try FileManager.default.createDirectory(
                at: url.deletingLastPathComponent(),
                withIntermediateDirectories: true)
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
            // The exception handler must never throw or recurse.
        }
    }
}
