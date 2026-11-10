// The application menu. Built in code so there is no nib and no Xcode project.
//
// Terminal commands (split, close pane, focus movement) live here because the
// menu bar wins Command chords before the terminal view sees them. Editing
// commands still belong to Ghostty's keybindings: ⌘C/⌘V/⌘K are handled inside
// the terminal, so this menu must not steal them.

import AppKit

enum MainMenu {
    @MainActor
    static func install(appName: String = "Berth") {
        let mainMenu = NSMenu()
        let delegate = NSApp.delegate

        // Application menu. The first item is titled with the app name by
        // AppKit regardless of what we put here.
        let appItem = NSMenuItem()
        mainMenu.addItem(appItem)
        let appMenu = NSMenu()
        appItem.submenu = appMenu
        appMenu.addItem(
            withTitle: "About \(appName)",
            action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)),
            keyEquivalent: "")
        appMenu.addItem(.separator())
        appMenu.addItem(
            withTitle: "Hide \(appName)",
            action: #selector(NSApplication.hide(_:)),
            keyEquivalent: "h")
        let hideOthers = appMenu.addItem(
            withTitle: "Hide Others",
            action: #selector(NSApplication.hideOtherApplications(_:)),
            keyEquivalent: "h")
        hideOthers.keyEquivalentModifierMask = [.command, .option]
        appMenu.addItem(
            withTitle: "Show All",
            action: #selector(NSApplication.unhideAllApplications(_:)),
            keyEquivalent: "")
        appMenu.addItem(.separator())
        appMenu.addItem(
            withTitle: "Quit \(appName)",
            action: #selector(NSApplication.terminate(_:)),
            keyEquivalent: "q")

        // Terminal menu: the split-focus model of the core, exposed without a
        // sidebar yet.
        let terminalItem = NSMenuItem()
        mainMenu.addItem(terminalItem)
        let terminalMenu = NSMenu(title: "Terminal")
        terminalItem.submenu = terminalMenu

        func add(
            _ title: String,
            _ action: Selector,
            _ key: String,
            _ mask: NSEvent.ModifierFlags = [.command]
        ) {
            let item = terminalMenu.addItem(
                withTitle: title, action: action, keyEquivalent: key)
            item.keyEquivalentModifierMask = mask
            item.target = delegate
        }

        add("Split Right", #selector(AppDelegate.splitRight(_:)), "d")
        add("Split Down", #selector(AppDelegate.splitDown(_:)), "d", [.command, .shift])
        terminalMenu.addItem(.separator())
        add("Close Surface", #selector(AppDelegate.closeFocusedSurface(_:)), "w")
        add("Next Surface", #selector(AppDelegate.nextSurface(_:)), "]")
        add("Previous Surface", #selector(AppDelegate.previousSurface(_:)), "[")
        terminalMenu.addItem(.separator())
        add("New Workspace", #selector(AppDelegate.newWorkspace(_:)), "n")

        // Window menu. ⌘W is the surface above; ⌘⇧W is the window itself.
        let windowItem = NSMenuItem()
        mainMenu.addItem(windowItem)
        let windowMenu = NSMenu(title: "Window")
        windowItem.submenu = windowMenu
        windowMenu.addItem(
            withTitle: "Minimize",
            action: #selector(NSWindow.performMiniaturize(_:)),
            keyEquivalent: "m")
        windowMenu.addItem(
            withTitle: "Zoom",
            action: #selector(NSWindow.performZoom(_:)),
            keyEquivalent: "")
        windowMenu.addItem(.separator())
        let closeWindow = windowMenu.addItem(
            withTitle: "Close Window",
            action: #selector(NSWindow.performClose(_:)),
            keyEquivalent: "w")
        closeWindow.keyEquivalentModifierMask = [.command, .shift]

        NSApp.mainMenu = mainMenu
        NSApp.windowsMenu = windowMenu
    }
}
