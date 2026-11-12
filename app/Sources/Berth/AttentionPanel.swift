// A small attention popover: unread agent notifications, newest first, with
// one-click jump + acknowledgement. This is the ⌘I surface; the command
// palette remains for verbs.

import AppKit

struct AttentionItem {
    let id: String
    let title: String
    let body: String
    let workspaceID: String?
    let surfaceID: String?
}

@MainActor
final class AttentionPanel: NSObject, NSTableViewDataSource, NSTableViewDelegate,
                           NSWindowDelegate {
    private static var active: AttentionPanel?

    private let items: [AttentionItem]
    private let onSelect: @MainActor (AttentionItem) -> Void
    private let panel: NSPanel
    private let table = NSTableView()

    static func present(
        items: [AttentionItem],
        onSelect: @escaping @MainActor (AttentionItem) -> Void
    ) {
        active?.close()
        let controller = AttentionPanel(items: items, onSelect: onSelect)
        active = controller
        controller.show()
    }

    private init(items: [AttentionItem], onSelect: @escaping @MainActor (AttentionItem) -> Void) {
        self.items = items
        self.onSelect = onSelect
        panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 440, height: 300),
            styleMask: [.titled, .fullSizeContentView, .nonactivatingPanel],
            backing: .buffered,
            defer: false)
        super.init()
        panel.titleVisibility = .hidden
        panel.titlebarAppearsTransparent = true
        panel.isMovableByWindowBackground = true
        panel.isReleasedWhenClosed = false
        panel.delegate = self
        panel.level = .floating

        let content = NSView()
        panel.contentView = content

        let header = NSTextField(labelWithString: "Attention")
        header.font = .systemFont(ofSize: 15, weight: .semibold)
        header.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(header)

        let hint = NSTextField(labelWithString: "Double-click to jump and acknowledge")
        hint.font = .systemFont(ofSize: 11)
        hint.textColor = Theme.Colors.textSecondary
        hint.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(hint)

        table.headerView = nil
        table.rowHeight = 44
        table.intercellSpacing = NSSize(width: 0, height: 2)
        table.backgroundColor = .clear
        table.dataSource = self
        table.delegate = self
        table.allowsEmptySelection = false
        table.target = self
        table.doubleAction = #selector(jump(_:))
        table.addTableColumn(NSTableColumn(identifier: NSUserInterfaceItemIdentifier("attention")))

        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(scroll)

        NSLayoutConstraint.activate([
            header.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 16),
            header.topAnchor.constraint(equalTo: content.topAnchor, constant: 14),
            hint.leadingAnchor.constraint(equalTo: header.leadingAnchor),
            hint.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 2),
            scroll.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: hint.bottomAnchor, constant: 10),
            scroll.bottomAnchor.constraint(equalTo: content.bottomAnchor),
        ])
        if !items.isEmpty {
            table.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        }
    }

    private func show() {
        guard let screen = NSScreen.main else { return }
        let frame = panel.frame
        panel.setFrameOrigin(NSPoint(
            x: screen.visibleFrame.maxX - frame.width - 24,
            y: screen.visibleFrame.maxY - frame.height - 24))
        panel.makeKeyAndOrderFront(nil)
        panel.makeFirstResponder(table)
        NSApp.activate(ignoringOtherApps: true)
    }

    private func close() {
        panel.orderOut(nil)
        if AttentionPanel.active === self {
            AttentionPanel.active = nil
        }
    }

    func windowDidResignKey(_ notification: Notification) {
        close()
    }

    @objc private func jump(_ sender: Any?) {
        guard items.indices.contains(table.selectedRow) else { return }
        let item = items[table.selectedRow]
        close()
        onSelect(item)
    }

    func numberOfRows(in tableView: NSTableView) -> Int { items.count }

    func tableView(
        _ tableView: NSTableView,
        viewFor tableColumn: NSTableColumn?,
        row: Int
    ) -> NSView? {
        let item = items[row]
        let title = NSTextField(labelWithString: item.title)
        title.font = .systemFont(ofSize: 12, weight: .medium)
        title.lineBreakMode = .byTruncatingTail
        let body = NSTextField(labelWithString: item.body)
        body.font = .systemFont(ofSize: 10.5)
        body.textColor = Theme.Colors.textSecondary
        body.lineBreakMode = .byTruncatingTail
        body.isHidden = item.body.isEmpty

        let cell = NSTableCellView()
        title.translatesAutoresizingMaskIntoConstraints = false
        body.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(title)
        cell.addSubview(body)
        NSLayoutConstraint.activate([
            title.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 14),
            title.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -14),
            title.topAnchor.constraint(equalTo: cell.topAnchor, constant: 6),
            body.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            body.trailingAnchor.constraint(equalTo: title.trailingAnchor),
            body.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 1),
        ])
        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) {}
}
