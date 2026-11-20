// User configuration: shortcut overrides, terminal font size, appearance and
// socket mode in ~/.config/berth/config.json.
//
//   {"shortcuts": {"splitRight": "cmd+d"}, "fontSize": 14, "opacity": 0.95,
//    "backgroundBlur": 20, "theme": "berth-light", "socket": "local"}
//
// Shortcuts are parsed into an AppKit key equivalent + modifier mask. Unknown
// keys are ignored, and a missing file means the built-in defaults.

import AppKit

struct Shortcut {
    let key: String
    let modifiers: NSEvent.ModifierFlags

    static func parse(_ value: String) -> Shortcut? {
        let parts = value
            .lowercased()
            .split(separator: "+", omittingEmptySubsequences: true)
            .map(String.init)
        guard let key = parts.last, !key.isEmpty else { return nil }
        var modifiers: NSEvent.ModifierFlags = []
        for part in parts.dropLast() {
            switch part {
            case "cmd", "command", "super": modifiers.insert(.command)
            case "shift": modifiers.insert(.shift)
            case "opt", "alt", "option": modifiers.insert(.option)
            case "ctrl", "control": modifiers.insert(.control)
            default: return nil
            }
        }
        return Shortcut(key: key, modifiers: modifiers)
    }

    var display: String {
        var parts: [String] = []
        if modifiers.contains(.control) { parts.append("ctrl") }
        if modifiers.contains(.option) { parts.append("opt") }
        if modifiers.contains(.shift) { parts.append("shift") }
        if modifiers.contains(.command) { parts.append("cmd") }
        parts.append(key)
        return parts.joined(separator: "+")
    }
}

enum ShortcutAction: String, CaseIterable {
    case splitRight
    case splitDown
    case newTab
    case closeSurface
    case nextTab
    case previousTab
    case nextPane
    case previousPane
    case newWorkspace
    case palette
    case jumpAttention
    case showAttention
    case toggleSidebar
    case zoomIn
    case zoomOut
    case zoomReset
    case settings

    var title: String {
        switch self {
        case .splitRight: return "Split Right"
        case .splitDown: return "Split Down"
        case .newTab: return "New Tab"
        case .closeSurface: return "Close Tab"
        case .nextTab: return "Next Tab"
        case .previousTab: return "Previous Tab"
        case .nextPane: return "Next Pane"
        case .previousPane: return "Previous Pane"
        case .newWorkspace: return "New Workspace"
        case .palette: return "Command Palette"
        case .jumpAttention: return "Jump to Attention"
        case .showAttention: return "Attention…"
        case .toggleSidebar: return "Toggle Sidebar"
        case .zoomIn: return "Zoom In"
        case .zoomOut: return "Zoom Out"
        case .zoomReset: return "Actual Size"
        case .settings: return "Settings…"
        }
    }

    var defaultShortcut: String {
        switch self {
        case .splitRight: return "cmd+d"
        case .splitDown: return "cmd+shift+d"
        case .newTab: return "cmd+t"
        case .closeSurface: return "cmd+w"
        case .nextTab: return "cmd+shift+]"
        case .previousTab: return "cmd+shift+["
        case .nextPane: return "cmd+]"
        case .previousPane: return "cmd+["
        case .newWorkspace: return "cmd+n"
        case .palette: return "cmd+shift+p"
        case .jumpAttention: return "cmd+shift+u"
        case .showAttention: return "cmd+i"
        case .toggleSidebar: return "cmd+opt+s"
        case .zoomIn: return "cmd+="
        case .zoomOut: return "cmd+-"
        case .zoomReset: return "cmd+0"
        case .settings: return "cmd+,"
        }
    }
}

@MainActor
final class AppConfig {
    static let shared = AppConfig()

    /// Zoom and the settings window share this floor and ceiling.
    static let fontSizeRange: ClosedRange<Double> = 8...72
    static let defaultFontSize: Double = 14

    private(set) var shortcuts: [String: String] = [:]
    /// Terminal font size in points, handed to Ghostty as `font-size`.
    private(set) var fontSize: Double = AppConfig.defaultFontSize
    /// Per-display multiplier. Zero means automatic (2 / backing scale), so a
    /// 1x monitor gets the same physical glyph size as a Retina laptop.
    private(set) var fontScale: Double = 0
    /// Hide the metadata line and tighten row height.
    private(set) var compactSidebar = false
    /// `off`, `local` (token for mutations) or `trusted` (same-uid only).
    private(set) var socketMode: SocketMode = .local
    /// Palette name for the portable window. This shell keeps the key so a save
    /// here does not drop it, and leaves its palette to the Ghostty config.
    private(set) var theme: String = ""
    /// Terminal background opacity, 0.3 (very transparent) to 1.0 (solid).
    private(set) var opacity: Double = 1.0
    /// Background blur radius, 0 to 64; 0 turns blur off.
    private(set) var backgroundBlur: Int = 0

    private init() {
        load()
    }

    /// A config read from one file instead of the default path. `shared` is
    /// what the app uses; this exists so tests do not touch the user's file.
    init(url: URL) {
        load(from: url)
    }

    func setSocketMode(_ mode: SocketMode) {
        socketMode = mode
    }

    func setFontSize(_ value: Double) {
        fontSize = min(max(value, Self.fontSizeRange.lowerBound), Self.fontSizeRange.upperBound)
    }

    func setTheme(_ value: String) {
        theme = value.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func setOpacity(_ value: Double) {
        opacity = min(max(value, 0.3), 1.0)
    }

    func setBackgroundBlur(_ value: Int) {
        backgroundBlur = min(max(value, 0), 64)
    }

    func setFontScale(_ value: Double) {
        // 0 = automatic. Otherwise keep the override in a sane range.
        fontScale = value == 0 ? 0 : min(max(value, 0.5), 3)
    }

    func setCompactSidebar(_ value: Bool) {
        compactSidebar = value
    }

    func shortcut(for action: ShortcutAction) -> Shortcut {
        let raw = shortcuts[action.rawValue] ?? action.defaultShortcut
        return Shortcut.parse(raw) ?? Shortcut.parse(action.defaultShortcut)!
    }

    func rawValue(for action: ShortcutAction) -> String {
        shortcuts[action.rawValue] ?? action.defaultShortcut
    }

    func setShortcut(_ value: String, for action: ShortcutAction) {
        if value.trimmingCharacters(in: .whitespaces).isEmpty {
            shortcuts.removeValue(forKey: action.rawValue)
        } else {
            shortcuts[action.rawValue] = value
        }
    }

    func load() {
        load(from: Self.url)
    }

    func load(from url: URL) {
        guard let data = try? Data(contentsOf: url),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return }
        if let values = object["shortcuts"] as? [String: String] {
            shortcuts = values
        }
        if let value = object["fontSize"] as? NSNumber {
            setFontSize(value.doubleValue)
        }
        if let value = object["fontScale"] as? NSNumber {
            setFontScale(value.doubleValue)
        }
        if let value = object["compactSidebar"] as? Bool {
            setCompactSidebar(value)
        }
        if let value = object["theme"] as? String {
            setTheme(value)
        }
        if let value = object["opacity"] as? NSNumber {
            setOpacity(value.doubleValue)
        }
        if let value = object["backgroundBlur"] as? NSNumber {
            setBackgroundBlur(value.intValue)
        }
        if let value = object["socket"] as? String, let mode = SocketMode(rawValue: value) {
            socketMode = mode
        }
    }

    func save() throws {
        try save(to: Self.url)
    }

    /// The portable window reads the same document, so merge into what is
    /// already there instead of writing only the keys this shell owns.
    func save(to url: URL) throws {
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true)
        var object = try Self.document(at: url)
        object["shortcuts"] = shortcuts
        object["fontSize"] = fontSize
        object["fontScale"] = fontScale
        object["compactSidebar"] = compactSidebar
        object["theme"] = theme
        object["opacity"] = opacity
        object["backgroundBlur"] = backgroundBlur
        object["socket"] = socketMode.rawValue
        let data = try JSONSerialization.data(
            withJSONObject: object,
            options: [.prettyPrinted, .sortedKeys])
        try data.write(to: url, options: .atomic)
    }

    private static func document(at url: URL) throws -> [String: Any] {
        guard FileManager.default.fileExists(atPath: url.path) else { return [:] }
        let data = try Data(contentsOf: url)
        guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            // Merging into a document we cannot read would replace it, so say
            // so and keep the file for the user to fix.
            throw NSError(
                domain: "dev.berth.config",
                code: 1,
                userInfo: [
                    NSLocalizedDescriptionKey:
                        "\(url.lastPathComponent) is not a JSON object; fix or delete it"
                ])
        }
        return object
    }

    static var url: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".config/berth/config.json")
    }
}
