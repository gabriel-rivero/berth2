// Keyboard-first command palette: fuzzy-match a handful of shell verbs and run
// the highlighted one. Purely a front end for AppDelegate actions; no state of
// its own beyond the list of commands for the current presentation.

import AppKit

struct PaletteCommand {
    let title: String
    let run: @MainActor () -> Void

    init(_ title: String, run: @escaping @MainActor () -> Void) {
        self.title = title
        self.run = run
    }
}

@MainActor
final class CommandPalette: NSObject, NSTableViewDataSource, NSTableViewDelegate,
                           NSSearchFieldDelegate, NSWindowDelegate {
    private static var active: CommandPalette?

    private let commands: [PaletteCommand]
    private let panel: PalettePanel
    private let search = NSSearchField()
    private let table = NSTableView()
    private var matches: [Int] = []
    private var isClosing = false

    static func present(commands: [PaletteCommand]) {
        active?.close()
        let palette = CommandPalette(commands: commands)
        active = palette
        palette.show()
    }

    private init(commands: [PaletteCommand]) {
        self.commands = commands
        panel = PalettePanel(
            contentRect: NSRect(x: 0, y: 0, width: 540, height: 340),
            styleMask: [.titled, .fullSizeContentView],
            backing: .buffered,
            defer: false)
        super.init()

        panel.titleVisibility = .hidden
        panel.titlebarAppearsTransparent = true
        panel.isMovableByWindowBackground = true
        panel.delegate = self
        panel.isReleasedWhenClosed = false

        let content = NSView()
        panel.contentView = content

        search.placeholderString = "Type a command…"
        search.font = .systemFont(ofSize: 18)
        search.delegate = self
        search.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(search)

        table.headerView = nil
        table.rowHeight = 30
        table.intercellSpacing = NSSize(width: 0, height: 0)
        table.backgroundColor = .clear
        table.dataSource = self
        table.delegate = self
        table.allowsEmptySelection = false
        table.addTableColumn(NSTableColumn(identifier: NSUserInterfaceItemIdentifier("command")))

        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        content.addSubview(scroll)

        NSLayoutConstraint.activate([
            search.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 16),
            search.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -16),
            search.topAnchor.constraint(equalTo: content.topAnchor, constant: 16),
            scroll.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: search.bottomAnchor, constant: 10),
            scroll.bottomAnchor.constraint(equalTo: content.bottomAnchor),
        ])

        refilter("")
    }

    private func show() {
        guard let screen = NSScreen.main else { return }
        let frame = panel.frame
        panel.setFrameOrigin(NSPoint(
            x: screen.visibleFrame.midX - frame.width / 2,
            y: screen.visibleFrame.midY + screen.visibleFrame.height / 4))
        panel.makeKeyAndOrderFront(nil)
        panel.makeFirstResponder(search)
        NSApp.activate(ignoringOtherApps: true)
    }

    private func close() {
        guard !isClosing else { return }
        isClosing = true
        panel.orderOut(nil)
        if CommandPalette.active === self {
            CommandPalette.active = nil
        }
    }

    func windowDidResignKey(_ notification: Notification) {
        close()
    }

    private func refilter(_ query: String) {
        let trimmed = query.trimmingCharacters(in: .whitespaces)
        if trimmed.isEmpty {
            matches = Array(commands.indices)
        } else {
            matches = commands.indices
                .compactMap { index -> (Int, Int)? in
                    guard let score = Self.score(trimmed, commands[index].title) else {
                        return nil
                    }
                    return (index, score)
                }
                .sorted { $0.1 > $1.1 }
                .map(\.0)
        }
        table.reloadData()
        if !matches.isEmpty {
            table.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
            table.scrollRowToVisible(0)
        }
    }

    /// Subsequence match with a bonus for contiguous and word-start hits.
    private static func score(_ query: String, _ candidate: String) -> Int? {
        let needle = Array(query.lowercased())
        let haystack = Array(candidate.lowercased())
        guard !needle.isEmpty else { return 0 }
        var score = 0
        var cursor = 0
        var previous = -2
        for character in needle {
            var found = false
            while cursor < haystack.count {
                if haystack[cursor] == character {
                    score += cursor == previous + 1 ? 8 : 1
                    if cursor == 0 || haystack[cursor - 1] == " " {
                        score += 4
                    }
                    previous = cursor
                    cursor += 1
                    found = true
                    break
                }
                cursor += 1
            }
            if !found { return nil }
        }
        return score - haystack.count / 4
    }

    private func runSelected() {
        guard matches.indices.contains(table.selectedRow) else { return }
        let command = commands[matches[table.selectedRow]]
        close()
        command.run()
    }

    // MARK: - Table

    func numberOfRows(in tableView: NSTableView) -> Int {
        matches.count
    }

    func tableView(
        _ tableView: NSTableView,
        viewFor tableColumn: NSTableColumn?,
        row: Int
    ) -> NSView? {
        let label = NSTextField(labelWithString: commands[matches[row]].title)
        label.font = .systemFont(ofSize: 14)
        label.lineBreakMode = .byTruncatingTail
        let cell = NSTableCellView()
        cell.textField = label
        label.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 14),
            label.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -14),
            label.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
        ])
        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) {}

    // MARK: - Search field

    func controlTextDidChange(_ notification: Notification) {
        refilter(search.stringValue)
    }

    func control(
        _ control: NSControl,
        textView: NSTextView,
        doCommandBy commandSelector: Selector
    ) -> Bool {
        switch commandSelector {
        case #selector(NSResponder.insertNewline(_:)):
            runSelected()
            return true
        case #selector(NSResponder.moveDown(_:)):
            moveSelection(1)
            return true
        case #selector(NSResponder.moveUp(_:)):
            moveSelection(-1)
            return true
        case #selector(NSResponder.cancelOperation(_:)):
            close()
            return true
        default:
            return false
        }
    }

    private func moveSelection(_ step: Int) {
        guard !matches.isEmpty else { return }
        let next = max(0, min(matches.count - 1, table.selectedRow + step))
        table.selectRowIndexes(IndexSet(integer: next), byExtendingSelection: false)
        table.scrollRowToVisible(next)
    }
}

/// Borderless-ish panel that can take keyboard focus while the app is active.
private final class PalettePanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
}
