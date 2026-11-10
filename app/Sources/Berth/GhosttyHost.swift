// Ghostty process-level bootstrap: init, config, app, tick loop.
//
// THIS IS THE ONLY FILE THAT OWNS `ghostty_app_t`. Everything else talks to
// `GhosttySurfaceView`. Keeping the C surface area pinned here is what makes a
// Ghostty upgrade a one-file change (see docs/02-architecture.md §2.1).

import Foundation
import GhosttyKit

/// Wakeup from Ghostty: coalesce to the next runloop turn, never run work inline.
private func berthWakeupCallback(_ userdata: UnsafeMutableRawPointer?) {
    DispatchQueue.main.async {
        GhosttyHost.shared.tick()
    }
}

/// Actions Ghostty wants the shell to perform.
///
/// TODO(M0): switch on `action.tag` and route the interesting ones into the
/// core event stream. Tags that matter, in priority order:
///   GHOSTTY_ACTION_SET_TITLE          -> surface.title_changed
///   GHOSTTY_ACTION_PWD                -> surface.pwd_changed
///   GHOSTTY_ACTION_PROGRESS_REPORT    -> surface.progress
///   GHOSTTY_ACTION_RING_BELL          -> surface.bell
///   GHOSTTY_ACTION_DESKTOP_NOTIFICATION -> notification.create
///   GHOSTTY_ACTION_OPEN_URL           -> NSWorkspace.shared.open
///   GHOSTTY_ACTION_SHOW_CHILD_EXITED  -> surface closed / error state
/// Return `true` only for actions Berth fully handles; `false` lets Ghostty
/// apply its own default behaviour.
private let berthActionCallback: ghostty_runtime_action_cb = { _, _, _ in
    false
}

private let berthReadClipboardCallback: ghostty_runtime_read_clipboard_cb = { _, _, _ in
    nil
}

private let berthConfirmReadClipboardCallback: ghostty_runtime_confirm_read_clipboard_cb = {
    _, _, _, _, _ in
}

private let berthWriteClipboardCallback: ghostty_runtime_write_clipboard_cb = {
    _, _, _, _, _ in
}

private let berthCloseSurfaceCallback: ghostty_runtime_close_surface_cb = { _, _ in
    // TODO(M0): close the surface view that owns `userdata`.
}

final class GhosttyHost {
    static let shared = GhosttyHost()

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
