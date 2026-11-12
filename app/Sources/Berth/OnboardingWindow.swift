// One-screen first-run orientation. Deliberately short: where the terminal is,
// how splits/tabs work, and the two attention shortcuts that make agents
// useful.

import AppKit

@MainActor
final class OnboardingWindow: NSObject, NSWindowDelegate {
    private static let seenKey = "berth.onboarding.seen"
    private static var active: OnboardingWindow?

    private let window: NSWindow

    static func presentIfNeeded() {
        guard !UserDefaults.standard.bool(forKey: seenKey) else { return }
        UserDefaults.standard.set(true, forKey: seenKey)
        let controller = OnboardingWindow()
        active = controller
        controller.window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    private override init() {
        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 560, height: 390),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false)
        super.init()
        window.title = "Welcome to Berth"
        window.isReleasedWhenClosed = false
        window.delegate = self

        let content = NSView()
        window.contentView = content

        let title = NSTextField(labelWithString: "Berth")
        title.font = Theme.Fonts.onboardingTitle
        title.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(title)

        let subtitle = NSTextField(
            labelWithString: "Terminals, workspaces, and agents that stay visible.")
        subtitle.font = .systemFont(ofSize: 13)
        subtitle.textColor = Theme.Colors.textSecondary
        subtitle.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(subtitle)

        let bullets = [
            "⌘D / ⌘⇧D split right and down. ⌘T opens a tab; ⌘W closes one.",
            "⌘N creates a workspace. Drag rows to reorder; ⌘1…⌘9 jump.",
            "Agents show a provider glyph and state in the sidebar. Run `berth run -- <cli>` to make one resumable.",
            "⌘⇧U jumps to the newest agent needing attention; ⌘I lists unread attention.",
            "Quit and relaunch: panes, tabs, cwd, and agent sessions come back.",
        ]
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false
        for bullet in bullets {
            let label = NSTextField(wrappingLabelWithString: "•  \(bullet)")
            label.font = .systemFont(ofSize: 12.5)
            label.textColor = Theme.Colors.textPrimary
            stack.addArrangedSubview(label)
        }
        content.addSubview(stack)

        let close = NSButton(title: "Get started", target: self, action: #selector(dismiss))
        close.keyEquivalent = "\r"
        close.bezelStyle = .rounded
        close.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(close)

        NSLayoutConstraint.activate([
            title.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 28),
            title.topAnchor.constraint(equalTo: content.topAnchor, constant: 26),
            subtitle.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            subtitle.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 2),
            stack.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -28),
            stack.topAnchor.constraint(equalTo: subtitle.bottomAnchor, constant: 22),
            close.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -28),
            close.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -22),
        ])
    }

    func windowWillClose(_ notification: Notification) {
        if OnboardingWindow.active === self {
            OnboardingWindow.active = nil
        }
    }

    @objc private func dismiss() {
        window.close()
    }
}
