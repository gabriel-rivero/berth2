// App lifecycle: one window in M0, one Ghostty app instance per process.
import AppKit

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var window: MainWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
        guard GhosttyHost.shared.start() else {
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

        let window = MainWindow()
        window.makeKeyAndOrderFront(nil)
        self.window = window
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func applicationWillTerminate(_ notification: Notification) {
        GhosttyHost.shared.stop()
    }
}
