// Entry point. Deliberately thin: AppKit lifecycle lives in AppDelegate.
import AppKit

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
