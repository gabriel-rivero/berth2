// App lifecycle. The shell no longer creates windows directly: the core decides
// what exists (workspaces, surfaces), and its `HostOps` implementation opens
// the windows. This is the same path `berth new` takes over the socket.

import AppKit
import BerthCore

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var engine: Engine?
    private var socket: SocketServer?
    private var handler: CoreHandler?
    private var sweepTimer: Timer?

    func applicationDidFinishLaunching(_ notification: Notification) {
        guard GhosttyHost.shared.start() else {
            let alert = NSAlert()
            alert.messageText = "Berth could not start Ghostty"
            alert.informativeText = """
            ghostty_init() or ghostty_app_new() failed. Check that \
            vendor/ghostty/macos/GhosttyKit.xcframework is built and that \
            GHOSTTY_RESOURCES_DIR points at the bundled resources.
            """
            alert.runModal()
            NSApp.terminate(nil)
            return
        }

        MainMenu.install()

        let engine = Engine(host: BerthCoreHost.shared)
        self.engine = engine

        let handler = CoreHandler(engine: engine)
        self.handler = handler
        let socket = SocketServer(path: Self.socketPath, handler: handler)
        handler.server = socket
        do {
            try socket.start()
        } catch {
            NSLog("berth: socket unavailable at \(Self.socketPath): \(error)")
        }
        self.socket = socket

        BerthCoreHost.shared.onSurfaceExited = { id in
            // The core learns about exits through `surface.close`; a crash inside
            // the child is reported as an event once Ghostty's action callback
            // is wired (docs/05-build.md, gap 1).
            NSLog("berth: surface \(id) child exited")
        }

        restoreOrBootstrap(engine)
        scheduleStaleSweep(engine)
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func applicationWillTerminate(_ notification: Notification) {
        saveSession()
        sweepTimer?.invalidate()
        socket?.stop()
        socket = nil
        GhosttyHost.shared.stop()
    }

    // MARK: - Session

    private func restoreOrBootstrap(_ engine: Engine) {
        if let data = try? Data(contentsOf: Self.sessionURL),
           let text = String(data: data, encoding: .utf8),
           !text.isEmpty {
            do {
                let restored = try engine.restore(snapshot: text)
                if restored > 0 {
                    NSLog("berth: restored \(restored) surface(s)")
                    return
                }
            } catch {
                NSLog("berth: restore failed, starting fresh: \(error)")
            }
        }

        let response = engine.dispatch(
            request: #"{"id":"boot","method":"workspace.create","params":{"name":"main"}}"#)
        NSLog("berth: bootstrap → \(response)")
    }

    private func saveSession() {
        guard let engine else { return }
        do {
            try FileManager.default.createDirectory(
                at: Self.supportDirectory,
                withIntermediateDirectories: true)
            try Data(engine.snapshot().utf8).write(to: Self.sessionURL, options: .atomic)
        } catch {
            NSLog("berth: could not save session: \(error)")
        }
    }

    /// Agent staleness: an agent that stops reporting for three minutes is
    /// treated as blocked, which is how a crashed CLI surfaces in the sidebar.
    private func scheduleStaleSweep(_ engine: Engine) {
        let timer = Timer(timeInterval: 30, repeats: true) { [weak self] _ in
            let changed = engine.sweepStale(ttlMs: 180_000)
            if !changed.isEmpty {
                NSLog("berth: \(changed.count) agent(s) went stale")
            }
            self?.handler?.flushEvents()
        }
        RunLoop.main.add(timer, forMode: .common)
        sweepTimer = timer
    }

    // MARK: - Paths

    private static var supportDirectory: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/Berth", isDirectory: true)
    }

    private static var socketPath: String {
        if let override = ProcessInfo.processInfo.environment["BERTH_SOCKET_PATH"], !override.isEmpty {
            return override
        }
        return supportDirectory.appendingPathComponent("berth.sock").path
    }

    private static var sessionURL: URL {
        supportDirectory.appendingPathComponent("session.json")
    }
}
