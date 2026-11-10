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
    nonisolated(unsafe) private var workingDirectoryRef: UnsafeMutablePointer<CChar>?

    override var isOpaque: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override var wantsUpdateLayer: Bool { false }

    override func makeBackingLayer() -> CALayer {
        let layer = CAMetalLayer()
        layer.framebufferOnly = true
        return layer
    }

    // MARK: - Lifecycle

    func createSurface(workingDirectory: String?) -> Bool {
        guard let app = GhosttyHost.shared.app else { return false }
        // Ghostty's field is a non-optional `const char*`; an empty string means
        // "inherit", which is what we want when the caller has no preference.
        workingDirectoryRef = strdup(workingDirectory ?? "")

        var config = ghostty_surface_config_new()
        config.platform_tag = GHOSTTY_PLATFORM_MACOS
        config.platform.macos.nsview = Unmanaged.passUnretained(self).toOpaque()
        config.scale_factor = Double(window?.backingScaleFactor ?? 2.0)
        config.working_directory = UnsafePointer(workingDirectoryRef!)
        config.context = GHOSTTY_SURFACE_CONTEXT_WINDOW

        guard let surface = withUnsafePointer(to: &config, { ghostty_surface_new(app, $0) }) else {
            NSLog("berth: ghostty_surface_new failed")
            return false
        }
        self.surface = surface
        syncSize()
        ghostty_surface_set_focus(surface, true)
        needsDisplay = true
        return true
    }

    deinit {
        if let surface {
            ghostty_surface_free(surface)
        }
        free(workingDirectoryRef)
    }

    // MARK: - Sizing

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard window != nil else { return }
        if surface == nil {
            _ = createSurface(workingDirectory: nil)
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
