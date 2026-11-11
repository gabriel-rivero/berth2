// Workspace sidebar. Rows come straight from the core's workspace list:
// title/cwd/unread/pinned/group plus shell-derived git branch and listening
// ports. Selection and drag-reorder turn into protocol calls; this file owns no
// workspace state.

import AppKit

struct SidebarAgent {
    let id: String
    let workspaceID: String
    let surfaceID: String
    let provider: String
    let name: String
    let state: String
    let detail: String
    let progress: Double?
    let attention: String
    let sinceMs: UInt64

    var glyph: String {
        switch provider {
        case "claude-code": return "✦"
        case "codex": return "⌬"
        case "opencode": return "◇"
        default: return "•"
        }
    }

    var stateLabel: String {
        switch state {
        case "thinking": return "thinking"
        case "tool": return "tool"
        case "waiting_input": return "waiting"
        case "blocked": return "needs input"
        case "done": return "done"
        case "errored": return "error"
        case "idle": return "idle"
        default: return "detected"
        }
    }

    var needsAttention: Bool { attention == "needs_input" || attention == "error" }

    var attentionRank: Int {
        switch attention {
        case "needs_input": return 3
        case "error": return 2
        case "info": return 1
        default: return 0
        }
    }
}

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
    var agent: SidebarAgent? = nil
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

/// Table with a hook for double-clicks on empty space (new workspace).
final class SidebarTable: NSTableView {
    var onBackgroundDoubleClick: (@MainActor () -> Void)?

    override func mouseDown(with event: NSEvent) {
        if event.clickCount == 2 {
            let point = convert(event.locationInWindow, from: nil)
            if row(at: point) == -1 {
                onBackgroundDoubleClick?()
                return
            }
        }
        super.mouseDown(with: event)
    }
}

final class SidebarView: NSView, NSTableViewDataSource, NSTableViewDelegate {
    private enum Row {
        case group(SidebarGroupInfo)
        case workspace(SidebarWorkspace)
    }

    var onSelect: (@MainActor (String) -> Void)?
    var onMove: (@MainActor (String, Int) -> Void)?
    var onToggleGroup: (@MainActor (String, Bool) -> Void)?
    var onNewWorkspace: (@MainActor () -> Void)?
    var onClose: (@MainActor (String) -> Void)?

    private let table = SidebarTable()
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

        let add = NSButton(title: "+", target: self, action: #selector(newWorkspace(_:)))
        add.bezelStyle = .inline
        add.isBordered = false
        add.font = .systemFont(ofSize: 13, weight: .medium)
        add.toolTip = "New workspace (⌘N)"
        add.translatesAutoresizingMaskIntoConstraints = false
        addSubview(add)

        table.headerView = nil
        table.rowHeight = 62
        table.intercellSpacing = NSSize(width: 0, height: 2)
        table.backgroundColor = .clear
        table.selectionHighlightStyle = .sourceList
        table.allowsMultipleSelection = false
        table.dataSource = self
        table.delegate = self
        table.addTableColumn(NSTableColumn(identifier: NSUserInterfaceItemIdentifier("workspace")))
        table.registerForDraggedTypes([.string])
        table.target = self
        table.doubleAction = #selector(doubleClick(_:))
        table.onBackgroundDoubleClick = { [weak self] in self?.onNewWorkspace?() }

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
            add.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            add.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            add.widthAnchor.constraint(equalToConstant: 20),
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

    /// Pinned first, then agents that need attention, then recency order from
    /// the core. The sort key carries the original index so equal ranks keep
    /// their workspace order.
    private func ordered(_ workspaces: [SidebarWorkspace]) -> [SidebarWorkspace] {
        workspaces.enumerated()
            .sorted { lhs, rhs in
                if lhs.element.pinned != rhs.element.pinned {
                    return lhs.element.pinned
                }
                let l = lhs.element.agent?.attentionRank ?? 0
                let r = rhs.element.agent?.attentionRank ?? 0
                if l != r { return l > r }
                return lhs.offset < rhs.offset
            }
            .map(\.element)
    }

    @objc private func newWorkspace(_ sender: Any?) {
        onNewWorkspace?()
    }

    /// Double-clicking empty sidebar space creates a workspace; double-clicking
    /// a row just keeps it selected (single click already opens it).
    @objc private func doubleClick(_ sender: Any?) {
        guard table.clickedRow < 0 else { return }
        onNewWorkspace?()
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
            view.onClose = { [weak self] in self?.onClose?(workspace.id) }
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

/// A workspace row: attention dot, live title, agent state, cwd/branch/ports.
final class WorkspaceRowView: NSTableCellView {
    var onClose: (@MainActor () -> Void)?
    private let dot = NSView()
    private let title = NSTextField(labelWithString: "")
    private let agentLine = NSTextField(labelWithString: "")
    private let detail = NSTextField(labelWithString: "")
    private let progress = NSProgressIndicator()

    override init(frame: NSRect) {
        super.init(frame: frame)
        dot.wantsLayer = true
        dot.layer?.cornerRadius = 3
        dot.translatesAutoresizingMaskIntoConstraints = false
        title.font = .systemFont(ofSize: 12, weight: .medium)
        title.lineBreakMode = .byTruncatingTail
        title.translatesAutoresizingMaskIntoConstraints = false
        agentLine.font = .systemFont(ofSize: 10.5)
        agentLine.lineBreakMode = .byTruncatingTail
        agentLine.translatesAutoresizingMaskIntoConstraints = false
        detail.font = .systemFont(ofSize: 10)
        detail.textColor = .secondaryLabelColor
        detail.lineBreakMode = .byTruncatingTail
        detail.translatesAutoresizingMaskIntoConstraints = false
        progress.isIndeterminate = false
        progress.style = .bar
        progress.controlSize = .small
        progress.minValue = 0
        progress.maxValue = 100
        progress.translatesAutoresizingMaskIntoConstraints = false
        addSubview(dot)
        addSubview(title)
        addSubview(agentLine)
        addSubview(detail)
        addSubview(progress)
        NSLayoutConstraint.activate([
            dot.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 13),
            dot.topAnchor.constraint(equalTo: topAnchor, constant: 9),
            dot.widthAnchor.constraint(equalToConstant: 6),
            dot.heightAnchor.constraint(equalToConstant: 6),
            title.leadingAnchor.constraint(equalTo: dot.trailingAnchor, constant: 7),
            title.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            title.topAnchor.constraint(equalTo: topAnchor, constant: 5),
            agentLine.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            agentLine.trailingAnchor.constraint(equalTo: title.trailingAnchor),
            agentLine.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 1),
            detail.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            detail.trailingAnchor.constraint(equalTo: title.trailingAnchor),
            detail.topAnchor.constraint(equalTo: agentLine.bottomAnchor, constant: 1),
            progress.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            progress.trailingAnchor.constraint(equalTo: title.trailingAnchor),
            progress.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -4),
            progress.heightAnchor.constraint(equalToConstant: 3),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    /// Middle-click closes the workspace, matching tab-bar behaviour.
    override func otherMouseDown(with event: NSEvent) {
        guard event.buttonNumber == 2 else {
            super.otherMouseDown(with: event)
            return
        }
        onClose?()
    }

    func configure(_ workspace: SidebarWorkspace) {
        let agent = workspace.agent
        let needsAttention = agent?.needsAttention ?? false
        dot.isHidden = workspace.unread == 0 && !needsAttention
        if let attention = agent?.attention, attention == "error" {
            dot.layer?.backgroundColor = NSColor.systemRed.cgColor
        } else if let attention = agent?.attention, attention == "needs_input" {
            dot.layer?.backgroundColor = NSColor.systemOrange.cgColor
        } else {
            dot.layer?.backgroundColor = NSColor.systemRed.cgColor
        }

        let live = workspace.title.trimmingCharacters(in: .whitespacesAndNewlines)
        let label = live.isEmpty ? workspace.name : live
        title.stringValue = workspace.pinned ? "★ \(label)" : label

        if let agent {
            let text = NSMutableAttributedString(
                string: "\(agent.glyph) \(agent.name) · ",
                attributes: [.foregroundColor: NSColor.secondaryLabelColor]
            )
            text.append(NSAttributedString(
                string: agent.stateLabel,
                attributes: [
                    .foregroundColor: Self.stateColor(agent),
                    .font: NSFont.systemFont(ofSize: 10.5, weight: .semibold),
                ]
            ))
            if !agent.detail.isEmpty {
                text.append(NSAttributedString(
                    string: " · \(agent.detail)",
                    attributes: [.foregroundColor: NSColor.tertiaryLabelColor]
                ))
            }
            agentLine.attributedStringValue = text
            agentLine.isHidden = false
        } else {
            agentLine.stringValue = ""
            agentLine.isHidden = true
        }

        var parts: [String] = []
        if !workspace.name.isEmpty { parts.append(workspace.name) }
        if !workspace.cwd.isEmpty { parts.append(abbreviate(workspace.cwd)) }
        if let branch = workspace.branch, !branch.isEmpty { parts.append("⑂ \(branch)") }
        if !workspace.ports.isEmpty {
            parts.append(workspace.ports.prefix(3).map { ":\($0)" }.joined(separator: " "))
        }
        detail.stringValue = parts.joined(separator: "  ")

        if let value = agent?.progress {
            progress.doubleValue = value <= 1 ? value * 100 : value
            progress.isHidden = false
        } else {
            progress.isHidden = true
        }
    }

    private static func stateColor(_ agent: SidebarAgent) -> NSColor {
        switch agent.state {
        case "blocked", "waiting_input": return .systemOrange
        case "errored": return .systemRed
        case "done": return .systemGreen
        case "unknown": return .tertiaryLabelColor
        default: return .controlAccentColor
        }
    }

    private func abbreviate(_ path: String) -> String {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        if path == home { return "~" }
        if path.hasPrefix(home + "/") { return "~" + path.dropFirst(home.count) }
        return path
    }
}


