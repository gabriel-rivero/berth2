// Workspace sidebar. Rows come straight from the core's workspace list:
// title/cwd/unread/pinned/group plus shell-derived git branch and listening
// ports. Selection and drag-reorder turn into protocol calls; this file owns no
// workspace state.

import AppKit

struct SidebarWorkspace {
    let id: String
    let name: String
    let title: String
    let cwd: String
    let branch: String?
    let ports: [Int]
    let unread: Int
    let pinned: Bool
    let groupID: String?
    let pid: Int?
}

struct SidebarGroupInfo {
    let id: String
    let name: String
    let collapsed: Bool
}

struct SidebarModel {
    var groups: [SidebarGroupInfo] = []
    var workspaces: [SidebarWorkspace] = []
    var selectedWorkspace: String?
}

final class SidebarView: NSView, NSTableViewDataSource, NSTableViewDelegate {
    private enum Row {
        case group(SidebarGroupInfo)
        case workspace(SidebarWorkspace)
    }

    var onSelect: (@MainActor (String) -> Void)?
    var onMove: (@MainActor (String, Int) -> Void)?
    var onToggleGroup: (@MainActor (String, Bool) -> Void)?

    private let table = NSTableView()
    private let scroll = NSScrollView()
    private let empty = NSTextField(labelWithString: "No workspaces")
    private var model = SidebarModel()
    private var rows: [Row] = []
    private var isUpdating = false

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor

        let header = NSTextField(labelWithString: "WORKSPACES")
        header.font = .systemFont(ofSize: 10, weight: .semibold)
        header.textColor = .secondaryLabelColor
        header.translatesAutoresizingMaskIntoConstraints = false
        addSubview(header)

        table.headerView = nil
        table.rowHeight = 50
        table.intercellSpacing = NSSize(width: 0, height: 2)
        table.backgroundColor = .clear
        table.selectionHighlightStyle = .sourceList
        table.allowsMultipleSelection = false
        table.dataSource = self
        table.delegate = self
        table.addTableColumn(NSTableColumn(identifier: NSUserInterfaceItemIdentifier("workspace")))
        table.registerForDraggedTypes([.string])

        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        addSubview(scroll)

        empty.font = .systemFont(ofSize: 12)
        empty.textColor = .tertiaryLabelColor
        empty.translatesAutoresizingMaskIntoConstraints = false
        addSubview(empty)

        NSLayoutConstraint.activate([
            header.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 14),
            header.topAnchor.constraint(equalTo: topAnchor, constant: 12),
            scroll.leadingAnchor.constraint(equalTo: leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: trailingAnchor),
            scroll.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 6),
            scroll.bottomAnchor.constraint(equalTo: bottomAnchor),
            empty.centerXAnchor.constraint(equalTo: centerXAnchor),
            empty.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 24),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    func update(_ model: SidebarModel) {
        self.model = model
        rows = buildRows(model)
        isUpdating = true
        table.reloadData()
        if let id = model.selectedWorkspace,
           let row = rows.firstIndex(where: {
               if case let .workspace(workspace) = $0 { return workspace.id == id }
               return false
           }) {
            table.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        } else {
            table.deselectAll(nil)
        }
        isUpdating = false
        empty.isHidden = !model.workspaces.isEmpty
    }

    private func buildRows(_ model: SidebarModel) -> [Row] {
        var rows: [Row] = []
        let grouped = Set(model.groups.map(\.id))
        for group in model.groups {
            rows.append(.group(group))
            if !group.collapsed {
                rows.append(contentsOf: ordered(model.workspaces.filter { $0.groupID == group.id })
                    .map(Row.workspace))
            }
        }
        let loose = model.workspaces.filter {
            $0.groupID == nil || !grouped.contains($0.groupID!)
        }
        rows.append(contentsOf: ordered(loose).map(Row.workspace))
        return rows
    }

    private func ordered(_ workspaces: [SidebarWorkspace]) -> [SidebarWorkspace] {
        workspaces.filter(\.pinned) + workspaces.filter { !$0.pinned }
    }

    // MARK: - Table

    func numberOfRows(in tableView: NSTableView) -> Int { rows.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        switch rows[row] {
        case let .group(group):
            let view = GroupRowView()
            view.configure(group)
            view.onToggle = { [weak self] collapsed in
                self?.onToggleGroup?(group.id, collapsed)
            }
            return view
        case let .workspace(workspace):
            let view = WorkspaceRowView()
            view.configure(workspace)
            return view
        }
    }

    func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool {
        if case .workspace = rows[row] { return true }
        return false
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        guard !isUpdating else { return }
        let row = table.selectedRow
        guard rows.indices.contains(row), case let .workspace(workspace) = rows[row] else { return }
        if workspace.id != model.selectedWorkspace {
            onSelect?(workspace.id)
        }
    }

    // MARK: - Drag reorder

    func tableView(_ tableView: NSTableView, pasteboardWriterForRow row: Int) -> NSPasteboardWriting? {
        guard case let .workspace(workspace) = rows[row] else { return nil }
        let item = NSPasteboardItem()
        item.setString(workspace.id, forType: .string)
        return item
    }

    func tableView(
        _ tableView: NSTableView,
        validateDrop info: NSDraggingInfo,
        proposedRow row: Int,
        proposedDropOperation dropOperation: NSTableView.DropOperation
    ) -> NSDragOperation {
        guard info.draggingSource as? NSTableView === table else { return [] }
        return .move
    }

    func tableView(
        _ tableView: NSTableView,
        acceptDrop info: NSDraggingInfo,
        row: Int,
        dropOperation: NSTableView.DropOperation
    ) -> Bool {
        guard !rows.isEmpty,
              let id = info.draggingPasteboard.string(forType: .string)
        else { return false }
        let nextID: String? = rows[min(row, rows.count - 1)...].first(where: {
            if case .workspace = $0 { return true }
            return false
        }).flatMap {
            if case let .workspace(workspace) = $0 { return workspace.id }
            return nil
        }
        let index = nextID.flatMap { target in
            model.workspaces.firstIndex { $0.id == target }
        } ?? max(0, model.workspaces.count - 1)
        onMove?(id, index)
        return true
    }
}

/// A group header, clickable to collapse/expand.
final class GroupRowView: NSView {
    var onToggle: (@MainActor (Bool) -> Void)?
    private let label = NSTextField(labelWithString: "")
    private let chevron = NSButton(frame: .zero)
    private var collapsed = false

    override init(frame: NSRect) {
        super.init(frame: frame)
        chevron.isBordered = false
        chevron.bezelStyle = .inline
        chevron.target = self
        chevron.action = #selector(toggle)
        chevron.translatesAutoresizingMaskIntoConstraints = false
        label.font = .systemFont(ofSize: 11, weight: .semibold)
        label.textColor = .secondaryLabelColor
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(chevron)
        addSubview(label)
        NSLayoutConstraint.activate([
            chevron.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            chevron.centerYAnchor.constraint(equalTo: centerYAnchor),
            chevron.widthAnchor.constraint(equalToConstant: 14),
            label.leadingAnchor.constraint(equalTo: chevron.trailingAnchor, constant: 4),
            label.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -10),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    func configure(_ group: SidebarGroupInfo) {
        collapsed = group.collapsed
        label.stringValue = group.name.uppercased()
        chevron.title = collapsed ? "▸" : "▾"
    }

    @objc private func toggle() {
        collapsed.toggle()
        chevron.title = collapsed ? "▸" : "▾"
        onToggle?(collapsed)
    }
}

/// A workspace row: unread dot, live terminal title, cwd + branch + ports.
final class WorkspaceRowView: NSTableCellView {
    private let dot = NSView()
    private let title = NSTextField(labelWithString: "")
    private let detail = NSTextField(labelWithString: "")

    override init(frame: NSRect) {
        super.init(frame: frame)
        dot.wantsLayer = true
        dot.layer?.cornerRadius = 3
        dot.layer?.backgroundColor = NSColor.systemRed.cgColor
        dot.translatesAutoresizingMaskIntoConstraints = false
        title.font = .systemFont(ofSize: 12, weight: .medium)
        title.lineBreakMode = .byTruncatingTail
        title.translatesAutoresizingMaskIntoConstraints = false
        detail.font = .systemFont(ofSize: 10)
        detail.textColor = .secondaryLabelColor
        detail.lineBreakMode = .byTruncatingTail
        detail.translatesAutoresizingMaskIntoConstraints = false
        addSubview(dot)
        addSubview(title)
        addSubview(detail)
        NSLayoutConstraint.activate([
            dot.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 13),
            dot.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            dot.widthAnchor.constraint(equalToConstant: 6),
            dot.heightAnchor.constraint(equalToConstant: 6),
            title.leadingAnchor.constraint(equalTo: dot.trailingAnchor, constant: 7),
            title.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            title.topAnchor.constraint(equalTo: topAnchor, constant: 5),
            detail.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            detail.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            detail.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 2),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    func configure(_ workspace: SidebarWorkspace) {
        dot.isHidden = workspace.unread == 0
        let live = workspace.title.trimmingCharacters(in: .whitespacesAndNewlines)
        let label = live.isEmpty ? workspace.name : live
        title.stringValue = workspace.pinned ? "★ \(label)" : label
        var parts: [String] = []
        if !workspace.name.isEmpty { parts.append(workspace.name) }
        if !workspace.cwd.isEmpty { parts.append(abbreviate(workspace.cwd)) }
        if let branch = workspace.branch, !branch.isEmpty { parts.append("⑂ \(branch)") }
        if !workspace.ports.isEmpty {
            parts.append(workspace.ports.prefix(3).map { ":\($0)" }.joined(separator: " "))
        }
        detail.stringValue = parts.joined(separator: "  ")
    }

    private func abbreviate(_ path: String) -> String {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        if path == home { return "~" }
        if path.hasPrefix(home + "/") { return "~" + path.dropFirst(home.count) }
        return path
    }
}


