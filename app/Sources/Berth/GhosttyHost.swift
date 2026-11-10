// Ghostty process-level bootstrap: init, config, app, tick loop, and the
// runtime callbacks that need raw C access (clipboard, actions).
//
// THIS IS THE ONLY FILE THAT OWNS `ghostty_app_t` AND THE CLIPBOARD BRIDGE.
// Everything else talks to `GhosttySurfaceView` or `BerthCoreHost`. Keeping the
// C surface area pinned here is what makes a Ghostty upgrade a one-file change
// (see docs/02-architecture.md §2.1).

import AppKit
import Foundation
import GhosttyKit

/// Wakeup from Ghostty: coalesce to the next runloop turn, never run work inline.
private func berthWakeupCallback(_ userdata: UnsafeMutableRawPointer?) {
    DispatchQueue.main.async {
        GhosttyHost.shared.tick()
    }
}

/// Every runtime callback receives the surface's `userdata`, which
/// `GhosttySurfaceView.createSurface` sets to the view itself.
private func berthSurfaceView(_ userdata: UnsafeMutableRawPointer?) -> GhosttySurfaceView? {
    guard let userdata else { return nil }
    return Unmanaged<GhosttySurfaceView>.fromOpaque(userdata).takeUnretainedValue()
}

/// Actions Ghostty wants the shell to perform. The host decides which ones it
/// handles; `false` lets Ghostty apply its own default behaviour.
private let berthActionCallback: ghostty_runtime_action_cb = { _, target, action in
    guard Thread.isMainThread else { return false }
    return BerthCoreHost.shared.handleAction(target: target, action: action)
}

// MARK: - Clipboard

private struct ClipboardContent {
    let mime: String
    let data: Data
}

private enum ClipboardBridge {
    /// NSPasteboard for a Ghostty clipboard kind. Selection/primary have no
    /// macOS equivalent, so they are unsupported rather than silently wrong.
    static func pasteboard(_ location: ghostty_clipboard_e) -> NSPasteboard? {
        location == GHOSTTY_CLIPBOARD_STANDARD ? NSPasteboard.general : nil
    }

    /// Payload for a requested MIME type, if the pasteboard can serve it.
    static func data(for mime: String, from pasteboard: NSPasteboard) -> Data? {
        switch mime {
        case "text/plain", "text/plain;charset=utf-8":
            return pasteboard.string(forType: .string)?.data(using: .utf8)
        case "text/html":
            return pasteboard.data(forType: .html)
        case "image/png":
            return pasteboard.data(forType: .png)
        default:
            return nil
        }
    }

    static func requested(_ mimes: UnsafePointer<UnsafePointer<CChar>?>?, _ count: Int) -> [String] {
        guard let mimes else { return [] }
        var out: [String] = []
        for i in 0..<count {
            guard let ptr = mimes[i] else { continue }
            out.append(String(cString: ptr))
        }
        return out
    }

    /// Hand `contents` back to Ghostty. C memory lives only for the call.
    static func complete(
        surface: ghostty_surface_t,
        contents: [ClipboardContent],
        confirmed: Bool,
        remember: Bool = false,
        state: UnsafeMutableRawPointer?
    ) {
        var mimes: [UnsafeMutablePointer<CChar>] = []
        var buffers: [UnsafeMutableRawPointer] = []
        defer {
            mimes.forEach { free($0) }
            buffers.forEach { $0.deallocate() }
        }

        var cContents: [ghostty_clipboard_content_s] = []
        cContents.reserveCapacity(contents.count)
        for content in contents {
            let mime = strdup(content.mime)!
            mimes.append(mime)
            let count = content.data.count
            let buffer = UnsafeMutableRawPointer.allocate(byteCount: max(count, 1), alignment: 1)
            buffers.append(buffer)
            content.data.copyBytes(to: buffer.assumingMemoryBound(to: UInt8.self), count: count)
            cContents.append(
                ghostty_clipboard_content_s(
                    mime: mime,
                    data: UnsafeRawPointer(buffer).assumingMemoryBound(to: CChar.self),
                    len: count))
        }

        var complete = ghostty_clipboard_complete_s(
            contents: nil,
            contents_len: 0,
            available: nil,
            available_len: 0,
            confirmed: confirmed,
            remember: remember)
        cContents.withUnsafeMutableBufferPointer { buffer in
            complete.contents = buffer.baseAddress
            complete.contents_len = buffer.count
            ghostty_surface_complete_clipboard_request(surface, &complete, state)
        }
    }
}

private let berthReadClipboardCallback: ghostty_runtime_read_clipboard_cb = {
    userdata, location, state, mimes, mimesLen, list in
    guard let view = berthSurfaceView(userdata), let surface = view.surfaceHandle else {
        return GHOSTTY_CLIPBOARD_READ_UNSUPPORTED
    }
    guard let pasteboard = ClipboardBridge.pasteboard(location) else {
        return GHOSTTY_CLIPBOARD_READ_UNSUPPORTED
    }

    var contents: [ClipboardContent] = []
    if !list {
        for mime in ClipboardBridge.requested(mimes, mimesLen) {
            if let data = ClipboardBridge.data(for: mime, from: pasteboard) {
                contents.append(ClipboardContent(mime: mime, data: data))
            }
        }
        if contents.isEmpty { return GHOSTTY_CLIPBOARD_READ_UNAVAILABLE }
    }

    ClipboardBridge.complete(
        surface: surface, contents: contents, confirmed: !list, state: state)
    return GHOSTTY_CLIPBOARD_READ_STARTED
}

/// Ghostty asks before pasting something risky (multi-line text, OSC 52 reads).
/// The dialog is the whole point of this callback, so it is a real prompt.
private let berthConfirmReadClipboardCallback: ghostty_runtime_confirm_read_clipboard_cb = {
    userdata, confirm, state, _ in
    guard let view = berthSurfaceView(userdata),
          let surface = view.surfaceHandle,
          let confirm
    else { return }

    let request = confirm.pointee
    var contents: [ClipboardContent] = []
    if let raw = request.contents {
        for i in 0..<request.contents_len {
            let item = raw[i]
            guard let mime = item.mime, let data = item.data else { continue }
            contents.append(
                ClipboardContent(
                    mime: String(cString: mime),
                    data: Data(bytes: data, count: Int(item.len))))
        }
    }

    let name = request.name.map { String(cString: $0) } ?? "the clipboard"
    // The payload is only valid during this call; everything the dialog needs
    // must be copied before the async hop.
    let canRemember = request.can_remember
    let preview = contents.first(where: { $0.mime.hasPrefix("text/plain") })
        .flatMap { String(data: $0.data, encoding: .utf8) }
        .map { String($0.prefix(1200)) } ?? "(binary data)"

    DispatchQueue.main.async {
        let alert = NSAlert()
        alert.messageText = "Allow paste from \(name)?"
        alert.informativeText = preview
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Paste")
        alert.addButton(withTitle: "Cancel")
        let accepted = alert.runModal() == .alertFirstButtonReturn
        if accepted {
            ClipboardBridge.complete(
                surface: surface,
                contents: contents,
                confirmed: true,
                remember: canRemember,
                state: state)
        } else {
            ghostty_surface_deny_clipboard_request(surface, state)
        }
    }
}

private let berthWriteClipboardCallback: ghostty_runtime_write_clipboard_cb = {
    userdata, location, content, len, _ in
    guard let view = berthSurfaceView(userdata), view.surfaceHandle != nil else { return }
    guard let pasteboard = ClipboardBridge.pasteboard(location), let content else { return }

    // OSC 52 and copies arrive here. Text is the only representation Berth
    // writes; anything else is left untouched rather than mangled.
    for i in 0..<len {
        let item = content[i]
        guard let mime = item.mime, mime.hasPrefix("text/plain"), let data = item.data else {
            continue
        }
        let text = String(decoding: Data(bytes: data, count: Int(item.len)), as: UTF8.self)
        pasteboard.clearContents()
        pasteboard.setString(text, forType: .string)
        return
    }
}

/// Ghostty asks the shell to close a surface when the child is gone and the
/// config says so. Closing goes through the core so panes and agents agree.
private let berthCloseSurfaceCallback: ghostty_runtime_close_surface_cb = { userdata, _ in
    guard let view = berthSurfaceView(userdata), let id = view.berthSurfaceID else { return }
    DispatchQueue.main.async {
        BerthCoreHost.shared.closeSurfaceFromTerminal(id)
    }
}

/// Process-wide Ghostty instance. Lives on the main thread only; the
/// `nonisolated(unsafe)` is the explicit opt-out from Swift 6's Sendable check
/// for a singleton that AppKit already guarantees is single-threaded.
final class GhosttyHost {
    nonisolated(unsafe) static let shared = GhosttyHost()

    private(set) var app: ghostty_app_t?
    private var config: ghostty_config_t?
    private var tickTimer: Timer?
    private var runtime: ghostty_runtime_config_s?
    private var started = false

    private init() {}

    func start() -> Bool {
        guard !started else { return true }
        guard ghostty_init(UInt(CommandLine.argc), CommandLine.unsafeArgv) == 0 else {
            NSLog("berth: ghostty_init failed")
            return false
        }

        let cfg = ghostty_config_new()
        ghostty_config_load_default_files(cfg)
        ghostty_config_finalize(cfg)
        config = cfg

        var rt = ghostty_runtime_config_s(
            userdata: Unmanaged.passUnretained(self).toOpaque(),
            supports_selection_clipboard: true,
            wakeup_cb: berthWakeupCallback,
            action_cb: berthActionCallback,
            read_clipboard_cb: berthReadClipboardCallback,
            confirm_read_clipboard_cb: berthConfirmReadClipboardCallback,
            write_clipboard_cb: berthWriteClipboardCallback,
            close_surface_cb: berthCloseSurfaceCallback
        )
        runtime = rt

        guard let app = withUnsafePointer(to: &rt, { ghostty_app_new($0, cfg) }) else {
            NSLog("berth: ghostty_app_new failed")
            return false
        }
        self.app = app

        // 60 Hz ceiling. Ghostty only draws when it marks itself dirty; the timer
        // exists so progress spinners and cursor blink keep moving while idle.
        let timer = Timer(timeInterval: 1.0 / 60.0, repeats: true) { [weak self] _ in
            self?.tick()
        }
        RunLoop.main.add(timer, forMode: .common)
        tickTimer = timer
        started = true
        return true
    }

    func tick() {
        guard let app else { return }
        ghostty_app_tick(app)
    }

    func stop() {
        tickTimer?.invalidate()
        tickTimer = nil
        if let app {
            ghostty_app_free(app)
        }
        app = nil
        started = false
    }
}
