// A find bar for the focused terminal, backed by Ghostty's own incremental
// search. Ghostty owns the matching thread and the highlighting; this file is
// the box, the count and the next/previous navigation.
//
// The panel is a child of the terminal window rather than a separate window, so
// it follows the window's moves and never steals focus from the shell. Escape
// ends the search and returns focus to the terminal, which is where the user
// was before they started typing.

import AppKit
import GhosttyKit

@MainActor
final class SearchBar: NSObject, NSSearchFieldDelegate {
    private static var active: SearchBar?

    private let panel: NSPanel
    private let field = NSSearchField()
    private let count = NSTextField(labelWithString: "")
    private let surfaceID: String
    private let surfaceView: GhosttySurfaceView
    /// The terminal's key window, so the bar is positioned and closed with it.
    private weak var parent: NSWindow?

    /// `total` and `selected` come from Ghostty's search actions; `selected`
    /// is one-based and zero means nothing is selected yet.
    private var total = 0
    private var selected = 0
    private var isClosing = false

    static func present(for view: GhosttySurfaceView) {
        active?.close()
        guard let surfaceID = view.berthSurfaceID, let parent = view.window else { return }
        let bar = SearchBar(surfaceID: surfaceID, view: view, parent: parent)
        active = bar
        bar.show()
    }

    /// Route a Ghostty search action to the open bar, if it belongs to it.
    /// Returns false when the action is not a search action, so the caller can
    /// keep falling through to its own switch.
    @discardableResult
    static func handle(surfaceID: String, action: ghostty_action_s) -> Bool {
        guard let bar = active, bar.surfaceID == surfaceID else { return false }
        switch action.tag {
        case GHOSTTY_ACTION_START_SEARCH:
            bar.show()
            return true
        case GHOSTTY_ACTION_END_SEARCH:
            bar.close()
            return true
        case GHOSTTY_ACTION_SEARCH_TOTAL:
            bar.total = Int(action.action.search_total.total)
            bar.refresh()
            return true
        case GHOSTTY_ACTION_SEARCH_SELECTED:
            // Ghostty reports -1 for "no selection".
            let value = Int(action.action.search_selected.selected)
            bar.selected = max(0, value + 1)
            bar.refresh()
            return true
        default:
            return false
        }
    }

    private init(surfaceID: String, view: GhosttySurfaceView, parent: NSWindow) {
        self.surfaceID = surfaceID
        self.surfaceView = view
        self.parent = parent
        panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 360, height: 40),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false)
        super.init()

        panel.isFloatingPanel = true
        panel.hidesOnDeactivate = false
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = true
        panel.animationBehavior = .none
        panel.isReleasedWhenClosed = false

        let content = NSView()
        content.wantsLayer = true
        content.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        content.layer?.cornerRadius = 8
        content.layer?.borderWidth = 1
        content.layer?.borderColor = NSColor.separatorColor.cgColor
        panel.contentView = content

        field.placeholderString = "Find"
        field.font = .systemFont(ofSize: 13)
        field.delegate = self
        field.sendsSearchStringImmediately = true
        field.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(field)

        count.font = .monospacedDigitSystemFont(ofSize: 11, weight: .regular)
        count.textColor = .secondaryLabelColor
        count.alignment = .center
        count.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(count)

        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 10),
            field.topAnchor.constraint(equalTo: content.topAnchor, constant: 7),
            field.bottomAnchor.constraint(equalTo: content.bottomAnchor, constant: -7),
            count.leadingAnchor.constraint(equalTo: field.trailingAnchor, constant: 8),
            count.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -10),
            count.centerYAnchor.constraint(equalTo: content.centerYAnchor),
            count.widthAnchor.constraint(greaterThanOrEqualToConstant: 54),
        ])
    }

    private func show() {
        guard let parent else { return }
        if panel.parent == nil {
            parent.addChildWindow(panel, ordered: .above)
        }
        let frame = parent.frame
        let size = panel.frame.size
        // Top-right of the terminal, the corner a find bar sits in on macOS.
        panel.setFrameOrigin(NSPoint(
            x: frame.maxX - size.width - 16,
            y: frame.maxY - size.height - 16))
        panel.orderFront(nil)
        parent.makeFirstResponder(field)
        field.selectText(nil)
    }

    private func close() {
        guard !isClosing else { return }
        isClosing = true
        run("end_search")
        if let parent {
            parent.removeChildWindow(panel)
            parent.makeFirstResponder(surfaceView)
        }
        panel.orderOut(nil)
        if SearchBar.active === self {
            SearchBar.active = nil
        }
    }

    private func refresh() {
        if total == 0 {
            count.stringValue = field.stringValue.isEmpty ? "" : "no results"
        } else {
            count.stringValue = "\(min(selected, total)) / \(total)"
        }
    }

    /// Run a Ghostty binding action on the surface this bar belongs to.
    @discardableResult
    private func run(_ name: String) -> Bool {
        surfaceView.bindingAction(name)
    }

    /// Ghostty's `search` action takes its needle after a colon, so the bar
    /// drives the same code path the keybindings do.
    private func search(_ text: String) {
        run("search:\(text)")
        refresh()
    }

    func controlTextDidChange(_ notification: Notification) {
        search(field.stringValue)
    }

    func control(
        _ control: NSControl,
        textView: NSTextView,
        doCommandBy commandSelector: Selector
    ) -> Bool {
        switch commandSelector {
        case #selector(NSResponder.insertNewline(_:)):
            run("navigate_search:next")
            return true
        case #selector(NSResponder.insertBacktab(_:)):
            run("navigate_search:previous")
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            close()
            return true
        default:
            return false
        }
    }
}
