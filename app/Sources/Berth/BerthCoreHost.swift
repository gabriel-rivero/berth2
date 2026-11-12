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
    private var window: MainWindow?
    private var surfaceWorkspace: [String: String] = [:]
    private var didAskForNotificationPermission = false
    private var lastLayoutJSON: [String: String] = [:]

    private var sidebarBase: [SidebarWorkspace] = []
    private var sidebarGroups: [SidebarGroupInfo] = []
    private var selectedWorkspaceID: String?
    private var lastAppliedStructure: String?
    private var lastSelectedWorkspace: String?
    private var lastActiveSurface: String?
    private var lastAttention: [String: String] = [:]

    private struct WorkspaceMeta {
        var branch: String?
        var ports: [Int]
    }

    private var metadata: [String: WorkspaceMeta] = [:]
    private var metadataInFlight: Set<String> = []
    private var lastMetadataAt: [String: TimeInterval] = [:]

    /// Set by AppDelegate once the engine exists; callbacks (titles, cwd, bells)
    /// report through it.
    weak var engine: Engine?
    /// Called after a report so queued events reach socket subscribers.
    var onEvents: (() -> Void)?

    /// Injected into every child so CLI/hooks can authenticate to the socket.
    private var transportToken: String?
    private var transportSocketPath: String?

    private init() {}

    func configureTransport(token: String?, socketPath: String) {
        transportToken = token
        transportSocketPath = socketPath
    }

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
            var childEnv = req.env
            if !childEnv.contains(where: { $0.hasPrefix("PATH=") }),
               let path = ProcessInfo.processInfo.environment["PATH"] {
                childEnv.append("PATH=\(path)")
            }
            if let transportToken {
                childEnv.append("BERTH_TOKEN=\(transportToken)")
            }
            if let transportSocketPath {
                childEnv.append("BERTH_SOCKET_PATH=\(transportSocketPath)")
            }
            guard view.createSurface(
                command: req.command,
                workingDirectory: req.cwd,
                env: childEnv
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

    /// Tear every PTY down while Ghostty is still alive. Called on terminate,
    /// before `ghostty_app_free`, so children get a clean close instead of
    /// being orphaned when the process exits.
    func shutdown() {
        try? onMain {
            for view in surfaces.values {
                view.teardown()
            }
            surfaces.removeAll()
            surfaceWorkspace.removeAll()
            lastActiveSurface = nil
            window?.orderOut(nil)
            window = nil
        }
    }

    func focus(surfaceId: String) throws {
        try onMain {
            guard let view = surfaces[surfaceId] else {
                throw FfiError.Internal(message: "unknown surface \(surfaceId)")
            }
            guard let window else {
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
            // No banner for work the user is looking at; the sidebar and pane
            // ring already carry that signal.
            if let workspaceId, workspaceId == selectedWorkspaceID { return }
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

    /// Rebuild the window from the core's state. Cheap when nothing changed:
    /// structural JSON is compared per workspace and only the selected
    /// workspace's tree is actually applied.
    func reconcileAll() {
        try? onMain { [self] in
            guard let engine else { return }
            let listed = engine.dispatch(
                request: #"{"id":"sync","method":"workspace.list","params":{}}"#)
            guard let result = Self.result(listed),
                  let workspaces = result["workspaces"] as? [[String: Any]]
            else { return }

            let groups = (result["groups"] as? [[String: Any]] ?? []).compactMap {
                item -> SidebarGroupInfo? in
                guard let id = item["id"] as? String else { return nil }
                return SidebarGroupInfo(
                    id: id,
                    name: item["name"] as? String ?? "Group",
                    collapsed: item["collapsed"] as? Bool ?? false)
            }
            sidebarGroups = groups
            selectedWorkspaceID = result["selected_workspace"] as? String

            let agentItems = Self.agentItems(engine)
            let agentInfos = agentItems.compactMap(Self.sidebarAgent(from:))
            var primaryAgent: [String: SidebarAgent] = [:]
            for info in agentInfos {
                if let existing = primaryAgent[info.workspaceID] {
                    if info.attentionRank > existing.attentionRank
                        || (info.attentionRank == existing.attentionRank
                            && info.sinceMs > existing.sinceMs) {
                        primaryAgent[info.workspaceID] = info
                    }
                } else {
                    primaryAgent[info.workspaceID] = info
                }
            }

            if workspaces.isEmpty {
                window?.close()
                window = nil
                sidebarBase = []
                lastAppliedStructure = nil
                lastSelectedWorkspace = nil
                return
            }

            var base: [SidebarWorkspace] = []
            var layouts: [String: WorkspaceLayout] = [:]
            for item in workspaces {
                guard let id = item["id"] as? String,
                      let json = layoutJSON(id),
                      let layout = WorkspaceLayout.parse(json)
                else { continue }
                layouts[id] = layout
                lastLayoutJSON[id] = json
                let active = layout.surface(layout.activeSurface)
                let known = metadata[id]
                base.append(SidebarWorkspace(
                    id: id,
                    name: item["name"] as? String ?? layout.name,
                    title: active?.title ?? "",
                    cwd: active?.cwd ?? "",
                    branch: known?.branch,
                    ports: known?.ports ?? [],
                    unread: layout.unread,
                    pinned: layout.pinned,
                    groupID: layout.groupID,
                    pid: active?.pid,
                    agent: primaryAgent[id]))
            }
            sidebarBase = base
            lastLayoutJSON = lastLayoutJSON.filter { layouts[$0.key] != nil }

            let win = window ?? MainWindow()
            if window == nil {
                window = win
                win.sidebar.onSelect = { [weak self] id in
                    self?.sendFromUI("workspace.select", ["workspace_id": id])
                }
                win.sidebar.onMove = { [weak self] id, index in
                    self?.sendFromUI(
                        "workspace.move", ["workspace_id": id, "index": index])
                }
                win.sidebar.onToggleGroup = { [weak self] id, collapsed in
                    self?.sendFromUI(
                        "group.update", ["group_id": id, "collapsed": collapsed])
                }
                win.sidebar.onNewWorkspace = { [weak self] in
                    self?.sendFromUI("workspace.create", ["name": "workspace"])
                }
                win.sidebar.onClose = { [weak self] id in
                    self?.sendFromUI("workspace.close", ["workspace_id": id])
                }
                win.makeKeyAndOrderFront(nil)
                NSApp.activate(ignoringOtherApps: true)
            }
            let focused = focusedSurfaceID
            for info in agentInfos
            where info.needsAttention
                && lastAttention[info.id] != info.attention
                && info.surfaceID != focused {
                win.ring(surfaceID: info.surfaceID)
            }
            lastAttention = Dictionary(
                uniqueKeysWithValues: agentInfos.map { ($0.id, $0.attention) })
            publishSidebar()

            let selected = selectedWorkspaceID.flatMap { layouts[$0] != nil ? $0 : nil }
                ?? layouts.keys.sorted().first
            if let selected, let layout = layouts[selected] {
                let structuralChange = lastSelectedWorkspace != selected
                    || lastAppliedStructure != layout.structureKey
                if structuralChange {
                    win.apply(layout: layout, views: surfaces, actions: paneActions)
                    lastAppliedStructure = layout.structureKey
                    lastSelectedWorkspace = selected
                } else if win.isKeyWindow,
                          let active = layout.activeSurface,
                          let view = surfaces[active],
                          win.firstResponder !== view {
                    win.makeFirstResponder(view)
                }
                lastActiveSurface = layout.activeSurface ?? layout.root.firstLeafSurface()
            }
            refreshMetadata(base)
        }
    }

    @MainActor
    private func layoutJSON(_ workspaceID: String) -> String? {
        guard let engine else { return nil }
        let request = #"{"id":"layout","method":"workspace.layout","params":{"workspace_id":"\#(workspaceID)"}}"#
        guard let result = Self.result(engine.dispatch(request: request)),
              let data = try? JSONSerialization.data(withJSONObject: result)
        else { return nil }
        return String(data: data, encoding: .utf8)
    }

    @MainActor
    private func publishSidebar() {
        guard let window else { return }
        let selected = selectedWorkspaceID ?? sidebarBase.first?.id
        let workspaces = sidebarBase.map { workspace in
            let known = metadata[workspace.id]
            return SidebarWorkspace(
                id: workspace.id,
                name: workspace.name,
                title: workspace.title,
                cwd: workspace.cwd,
                branch: known?.branch,
                ports: known?.ports ?? [],
                unread: workspace.unread,
                pinned: workspace.pinned,
                groupID: workspace.groupID,
                pid: workspace.pid,
                agent: workspace.agent)
        }
        window.updateSidebar(SidebarModel(
            groups: sidebarGroups,
            workspaces: workspaces,
            selectedWorkspace: selected))
    }

    /// Dispatch a request that originated in AppKit. Always main thread.
    @MainActor
    private func sendFromUI(_ method: String, _ params: [String: Any]) {
        guard let engine,
              let data = try? JSONSerialization.data(withJSONObject: params),
              let json = String(data: data, encoding: .utf8)
        else { return }
        _ = engine.dispatch(
            request: #"{"id":"ui","method":"\#(method)","params":\#(json)}"#)
        onEvents?()
        reconcileAll()
    }

    @MainActor
    private var paneActions: PaneActions {
        PaneActions(
            activate: { [weak self] id in
                self?.sendFromUI("surface.activate", ["surface_id": id])
            },
            close: { [weak self] id in
                self?.sendFromUI("surface.close", ["surface_id": id])
            },
            newTab: { [weak self] pane in
                self?.sendFromUI(
                    "surface.create", ["pane_id": pane, "focus": true])
            })
    }

    /// Ghostty asked to close a surface (child exited and the config says so).
    /// Goes through the core so pane/agent bookkeeping stays in one place.
    func toggleSidebar() {
        try? onMain { [self] in
            window?.toggleSidebar()
        }
    }

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
        if let view = window?.firstResponder as? GhosttySurfaceView {
            return view.berthSurfaceID
        }
        return lastActiveSurface
    }

    // MARK: - Agents

    /// Ask the core to sniff each unclaimed surface's foreground process.
    /// `agent.detect` is idempotent, so the timer can simply keep asking.
    func detectAgents() {
        try? onMain { [self] in
            guard let engine else { return }
            let agents = Self.agentItems(engine)
            let bound = Set(agents.compactMap { $0["surface_id"] as? String })
            let listed = engine.dispatch(
                request: #"{"id":"detect","method":"surface.list","params":{}}"#)
            guard let result = Self.result(listed),
                  let surfaces = result["surfaces"] as? [[String: Any]]
            else { return }

            var created = false
            for surface in surfaces {
                guard let id = surface["id"] as? String, !bound.contains(id) else { continue }
                let request =
                    #"{"id":"detect","method":"agent.detect","params":{"surface_id":"\#(id)"}}"#
                if let detected = Self.result(engine.dispatch(request: request)),
                   detected["agent"] is [String: Any] {
                    created = true
                }
            }
            if created {
                onEvents?()
                reconcileAll()
            }
        }
    }

    private static func agentItems(_ engine: Engine) -> [[String: Any]] {
        let response = engine.dispatch(
            request: #"{"id":"agents","method":"agent.list","params":{}}"#)
        return result(response)?["agents"] as? [[String: Any]] ?? []
    }

    private static func sidebarAgent(from item: [String: Any]) -> SidebarAgent? {
        guard let id = item["id"] as? String,
              let workspaceID = item["workspace_id"] as? String,
              let surfaceID = item["surface_id"] as? String
        else { return nil }
        return SidebarAgent(
            id: id,
            workspaceID: workspaceID,
            surfaceID: surfaceID,
            provider: item["provider"] as? String ?? "custom",
            name: item["name"] as? String ?? "agent",
            state: item["state"] as? String ?? "unknown",
            detail: item["detail"] as? String ?? "",
            progress: (item["progress"] as? NSNumber)?.doubleValue,
            attention: item["attention"] as? String ?? "none",
            sinceMs: (item["since_ms"] as? NSNumber)?.uint64Value ?? 0)
    }

    // MARK: - Sidebar metadata

    /// Git branch and listening ports are shell facts, not core facts. Query
    /// them off the main thread, cache the answer per workspace for a couple of
    /// seconds, and publish once per refresh.
    @MainActor
    private func refreshMetadata(_ workspaces: [SidebarWorkspace]) {
        let now = Date.timeIntervalSinceReferenceDate
        for workspace in workspaces {
            guard !workspace.cwd.isEmpty,
                  workspace.pid != nil,
                  !metadataInFlight.contains(workspace.id)
            else { continue }
            if let last = lastMetadataAt[workspace.id], now - last < 2 { continue }
            metadataInFlight.insert(workspace.id)
            lastMetadataAt[workspace.id] = now

            let id = workspace.id
            let cwd = workspace.cwd
            let pid = workspace.pid
            DispatchQueue.global(qos: .utility).async { [weak self] in
                let branch = Self.gitBranch(cwd)
                let ports = pid.map(Self.listeningPorts) ?? []
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    self.metadata[id] = WorkspaceMeta(branch: branch, ports: ports)
                    self.metadataInFlight.remove(id)
                    self.publishSidebar()
                }
            }
        }
    }

    private static func gitBranch(_ cwd: String) -> String? {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        process.arguments = ["-C", cwd, "rev-parse", "--abbrev-ref", "HEAD"]
        process.standardError = FileHandle.nullDevice
        let pipe = Pipe()
        process.standardOutput = pipe
        guard (try? process.run()) != nil else { return nil }
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { return nil }
        let branch = String(decoding: data, as: UTF8.self)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return branch.isEmpty || branch == "HEAD" ? nil : branch
    }

    private static func listeningPorts(pid: Int) -> [Int] {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/sbin/lsof")
        process.arguments = ["-nP", "-a", "-p", "\(pid)", "-iTCP", "-sTCP:LISTEN", "-F", "n"]
        process.standardError = FileHandle.nullDevice
        let pipe = Pipe()
        process.standardOutput = pipe
        guard (try? process.run()) != nil else { return [] }
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        let output = String(decoding: data, as: UTF8.self)
        var ports: [Int] = []
        for line in output.split(separator: "\n") where line.hasPrefix("n") {
            guard let colon = line.lastIndex(of: ":") else { continue }
            if let port = Int(line[line.index(after: colon)...]), !ports.contains(port) {
                ports.append(port)
            }
        }
        return ports.sorted()
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
            self.reconcileAll()
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
