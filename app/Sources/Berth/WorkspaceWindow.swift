// The single application window: workspace sidebar on the left, the selected
// workspace's pane tree on the right.
//
// The pane tree is rebuilt on every structural layout change and the Ghostty
// surface views are re-parented into it. That is deliberately dumb: a
// workspace has a handful of panes, and rebuilding avoids a whole class of
// reconciliation bugs (stale views, half-moved dividers). Surface views
// survive re-parenting because they are owned by BerthCoreHost.

import AppKit
import ObjectiveC
import QuartzCore

struct PaneActions {
    let activate: @MainActor (String) -> Void
    let close: @MainActor (String) -> Void
    let newTab: @MainActor (String) -> Void
    /// `(split_id, child_index, new share)` after a divider drag.
    let resizeSplit: @MainActor (String, Int, Double) -> Void
}

final class MainWindow: NSWindow {
    let sidebar = SidebarView()
    private let contentHost = NSView()
    private let splitView = NSSplitView()

    init() {
        super.init(
            contentRect: NSRect(x: 0, y: 0, width: 1180, height: 720),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
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

    /// Flash the pane that holds a surface, used when an agent starts needing
    /// attention while the user is looking elsewhere.
    func ring(surfaceID: String) {
        guard let pane = Self.pane(in: contentHost, containing: surfaceID) else { return }
        pane.ring()
    }

    private static func pane(in view: NSView, containing surfaceID: String) -> PaneView? {
        if let pane = view as? PaneView, pane.surfaceIDs.contains(surfaceID) {
            return pane
        }
        for child in view.subviews {
            if let found = Self.pane(in: child, containing: surfaceID) { return found }
        }
        return nil
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

        case let .split(id, isVertical, children, weights):
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
            let coordinator = SplitCoordinator(
                splitID: id,
                isVertical: isVertical,
                weights: weights,
                actions: actions)
            split.delegate = coordinator
            objc_setAssociatedObject(
                split, &splitCoordinatorKey, coordinator,
                .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
            // NSSplitView has no weight API. Apply the persisted fractions
            // once the view has a real size, then let the delegate feed drags
            // back into the core.
            DispatchQueue.main.async {
                coordinator.applyWeights(to: split)
            }
            return split
        }
    }
}

private nonisolated(unsafe) var splitCoordinatorKey: UInt8 = 0

/// Owns divider positions for one NSSplitView and reports drags to the core.
/// It is retained by the split view itself (associated object) because
/// NSSplitView's delegate is weak.
@MainActor
final class SplitCoordinator: NSObject, NSSplitViewDelegate {
    private let splitID: String
    private let isVertical: Bool
    private var weights: [Double]
    private let actions: PaneActions
    private var applying = false

    init(splitID: String, isVertical: Bool, weights: [Double], actions: PaneActions) {
        self.splitID = splitID
        self.isVertical = isVertical
        self.weights = weights
        self.actions = actions
    }

    func applyWeights(to split: NSSplitView) {
        guard !applying, split.arrangedSubviews.count > 1 else { return }
        applying = true
        defer { applying = false }
        var normalized = weights
        if normalized.count != split.arrangedSubviews.count {
            normalized = Array(repeating: 1.0 / Double(split.arrangedSubviews.count),
                               count: split.arrangedSubviews.count)
        }
        let total = isVertical ? split.bounds.width : split.bounds.height
        guard total > 0 else { return }
        var origin: CGFloat = 0
        for index in 0..<(split.arrangedSubviews.count - 1) {
            origin += total * CGFloat(normalized[index])
            split.setPosition(origin, ofDividerAt: index)
        }
    }

    func splitViewDidResizeSubviews(_ notification: Notification) {
        guard !applying,
              let split = notification.object as? NSSplitView,
              split.arrangedSubviews.count > 1
        else { return }
        let total = isVertical ? split.bounds.width : split.bounds.height
        guard total > 0 else { return }
        for index in 0..<(split.arrangedSubviews.count - 1) {
            let child = split.arrangedSubviews[index]
            let size = isVertical ? child.frame.width : child.frame.height
            let share = Double(size / total)
            actions.resizeSplit(splitID, index, min(max(share, 0.05), 0.95))
        }
    }
}

/// A pane holding one terminal and an always-visible tab strip. The strip is
/// the only UI the core's `Pane.surfaces` stack needs; keeping the `+` visible
/// for single-tab panes is what makes tab creation discoverable.
final class PaneView: NSView {
    let surfaceID: String
    let surfaceIDs: [String]
    private let terminalHost = NSView()
    private let tabBar: PaneTabBar
    private static let tabHeight = Theme.Metrics.paneTabHeight

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
        surfaceIDs = (pane?.surfaceIDs.isEmpty == false) ? pane!.surfaceIDs : [surfaceID]
        tabBar = PaneView.makeTabBar(
            paneID: paneID,
            pane: pane,
            layout: layout,
            fallbackSurface: surfaceID,
            actions: actions)
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = Theme.Colors.terminalBackground.cgColor
        layer?.borderColor = Theme.Colors.paneBorder.cgColor
        layer?.borderWidth = 1

        addSubview(tabBar)
        terminalHost.wantsLayer = true
        terminalHost.layer?.backgroundColor = Theme.Colors.terminalBackground.cgColor
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
        fallbackSurface: String,
        actions: PaneActions
    ) -> PaneTabBar {
        let ids = (pane?.surfaceIDs.isEmpty == false) ? pane!.surfaceIDs : [fallbackSurface]
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

    func ring() {
        guard let layer else { return }
        layer.borderWidth = 2
        let flash = CABasicAnimation(keyPath: "borderColor")
        flash.fromValue = Theme.Colors.accent.cgColor
        flash.toValue = Theme.Colors.paneBorder.cgColor
        flash.duration = Theme.Metrics.ringDuration
        layer.borderColor = Theme.Colors.paneBorder.cgColor
        layer.add(flash, forKey: "attention")
    }

    override func layout() {
        super.layout()
        tabBar.frame = NSRect(
            x: 0, y: bounds.height - Self.tabHeight,
            width: bounds.width, height: Self.tabHeight)
        terminalHost.frame = NSRect(
            x: 0, y: 0,
            width: bounds.width, height: bounds.height - Self.tabHeight)
    }
}

/// A tab button. Single click activates; double click and middle click close,
/// matching the gestures people expect from browser and terminal tabs.
final class PaneTabButton: NSButton {
    var onClose: (@MainActor () -> Void)?
    var isActiveTab = false {
        didSet { refreshAppearance() }
    }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        isBordered = false
        wantsLayer = true
        layer?.cornerRadius = 4
        font = Theme.Fonts.tab
        cell?.lineBreakMode = .byTruncatingTail
        refreshAppearance()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    private func refreshAppearance() {
        layer?.backgroundColor = (isActiveTab
            ? Theme.Colors.paneActiveTab
            : NSColor.clear).cgColor
        contentTintColor = isActiveTab ? Theme.Colors.textPrimary : Theme.Colors.textSecondary
    }

    override func mouseDown(with event: NSEvent) {
        if event.clickCount == 2 {
            onClose?()
            return
        }
        super.mouseDown(with: event)
    }

    override func otherMouseDown(with event: NSEvent) {
        guard event.buttonNumber == 2 else {
            super.otherMouseDown(with: event)
            return
        }
        onClose?()
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
        layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor

        let tabs = NSStackView()
        tabs.orientation = .horizontal
        tabs.spacing = 4
        tabs.translatesAutoresizingMaskIntoConstraints = false
        for (index, label) in labels.enumerated() {
            let button = PaneTabButton(frame: .zero)
            button.title = label
            button.target = self
            button.action = #selector(activate(_:))
            button.tag = index
            button.isActiveTab = index == activeIndex
            button.onClose = { [weak self] in self?.closeTab(at: index) }
            button.translatesAutoresizingMaskIntoConstraints = false
            button.heightAnchor.constraint(equalToConstant: 20).isActive = true
            button.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
            tabs.addArrangedSubview(button)
        }

        let add = NSButton(title: "+", target: self, action: #selector(newTab(_:)))
        add.bezelStyle = .inline
        add.isBordered = false
        add.font = .systemFont(ofSize: 14, weight: .medium)
        add.toolTip = "New tab (⌘T)"
        add.translatesAutoresizingMaskIntoConstraints = false

        let close = NSButton(title: "×", target: self, action: #selector(close(_:)))
        close.bezelStyle = .inline
        close.isBordered = false
        close.font = .systemFont(ofSize: 14)
        close.toolTip = "Close tab (⌘W)"
        close.translatesAutoresizingMaskIntoConstraints = false

        addSubview(tabs)
        addSubview(add)
        addSubview(close)
        NSLayoutConstraint.activate([
            tabs.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
            tabs.centerYAnchor.constraint(equalTo: centerYAnchor),
            tabs.trailingAnchor.constraint(lessThanOrEqualTo: add.leadingAnchor, constant: -4),
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

    /// Double-clicking the empty part of the strip opens another tab.
    override func mouseDown(with event: NSEvent) {
        if event.clickCount == 2 {
            actions.newTab(paneID)
            return
        }
        super.mouseDown(with: event)
    }

    @objc private func activate(_ sender: PaneTabButton) {
        guard ids.indices.contains(sender.tag) else { return }
        actions.activate(ids[sender.tag])
    }

    private func closeTab(at index: Int) {
        guard ids.indices.contains(index) else { return }
        actions.close(ids[index])
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
        case let .split(_, _, children, _):
            for child in children {
                if let found = child.firstLeafSurface() { return found }
            }
            return nil
        }
    }
}
