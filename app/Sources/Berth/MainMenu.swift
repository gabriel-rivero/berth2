// The application menu. Built in code so there is no nib and no Xcode project.
//
// Terminal commands (split, tabs, panes, workspaces) live here because the menu
// bar wins Command chords before the terminal view sees them. Editing commands
// still belong to Ghostty's keybindings: ⌘C/⌘V/⌘K are handled inside the
// terminal, so this menu must not steal them.
//
// Key equivalents come from AppConfig so the settings window can change them
// without restarting.

import AppKit

enum MainMenu {
    @MainActor
    static func install(appName: String = "Berth") {
        let mainMenu = NSMenu()
        let delegate = NSApp.delegate
        func shortcut(_ action: ShortcutAction) -> (String, NSEvent.ModifierFlags) {
            let value = AppConfig.shared.shortcut(for: action)
            return (value.key, value.modifiers)
        }

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
        let settings = appMenu.addItem(
            withTitle: "Settings…",
            action: #selector(AppDelegate.showSettings(_:)),
            keyEquivalent: shortcut(.settings).0)
        settings.keyEquivalentModifierMask = shortcut(.settings).1
        settings.target = delegate
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

        func add(
            _ menu: NSMenu,
            _ title: String,
            _ action: Selector,
            _ shortcutAction: ShortcutAction
        ) {
            let (key, mask) = shortcut(shortcutAction)
            let item = menu.addItem(withTitle: title, action: action, keyEquivalent: key)
            item.keyEquivalentModifierMask = mask
            item.target = delegate
        }

        // Terminal menu: splits, tabs and pane focus.
        let terminalItem = NSMenuItem()
        mainMenu.addItem(terminalItem)
        let terminalMenu = NSMenu(title: "Terminal")
        terminalItem.submenu = terminalMenu
        add(terminalMenu, "Split Right", #selector(AppDelegate.splitRight(_:)), .splitRight)
        add(terminalMenu, "Split Down", #selector(AppDelegate.splitDown(_:)), .splitDown)
        terminalMenu.addItem(.separator())
        add(terminalMenu, "New Tab", #selector(AppDelegate.newTab(_:)), .newTab)
        add(terminalMenu, "Close Tab", #selector(AppDelegate.closeFocusedSurface(_:)), .closeSurface)
        terminalMenu.addItem(.separator())
        add(terminalMenu, "Next Tab", #selector(AppDelegate.nextTab(_:)), .nextTab)
        add(terminalMenu, "Previous Tab", #selector(AppDelegate.previousTab(_:)), .previousTab)
        add(terminalMenu, "Next Pane", #selector(AppDelegate.nextPane(_:)), .nextPane)
        add(terminalMenu, "Previous Pane", #selector(AppDelegate.previousPane(_:)), .previousPane)

        // Workspace menu: creation plus 1..9 jump targets.
        let workspaceItem = NSMenuItem()
        mainMenu.addItem(workspaceItem)
        let workspaceMenu = NSMenu(title: "Workspace")
        workspaceItem.submenu = workspaceMenu
        add(workspaceMenu, "New Workspace", #selector(AppDelegate.newWorkspace(_:)), .newWorkspace)
        workspaceMenu.addItem(.separator())
        for index in 1...9 {
            let item = workspaceMenu.addItem(
                withTitle: "Workspace \(index)",
                action: #selector(AppDelegate.selectWorkspace(_:)),
                keyEquivalent: "\(index)")
            item.keyEquivalentModifierMask = [.command]
            item.tag = index - 1
            item.target = delegate
        }

        // View menu.
        let viewItem = NSMenuItem()
        mainMenu.addItem(viewItem)
        let viewMenu = NSMenu(title: "View")
        viewItem.submenu = viewMenu
        add(viewMenu, "Command Palette", #selector(AppDelegate.showPalette(_:)), .palette)
        add(viewMenu, "Toggle Sidebar", #selector(AppDelegate.toggleSidebar(_:)), .toggleSidebar)

        // Window menu. ⌘W is the tab above; ⌘⇧W is the window itself.
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
