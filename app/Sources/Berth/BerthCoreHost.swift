// The Swift side of the UniFFI bridge: implements `HostOps`, so every core
// decision that needs AppKit or Ghostty lands here. `berth-core` stays
// platform-free; this file is the portability seam.
//
// Threading: HostOps methods are called on whichever thread dispatched the
// request (usually a socket client queue). All AppKit/Ghostty work hops to the
// main thread synchronously — the core expects spawn/send/close results inline.
//
// Windows are not created here: `spawn` only makes the surface view, and
// `reconcileAll()` then rebuilds every window from the core's pane tree. That
// keeps one code path for boot, `berth new`, splits and session restore.

import AppKit
import BerthCore
import Foundation
import GhosttyKit
import UserNotifications

final class BerthCoreHost: HostOps, @unchecked Sendable {
    static let shared = BerthCoreHost()

    /// Main-thread confined.
    private var surfaces: [String: GhosttySurfaceView] = [:]
    private var windows: [String: WorkspaceWindow] = [:]
    private var surfaceWorkspace: [String: String] = [:]
    private var didAskForNotificationPermission = false
    private var lastLayoutJSON: [String: String] = [:]

    /// Set by AppDelegate once the engine exists; callbacks (titles, cwd, bells)
    /// report through it.
    weak var engine: Engine?
    /// Called after a report so queued events reach socket subscribers.
    var onEvents: (() -> Void)?

    private init() {}

    // MARK: - HostOps

    func spawn(req: FfiSpawnRequest) throws -> FfiSpawned {
        try onMain {
            let env = Self.environment(from: req.env)
            guard let id = env["BERTH_SURFACE_ID"] else {
                throw FfiError.Internal(message: "spawn request is missing BERTH_SURFACE_ID")
            }
            if let existing = surfaces[id] {
                let grid = existing.gridSize()
                return FfiSpawned(
                    pid: existing.foregroundPID(),
                    tty: existing.ttyName(),
                    cols: grid.cols,
                    rows: grid.rows)
            }

            let view = GhosttySurfaceView(frame: NSRect(x: 0, y: 0, width: 900, height: 600))
            view.berthSurfaceID = id
            guard view.createSurface(
                command: req.command,
                workingDirectory: req.cwd,
                env: req.env
            ) else {
                throw FfiError.Internal(message: "ghostty_surface_new failed")
            }

            surfaces[id] = view
            if let workspace = env["BERTH_WORKSPACE_ID"] {
                surfaceWorkspace[id] = workspace
            }

            let grid = view.gridSize()
            return FfiSpawned(
                pid: view.foregroundPID(),
                tty: view.ttyName(),
                cols: grid.cols,
                rows: grid.rows)
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

    /// Detach a surface the core no longer wants. The window may still exist
    /// with other panes; `reconcileAll` (right after this call) removes it from
    /// the tree, and the last pane closing takes the window with it.
    func close(surfaceId: String) throws {
        try onMain {
            guard let view = surfaces.removeValue(forKey: surfaceId) else { return }
            surfaceWorkspace.removeValue(forKey: surfaceId)
            view.removeFromSuperview()
        }
    }

    func focus(surfaceId: String) throws {
        try onMain {
            guard let view = surfaces[surfaceId] else {
                throw FfiError.Internal(message: "unknown surface \(surfaceId)")
            }
            let workspaceID = surfaceWorkspace[surfaceId]
            guard let window = workspaceID.flatMap({ windows[$0] }) else {
                // During restore the window is built by reconcileAll.
                return
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
        // Best effort: a missed banner must never fail the core call.
        try? onMain { [self] in
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

    // MARK: - Layout

    /// Rebuild every window from the core's tree. Cheap when nothing changed:
    /// the layout JSON is compared first and identical workspaces are skipped.
    func reconcileAll() {
        try? onMain { [self] in
            guard let engine else { return }
            let listed = engine.dispatch(
                request: #"{"id":"sync","method":"workspace.list","params":{}}"#)
            guard let result = Self.result(listed),
                  let workspaces = result["workspaces"] as? [[String: Any]]
            else { return }

            var live = Set<String>()
            for workspace in workspaces {
                guard let id = workspace["id"] as? String else { continue }
                live.insert(id)
                reconcile(id, name: workspace["name"] as? String ?? "Berth")
            }
            for (id, window) in windows where !live.contains(id) {
                windows.removeValue(forKey: id)
                lastLayoutJSON.removeValue(forKey: id)
                window.close()
            }
        }
    }

    @MainActor
    private func reconcile(_ workspaceID: String, name: String) {
        guard let engine else { return }
        let request = #"{"id":"layout","method":"workspace.layout","params":{"workspace_id":"\#(workspaceID)"}}"#
        let response = engine.dispatch(request: request)
        guard let result = Self.result(response),
              let data = try? JSONSerialization.data(withJSONObject: result),
              let json = String(data: data, encoding: .utf8),
              let layout = WorkspaceLayout.parse(json)
        else { return }

        if lastLayoutJSON[workspaceID] == json, let window = windows[workspaceID] {
            // Same tree, but a surface may have run out of processes or the
            // key window may have changed; focus is cheap to reapply.
            if window.isKeyWindow, let active = layout.activeSurface, let view = surfaces[active],
               window.firstResponder !== view {
                window.makeFirstResponder(view)
            }
            return
        }
        lastLayoutJSON[workspaceID] = json

        let isNew = windows[workspaceID] == nil
        let window = windows[workspaceID] ?? WorkspaceWindow(workspaceID: workspaceID, name: name)
        windows[workspaceID] = window
        window.apply(layout: layout, views: surfaces)
        if isNew {
            window.makeKeyAndOrderFront(nil)
            if let active = layout.activeSurface ?? layout.root.firstLeafSurface(),
               let view = surfaces[active] {
                window.makeFirstResponder(view)
            }
        }
    }

    /// Ghostty asked to close a surface (child exited and the config says so).
    /// Goes through the core so pane/agent bookkeeping stays in one place.
    func closeSurfaceFromTerminal(_ surfaceID: String) {
        try? onMain { [self] in
            guard let engine else { return }
            _ = engine.dispatch(
                request: #"{"id":"exit","method":"surface.close","params":{"surface_id":"\#(surfaceID)"}}"#)
            reconcileAll()
            onEvents?()
        }
    }

    /// The surface view that currently has focus, if any.
    @MainActor
    var focusedSurfaceID: String? {
        for window in windows.values {
            if let view = window.firstResponder as? GhosttySurfaceView {
                return view.berthSurfaceID
            }
        }
        return nil
    }

    // MARK: - Ghostty actions

    /// Called from Ghostty's action callback (main thread, during tick).
    @MainActor
    func handleAction(target: ghostty_target_s, action: ghostty_action_s) -> Bool {
        guard target.tag == GHOSTTY_TARGET_SURFACE,
              let handle = target.target.surface,
              let userdata = ghostty_surface_userdata(handle)
        else { return false }
        let view = Unmanaged<GhosttySurfaceView>.fromOpaque(userdata).takeUnretainedValue()
        guard let id = view.berthSurfaceID else { return false }

        switch action.tag {
        case GHOSTTY_ACTION_SET_TITLE:
            guard let title = action.action.set_title.title else { return false }
            let text = String(cString: title)
            view.window?.title = text
            report(id, ["title": text])
            return true

        case GHOSTTY_ACTION_PWD:
            guard let pwd = action.action.pwd.pwd else { return false }
            report(id, ["cwd": String(cString: pwd)])
            return true

        case GHOSTTY_ACTION_PROGRESS_REPORT:
            let report = action.action.progress_report
            switch report.state {
            case GHOSTTY_PROGRESS_STATE_REMOVE:
                self.report(id, ["clear_progress": true])
            case GHOSTTY_PROGRESS_STATE_SET:
                let percent = max(0, min(100, Int(report.progress)))
                self.report(id, ["progress": Double(percent) / 100.0])
            default:
                break
            }
            return true

        case GHOSTTY_ACTION_RING_BELL:
            report(id, ["bell": true])
            return true

        case GHOSTTY_ACTION_DESKTOP_NOTIFICATION:
            guard let title = action.action.desktop_notification.title,
                  let body = action.action.desktop_notification.body
            else { return false }
            createNotification(
                title: String(cString: title),
                body: String(cString: body),
                workspaceID: surfaceWorkspace[id])
            return true

        case GHOSTTY_ACTION_OPEN_URL:
            let url = action.action.open_url
            guard let ptr = url.url else { return false }
            let string = String(decoding: UnsafeRawBufferPointer(start: ptr, count: Int(url.len)),
                                as: UTF8.self)
            if let parsed = URL(string: string) {
                NSWorkspace.shared.open(parsed)
                return true
            }
            return false

        case GHOSTTY_ACTION_SHOW_CHILD_EXITED:
            report(id, ["exited": true])
            return false

        default:
            return false
        }
    }

    /// Tell the core about something the terminal learned from escape
    /// sequences. Deferred to the next runloop turn: dispatching re-enters
    /// Ghostty for some actions (focus, spawn) and this callback runs inside
    /// its tick.
    private func report(_ surfaceID: String, _ fields: [String: Any]) {
        var params = fields
        params["surface_id"] = surfaceID
        guard let data = try? JSONSerialization.data(withJSONObject: params),
              let json = String(data: data, encoding: .utf8)
        else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self, let engine = self.engine else { return }
            _ = engine.dispatch(
                request: #"{"id":"osc","method":"surface.update","params":\#(json)}"#)
            self.onEvents?()
        }
    }

    private func createNotification(title: String, body: String, workspaceID: String?) {
        var params: [String: Any] = ["level": "info", "title": title, "body": body]
        if let workspaceID { params["workspace_id"] = workspaceID }
        guard let data = try? JSONSerialization.data(withJSONObject: params),
              let json = String(data: data, encoding: .utf8)
        else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self, let engine = self.engine else { return }
            _ = engine.dispatch(
                request: #"{"id":"osc","method":"notification.create","params":\#(json)}"#)
            self.onEvents?()
        }
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

    /// `ok:true` result object of a protocol response.
    private static func result(_ response: String) -> [String: Any]? {
        guard let data = response.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              object["ok"] as? Bool == true
        else { return nil }
        return object["result"] as? [String: Any]
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
