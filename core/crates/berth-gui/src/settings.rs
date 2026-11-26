//! Settings panel model: rows, values, and edits that persist to config.json.
//!
//! The panel edits the same document the macOS shell reads, so it writes only
//! the keys it owns and leaves the rest of the file alone.

use crate::config::{
    Action, BellMode, Config, DEFAULT_WALLPAPER_OPACITY, MAX_FONT_SIZE, MIN_FONT_SIZE, MIN_OPACITY,
};
use crate::theme::THEMES;
use berth_headless::engine::Backend;

/// Values the display-scale row cycles through. `0` means automatic.
const SCALES: [f32; 9] = [0.0, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 2.5, 3.0];

const SOCKET_MODES: [Option<&str>; 3] = [Some("local"), Some("trusted"), Some("off")];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    FontFamily,
    UiFontFamily,
    FontSize,
    FontScale,
    Theme,
    Opacity,
    Wallpaper,
    WallpaperOpacity,
    /// Open the system file picker and adopt the chosen image.
    WallpaperChoose,
    Bell,
    Shell,
    TerminalBackend,
    CompactSidebar,
    CloseExitedPanes,
    RestoreLaunchFlags,
    ClaudeBypass,
    CopyOnSelect,
    ReinstallHooks,
    SocketMode,
    Shortcut(Action),
}

pub struct Row {
    pub field: Field,
    pub label: String,
    pub value: String,
}

/// What a change affects in the running window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Nothing changed.
    None,
    /// Terminal font metrics changed; the renderer must be rebuilt.
    Font,
    /// Window/chrome font changed; only the chrome renderer must be rebuilt.
    UiFont,
    /// The palette changed; the window must adopt the new tokens.
    Theme,
    /// Window opacity changed; the platform window must be told.
    Opacity,
    /// The wallpaper source changed; the picture must be decoded again.
    Wallpaper,
    /// Only the wallpaper's strength changed; the decoded pixels still apply.
    WallpaperOpacity,
    /// Layout or bindings changed; only a redraw is needed.
    View,
    /// The shell for new panes changed; the host must be told.
    Shell,
}

/// The rows of the settings panel, in display order.
pub fn rows(config: &Config, recording: Option<Action>, notice: Option<&str>) -> Vec<Row> {
    let opacity_label = if cfg!(target_os = "macos") {
        "Window opacity"
    } else {
        "Window opacity (macOS)"
    };
    let mut rows = vec![
        Row {
            field: Field::FontFamily,
            label: "Font".into(),
            value: font_label(config),
        },
        Row {
            field: Field::UiFontFamily,
            label: "Window font".into(),
            value: ui_font_label(config),
        },
        Row {
            field: Field::FontSize,
            label: "Font size".into(),
            value: format!("{:.0} pt", config.font_size),
        },
        Row {
            field: Field::FontScale,
            label: "Display scale".into(),
            value: if config.font_scale > 0.0 {
                format!("{:.2}×", config.font_scale)
            } else {
                "auto".to_string()
            },
        },
        Row {
            field: Field::Theme,
            label: "Theme".into(),
            value: theme_name(config).to_string(),
        },
        Row {
            field: Field::Opacity,
            label: opacity_label.into(),
            value: format!("{:.0}%", config.opacity * 100.0),
        },
        Row {
            field: Field::Wallpaper,
            label: "Wallpaper".into(),
            value: wallpaper_label(config),
        },
        Row {
            field: Field::WallpaperOpacity,
            label: "Wallpaper strength".into(),
            value: format!("{}%", config.wallpaper.opacity),
        },
        Row {
            field: Field::WallpaperChoose,
            label: "Wallpaper image…".into(),
            value: "press enter to browse".into(),
        },
        Row {
            field: Field::Bell,
            label: "Bell".into(),
            value: config.bell.name().to_string(),
        },
        Row {
            field: Field::Shell,
            label: "Shell".into(),
            value: crate::shells::label_for(&config.shell),
        },
        Row {
            field: Field::TerminalBackend,
            label: "VT engine (relaunch)".into(),
            value: config.terminal_backend.name().to_string(),
        },
        Row {
            field: Field::CompactSidebar,
            label: "Compact sidebar".into(),
            value: if config.compact_sidebar { "on" } else { "off" }.to_string(),
        },
        Row {
            field: Field::CloseExitedPanes,
            label: "Close exited panes".into(),
            value: if config.close_exited_panes {
                "on"
            } else {
                "off"
            }
            .to_string(),
        },
        Row {
            field: Field::RestoreLaunchFlags,
            label: "Restore launch flags".into(),
            value: if config.restore_launch_flags {
                "on"
            } else {
                "off"
            }
            .to_string(),
        },
        Row {
            field: Field::ClaudeBypass,
            label: "Claude bypass".into(),
            value: if config.claude_bypass_permissions {
                "on"
            } else {
                "off"
            }
            .to_string(),
        },
        Row {
            field: Field::CopyOnSelect,
            label: "Copy on select".into(),
            value: if config.copy_on_select { "on" } else { "off" }.to_string(),
        },
        Row {
            field: Field::ReinstallHooks,
            label: "Reinstall agent hooks".into(),
            value: notice.unwrap_or("enter to install").to_string(),
        },
        Row {
            field: Field::SocketMode,
            label: "Automation socket".into(),
            value: match config.socket_mode() {
                Some("off") => "off (relaunch)".to_string(),
                Some("trusted") => "trusted (relaunch)".to_string(),
                Some("local") => "local + token (relaunch)".to_string(),
                _ => "default (relaunch)".to_string(),
            },
        },
    ];
    for action in Action::ALL {
        let value = if recording == Some(action) {
            "press a chord…".to_string()
        } else {
            let text = config.shortcuts.label(action);
            if text.is_empty() {
                "unbound".to_string()
            } else {
                text
            }
        };
        rows.push(Row {
            field: Field::Shortcut(action),
            label: action.label().to_string(),
            value,
        });
    }
    rows
}

/// One choice on the wallpaper row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WallpaperChoice {
    /// No picture.
    Off,
    /// The desktop's own wallpaper.
    Desktop,
    /// An explicit path from `config.json`.
    Path(String),
}

impl WallpaperChoice {
    fn label(&self) -> String {
        match self {
            Self::Off => "off".to_string(),
            Self::Desktop => "desktop wallpaper".to_string(),
            Self::Path(path) => std::path::Path::new(path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone()),
        }
    }
}

/// The wallpaper row's value, so the panel says what it will draw.
fn wallpaper_label(config: &Config) -> String {
    if !config.wallpaper.active() {
        return WallpaperChoice::Off.label();
    }
    if config.wallpaper.follow_desktop {
        return WallpaperChoice::Desktop.label();
    }
    WallpaperChoice::Path(config.wallpaper.path.trim().to_string()).label()
}

/// The wallpaper sources the row cycles through. The desktop option only
/// exists where the OS can report its picture, so the row does not advertise a
/// choice that would do nothing. An explicit path from `config.json` stays in
/// the list, so cycling away from it can come back.
fn wallpaper_choices(config: &Config) -> Vec<WallpaperChoice> {
    let mut choices = vec![WallpaperChoice::Off];
    if cfg!(target_os = "macos") {
        choices.push(WallpaperChoice::Desktop);
    }
    let path = config.wallpaper.path.trim();
    if !path.is_empty() {
        choices.push(WallpaperChoice::Path(path.to_string()));
    }
    choices
}

/// Which choice the current config selects.
fn wallpaper_index(config: &Config) -> i32 {
    let choices = wallpaper_choices(config);
    let current = if !config.wallpaper.active() {
        WallpaperChoice::Off
    } else if config.wallpaper.follow_desktop {
        WallpaperChoice::Desktop
    } else {
        WallpaperChoice::Path(config.wallpaper.path.trim().to_string())
    };
    choices
        .iter()
        .position(|choice| *choice == current)
        .unwrap_or(0) as i32
}

/// The palette name to show, defaulting the empty and unknown cases to the
/// first entry in the list.
fn theme_name(config: &Config) -> &str {
    let name = config.theme.trim();
    if THEMES.contains(&name) {
        name
    } else {
        THEMES[0]
    }
}

/// The font row's value: the configured primary face, or the name the default
/// resolves to so the row says which face the window actually uses.
fn font_label(config: &Config) -> String {
    match config.font_family.first() {
        Some(name) => name.clone(),
        None => format!("Default ({})", crate::font::default_family_name()),
    }
}

/// The font row's choices: the built-in default, every installed monospace
/// face, then any configured name this system lacks so it stays visible.
fn font_choices(config: &Config) -> Vec<Option<String>> {
    let mut choices: Vec<Option<String>> = vec![None];
    let installed = crate::font::installed_monospace_families();
    choices.extend(installed.iter().cloned().map(Some));
    for name in &config.font_family {
        let known = choices
            .iter()
            .flatten()
            .any(|choice| choice.eq_ignore_ascii_case(name));
        if !known {
            choices.push(Some(name.clone()));
        }
    }
    choices
}

/// Position of the configured primary face in the choice list.
fn font_index(config: &Config, choices: &[Option<String>]) -> i32 {
    index_of(config.font_family.first(), choices)
}

/// The window font row's value: the configured chrome face, or the system
/// sans-serif the default resolves to.
fn ui_font_label(config: &Config) -> String {
    match config.ui_font_family.first() {
        Some(name) => name.clone(),
        None => format!("Default ({})", crate::font::default_ui_family_name()),
    }
}

/// The window font row's choices: the default, every installed family (not
/// only the monospaced ones), then any configured name this system lacks.
fn ui_font_choices(config: &Config) -> Vec<Option<String>> {
    let mut choices: Vec<Option<String>> = vec![None];
    let installed = crate::font::installed_families();
    choices.extend(installed.iter().cloned().map(Some));
    for name in &config.ui_font_family {
        let known = choices
            .iter()
            .flatten()
            .any(|choice| choice.eq_ignore_ascii_case(name));
        if !known {
            choices.push(Some(name.clone()));
        }
    }
    choices
}

fn index_of(current: Option<&String>, choices: &[Option<String>]) -> i32 {
    let Some(current) = current else {
        return 0;
    };
    choices
        .iter()
        .position(|choice| {
            choice
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(current))
        })
        .unwrap_or(0) as i32
}

/// Move a numeric or cycling value by one step.
pub fn adjust(config: &mut Config, field: Field, delta: i32) -> Change {
    match field {
        Field::FontFamily => {
            let choices = font_choices(config);
            let index = font_index(config, &choices);
            let next = (index + delta).rem_euclid(choices.len() as i32) as usize;
            let picked = choices[next].clone();
            let unchanged = match (&picked, config.font_family.first()) {
                (None, None) => true,
                (Some(next), Some(current)) => next.eq_ignore_ascii_case(current),
                _ => false,
            };
            if unchanged {
                return Change::None;
            }
            config.set_font_family(picked);
            Change::Font
        }
        Field::UiFontFamily => {
            let choices = ui_font_choices(config);
            let index = index_of(config.ui_font_family.first(), &choices);
            let next = (index + delta).rem_euclid(choices.len() as i32) as usize;
            let picked = choices[next].clone();
            let unchanged = match (&picked, config.ui_font_family.first()) {
                (None, None) => true,
                (Some(next), Some(current)) => next.eq_ignore_ascii_case(current),
                _ => false,
            };
            if unchanged {
                return Change::None;
            }
            config.set_ui_font_family(picked);
            Change::UiFont
        }
        Field::FontSize => {
            let size = (config.font_size + delta as f32).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
            if (size - config.font_size).abs() < f32::EPSILON {
                return Change::None;
            }
            config.font_size = size;
            Change::Font
        }
        Field::Theme => {
            let names = THEMES;
            let index = names
                .iter()
                .position(|name| *name == theme_name(config))
                .unwrap_or(0) as i32;
            let next = (index + delta).rem_euclid(names.len() as i32) as usize;
            config.theme = names[next].to_string();
            Change::Theme
        }
        Field::Opacity => {
            let step = f32::from(delta as i16) * 0.05;
            let opacity = (config.opacity + step).clamp(MIN_OPACITY, 1.0);
            if (opacity - config.opacity).abs() < f32::EPSILON {
                return Change::None;
            }
            config.opacity = opacity;
            Change::Opacity
        }
        Field::Wallpaper => {
            let choices = wallpaper_choices(config);
            let index = wallpaper_index(config);
            let next = (index + delta).rem_euclid(choices.len() as i32) as usize;
            match &choices[next] {
                WallpaperChoice::Off => {
                    if !config.wallpaper.active() {
                        return Change::None;
                    }
                    // Off is "draw nothing", not "forget the source": zeroing
                    // the strength keeps the chosen picture in the cycle.
                    config.wallpaper.opacity = 0;
                    return Change::Wallpaper;
                }
                WallpaperChoice::Desktop => {
                    if config.wallpaper.follow_desktop && config.wallpaper.active() {
                        return Change::None;
                    }
                    config.wallpaper.follow_desktop = true;
                }
                WallpaperChoice::Path(path) => {
                    // Coming back round to the configured path after Off left it
                    // at zero strength must restore it, so "already on this
                    // source" only short-circuits when it is actually drawn.
                    if !config.wallpaper.follow_desktop
                        && config.wallpaper.path == *path
                        && config.wallpaper.active()
                    {
                        return Change::None;
                    }
                    config.wallpaper.follow_desktop = false;
                    config.wallpaper.path = path.clone();
                }
            }
            // Picking a source with no strength would draw nothing, so give it
            // the legible default the first time. The check is on the source,
            // not `active()`: a source at zero strength is still inactive.
            let has_source = config.wallpaper.follow_desktop || !config.wallpaper.path.is_empty();
            if has_source && config.wallpaper.opacity == 0 {
                config.wallpaper.opacity = (DEFAULT_WALLPAPER_OPACITY * 100.0).round() as u8;
            }
            Change::Wallpaper
        }
        Field::WallpaperOpacity => {
            // Steps of five percent, clamped to the range the docs promise.
            let step = delta * 5;
            let opacity = (i32::from(config.wallpaper.opacity) + step).clamp(0, 100);
            let opacity = opacity as u8;
            if opacity == config.wallpaper.opacity {
                return Change::None;
            }
            config.wallpaper.opacity = opacity;
            if config.wallpaper.active() {
                Change::WallpaperOpacity
            } else {
                Change::Wallpaper
            }
        }
        Field::Bell => {
            const MODES: [BellMode; 3] = [BellMode::None, BellMode::Visual, BellMode::Audio];
            let index = MODES
                .iter()
                .position(|mode| *mode == config.bell)
                .unwrap_or(1) as i32;
            let next = (index + delta).rem_euclid(MODES.len() as i32) as usize;
            config.bell = MODES[next];
            Change::View
        }
        Field::FontScale => {
            let index = SCALES
                .iter()
                .position(|scale| (scale - config.font_scale).abs() < 0.001)
                .unwrap_or(0) as i32;
            let next = (index + delta).rem_euclid(SCALES.len() as i32) as usize;
            config.font_scale = SCALES[next];
            Change::Font
        }
        Field::SocketMode => {
            let current = config.socket.as_deref().and_then(|mode| {
                SOCKET_MODES
                    .iter()
                    .position(|candidate| *candidate == Some(mode))
            });
            let next = match current {
                Some(index) => {
                    (index as i32 + delta).rem_euclid(SOCKET_MODES.len() as i32) as usize
                }
                None => 0,
            };
            config.socket = SOCKET_MODES[next].map(str::to_string);
            Change::View
        }
        Field::Shell => {
            let choices = crate::shells::choices();
            let index = choices
                .iter()
                .position(|choice| choice.argv == config.shell)
                .unwrap_or(0) as i32;
            let next = (index + delta).rem_euclid(choices.len() as i32) as usize;
            config.shell = choices[next].argv.clone();
            Change::Shell
        }
        Field::TerminalBackend => {
            let backends = Backend::ALL;
            let index = backends
                .iter()
                .position(|backend| *backend == config.terminal_backend)
                .unwrap_or(0) as i32;
            let next = (index + delta).rem_euclid(backends.len() as i32) as usize;
            config.terminal_backend = backends[next];
            Change::View
        }
        Field::CompactSidebar
        | Field::CloseExitedPanes
        | Field::RestoreLaunchFlags
        | Field::ClaudeBypass
        | Field::CopyOnSelect
        | Field::ReinstallHooks
        | Field::WallpaperChoose
        | Field::Shortcut(_) => Change::None,
    }
}

/// Flip a boolean row.
pub fn toggle(config: &mut Config, field: Field) -> Change {
    match field {
        Field::CompactSidebar => {
            config.compact_sidebar = !config.compact_sidebar;
            Change::View
        }
        Field::CloseExitedPanes => {
            config.close_exited_panes = !config.close_exited_panes;
            Change::View
        }
        Field::RestoreLaunchFlags => {
            config.restore_launch_flags = !config.restore_launch_flags;
            Change::View
        }
        Field::ClaudeBypass => {
            config.claude_bypass_permissions = !config.claude_bypass_permissions;
            Change::View
        }
        Field::CopyOnSelect => {
            config.copy_on_select = !config.copy_on_select;
            Change::View
        }
        _ => Change::None,
    }
}

/// Store a chord for one action; an empty string unbinds it.
pub fn set_shortcut(config: &mut Config, action: Action, chord: &str) -> Change {
    config.shortcuts.set(action, chord);
    Change::View
}

/// Persist the owned keys, keeping everything else in the file.
pub fn save(config: &Config) -> Result<(), String> {
    config
        .save()
        .map_err(|error| format!("{}: {error}", berth_core::paths::config_path().display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_the_wallpaper_row_when_adjusted_then_it_cycles_the_sources() {
        let mut config = Config::default();
        assert_eq!(wallpaper_label(&config), "off");
        // Off -> the desktop, where the platform can report one.
        let change = adjust(&mut config, Field::Wallpaper, 1);
        if cfg!(target_os = "macos") {
            assert_eq!(change, Change::Wallpaper);
            assert!(config.wallpaper.follow_desktop);
            assert!(
                config.wallpaper.opacity > 0,
                "picking a source must give it a visible strength"
            );
            assert_eq!(wallpaper_label(&config), "desktop wallpaper");
            // Back to off.
            assert_eq!(adjust(&mut config, Field::Wallpaper, 1), Change::Wallpaper);
        }
        assert!(!config.wallpaper.active());
        assert_eq!(wallpaper_label(&config), "off");
    }

    #[test]
    fn given_the_wallpaper_strength_row_when_adjusted_then_it_steps_and_clamps() {
        let mut config = Config::default();
        config.wallpaper.path = "/tmp/w.png".into();
        config.wallpaper.opacity = 10;
        assert_eq!(
            adjust(&mut config, Field::WallpaperOpacity, 1),
            Change::WallpaperOpacity
        );
        assert_eq!(config.wallpaper.opacity, 15);
        for _ in 0..30 {
            adjust(&mut config, Field::WallpaperOpacity, -1);
        }
        assert_eq!(config.wallpaper.opacity, 0);
        assert_eq!(
            adjust(&mut config, Field::WallpaperOpacity, -1),
            Change::None
        );
        // Raising it from zero back to visible re-reads the picture.
        assert_eq!(
            adjust(&mut config, Field::WallpaperOpacity, 1),
            Change::WallpaperOpacity
        );
        config.wallpaper.path.clear();
        assert_eq!(
            adjust(&mut config, Field::WallpaperOpacity, 1),
            Change::Wallpaper,
            "with no source a strength change still needs the loader to look"
        );
    }

    #[test]
    fn given_a_configured_path_then_the_wallpaper_row_shows_its_name() {
        let config = Config {
            wallpaper: crate::config::Wallpaper {
                path: "/Users/x/Pictures/peaks.jpg".into(),
                follow_desktop: false,
                opacity: 35,
            },
            ..Config::default()
        };
        assert_eq!(wallpaper_label(&config), "peaks.jpg");
    }

    #[test]
    fn given_a_configured_path_then_cycling_keeps_it_reachable() {
        // The row must not silently drop the explicit path when it cycles away:
        // coming back around must land on it again.
        let mut config = Config {
            wallpaper: crate::config::Wallpaper {
                path: "/tmp/peaks.jpg".into(),
                follow_desktop: false,
                opacity: 30,
            },
            ..Config::default()
        };
        let choices = wallpaper_choices(&config);
        assert!(
            choices.contains(&WallpaperChoice::Path("/tmp/peaks.jpg".into())),
            "the configured path must stay in the cycle: {choices:?}"
        );
        // Cycle the full length of the list and confirm the path comes back.
        let mut seen = Vec::new();
        for _ in 0..choices.len() {
            adjust(&mut config, Field::Wallpaper, 1);
            seen.push(wallpaper_label(&config));
        }
        assert_eq!(
            wallpaper_label(&config),
            "peaks.jpg",
            "the path was lost after a full cycle"
        );
        assert!(seen.iter().any(|label| label == "off"));
    }

    #[test]
    fn given_off_then_picking_the_configured_path_again_restores_it() {
        // On a platform without the desktop source the cycle is just
        // Off <-> Path, so this ordering is the one Linux and Windows take.
        // Off zeroes the strength but keeps the path, and the step back to the
        // path must restore a drawn source rather than short-circuit.
        let mut config = Config {
            wallpaper: crate::config::Wallpaper {
                path: "/tmp/peaks.jpg".into(),
                follow_desktop: false,
                opacity: 30,
            },
            ..Config::default()
        };
        adjust(&mut config, Field::Wallpaper, 1);
        assert_eq!(wallpaper_label(&config), "off");
        adjust(&mut config, Field::Wallpaper, -1);
        assert_eq!(wallpaper_label(&config), "peaks.jpg");
        assert!(
            config.wallpaper.active(),
            "coming back to the path must draw it again, not stay at zero strength"
        );
    }

    #[test]
    fn given_the_wallpaper_rows_then_they_are_ordered_and_labelled() {
        // The panel is the only way to reach the picker, so the rows' order and
        // wording are part of the feature, not incidental.
        let rows = rows(&Config::default(), None, None);
        let labels: Vec<(&str, &str)> = rows
            .iter()
            .filter(|row| {
                matches!(
                    row.field,
                    Field::Wallpaper | Field::WallpaperOpacity | Field::WallpaperChoose
                )
            })
            .map(|row| (row.label.as_str(), row.value.as_str()))
            .collect();
        assert_eq!(
            labels,
            [
                ("Wallpaper", "off"),
                ("Wallpaper strength", "0%"),
                ("Wallpaper image…", "press enter to browse"),
            ]
        );
    }

    #[test]
    fn given_the_settings_rows_when_built_then_every_action_has_a_row() {
        let config = Config::default();
        let rows = rows(&config, None, None);
        assert_eq!(rows.len(), 19 + Action::ALL.len());
        assert_eq!(rows[0].label, "Font");
        assert_eq!(rows[1].label, "Window font");
        assert_eq!(rows[2].label, "Font size");
        let wallpaper = rows
            .iter()
            .find(|row| row.field == Field::Wallpaper)
            .expect("the wallpaper row is in the panel");
        assert_eq!(wallpaper.value, "off");
        let strength = rows
            .iter()
            .find(|row| row.field == Field::WallpaperOpacity)
            .expect("the strength row is in the panel");
        assert_eq!(strength.value, "0%");
    }

    #[test]
    fn given_the_font_row_when_adjusted_then_it_cycles_installed_faces() {
        let mut config = Config::default();
        let choices = font_choices(&config);
        assert_eq!(choices.first(), Some(&None));
        if choices.len() < 2 {
            // No installed monospace face beyond the default on this system.
            assert_eq!(adjust(&mut config, Field::FontFamily, 1), Change::None);
            return;
        }
        assert_eq!(adjust(&mut config, Field::FontFamily, 1), Change::Font);
        assert_eq!(
            config.font_family.first().map(String::as_str),
            choices[1].as_deref()
        );
        assert_eq!(adjust(&mut config, Field::FontFamily, -1), Change::Font);
        assert!(config.font_family.is_empty());
    }

    #[test]
    fn given_the_window_font_row_when_adjusted_then_it_cycles_installed_faces() {
        let mut config = Config::default();
        let choices = ui_font_choices(&config);
        assert_eq!(choices.first(), Some(&None));
        assert!(
            choices.len() > font_choices(&config).len() - 1,
            "the window font list should not be limited to monospace faces"
        );
        if choices.len() < 2 {
            assert_eq!(adjust(&mut config, Field::UiFontFamily, 1), Change::None);
            return;
        }
        assert_eq!(adjust(&mut config, Field::UiFontFamily, 1), Change::UiFont);
        assert_eq!(
            config.ui_font_family.first().map(String::as_str),
            choices[1].as_deref()
        );
        assert!(
            config.font_family.is_empty(),
            "the terminal font must not change"
        );
    }

    #[test]
    fn given_an_uninstalled_configured_font_then_the_row_keeps_it_visible() {
        let config = Config {
            font_family: vec!["No Such Face 12345".to_string()],
            ..Config::default()
        };
        let choices = font_choices(&config);
        assert_eq!(
            choices.last(),
            Some(&Some("No Such Face 12345".to_string()))
        );
        assert_eq!(font_index(&config, &choices), choices.len() as i32 - 1);
    }

    #[test]
    fn given_a_recording_row_when_built_then_it_asks_for_a_chord() {
        let config = Config::default();
        let rows = rows(&config, Some(Action::NewTab), None);
        let row = rows
            .iter()
            .find(|row| row.field == Field::Shortcut(Action::NewTab))
            .unwrap();
        assert_eq!(row.value, "press a chord…");
    }

    #[test]
    fn given_a_font_size_when_adjusted_then_it_clamps_to_the_documented_range() {
        let mut config = Config {
            font_size: MAX_FONT_SIZE,
            ..Config::default()
        };
        assert_eq!(adjust(&mut config, Field::FontSize, 1), Change::None);
        assert_eq!(adjust(&mut config, Field::FontSize, -1), Change::Font);
        assert_eq!(config.font_size, MAX_FONT_SIZE - 1.0);
        config.font_size = MIN_FONT_SIZE;
        assert_eq!(adjust(&mut config, Field::FontSize, -1), Change::None);
    }

    #[test]
    fn given_the_theme_when_adjusted_then_it_cycles_through_the_list() {
        let mut config = Config::default();
        assert_eq!(adjust(&mut config, Field::Theme, 1), Change::Theme);
        assert_eq!(config.theme, "berth-dark");
        assert_eq!(adjust(&mut config, Field::Theme, 1), Change::Theme);
        assert_eq!(config.theme, "berth-light");
        assert_eq!(adjust(&mut config, Field::Theme, 1), Change::Theme);
        assert_eq!(config.theme, "system");
        config.theme = "not-a-theme".into();
        assert_eq!(adjust(&mut config, Field::Theme, 1), Change::Theme);
        assert_eq!(config.theme, "berth-dark");
    }

    #[test]
    fn given_the_bell_when_adjusted_then_it_cycles_through_the_modes() {
        let mut config = Config::default();
        assert_eq!(adjust(&mut config, Field::Bell, 1), Change::View);
        assert_eq!(config.bell, BellMode::Audio);
        adjust(&mut config, Field::Bell, 1);
        assert_eq!(config.bell, BellMode::None);
        adjust(&mut config, Field::Bell, 1);
        assert_eq!(config.bell, BellMode::Visual);
    }

    #[test]
    fn given_the_opacity_when_adjusted_then_it_steps_and_clamps() {
        let mut config = Config::default();
        assert_eq!(adjust(&mut config, Field::Opacity, -1), Change::Opacity);
        assert!((config.opacity - 0.95).abs() < 0.001);
        for _ in 0..30 {
            adjust(&mut config, Field::Opacity, -1);
        }
        assert_eq!(config.opacity, MIN_OPACITY);
        assert_eq!(adjust(&mut config, Field::Opacity, -1), Change::None);
        for _ in 0..40 {
            adjust(&mut config, Field::Opacity, 1);
        }
        assert_eq!(config.opacity, 1.0);
        assert_eq!(adjust(&mut config, Field::Opacity, 1), Change::None);
    }

    #[test]
    fn given_the_scale_when_adjusted_past_an_end_then_it_wraps_through_auto() {
        let mut config = Config {
            font_scale: 0.0,
            ..Config::default()
        };
        assert_eq!(adjust(&mut config, Field::FontScale, -1), Change::Font);
        assert_eq!(config.font_scale, 3.0);
        assert_eq!(adjust(&mut config, Field::FontScale, 1), Change::Font);
        assert_eq!(config.font_scale, 0.0);
    }

    #[test]
    fn given_the_socket_mode_when_adjusted_then_it_cycles_through_the_three_modes() {
        let mut config = Config {
            socket: Some("local".into()),
            ..Config::default()
        };
        adjust(&mut config, Field::SocketMode, 1);
        assert_eq!(config.socket.as_deref(), Some("trusted"));
        adjust(&mut config, Field::SocketMode, 1);
        assert_eq!(config.socket.as_deref(), Some("off"));
        adjust(&mut config, Field::SocketMode, 1);
        assert_eq!(config.socket.as_deref(), Some("local"));
    }

    #[test]
    fn given_the_terminal_backend_when_adjusted_then_it_cycles_and_wraps() {
        let mut config = Config::default();
        assert_eq!(config.terminal_backend, Backend::Builtin);
        assert_eq!(adjust(&mut config, Field::TerminalBackend, 1), Change::View);
        assert_eq!(config.terminal_backend, Backend::Alacritty);
        for _ in 0..Backend::ALL.len() - 1 {
            adjust(&mut config, Field::TerminalBackend, 1);
        }
        assert_eq!(config.terminal_backend, Backend::Builtin);
    }

    #[test]
    fn given_the_shell_when_cycled_past_the_end_then_it_returns_to_the_default() {
        let mut config = Config::default();
        let choices = crate::shells::choices();
        for _ in 0..choices.len() {
            assert_eq!(adjust(&mut config, Field::Shell, 1), Change::Shell);
        }
        assert!(config.shell.is_empty());
        if choices.len() > 1 {
            adjust(&mut config, Field::Shell, 1);
            assert_eq!(config.shell, choices[1].argv);
        }
    }

    #[test]
    fn given_compact_sidebar_when_toggled_then_it_flips() {
        let mut config = Config::default();
        assert_eq!(toggle(&mut config, Field::CompactSidebar), Change::View);
        assert!(config.compact_sidebar);
        assert_eq!(toggle(&mut config, Field::FontSize), Change::None);
    }

    #[test]
    fn given_close_exited_panes_when_toggled_then_it_flips() {
        let mut config = Config::default();
        assert_eq!(toggle(&mut config, Field::CloseExitedPanes), Change::View);
        assert!(!config.close_exited_panes);
        assert_eq!(toggle(&mut config, Field::CloseExitedPanes), Change::View);
        assert!(config.close_exited_panes);
    }

    #[test]
    fn given_restore_launch_flags_when_toggled_then_it_flips() {
        let mut config = Config::default();
        assert_eq!(toggle(&mut config, Field::RestoreLaunchFlags), Change::View);
        assert!(!config.restore_launch_flags);
    }

    #[test]
    fn given_claude_bypass_when_toggled_then_it_flips() {
        let mut config = Config::default();
        assert_eq!(toggle(&mut config, Field::ClaudeBypass), Change::View);
        assert!(!config.claude_bypass_permissions);
    }

    #[test]
    fn given_copy_on_select_when_toggled_then_it_flips() {
        let mut config = Config::default();
        assert_eq!(toggle(&mut config, Field::CopyOnSelect), Change::View);
        assert!(!config.copy_on_select);
    }

    #[test]
    fn given_a_new_chord_when_set_then_the_table_and_label_agree() {
        let mut config = Config::default();
        set_shortcut(&mut config, Action::NewTab, "ctrl+alt+t");
        assert_eq!(config.shortcuts.label(Action::NewTab), "ctrl+alt+t");
        let binding = config.shortcuts.binding(Action::NewTab).unwrap();
        assert!(binding.ctrl && binding.alt);
        set_shortcut(&mut config, Action::NewTab, "");
        assert!(config.shortcuts.binding(Action::NewTab).is_none());
    }
}
