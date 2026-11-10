// One window per workspace, rendering the core's pane tree.
//
// The tree is rebuilt from scratch on every layout change and the surface
// views are re-parented into it. That is deliberately dumb: a workspace has a
// handful of panes, and rebuilding avoids a whole class of reconciliation bugs
// (stale views, half-moved dividers). Ghostty surfaces survive re-parenting
// because they are just NSViews owned by BerthCoreHost.

import AppKit

final class WorkspaceWindow: NSWindow {
    let workspaceID: String

    init(workspaceID: String, name: String) {
        self.workspaceID = workspaceID
        super.init(
            contentRect: NSRect(x: 0, y: 0, width: 1000, height: 640),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false)
        title = name
        minSize = NSSize(width: 320, height: 200)
        isReleasedWhenClosed = false
        collectionBehavior = [.fullScreenPrimary]
        cascadeTopLeft(from: NSPoint(x: 40, y: 40))
        contentView = NSView(frame: contentRect(forFrameRect: frame))
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }

    /// Replace the pane tree. Surface views not in `views` are simply dropped.
    func apply(layout: WorkspaceLayout, views: [String: GhosttySurfaceView]) {
        title = layout.name
        let container = NSView(frame: contentView?.bounds ?? .zero)
        container.autoresizingMask = [.width, .height]
        if let node = LayoutBuilder.build(layout.root, views: views) {
            node.frame = container.bounds
            node.autoresizingMask = [.width, .height]
            container.addSubview(node)
        }
        contentView = container

        if isKeyWindow || NSApp.keyWindow == nil {
            let target = layout.activeSurface
                ?? layout.root.firstLeafSurface()
            if let target, let view = views[target] {
                makeFirstResponder(view)
            }
        }
    }
}

/// Builds NSSplitView/leaf views from the core's tree. Leaves host the actual
/// Ghostty surface views; everything else is a split with draggable dividers.
enum LayoutBuilder {
    static func build(_ node: WorkspaceLayout.Node, views: [String: GhosttySurfaceView]) -> NSView? {
        switch node {
        case let .leaf(_, activeSurface):
            guard let surfaceID = activeSurface, let view = views[surfaceID] else {
                let placeholder = PlaceholderPane()
                return placeholder
            }
            let pane = PaneView(surfaceID: surfaceID, surface: view)
            return pane

        case let .split(_, isVertical, children):
            let split = NSSplitView()
            split.isVertical = isVertical
            split.dividerStyle = .thin
            split.autosaveName = nil
            for child in children {
                guard let subview = build(child, views: views) else { continue }
                split.addArrangedSubview(subview)
            }
            guard split.arrangedSubviews.count > 0 else { return nil }
            return split
        }
    }
}

/// A pane holding one terminal. The border marks the focused pane; it is the
/// cheapest possible "where is my cursor" cue before the sidebar exists.
final class PaneView: NSView {
    let surfaceID: String

    init(surfaceID: String, surface: GhosttySurfaceView) {
        self.surfaceID = surfaceID
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = NSColor.black.cgColor
        surface.frame = bounds
        surface.autoresizingMask = [.width, .height]
        addSubview(surface)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override func layout() {
        super.layout()
        subviews.first?.frame = bounds
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
