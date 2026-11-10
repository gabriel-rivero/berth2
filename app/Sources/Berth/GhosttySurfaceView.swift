// One terminal surface: NSView + CAMetalLayer, sized in points, rendered by
// Ghostty. Ghostty owns the PTY and the child process — there is no "feed me
// output" API, which is why Berth runs the shell in-process.

import AppKit
import GhosttyKit

final class GhosttySurfaceView: NSView {
    private var surface: ghostty_surface_t?
    private var workingDirectoryRef: UnsafeMutablePointer<CChar>?

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

    /// TODO(M0): full key routing. This only forwards printable characters, so
    /// arrows/Esc/Ctrl-c do not work yet. The real path builds a
    /// `ghostty_input_key_s` per event (action PRESS/RELEASE/REPEAT, mods,
    /// consumed_mods, keycode, text, unshifted_codepoint, composing) and calls
    /// `ghostty_surface_key`, and must also call
    /// `ghostty_config_key_is_binding` first so app-level bindings win.
    override func keyDown(with event: NSEvent) {
        if let text = event.characters, !text.isEmpty, let surface {
            text.withCString { ptr in
                ghostty_surface_text(surface, ptr, UInt(strlen(ptr)))
            }
            return
        }
        super.keyDown(with: event)
    }

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        super.mouseDown(with: event)
    }
}
