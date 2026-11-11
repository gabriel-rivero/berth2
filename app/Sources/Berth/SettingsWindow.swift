// Minimal settings sheet: one editable shortcut per action, persisted to
// ~/.config/berth/config.json. Applying rebuilds the menu bar so new key
// equivalents take effect without a relaunch.

import AppKit

@MainActor
final class SettingsWindow: NSObject, NSWindowDelegate {
    private static var active: SettingsWindow?

    private let window: NSWindow
    private var fields: [ShortcutAction: NSTextField] = [:]

    static func present() {
        if let active {
            active.window.makeKeyAndOrderFront(nil)
            return
        }
        let controller = SettingsWindow()
        active = controller
        controller.window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    private override init() {
        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 460, height: 520),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false)
        super.init()
        window.title = "Berth Settings"
        window.isReleasedWhenClosed = false
        window.delegate = self

        let content = NSView()
        window.contentView = content

        let title = NSTextField(labelWithString: "Keyboard Shortcuts")
        title.font = .systemFont(ofSize: 15, weight: .semibold)
        title.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(title)

        let hint = NSTextField(labelWithString: "Format: cmd+shift+p  ·  leave empty for the default")
        hint.font = .systemFont(ofSize: 11)
        hint.textColor = .secondaryLabelColor
        hint.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(hint)

        let grid = NSGridView()
        grid.rowSpacing = 8
        grid.columnSpacing = 12
        for action in ShortcutAction.allCases {
            let label = NSTextField(labelWithString: action.title)
            label.alignment = .right
            let field = NSTextField(string: AppConfig.shared.rawValue(for: action))
            field.placeholderString = action.defaultShortcut
            field.translatesAutoresizingMaskIntoConstraints = false
            field.widthAnchor.constraint(equalToConstant: 220).isActive = true
            fields[action] = field
            grid.addRow(with: [label, field])
        }
        grid.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(grid)

        let save = NSButton(title: "Save", target: self, action: #selector(save))
        save.keyEquivalent = "\r"
        save.bezelStyle = .rounded
        save.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(save)

        let cancel = NSButton(title: "Cancel", target: self, action: #selector(cancel))
        cancel.keyEquivalent = "\u{1b}"
        cancel.bezelStyle = .rounded
        cancel.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(cancel)

        NSLayoutConstraint.activate([
            title.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 20),
            title.topAnchor.constraint(equalTo: content.topAnchor, constant: 20),
            hint.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            hint.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 2),
            grid.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 20),
            grid.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -20),
            grid.topAnchor.constraint(equalTo: hint.bottomAnchor, constant: 16),
            save.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -20),
            save.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -16),
            cancel.trailingAnchor.constraint(equalTo: save.leadingAnchor, constant: -8),
            cancel.centerYAnchor.constraint(equalTo: save.centerYAnchor),
        ])
    }

    func windowWillClose(_ notification: Notification) {
        if SettingsWindow.active === self {
            SettingsWindow.active = nil
        }
    }

    @objc private func save() {
        for (action, field) in fields {
            let value = field.stringValue.trimmingCharacters(in: .whitespaces)
            if !value.isEmpty, Shortcut.parse(value) == nil {
                NSSound.beep()
                field.becomeFirstResponder()
                return
            }
            AppConfig.shared.setShortcut(value, for: action)
        }
        do {
            try AppConfig.shared.save()
        } catch {
            let alert = NSAlert()
            alert.messageText = "Could not save settings"
            alert.informativeText = error.localizedDescription
            alert.runModal()
            return
        }
        MainMenu.install()
        window.close()
    }

    @objc private func cancel() {
        window.close()
    }
}
