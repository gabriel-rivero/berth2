// The single application window: workspace sidebar on the left, the selected
// workspace's pane tree on the right.
//
// The pane tree is rebuilt on every structural layout change and the Ghostty
// surface views are re-parented into it. That is deliberately dumb: a
// workspace has a handful of panes, and rebuilding avoids a whole class of
// reconciliation bugs (stale views, half-moved dividers). Surface views
// survive re-parenting because they are owned by BerthCoreHost.

import AppKit

struct PaneActions {
    let activate: @MainActor (String) -> Void
    let close: @MainActor (String) -> Void
    let newTab: @MainActor (String) -> Void
}

final class MainWindow: NSWindow {
    let sidebar = SidebarView()
    private let contentHost = NSView()
    private let splitView = NSSplitView()

    init() {
        super.init(
            contentRect: NSRect(x: 0, y: 0, width: 1180, height: 720),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false)
        title = "Berth"
        minSize = NSSize(width: 640, height: 320)
        isReleasedWhenClosed = false
        collectionBehavior = [.fullScreenPrimary]
        cascadeTopLeft(from: NSPoint(x: 60, y: 60))
        // Terminal surfaces need mouse-moved events for hover, link detection
        // and terminal mouse reporting; AppKit does not deliver them by default.
        acceptsMouseMovedEvents = true

        splitView.isVertical = true
        splitView.dividerStyle = .thin
        splitView.translatesAutoresizingMaskIntoConstraints = false
        sidebar.translatesAutoresizingMaskIntoConstraints = false
        contentHost.translatesAutoresizingMaskIntoConstraints = false
        splitView.addArrangedSubview(sidebar)
        splitView.addArrangedSubview(contentHost)
        sidebar.widthAnchor.constraint(greaterThanOrEqualToConstant: 180).isActive = true
        sidebar.widthAnchor.constraint(lessThanOrEqualToConstant: 360).isActive = true
        splitView.setHoldingPriority(.defaultLow, forSubviewAt: 1)

        let container = NSView(frame: contentRect(forFrameRect: frame))
        container.addSubview(splitView)
        NSLayoutConstraint.activate([
            splitView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            splitView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            splitView.topAnchor.constraint(equalTo: container.topAnchor),
            splitView.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])
        contentView = container
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }

    func updateSidebar(_ model: SidebarModel) {
        sidebar.update(model)
    }

    /// Replace the pane tree. Surface views not in `views` are simply dropped.
    func apply(layout: WorkspaceLayout, views: [String: GhosttySurfaceView], actions: PaneActions) {
        title = layout.name.isEmpty ? "Berth" : layout.name
        let node = LayoutBuilder.build(layout.root, layout: layout, views: views, actions: actions)
        contentHost.subviews.forEach { $0.removeFromSuperview() }
        guard let node else { return }
        node.frame = contentHost.bounds
        node.autoresizingMask = [.width, .height]
        contentHost.addSubview(node)

        if isKeyWindow || NSApp.keyWindow == nil {
            let target = layout.activeSurface ?? layout.root.firstLeafSurface()
            if let target, let view = views[target] {
                makeFirstResponder(view)
            }
        }
    }

    func focusSurface(_ id: String, views: [String: GhosttySurfaceView]) {
        if let view = views[id] {
            makeFirstResponder(view)
        }
    }

    func toggleSidebar() {
        sidebar.isHidden.toggle()
        splitView.adjustSubviews()
    }
}

/// Builds NSSplitView/leaf views from the core's tree. Leaves host the actual
/// Ghostty surface views plus a tab strip; everything else is a split with
/// draggable dividers.
enum LayoutBuilder {
    @MainActor
    static func build(
        _ node: WorkspaceLayout.Node,
        layout: WorkspaceLayout,
        views: [String: GhosttySurfaceView],
        actions: PaneActions
    ) -> NSView? {
        switch node {
        case let .leaf(paneID, activeSurface):
            guard let surfaceID = activeSurface, let view = views[surfaceID] else {
                // Never fail silently: a placeholder is a visible bug report.
                NSLog("berth: no view for pane leaf \(activeSurface ?? "<none>")")
                return PlaceholderPane()
            }
            return PaneView(
                paneID: paneID,
                pane: layout.pane(paneID),
                layout: layout,
                views: views,
                actions: actions,
                surfaceID: surfaceID,
                surface: view)

        case let .split(_, isVertical, children):
            let split = NSSplitView()
            split.isVertical = isVertical
            split.dividerStyle = .thin
            split.autosaveName = nil
            for child in children {
                guard let subview = build(child, layout: layout, views: views, actions: actions) else {
                    continue
                }
                split.addArrangedSubview(subview)
            }
            guard !split.arrangedSubviews.isEmpty else { return nil }
            return split
        }
    }
}

/// A pane holding one terminal plus a tab strip when the pane has more than
/// one tab. The strip is the only UI the core's `Pane.surfaces` stack needs.
final class PaneView: NSView {
    let surfaceID: String
    private let terminalHost = NSView()
    private let tabBar: NSView?
    private static let tabHeight: CGFloat = 28

    init(
        paneID: String,
        pane: PaneSpec?,
        layout: WorkspaceLayout,
        views: [String: GhosttySurfaceView],
        actions: PaneActions,
        surfaceID: String,
        surface: GhosttySurfaceView
    ) {
        self.surfaceID = surfaceID
        let tabs = pane?.surfaceIDs ?? [surfaceID]
        tabBar = tabs.count > 1
            ? PaneView.makeTabBar(paneID: paneID, pane: pane, layout: layout, actions: actions)
            : nil
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = NSColor.black.cgColor
        layer?.borderColor = NSColor.separatorColor.cgColor
        layer?.borderWidth = 1

        if let tabBar { addSubview(tabBar) }
        terminalHost.wantsLayer = true
        terminalHost.layer?.backgroundColor = NSColor.black.cgColor
        addSubview(terminalHost)

        surface.frame = terminalHost.bounds
        surface.autoresizingMask = [.width, .height]
        terminalHost.addSubview(surface)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    private static func makeTabBar(
        paneID: String,
        pane: PaneSpec?,
        layout: WorkspaceLayout,
        actions: PaneActions
    ) -> NSView {
        let ids = pane?.surfaceIDs ?? []
        let labels = ids.enumerated().map { index, id -> String in
            let title = layout.surface(id)?.title ?? ""
            return title.isEmpty ? "term \(index + 1)" : title
        }
        let activeIndex = ids.firstIndex(of: pane?.activeSurface ?? "") ?? 0
        return PaneTabBar(
            paneID: paneID,
            ids: ids,
            labels: labels,
            activeIndex: activeIndex,
            activeSurface: pane?.activeSurface,
            actions: actions)
    }

    override func layout() {
        super.layout()
        let barHeight = tabBar == nil ? 0 : Self.tabHeight
        tabBar?.frame = NSRect(x: 0, y: bounds.height - barHeight, width: bounds.width, height: barHeight)
        terminalHost.frame = NSRect(x: 0, y: 0, width: bounds.width, height: bounds.height - barHeight)
    }
}

/// One pane's tab strip. Each strip is its own target so several panes can show
/// tabs at once without a shared, overwritten callback target.
final class PaneTabBar: NSView {
    private let paneID: String
    private let ids: [String]
    private let activeSurface: String?
    private let actions: PaneActions

    init(
        paneID: String,
        ids: [String],
        labels: [String],
        activeIndex: Int,
        activeSurface: String?,
        actions: PaneActions
    ) {
        self.paneID = paneID
        self.ids = ids
        self.activeSurface = activeSurface
        self.actions = actions
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = NSColor.windowBackgroundColor.withAlphaComponent(0.96).cgColor

        let control = NSSegmentedControl(
            labels: labels,
            trackingMode: .selectOne,
            target: self,
            action: #selector(select(_:)))
        control.segmentStyle = .texturedRounded
        control.selectedSegment = activeIndex
        control.translatesAutoresizingMaskIntoConstraints = false

        let add = NSButton(title: "+", target: self, action: #selector(newTab(_:)))
        add.bezelStyle = .inline
        add.isBordered = false
        add.font = .systemFont(ofSize: 14, weight: .medium)
        add.translatesAutoresizingMaskIntoConstraints = false

        let close = NSButton(title: "×", target: self, action: #selector(close(_:)))
        close.bezelStyle = .inline
        close.isBordered = false
        close.font = .systemFont(ofSize: 14)
        close.translatesAutoresizingMaskIntoConstraints = false

        addSubview(control)
        addSubview(add)
        addSubview(close)
        NSLayoutConstraint.activate([
            control.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
            control.centerYAnchor.constraint(equalTo: centerYAnchor),
            control.trailingAnchor.constraint(lessThanOrEqualTo: add.leadingAnchor, constant: -4),
            add.trailingAnchor.constraint(equalTo: close.leadingAnchor, constant: -2),
            add.centerYAnchor.constraint(equalTo: centerYAnchor),
            add.widthAnchor.constraint(equalToConstant: 20),
            close.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            close.centerYAnchor.constraint(equalTo: centerYAnchor),
            close.widthAnchor.constraint(equalToConstant: 20),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    @objc private func select(_ sender: NSSegmentedControl) {
        let index = sender.selectedSegment
        guard ids.indices.contains(index) else { return }
        actions.activate(ids[index])
    }

    @objc private func newTab(_ sender: Any?) {
        actions.newTab(paneID)
    }

    @objc private func close(_ sender: Any?) {
        guard let id = activeSurface ?? ids.first else { return }
        actions.close(id)
    }
}

/// Shown when the core knows about a pane but no view exists (failed spawn).
final class PlaceholderPane: NSView {
    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }
}

extension WorkspaceLayout.Node {
    /// First surface in tree order — used as a focus fallback.
    func firstLeafSurface() -> String? {
        switch self {
        case let .leaf(_, active):
            return active
        case let .split(_, _, children):
            for child in children {
                if let found = child.firstLeafSurface() { return found }
            }
            return nil
        }
    }
}
