// Window hosting a single terminal surface. The split tree arrives in M1; M0 is
// intentionally one pane so the renderer integration can be validated alone.
import AppKit

final class MainWindow: NSWindow {
    private let terminal: GhosttySurfaceView

    init() {
        terminal = GhosttySurfaceView(frame: NSRect(x: 0, y: 0, width: 900, height: 600))
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
        center()
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }

    override func makeFirstResponder(_ responder: NSResponder?) -> Bool {
        if let terminal = contentView as? GhosttySurfaceView {
            return super.makeFirstResponder(terminal)
        }
        return super.makeFirstResponder(responder)
    }
}
