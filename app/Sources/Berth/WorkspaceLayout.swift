// The layout the core owns, decoded for AppKit. The core is the source of
// truth for panes and splits; this file only shapes its JSON into something
// a view tree can be built from (see BerthCoreHost.reconcile).
//
// Shape (from `workspace.layout`):
//   { workspace: {id,name,root,active_surface}, panes: [...], surfaces: [...] }
// where root is either
//   {type:"split", id, orientation:"Horizontal"|"Vertical", children:[...], weights:[...]}
// or
//   {type:"leaf", pane_id}

import Foundation

struct WorkspaceLayout {
    /// Node in the pane tree. `isVertical` is NSSplitView's sense: true means
    /// the divider is vertical and children sit side by side.
    enum Node {
        case split(id: String, isVertical: Bool, children: [Node])
        case leaf(paneID: String, activeSurface: String?)
    }

    let id: String
    let name: String
    let root: Node
    let surfaceIDs: Set<String>
    /// Surface that should hold focus, if the core named one.
    let activeSurface: String?

    static func parse(_ json: String) -> WorkspaceLayout? {
        guard let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data),
              let root = object as? [String: Any],
              let workspace = root["workspace"] as? [String: Any],
              let id = workspace["id"] as? String,
              let name = workspace["name"] as? String,
              let tree = workspace["root"]
        else { return nil }

        var activeByPane: [String: String] = [:]
        for pane in (root["panes"] as? [[String: Any]]) ?? [] {
            guard let paneID = pane["id"] as? String else { continue }
            activeByPane[paneID] = pane["active"] as? String
        }

        var surfaceIDs = Set<String>()
        for surface in (root["surfaces"] as? [[String: Any]]) ?? [] {
            if let sid = surface["id"] as? String { surfaceIDs.insert(sid) }
        }

        guard let node = parseNode(tree, activeByPane: activeByPane) else { return nil }
        return WorkspaceLayout(
            id: id,
            name: name,
            root: node,
            surfaceIDs: surfaceIDs,
            activeSurface: workspace["active_surface"] as? String)
    }

    private static func parseNode(_ value: Any, activeByPane: [String: String]) -> Node? {
        guard let object = value as? [String: Any], let type = object["type"] as? String else {
            return nil
        }
        switch type {
        case "leaf":
            guard let paneID = object["pane_id"] as? String else { return nil }
            return .leaf(paneID: paneID, activeSurface: activeByPane[paneID])
        case "split":
            guard let children = object["children"] as? [Any] else { return nil }
            let parsed = children.compactMap { parseNode($0, activeByPane: activeByPane) }
            guard !parsed.isEmpty else { return nil }
            // "Horizontal" puts children side by side (a vertical divider).
            let orientation = object["orientation"] as? String ?? "Horizontal"
            return .split(
                id: object["id"] as? String ?? UUID().uuidString,
                isVertical: orientation == "Horizontal",
                children: parsed)
        default:
            return nil
        }
    }
}
