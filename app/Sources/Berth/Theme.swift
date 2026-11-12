// Visual tokens. Views should read these instead of hard-coding colors and
// metrics so density and theme work has one place to change.

import AppKit

@MainActor
enum Theme {
    @MainActor
    enum Colors {
        static let terminalBackground = NSColor.black
        static let terminalForeground = NSColor.labelColor
        static let paneBorder = NSColor.separatorColor
        static let paneActiveTab = NSColor.controlAccentColor.withAlphaComponent(0.28)
        static let sidebarBackground = NSColor.windowBackgroundColor
        static let textPrimary = NSColor.labelColor
        static let textSecondary = NSColor.secondaryLabelColor
        static let textTertiary = NSColor.tertiaryLabelColor
        static let attentionError = NSColor.systemRed
        static let attentionInput = NSColor.systemOrange
        static let success = NSColor.systemGreen
        static let accent = NSColor.controlAccentColor
    }

    @MainActor
    enum Metrics {
        static let paneTabHeight: CGFloat = 28
        static let sidebarRowNormal: CGFloat = 62
        static let sidebarRowCompact: CGFloat = 52
        static let sidebarMinWidth: CGFloat = 180
        static let sidebarMaxWidth: CGFloat = 360
        static let ringDuration: TimeInterval = 1.2
    }

    @MainActor
    enum Fonts {
        static let sidebarTitle = NSFont.systemFont(ofSize: 12, weight: .medium)
        static let sidebarAgent = NSFont.systemFont(ofSize: 10.5)
        static let sidebarDetail = NSFont.systemFont(ofSize: 10)
        static let tab = NSFont.systemFont(ofSize: 11)
        static let onboardingTitle = NSFont.systemFont(ofSize: 24, weight: .semibold)
    }
}
