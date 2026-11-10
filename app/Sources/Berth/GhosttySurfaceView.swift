// One terminal surface: NSView + CAMetalLayer, sized in points, rendered by
// Ghostty. Ghostty owns the PTY and the child process — there is no "feed me
// output" API, which is why Berth runs the shell in-process.

import AppKit
import GhosttyKit

final class GhosttySurfaceView: NSView {
    // `nonisolated(unsafe)`: NSView is MainActor-isolated, but C teardown has to
    // happen in `deinit`, which is nonisolated. Every other access is on the
    // main thread, so this is a narrow, auditable escape hatch.
    nonisolated(unsafe) private var surface: ghostty_surface_t?
    // C strings handed to Ghostty must outlive the surface: Ghostty may keep
    // pointers into them (command, env values). Freeing happens in `deinit`.
    nonisolated(unsafe) private var cStrings: [UnsafeMutablePointer<CChar>] = []

    override var isOpaque: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override var wantsUpdateLayer: Bool { false }

    override func makeBackingLayer() -> CALayer {
        let layer = CAMetalLayer()
        layer.framebufferOnly = true
        return layer
    }

    // MARK: - Lifecycle

    /// Create the PTY, child process and renderer state. Called explicitly by
    /// the core bridge with the session's command/env; the lazy path in
    /// `viewDidMoveToWindow` only covers views created outside the core.
    @discardableResult
    func createSurface(
        command: String? = nil,
        workingDirectory: String? = nil,
        env: [String] = []
    ) -> Bool {
        guard let app = GhosttyHost.shared.app, surface == nil else { return false }

        var config = ghostty_surface_config_new()
        config.platform_tag = GHOSTTY_PLATFORM_MACOS
        config.platform.macos.nsview = Unmanaged.passUnretained(self).toOpaque()
        config.scale_factor = Double(window?.backingScaleFactor ?? 2.0)
        // Ghostty's field is a non-optional `const char*`; empty means inherit.
        config.working_directory = cString(workingDirectory ?? "")

        if let command, !command.isEmpty {
            config.command = cString(command)
        }

        // `KEY=VALUE` pairs from the core (BERTH_SURFACE_ID and friends).
        var envVars: [ghostty_env_var_s] = []
        envVars.reserveCapacity(env.count)
        for pair in env {
            guard let sep = pair.firstIndex(of: "=") else { continue }
            let key = String(pair[pair.startIndex..<sep])
            let value = String(pair[pair.index(after: sep)...])
            envVars.append(ghostty_env_var_s(key: cString(key), value: cString(value)))
        }
        let created = envVars.withUnsafeMutableBufferPointer { buffer -> Bool in
            config.env_vars = buffer.baseAddress
            config.env_var_count = buffer.count
            config.context = GHOSTTY_SURFACE_CONTEXT_WINDOW
            guard let surface = withUnsafePointer(to: &config, { ghostty_surface_new(app, $0) }) else {
                return false
            }
            self.surface = surface
            return true
        }
        guard created else {
            NSLog("berth: ghostty_surface_new failed")
            return false
        }

        syncSize()
        ghostty_surface_set_focus(surface!, true)
        needsDisplay = true
        return true
    }

    /// Keeps a `strdup`'d C string alive for the lifetime of the view.
    private func cString(_ value: String) -> UnsafePointer<CChar> {
        let copy = strdup(value)!
        cStrings.append(copy)
        return UnsafePointer(copy)
    }

    deinit {
        if let surface {
            ghostty_surface_free(surface)
        }
        for copy in cStrings {
            free(copy)
        }
    }

    // MARK: - Core-facing accessors

    var surfaceHandle: ghostty_surface_t? { surface }

    /// Terminal name (`/dev/ttys003`), empty until the PTY exists.
    func ttyName() -> String {
        guard let surface else { return "" }
        let str = ghostty_surface_tty_name(surface)
        defer { ghostty_string_free(str) }
        guard let ptr = str.ptr else { return "" }
        return String(cString: ptr)
    }

    /// Visible grid dimensions, as Ghostty computed them from the view size.
    func gridSize() -> (cols: UInt16, rows: UInt16) {
        guard let surface else { return (0, 0) }
        let size = ghostty_surface_size(surface)
        return (size.columns, size.rows)
    }

    /// Screen text for automation, via Ghostty's own text extraction.
    func readText(scrollback: Bool) -> String {
        guard let surface else { return "" }
        var text = ghostty_text_s()
        let tag = scrollback ? GHOSTTY_POINT_SCREEN : GHOSTTY_POINT_VIEWPORT
        let selection = ghostty_selection_s(
            top_left: ghostty_point_s(
                tag: tag, coord: GHOSTTY_POINT_COORD_TOP_LEFT, x: 0, y: 0),
            bottom_right: ghostty_point_s(
                tag: tag, coord: GHOSTTY_POINT_COORD_BOTTOM_RIGHT, x: 0, y: 0),
            rectangle: false)
        guard ghostty_surface_read_text(surface, selection, &text) else { return "" }
        defer { ghostty_surface_free_text(surface, &text) }
        guard let ptr = text.text else { return "" }
        let bytes = UnsafeRawBufferPointer(start: ptr, count: Int(text.text_len))
        return String(decoding: bytes, as: UTF8.self)
    }

    /// Resize the grid by columns/rows, keeping the cell size Ghostty chose.
    func resizeGrid(cols: UInt16, rows: UInt16) {
        guard let surface else { return }
        let size = ghostty_surface_size(surface)
        guard size.columns > 0, size.rows > 0 else { return }
        let cellW = Double(size.width_px) / Double(size.columns)
        let cellH = Double(size.height_px) / Double(size.rows)
        ghostty_surface_set_size(
            surface,
            UInt32(Double(cols) * cellW),
            UInt32(Double(rows) * cellH))
    }

    func sendText(_ text: String) {
        guard let surface, !text.isEmpty else { return }
        text.withCString { ptr in
            ghostty_surface_text(surface, ptr, UInt(text.utf8.count))
        }
    }

    /// Synthetic key press for `berth surface send_key`. Keys that arrive from a
    /// real keyboard go through `keyDown` instead; this path has no NSEvent.
    func sendKey(keycode: UInt32, mods: ghostty_input_mods_e, text: String?) {
        guard let surface else { return }
        if let text {
            text.withCString { ptr in
                let input = ghostty_input_key_s(
                    action: GHOSTTY_ACTION_PRESS,
                    mods: mods,
                    consumed_mods: GHOSTTY_MODS_NONE,
                    keycode: keycode,
                    text: ptr,
                    unshifted_codepoint: 0,
                    composing: false)
                _ = ghostty_surface_key(surface, input)
            }
        } else {
            let input = ghostty_input_key_s(
                action: GHOSTTY_ACTION_PRESS,
                mods: mods,
                consumed_mods: GHOSTTY_MODS_NONE,
                keycode: keycode,
                text: nil,
                unshifted_codepoint: 0,
                composing: false)
            _ = ghostty_surface_key(surface, input)
        }
    }

    // MARK: - Sizing

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard window != nil else { return }
        if surface == nil {
            _ = createSurface()
        } else {
            syncSize()
        }
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        syncSize()
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        guard let surface, let scale = window?.backingScaleFactor else { return }
        ghostty_surface_set_content_scale(surface, Double(scale), Double(scale))
        syncSize()
    }

    private func syncSize() {
        guard let surface, let scale = window?.backingScaleFactor else { return }
        ghostty_surface_set_content_scale(surface, Double(scale), Double(scale))
        let width = UInt32(max(bounds.width * scale, 1))
        let height = UInt32(max(bounds.height * scale, 1))
        ghostty_surface_set_size(surface, width, height)
    }

    // MARK: - Rendering

    override func draw(_ dirtyRect: NSRect) {
        guard let surface else { return }
        ghostty_surface_draw(surface)
    }

    // MARK: - Input

    /// All key handling goes through `ghostty_surface_key`: Ghostty decides
    /// whether the chord is a binding (perform the action) or terminal input
    /// (encode and write to the PTY). We never pre-encode C-c/arrow keys
    /// ourselves; the core needs the physical key to do that correctly.
    override func keyDown(with event: NSEvent) {
        guard let surface else {
            super.keyDown(with: event)
            return
        }

        let action: ghostty_input_action_e = event.isARepeat ? GHOSTTY_ACTION_REPEAT : GHOSTTY_ACTION_PRESS
        let keyEvent = event.berthKeyEvent(
            action,
            translationFlags: BerthInput.translationFlags(event, surface: surface))
        guard keyEvent.send(to: surface) else {
            super.keyDown(with: event)
            return
        }
    }

    override func keyUp(with event: NSEvent) {
        guard let surface else {
            super.keyUp(with: event)
            return
        }

        // Releases carry no text; Ghostty only needs the key and mods.
        var keyEvent = event.berthKeyEvent(
            GHOSTTY_ACTION_RELEASE,
            translationFlags: BerthInput.translationFlags(event, surface: surface))
        keyEvent.text = nil
        _ = keyEvent.send(to: surface)
    }

    /// Modifier press/release. Ghostty tracks these for protocols that report
    /// modifiers (Kitty keyboard protocol, terminal mouse reporting).
    override func flagsChanged(with event: NSEvent) {
        guard let surface else {
            super.flagsChanged(with: event)
            return
        }

        let mod: UInt32
        switch event.keyCode {
        case 0x39: mod = GHOSTTY_MODS_CAPS.rawValue
        case 0x38, 0x3C: mod = GHOSTTY_MODS_SHIFT.rawValue
        case 0x3B, 0x3E: mod = GHOSTTY_MODS_CTRL.rawValue
        case 0x3A, 0x3D: mod = GHOSTTY_MODS_ALT.rawValue
        case 0x37, 0x36: mod = GHOSTTY_MODS_SUPER.rawValue
        default: return super.flagsChanged(with: event)
        }

        // Press if that modifier is now down, release otherwise. Sided keys
        // (left vs right) are not distinguished yet; Ghostty treats the mod as
        // pressed because the mask is set either way.
        let pressed = BerthInput.mods(event.modifierFlags).rawValue & mod != 0
        var keyEvent = event.berthKeyEvent(pressed ? GHOSTTY_ACTION_PRESS : GHOSTTY_ACTION_RELEASE)
        keyEvent.text = nil
        _ = keyEvent.send(to: surface)
    }

    /// AppKit offers Command chords to the menu bar before `keyDown`. Claim
    /// the ones Ghostty binds, plus everything with Control, because macOS
    /// otherwise interprets C-/ and C-return itself (beep, context menu).
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard event.type == .keyDown, window?.firstResponder === self, let surface else {
            return super.performKeyEquivalent(with: event)
        }

        let keyEvent = event.berthKeyEvent(
            GHOSTTY_ACTION_PRESS,
            translationFlags: BerthInput.translationFlags(event, surface: surface))
        if keyEvent.isBinding(on: surface) {
            keyDown(with: event)
            return true
        }

        if event.modifierFlags.contains(.control) {
            keyDown(with: event)
            return true
        }

        // Let the menu bar (⌘Q, ⌘W…) and the system have it.
        return super.performKeyEquivalent(with: event)
    }

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        super.mouseDown(with: event)
    }
}
