// App lifecycle. The shell does not create surfaces directly: the core decides
// what exists (workspaces, panes, surfaces), and its `HostOps` implementation
// opens the windows. `reconcileAll()` after every mutation is what makes boot,
// `berth split`, menu commands and session restore share one code path.

import AppKit
import BerthCore

@MainActor
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
        BerthCoreHost.shared.engine = engine

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
        BerthCoreHost.shared.onEvents = { [weak handler] in
            handler?.flushEvents()
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

    // MARK: - Terminal commands

    @objc func splitRight(_ sender: Any?) {
        split(direction: "right")
    }

    @objc func splitDown(_ sender: Any?) {
        split(direction: "down")
    }

    @objc func closeFocusedSurface(_ sender: Any?) {
        guard let id = BerthCoreHost.shared.focusedSurfaceID else { return }
        send("surface.close", ["surface_id": id])
    }

    @objc func nextSurface(_ sender: Any?) {
        cycleSurface(1)
    }

    @objc func previousSurface(_ sender: Any?) {
        cycleSurface(-1)
    }

    @objc func newWorkspace(_ sender: Any?) {
        send("workspace.create", ["name": "workspace"])
    }

    private func split(direction: String) {
        guard let id = BerthCoreHost.shared.focusedSurfaceID else { return }
        send("surface.split", ["surface_id": id, "direction": direction, "focus": true])
    }

    /// Linear focus movement in tree order. Geometric navigation (left/right
    /// relative to the cursor) needs pane rectangles and lands with the sidebar.
    private func cycleSurface(_ step: Int) {
        guard let engine else { return }
        let response = engine.dispatch(
            request: #"{"id":"cycle","method":"surface.list","params":{}}"#)
        guard let data = response.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let result = object["result"] as? [String: Any],
              let surfaces = result["surfaces"] as? [[String: Any]]
        else { return }

        let current = BerthCoreHost.shared.focusedSurfaceID
        let currentWorkspace = surfaces
            .first { $0["id"] as? String == current }?["workspace_id"] as? String
        let sameWorkspace = surfaces.filter {
            ($0["workspace_id"] as? String) == currentWorkspace
        }
        guard sameWorkspace.count > 1 else { return }

        let index = sameWorkspace.firstIndex { $0["id"] as? String == current } ?? 0
        let next = sameWorkspace[(index + step + sameWorkspace.count) % sameWorkspace.count]
        guard let nextID = next["id"] as? String else { return }
        send("surface.focus", ["surface_id": nextID])
    }

    /// Fire a protocol request, then rebuild the window tree.
    private func send(_ method: String, _ params: [String: Any]) {
        guard let engine,
              let data = try? JSONSerialization.data(withJSONObject: params),
              let json = String(data: data, encoding: .utf8)
        else { return }
        _ = engine.dispatch(request: #"{"id":"ui","method":"\#(method)","params":\#(json)}"#)
        handler?.flushEvents()
        BerthCoreHost.shared.reconcileAll()
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
                    BerthCoreHost.shared.reconcileAll()
                    return
                }
            } catch {
                NSLog("berth: restore failed, starting fresh: \(error)")
            }
        }

        let response = engine.dispatch(
            request: #"{"id":"boot","method":"workspace.create","params":{"name":"main"}}"#)
        NSLog("berth: bootstrap → \(response)")
        BerthCoreHost.shared.reconcileAll()
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
        // Target/selector rather than the block API: the block is @Sendable and
        // the engine and handler are not, and a MainActor timer is the honest
        // model anyway.
        let timer = Timer(
            timeInterval: 30,
            target: self,
            selector: #selector(sweepAgents),
            userInfo: nil,
            repeats: true)
        RunLoop.main.add(timer, forMode: .common)
        sweepTimer = timer
    }

    @objc private func sweepAgents() {
        guard let engine else { return }
        let changed = engine.sweepStale(ttlMs: 180_000)
        if !changed.isEmpty {
            NSLog("berth: \(changed.count) agent(s) went stale")
        }
        handler?.flushEvents()
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
