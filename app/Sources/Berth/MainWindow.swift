// One window per surface in M0. The split tree replaces this in M1; until then
// a workspace is a window, which keeps the core's model honest and testable.
import AppKit

final class MainWindow: NSWindow {
    let terminal: GhosttySurfaceView

    init(surface: GhosttySurfaceView) {
        terminal = surface
        super.init(
            contentRect: NSRect(x: 0, y: 0, width: 900, height: 600),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        title = "Berth"
        minSize = NSSize(width: 400, height: 240)
        contentView = terminal
        terminal.autoresizingMask = [.width, .height]
        cascadeTopLeft(from: NSPoint(x: 0, y: 0))
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }

    override func makeFirstResponder(_ responder: NSResponder?) -> Bool {
        super.makeFirstResponder(terminal)
    }
}
