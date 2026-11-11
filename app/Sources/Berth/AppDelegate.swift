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
    private var saveTimer: Timer?
    /// A second copy must not bootstrap a second in-memory session on top of
    /// the socket owner's state, and must not overwrite session.json on exit.
    private var isDuplicateInstance = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        guard GhosttyHost.shared.start(fontSize: AppConfig.shared.fontSize) else {
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

        var mode = AppConfig.shared.socketMode
        var token: String?
        if mode != .off {
            token = try? SocketAuth.loadOrCreateToken(at: Self.tokenURL)
            if mode == .local, token == nil {
                NSLog("berth: could not create a socket token; disabling the socket")
                mode = .off
            }
        }
        BerthCoreHost.shared.configureTransport(token: token, socketPath: Self.socketPath)

        let handler = CoreHandler(engine: engine)
        self.handler = handler
        if mode != .off {
            let socket = SocketServer(
                path: Self.socketPath,
                handler: handler,
                mode: mode,
                token: token)
            handler.server = socket
            do {
                try socket.start()
                self.socket = socket
            } catch {
                // The socket path is the single-instance lock. A second copy
                // that fails to bind must exit instead of creating a divergent
                // window and state.
                NSLog("berth: socket unavailable at \(Self.socketPath): \(error)")
                NSLog("berth: another Berth instance owns the socket; exiting")
                isDuplicateInstance = true
                NSApp.terminate(nil)
                return
            }
        }
        BerthCoreHost.shared.onEvents = { [weak handler, weak self] in
            handler?.flushEvents()
            Task { @MainActor [weak self] in self?.scheduleSessionSave() }
        }

        restoreOrBootstrap(engine)
        scheduleStaleSweep(engine)
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func applicationWillTerminate(_ notification: Notification) {
        if isDuplicateInstance {
            GhosttyHost.shared.stop()
            return
        }
        saveTimer?.invalidate()
        saveSession(truncateJournal: true)
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

    @objc func newTab(_ sender: Any?) {
        guard let id = BerthCoreHost.shared.focusedSurfaceID,
              let pane = paneID(forSurface: id)
        else { return }
        send("surface.create", ["pane_id": pane, "focus": true])
    }

    @objc func nextTab(_ sender: Any?) {
        cycleTab(1)
    }

    @objc func previousTab(_ sender: Any?) {
        cycleTab(-1)
    }

    @objc func nextPane(_ sender: Any?) {
        cyclePane(1)
    }

    @objc func previousPane(_ sender: Any?) {
        cyclePane(-1)
    }

    @objc func newWorkspace(_ sender: Any?) {
        send("workspace.create", ["name": "workspace"])
    }

    @objc func selectWorkspace(_ sender: Any?) {
        guard let item = sender as? NSMenuItem,
              let id = workspaceID(at: item.tag)
        else { return }
        send("workspace.select", ["workspace_id": id])
    }

    @objc func showPalette(_ sender: Any?) {
        CommandPalette.present(commands: paletteCommands())
    }

    @objc func showSettings(_ sender: Any?) {
        SettingsWindow.present()
    }

    @objc func toggleSidebar(_ sender: Any?) {
        BerthCoreHost.shared.toggleSidebar()
    }

    private func split(direction: String) {
        guard let id = BerthCoreHost.shared.focusedSurfaceID else { return }
        send("surface.split", ["surface_id": id, "direction": direction, "focus": true])
    }

    private func cycleTab(_ step: Int) {
        guard let id = BerthCoreHost.shared.focusedSurfaceID else { return }
        send("surface.cycle", ["surface_id": id, "direction": step > 0 ? "next" : "prev"])
    }

    /// Pane focus moves in tree order. Geometric navigation by rectangle is a
    /// later refinement; tree order matches what the sidebar shows.
    private func cyclePane(_ step: Int) {
        guard let engine,
              let workspaceID = selectedWorkspaceID(),
              let layout = loadLayout(workspaceID, engine: engine),
              layout.panes.count > 1
        else { return }

        let current = BerthCoreHost.shared.focusedSurfaceID
        let index = layout.panes.firstIndex { $0.surfaceIDs.contains(current ?? "") } ?? 0
        let count = layout.panes.count
        let target = layout.panes[(index + step + count) % count]
        guard let surface = target.activeSurface ?? target.surfaceIDs.first else { return }
        send("surface.activate", ["surface_id": surface])
    }

    private func paneID(forSurface id: String) -> String? {
        guard let engine, let workspaceID = selectedWorkspaceID() else { return nil }
        return loadLayout(workspaceID, engine: engine)?.paneID(forSurface: id)
    }

    private func selectedWorkspaceID() -> String? {
        guard let engine else { return nil }
        let response = engine.dispatch(
            request: #"{"id":"ui","method":"workspace.list","params":{}}"#)
        guard let data = response.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let result = object["result"] as? [String: Any]
        else { return nil }
        return result["selected_workspace"] as? String
    }

    private func loadLayout(_ workspaceID: String, engine: Engine) -> WorkspaceLayout? {
        let request = #"{"id":"ui","method":"workspace.layout","params":{"workspace_id":"\#(workspaceID)"}}"#
        let response = engine.dispatch(request: request)
        guard let data = response.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let result = object["result"] as? [String: Any],
              let layoutData = try? JSONSerialization.data(withJSONObject: result),
              let json = String(data: layoutData, encoding: .utf8)
        else { return nil }
        return WorkspaceLayout.parse(json)
    }

    private func workspaceID(at index: Int) -> String? {
        guard let engine else { return nil }
        let response = engine.dispatch(
            request: #"{"id":"ui","method":"workspace.list","params":{}}"#)
        guard let data = response.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let result = object["result"] as? [String: Any],
              let workspaces = result["workspaces"] as? [[String: Any]],
              workspaces.indices.contains(index)
        else { return nil }
        return workspaces[index]["id"] as? String
    }

    private func paletteCommands() -> [PaletteCommand] {
        [
            PaletteCommand("New Workspace") { [weak self] in self?.newWorkspace(nil) },
            PaletteCommand("Split Right") { [weak self] in self?.splitRight(nil) },
            PaletteCommand("Split Down") { [weak self] in self?.splitDown(nil) },
            PaletteCommand("New Tab") { [weak self] in self?.newTab(nil) },
            PaletteCommand("Close Tab") { [weak self] in self?.closeFocusedSurface(nil) },
            PaletteCommand("Next Tab") { [weak self] in self?.nextTab(nil) },
            PaletteCommand("Previous Tab") { [weak self] in self?.previousTab(nil) },
            PaletteCommand("Next Pane") { [weak self] in self?.nextPane(nil) },
            PaletteCommand("Previous Pane") { [weak self] in self?.previousPane(nil) },
            PaletteCommand("Toggle Sidebar") { [weak self] in self?.toggleSidebar(nil) },
            PaletteCommand("Settings") { [weak self] in self?.showSettings(nil) },
        ]
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
        scheduleSessionSave()
    }

    // MARK: - Session

    private func restoreOrBootstrap(_ engine: Engine) {
        for snapshot in snapshotCandidates() where !snapshot.isEmpty {
            do {
                let restored = try engine.restore(snapshot: snapshot)
                if restored > 0 {
                    NSLog("berth: restored \(restored) surface(s)")
                    BerthCoreHost.shared.reconcileAll()
                    return
                }
            } catch {
                NSLog("berth: restore failed, trying older snapshot: \(error)")
            }
        }

        let response = engine.dispatch(
            request: #"{"id":"boot","method":"workspace.create","params":{"name":"main"}}"#)
        NSLog("berth: bootstrap → \(response)")
        BerthCoreHost.shared.reconcileAll()
    }

    /// Newest snapshot wins between `session.json` and the crash journal. The
    /// journal stores whole snapshots rather than structural events: replaying
    /// events cannot recreate PTYs, so restoring the last good tree is honest.
    private func snapshotCandidates() -> [String] {
        var candidates: [(Int, String)] = []

        if let data = try? Data(contentsOf: Self.sessionURL),
           let text = String(data: data, encoding: .utf8),
           !text.isEmpty {
            var ms = 0
            if let attrs = try? FileManager.default.attributesOfItem(
                atPath: Self.sessionURL.path),
               let date = attrs[.modificationDate] as? Date {
                ms = Int(date.timeIntervalSince1970 * 1000)
            }
            candidates.append((ms, text))
        }

        if let data = try? Data(contentsOf: Self.journalURL),
           let text = String(data: data, encoding: .utf8) {
            for line in text.split(separator: "\n").reversed() {
                guard let lineData = line.data(using: .utf8),
                      let object = try? JSONSerialization.jsonObject(with: lineData)
                        as? [String: Any],
                      object["op"] as? String == "snapshot",
                      let ts = object["ts_ms"] as? Int,
                      let snapshot = object["data"],
                      let snapshotData = try? JSONSerialization.data(withJSONObject: snapshot),
                      let json = String(data: snapshotData, encoding: .utf8)
                else { continue }
                candidates.append((ts, json))
                break
            }
        }

        return candidates.sorted { $0.0 > $1.0 }.map(\.1)
    }

    private func scheduleSessionSave() {
        saveTimer?.invalidate()
        let timer = Timer(
            timeInterval: 0.5,
            target: self,
            selector: #selector(flushSession),
            userInfo: nil,
            repeats: false)
        RunLoop.main.add(timer, forMode: .common)
        saveTimer = timer
    }

    @objc private func flushSession() {
        saveSession()
    }

    private func saveSession(truncateJournal: Bool = false) {
        guard let engine else { return }
        let snapshot = engine.snapshot()
        do {
            try FileManager.default.createDirectory(
                at: Self.supportDirectory,
                withIntermediateDirectories: true)
            try Data(snapshot.utf8).write(to: Self.sessionURL, options: .atomic)
            try appendJournal(snapshot)
            if truncateJournal {
                try? FileManager.default.removeItem(at: Self.journalURL)
            }
        } catch {
            NSLog("berth: could not save session: \(error)")
        }
    }

    private func appendJournal(_ snapshot: String) throws {
        guard let snapshotData = snapshot.data(using: .utf8),
              let object = try JSONSerialization.jsonObject(with: snapshotData) as? [String: Any]
        else { return }
        let line: [String: Any] = [
            "ts_ms": Int(Date().timeIntervalSince1970 * 1000),
            "op": "snapshot",
            "data": object,
        ]
        let data = try JSONSerialization.data(withJSONObject: line)
        if !FileManager.default.fileExists(atPath: Self.journalURL.path) {
            FileManager.default.createFile(atPath: Self.journalURL.path, contents: nil)
        }
        let handle = try FileHandle(forWritingTo: Self.journalURL)
        defer { try? handle.close() }
        try handle.seekToEnd()
        try handle.write(contentsOf: data)
        try handle.write(contentsOf: Data("\n".utf8))
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

    private static var tokenURL: URL {
        SocketAuth.tokenURL(supportDirectory: supportDirectory)
    }

    private static var journalURL: URL {
        supportDirectory.appendingPathComponent("journal.ndjson")
    }
}
