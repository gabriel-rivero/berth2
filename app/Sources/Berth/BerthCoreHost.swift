// The Swift side of the UniFFI bridge: implements `HostOps`, so every core
// decision that needs AppKit or Ghostty lands here. `berth-core` stays
// platform-free; this file is the portability seam.
//
// Threading: HostOps methods are called on whichever thread dispatched the
// request (usually a socket client queue). All AppKit/Ghostty work hops to the
// main thread synchronously — the core expects spawn/send/close results inline.

import AppKit
import BerthCore
import Foundation
import GhosttyKit
import UserNotifications

final class BerthCoreHost: HostOps, @unchecked Sendable {
    static let shared = BerthCoreHost()

    /// Main-thread confined.
    private var surfaces: [String: GhosttySurfaceView] = [:]
    private var windows: [String: MainWindow] = [:]
    private var didAskForNotificationPermission = false

    /// Called on the main thread when Ghostty reports a child exited.
    var onSurfaceExited: ((String) -> Void)?

    private init() {}

    // MARK: - HostOps

    func spawn(req: FfiSpawnRequest) throws -> FfiSpawned {
        try onMain {
            let env = Self.environment(from: req.env)
            guard let id = env["BERTH_SURFACE_ID"] else {
                throw FfiError.Internal(message: "spawn request is missing BERTH_SURFACE_ID")
            }

            let view = GhosttySurfaceView(frame: NSRect(x: 0, y: 0, width: 900, height: 600))
            guard view.createSurface(
                command: req.command,
                workingDirectory: req.cwd,
                env: req.env
            ) else {
                throw FfiError.Internal(message: "ghostty_surface_new failed")
            }

            surfaces[id] = view
            let window = MainWindow(surface: view)
            windows[id] = window
            window.makeKeyAndOrderFront(nil)

            let grid = view.gridSize()
            let pid = view.surfaceHandle.map { UInt32(truncatingIfNeeded: ghostty_surface_foreground_pid($0)) } ?? 0
            return FfiSpawned(pid: pid, tty: view.ttyName(), cols: grid.cols, rows: grid.rows)
        }
    }

    func sendText(surfaceId: String, text: String) throws {
        try onMain {
            try surface(surfaceId).sendText(text)
        }
    }

    func sendKey(surfaceId: String, key: String, mods: [String]) throws {
        try onMain {
            let view = try surface(surfaceId)
            let flags = Self.mods(mods)
            let (keycode, text) = Self.key(for: key)
            view.sendKey(keycode: keycode, mods: flags, text: text)
        }
    }

    func readText(surfaceId: String, scrollback: Bool, maxBytes: UInt32) throws -> String {
        try onMain {
            let text = try surface(surfaceId).readText(scrollback: scrollback)
            guard maxBytes > 0, text.utf8.count > Int(maxBytes) else { return text }
            return String(text.prefix(Int(maxBytes)))
        }
    }

    func close(surfaceId: String) throws {
        try onMain {
            guard let window = windows.removeValue(forKey: surfaceId) else {
                throw FfiError.Internal(message: "unknown surface \(surfaceId)")
            }
            surfaces.removeValue(forKey: surfaceId)
            window.close()
        }
    }

    func focus(surfaceId: String) throws {
        try onMain {
            guard let window = windows[surfaceId], let view = surfaces[surfaceId] else {
                throw FfiError.Internal(message: "unknown surface \(surfaceId)")
            }
            window.makeKeyAndOrderFront(nil)
            window.makeFirstResponder(view)
            NSApp.activate(ignoringOtherApps: true)
        }
    }

    func resize(surfaceId: String, cols: UInt16, rows: UInt16) throws {
        try onMain {
            try surface(surfaceId).resizeGrid(cols: cols, rows: rows)
        }
    }

    /// Foreground process facts for passive agent detection. `ps` is cheap at
    /// this frequency (once per shell spawn, not per keystroke).
    func pidInfo(surfaceId: String) throws -> FfiPidInfo? {
        try onMain {
            guard let view = surfaces[surfaceId], let handle = view.surfaceHandle else { return nil }
            let pid = UInt32(truncatingIfNeeded: ghostty_surface_foreground_pid(handle))
            guard pid > 0 else { return nil }

            let proc = Process()
            proc.executableURL = URL(fileURLWithPath: "/bin/ps")
            proc.arguments = ["-o", "comm=", "-o", "args=", "-p", "\(pid)"]
            let pipe = Pipe()
            proc.standardOutput = pipe
            proc.standardError = FileHandle.nullDevice
            try? proc.run()
            proc.waitUntilExit()

            let out = String(decoding: pipe.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
            let line = out.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !line.isEmpty else { return nil }
            let pieces = line.split(separator: " ", maxSplits: 1, omittingEmptySubsequences: true)
            let comm = pieces.first.map(String.init)
            let argv = pieces.count > 1
                ? pieces[1].split(separator: " ").map(String.init)
                : []
            return FfiPidInfo(pid: pid, comm: comm, argv: argv)
        }
    }

    func notify(title: String, body: String, workspaceId: String?) {
        onMain { [self] in
            requestNotificationPermissionIfNeeded()
            let content = UNMutableNotificationContent()
            content.title = title
            content.body = body
            content.sound = nil
            let request = UNNotificationRequest(
                identifier: UUID().uuidString,
                content: content,
                trigger: nil)
            UNUserNotificationCenter.current().add(request)
        }
    }

    // MARK: - Window bookkeeping

    /// Drop a surface whose child exited, so the core can be told.
    func surfaceDidExit(_ id: String) {
        onMain {
            surfaces.removeValue(forKey: id)
            windows.removeValue(forKey: id)
        }
        onSurfaceExited?(id)
    }

    // MARK: - Helpers

    private func surface(_ id: String) throws -> GhosttySurfaceView {
        guard let view = surfaces[id] else {
            throw FfiError.Internal(message: "unknown surface \(id)")
        }
        return view
    }

    /// Run `body` on the main thread. The closure is `@MainActor` so AppKit
    /// calls inside it type-check; `sync` blocks the caller (a socket queue),
    /// which is what the core's synchronous `HostOps` contract expects.
    ///
    /// The result travels in a box because `assumeIsolated` only hands back
    /// `Sendable` values, and UniFFI's generated records are not marked so.
    private func onMain<T>(_ body: @MainActor () throws -> T) throws -> T {
        let box = Box<T>()
        if Thread.isMainThread {
            MainActor.assumeIsolated { box.result = Result { try body() } }
        } else {
            DispatchQueue.main.sync {
                MainActor.assumeIsolated { box.result = Result { try body() } }
            }
        }
        guard let result = box.result else {
            throw FfiError.Internal(message: "main-actor hop produced no result")
        }
        return try result.get()
    }

    private final class Box<T>: @unchecked Sendable {
        var result: Result<T, Error>?
    }

    private func requestNotificationPermissionIfNeeded() {
        guard !didAskForNotificationPermission else { return }
        didAskForNotificationPermission = true
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .badge]) { _, _ in }
    }

    private static func environment(from pairs: [String]) -> [String: String] {
        var out: [String: String] = [:]
        for pair in pairs {
            guard let sep = pair.firstIndex(of: "=") else { continue }
            out[String(pair[pair.startIndex..<sep])] = String(pair[pair.index(after: sep)...])
        }
        return out
    }

    private static func mods(_ names: [String]) -> ghostty_input_mods_e {
        var value: UInt32 = 0
        for name in names.map({ $0.lowercased() }) {
            switch name {
            case "ctrl", "control": value |= GHOSTTY_MODS_CTRL.rawValue
            case "alt", "opt", "option": value |= GHOSTTY_MODS_ALT.rawValue
            case "shift": value |= GHOSTTY_MODS_SHIFT.rawValue
            case "cmd", "command", "super": value |= GHOSTTY_MODS_SUPER.rawValue
            default: break
            }
        }
        return ghostty_input_mods_e(value)
    }

    /// Native (ANSI) keycode + text for a synthetic key name.
    private static func key(for name: String) -> (UInt32, String?) {
        let lower = name.lowercased()
        if let known = namedKeys[lower] {
            return known
        }
        if lower.count == 1, let scalar = lower.unicodeScalars.first {
            if let code = letterKeycodes[scalar] {
                return (code, lower)
            }
        }
        return (0, name)
    }

    private static let namedKeys: [String: (UInt32, String?)] = [
        "enter": (36, nil), "return": (36, nil),
        "tab": (48, nil), "escape": (53, nil), "esc": (53, nil),
        "space": (49, " "), "backspace": (51, nil), "delete": (117, nil),
        "up": (126, nil), "down": (125, nil), "left": (123, nil), "right": (124, nil),
        "home": (115, nil), "end": (119, nil),
        "pageup": (116, nil), "pagedown": (121, nil), "pgup": (116, nil), "pgdn": (121, nil),
        "f1": (122, nil), "f2": (120, nil), "f3": (99, nil), "f4": (118, nil),
        "f5": (96, nil), "f6": (97, nil), "f7": (98, nil), "f8": (100, nil),
        "f9": (101, nil), "f10": (109, nil), "f11": (103, nil), "f12": (111, nil),
    ]

    /// ANSI keycodes for printable ASCII, so Ctrl chords reach the PTY as the
    /// control byte Ghostty computes from the physical key.
    private static let letterKeycodes: [Unicode.Scalar: UInt32] = [
        "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9,
        "b": 11, "q": 12, "w": 13, "e": 14, "r": 15, "y": 16, "t": 17,
        "1": 18, "2": 19, "3": 20, "4": 21, "6": 22, "5": 23, "=": 24,
        "9": 25, "7": 26, "-": 27, "8": 28, "0": 29, "]": 30, "o": 31, "u": 32,
        "[": 33, "i": 34, "p": 35, "l": 37, "j": 38, "'": 39, "k": 40, ";": 41,
        "\\": 42, ",": 43, "/": 44, "n": 45, "m": 46, ".": 47, "`": 50,
    ]
}
