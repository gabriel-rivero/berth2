// One terminal surface: NSView + CAMetalLayer, sized in points, rendered by
// Ghostty. Ghostty owns the PTY and the child process — there is no "feed me
// output" API, which is why Berth runs the shell in-process.

import AppKit
import GhosttyKit

final class GhosttySurfaceView: NSView, @preconcurrency NSTextInputClient {
    /// Core-assigned id (`srf_...`), used by the action callback to report
    /// titles/cwd/bells back to the core.
    /// Set by the shell, read from C callbacks. Both are plain values that
    /// never change on a different thread, so they are intentionally
    /// nonisolated: the clipboard/action callbacks run outside actor context.
    nonisolated(unsafe) var berthSurfaceID: String?

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
        // Ghostty hands this pointer back to every runtime callback for this
        // surface (clipboard, actions); it must stay valid for the surface's
        // lifetime, which the window does.
        config.userdata = Unmanaged.passUnretained(self).toOpaque()
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

    nonisolated var surfaceHandle: ghostty_surface_t? { surface }

    /// Foreground process id, 0 until the child is running.
    func foregroundPID() -> UInt32 {
        guard let surface else { return 0 }
        return UInt32(truncatingIfNeeded: ghostty_surface_foreground_pid(surface))
    }

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

    /// Text accumulated by `interpretKeyEvents` during a single keyDown. The
    /// input method calls `insertText` synchronously, which appends here; the
    /// key event is then sent with that text instead of `event.characters`,
    /// which is empty for composed input.
    private var keyTextAccumulator: [String]?
    /// In-progress composition (`marked text` in AppKit terms). Ghostty shows
    /// it as preedit, underlined, without writing to the PTY.
    private var markedText = NSMutableAttributedString()

    /// All key handling goes through `ghostty_surface_key`: Ghostty decides
    /// whether the chord is a binding (perform the action) or terminal input
    /// (encode and write to the PTY). We never pre-encode C-c/arrow keys
    /// ourselves; the core needs the physical key to do that correctly.
    ///
    /// `interpretKeyEvents` runs first so input methods (CJK, dictation, dead
    /// keys like ´ + a) can commit text. That also means `doCommand` must be
    /// swallowed, or every arrow key would beep.
    override func keyDown(with event: NSEvent) {
        guard let surface else {
            super.keyDown(with: event)
            return
        }

        keyTextAccumulator = []
        defer { keyTextAccumulator = nil }

        let markedBefore = markedText.length > 0
        interpretKeyEvents([event])
        syncPreedit(clearIfNeeded: markedBefore)

        let composing = markedText.length > 0 || markedBefore
        let action: ghostty_input_action_e = event.isARepeat ? GHOSTTY_ACTION_REPEAT : GHOSTTY_ACTION_PRESS
        let keyEvent = event.berthKeyEvent(
            action,
            translationFlags: BerthInput.translationFlags(event, surface: surface))

        // Committed composition (or a plain character interpreted by AppKit):
        // send the text with the physical key so Ghostty can encode both.
        if let accumulated = keyTextAccumulator, !accumulated.isEmpty {
            for text in accumulated {
                guard !text.isEmpty else { continue }
                var commit = keyEvent
                commit.text = text
                commit.composing = false
                _ = commit.send(to: surface)
            }
            return
        }

        var normal = keyEvent
        normal.composing = composing
        if composing { normal.text = nil }
        if !normal.send(to: surface), !composing {
            super.keyDown(with: event)
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

    /// AppKit calls this for keys that produce no text (arrows, function keys)
    /// when it is interpreting events for an NSTextInputClient. Doing nothing
    /// keeps the key on the path to `ghostty_surface_key` instead of beeping.
    override func doCommand(by selector: Selector) {}

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

        // ⌘ chords belong to the app menu first. Ghostty also binds some of
        // them (⌘D split, ⌘W close); claiming those here made the menu items
        // unreachable. Unclaimed ⌘ chords fall through to keyDown, where
        // Ghostty still gets its normal bindings (⌘C copy, ⌘K clear, …).
        if !event.modifierFlags.contains(.command) {
            let keyEvent = event.berthKeyEvent(
                GHOSTTY_ACTION_PRESS,
                translationFlags: BerthInput.translationFlags(event, surface: surface))
            if keyEvent.isBinding(on: surface) || event.modifierFlags.contains(.control) {
                keyDown(with: event)
                return true
            }
        }

        return super.performKeyEquivalent(with: event)
    }

    // MARK: - Text input client (IME)

    func insertText(_ string: Any, replacementRange: NSRange) {
        unmarkText()
        let text: String
        switch string {
        case let value as NSAttributedString: text = value.string
        case let value as NSString: text = value as String
        default: return
        }
        guard !text.isEmpty else { return }

        // Inside keyDown: hand the text back so it rides on the key event.
        if keyTextAccumulator != nil {
            keyTextAccumulator?.append(text)
            return
        }

        // Outside keyDown (dictation, external commit): no NSEvent exists, so
        // there is no keycode to encode. Write it as terminal input.
        sendText(text)
    }

    func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        switch string {
        case let value as NSAttributedString:
            markedText = NSMutableAttributedString(attributedString: value)
        case let value as String:
            markedText = NSMutableAttributedString(string: value)
        default:
            return
        }

        // Outside keyDown (e.g. a keyboard layout change mid-composition) the
        // preedit must update immediately.
        if keyTextAccumulator == nil {
            syncPreedit()
        }
    }

    func unmarkText() {
        guard markedText.length > 0 else { return }
        markedText.mutableString.setString("")
        syncPreedit()
    }

    func hasMarkedText() -> Bool { markedText.length > 0 }

    func markedRange() -> NSRange {
        markedText.length > 0
            ? NSRange(location: 0, length: markedText.length)
            : NSRange()
    }

    func selectedRange() -> NSRange {
        guard let surface else { return NSRange() }
        var text = ghostty_text_s()
        guard ghostty_surface_read_selection(surface, &text) else { return NSRange() }
        defer { ghostty_surface_free_text(surface, &text) }
        return NSRange(location: Int(text.offset_start), length: Int(text.offset_len))
    }

    func validAttributesForMarkedText() -> [NSAttributedString.Key] { [] }

    func attributedSubstring(
        forProposedRange range: NSRange,
        actualRange: NSRangePointer?
    ) -> NSAttributedString? { nil }

    func characterIndex(for point: NSPoint) -> Int { NSNotFound }

    /// Where the candidate window should appear: Ghostty tells us the cursor
    /// cell in top-left coordinates, AppKit wants screen coordinates.
    func firstRect(forCharacterRange range: NSRange, actualRange: NSRangePointer?) -> NSRect {
        guard let surface else { return .zero }
        var x = 0.0
        var y = 0.0
        var width = 0.0
        var height = 0.0
        ghostty_surface_ime_point(surface, &x, &y, &width, &height)
        let inView = NSRect(x: x, y: bounds.height - y, width: width, height: height)
        let inWindow = convert(inView, to: nil)
        guard let window else { return inWindow }
        return window.convertToScreen(inWindow)
    }

    private func syncPreedit(clearIfNeeded: Bool = true) {
        guard let surface else { return }
        if markedText.length > 0 {
            let string = markedText.string
            string.withCString { pointer in
                ghostty_surface_preedit(surface, pointer, UInt(string.utf8.count))
            }
        } else if clearIfNeeded {
            ghostty_surface_preedit(surface, nil, 0)
        }
    }

    // MARK: - Mouse

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        for area in trackingAreas { removeTrackingArea(area) }
        addTrackingArea(NSTrackingArea(
            rect: .zero,
            options: [.mouseEnteredAndExited, .mouseMoved, .activeInActiveApp, .inVisibleRect],
            owner: self))
    }

    /// Ghostty's mouse coordinates are in points from the top-left of the view.
    private func mousePosition(_ event: NSEvent) -> (Double, Double, ghostty_input_mods_e) {
        let point = convert(event.locationInWindow, from: nil)
        return (Double(point.x), Double(bounds.height - point.y),
                BerthInput.mods(event.modifierFlags))
    }

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        guard let surface else { return super.mouseDown(with: event) }
        let (x, y, mods) = mousePosition(event)
        ghostty_surface_mouse_pos(surface, x, y, mods)
        _ = ghostty_surface_mouse_button(surface, GHOSTTY_MOUSE_PRESS, GHOSTTY_MOUSE_LEFT, mods)
    }

    override func mouseUp(with event: NSEvent) {
        guard let surface else { return super.mouseUp(with: event) }
        let (x, y, mods) = mousePosition(event)
        ghostty_surface_mouse_pos(surface, x, y, mods)
        _ = ghostty_surface_mouse_button(surface, GHOSTTY_MOUSE_RELEASE, GHOSTTY_MOUSE_LEFT, mods)
    }

    override func rightMouseDown(with event: NSEvent) {
        guard let surface else { return super.rightMouseDown(with: event) }
        let (x, y, mods) = mousePosition(event)
        ghostty_surface_mouse_pos(surface, x, y, mods)
        if !ghostty_surface_mouse_button(surface, GHOSTTY_MOUSE_PRESS, GHOSTTY_MOUSE_RIGHT, mods) {
            super.rightMouseDown(with: event)
        }
    }

    override func rightMouseUp(with event: NSEvent) {
        guard let surface else { return super.rightMouseUp(with: event) }
        let (x, y, mods) = mousePosition(event)
        ghostty_surface_mouse_pos(surface, x, y, mods)
        if !ghostty_surface_mouse_button(surface, GHOSTTY_MOUSE_RELEASE, GHOSTTY_MOUSE_RIGHT, mods) {
            super.rightMouseUp(with: event)
        }
    }

    override func otherMouseDown(with event: NSEvent) {
        guard let surface else { return super.otherMouseDown(with: event) }
        let (x, y, mods) = mousePosition(event)
        ghostty_surface_mouse_pos(surface, x, y, mods)
        _ = ghostty_surface_mouse_button(
            surface, GHOSTTY_MOUSE_PRESS, Self.mouseButton(event.buttonNumber), mods)
    }

    override func otherMouseUp(with event: NSEvent) {
        guard let surface else { return super.otherMouseUp(with: event) }
        let (x, y, mods) = mousePosition(event)
        ghostty_surface_mouse_pos(surface, x, y, mods)
        _ = ghostty_surface_mouse_button(
            surface, GHOSTTY_MOUSE_RELEASE, Self.mouseButton(event.buttonNumber), mods)
    }

    override func mouseMoved(with event: NSEvent) {
        super.mouseMoved(with: event)
        guard let surface else { return }
        let (x, y, mods) = mousePosition(event)
        ghostty_surface_mouse_pos(surface, x, y, mods)
    }

    override func mouseDragged(with event: NSEvent) {
        mouseMoved(with: event)
    }

    override func rightMouseDragged(with event: NSEvent) {
        mouseMoved(with: event)
    }

    override func otherMouseDragged(with event: NSEvent) {
        mouseMoved(with: event)
    }

    override func mouseEntered(with event: NSEvent) {
        super.mouseEntered(with: event)
        mouseMoved(with: event)
    }

    override func mouseExited(with event: NSEvent) {
        super.mouseExited(with: event)
        // A drag keeps reporting outside the view; only a clean exit tells the
        // terminal the pointer left the viewport.
        guard let surface, NSEvent.pressedMouseButtons == 0 else { return }
        ghostty_surface_mouse_pos(surface, -1, -1, BerthInput.mods(event.modifierFlags))
    }

    override func scrollWheel(with event: NSEvent) {
        guard let surface else { return super.scrollWheel(with: event) }
        let precise = event.hasPreciseScrollingDeltas
        // Trackpads report many small deltas; 2x matches how Ghostty feels.
        let multiplier: CGFloat = precise ? 2 : 1
        ghostty_surface_mouse_scroll(
            surface,
            Double(event.scrollingDeltaX * multiplier),
            Double(event.scrollingDeltaY * multiplier),
            Self.scrollMods(precision: precise, phase: event.momentumPhase))
    }

    private static func mouseButton(_ number: Int) -> ghostty_input_mouse_button_e {
        // AppKit numbers: 2 is middle, 3+ are the extra buttons in order.
        switch number {
        case 0: return GHOSTTY_MOUSE_LEFT
        case 1: return GHOSTTY_MOUSE_RIGHT
        case 2: return GHOSTTY_MOUSE_MIDDLE
        case 3: return GHOSTTY_MOUSE_FOUR
        case 4: return GHOSTTY_MOUSE_FIVE
        case 5: return GHOSTTY_MOUSE_SIX
        case 6: return GHOSTTY_MOUSE_SEVEN
        case 7: return GHOSTTY_MOUSE_EIGHT
        case 8: return GHOSTTY_MOUSE_NINE
        case 9: return GHOSTTY_MOUSE_TEN
        case 10: return GHOSTTY_MOUSE_ELEVEN
        default: return GHOSTTY_MOUSE_UNKNOWN
        }
    }

    /// Packed `ghostty_input_scroll_mods_t`: bit 0 precision, bits 1..3 momentum.
    private static func scrollMods(precision: Bool, phase: NSEvent.Phase) -> ghostty_input_scroll_mods_t {
        var value: Int32 = precision ? 1 : 0
        let momentum: Int32
        switch phase {
        case .began: momentum = 1
        case .stationary: momentum = 2
        case .changed: momentum = 3
        case .ended: momentum = 4
        case .cancelled: momentum = 5
        case .mayBegin: momentum = 6
        default: momentum = 0
        }
        value |= momentum << 1
        return value
    }
}
