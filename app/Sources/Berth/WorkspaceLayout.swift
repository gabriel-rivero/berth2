// The layout the core owns, decoded for AppKit. The core is the source of
// truth for panes, tabs and splits; this file only shapes its JSON into
// something a view tree can be built from (see BerthCoreHost.reconcileAll).
//
// Shape (from `workspace.layout`):
//   { workspace: {id,name,root,active_surface,group_id,pinned,unread},
//     panes: [{id,surfaces,active_surface}], surfaces: [...] }
// where root is either
//   {type:"split", id, orientation:"horizontal"|"vertical", children:[...], weights:[...]}
// or
//   {type:"leaf", pane_id}

import Foundation

struct SurfaceInfo {
    let id: String
    let title: String
    let cwd: String
    let pid: Int?
    let progress: Double?
    let bellCount: Int
}

struct PaneSpec {
    let id: String
    let surfaceIDs: [String]
    let activeSurface: String?
}

struct WorkspaceLayout {
    /// Node in the pane tree. `isVertical` is NSSplitView's sense: true means
    /// the divider is vertical and children sit side by side.
    enum Node {
        case split(id: String, isVertical: Bool, children: [Node])
        case leaf(paneID: String, activeSurface: String?)
    }

    let id: String
    let name: String
    let groupID: String?
    let pinned: Bool
    let unread: Int
    let root: Node
    let surfaceIDs: Set<String>
    /// Surface that should hold focus, if the core named one.
    let activeSurface: String?
    let panes: [PaneSpec]
    let surfaces: [String: SurfaceInfo]
    /// Splits, panes, tab order and active tab — everything that changes the
    /// view tree. Titles and cwd are deliberately excluded so OSC updates do
    /// not reparent terminal views on every prompt.
    var structureKey: String {
        "\(id)|\(root.structureKey)"
    }

    func surface(_ id: String?) -> SurfaceInfo? {
        guard let id else { return nil }
        return surfaces[id]
    }

    func pane(_ id: String?) -> PaneSpec? {
        guard let id else { return nil }
        return panes.first { $0.id == id }
    }

    /// The pane owning `surfaceID`, used by New Tab and Close Tab.
    func paneID(forSurface surfaceID: String?) -> String? {
        guard let surfaceID else { return nil }
        return panes.first { $0.surfaceIDs.contains(surfaceID) }?.id
    }

    static func parse(_ json: String) -> WorkspaceLayout? {
        guard let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data),
              let root = object as? [String: Any],
              let workspace = root["workspace"] as? [String: Any],
              let id = workspace["id"] as? String,
              let name = workspace["name"] as? String,
              let tree = workspace["root"]
        else { return nil }

        // The core resolves the active surface id for us (`active_surface`).
        // The older `active` index is still understood so a mismatched app and
        // core do not silently render empty panes.
        var paneSpecs: [PaneSpec] = []
        var activeByPane: [String: String] = [:]
        for pane in (root["panes"] as? [[String: Any]]) ?? [] {
            guard let paneID = pane["id"] as? String else { continue }
            let surfaces = (pane["surfaces"] as? [String]) ?? []
            var active = pane["active_surface"] as? String
            if active == nil,
               let index = pane["active"] as? Int,
               surfaces.indices.contains(index) {
                active = surfaces[index]
            }
            if let active { activeByPane[paneID] = active }
            paneSpecs.append(PaneSpec(id: paneID, surfaceIDs: surfaces, activeSurface: active))
        }

        var surfaceIDs = Set<String>()
        var surfaces: [String: SurfaceInfo] = [:]
        for surface in (root["surfaces"] as? [[String: Any]]) ?? [] {
            guard let sid = surface["id"] as? String else { continue }
            surfaceIDs.insert(sid)
            surfaces[sid] = SurfaceInfo(
                id: sid,
                title: surface["title"] as? String ?? "",
                cwd: surface["cwd"] as? String ?? "",
                pid: surface["pid"] as? Int,
                progress: surface["progress"] as? Double,
                bellCount: surface["bell_count"] as? Int ?? 0)
        }

        guard let node = parseNode(tree, activeByPane: activeByPane) else { return nil }
        return WorkspaceLayout(
            id: id,
            name: name,
            groupID: workspace["group_id"] as? String,
            pinned: workspace["pinned"] as? Bool ?? false,
            unread: workspace["unread"] as? Int ?? 0,
            root: node,
            surfaceIDs: surfaceIDs,
            activeSurface: workspace["active_surface"] as? String,
            panes: paneSpecs,
            surfaces: surfaces)
    }

    private static func parseNode(_ value: Any, activeByPane: [String: String]) -> Node? {
        // Kept below the public parser so callers never see raw JSON.
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
            // The core serializes `Orientation` in snake_case: "horizontal"
            // puts children side by side (a vertical divider), "vertical"
            // stacks them. Compare case-insensitively so older "Horizontal"
            // dumps keep working.
            let orientation = (object["orientation"] as? String ?? "horizontal").lowercased()
            return .split(
                id: object["id"] as? String ?? UUID().uuidString,
                isVertical: orientation == "horizontal",
                children: parsed)
        default:
            return nil
        }
    }
}

extension WorkspaceLayout.Node {
    /// Compact structural description used to decide whether the AppKit tree
    /// must be rebuilt. Titles, cwd and progress do not appear here.
    var structureKey: String {
        switch self {
        case let .leaf(paneID, active):
            return "L(\(paneID)|\(active ?? ""))"
        case let .split(_, isVertical, children):
            return "S(\(isVertical ? "v" : "h")[\(children.map(\.structureKey).joined(separator: ","))])"
        }
    }
}
