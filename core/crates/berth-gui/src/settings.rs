//! Settings panel model: rows, values, and edits that persist to config.json.
//!
//! The panel edits the same document the macOS shell reads, so it writes only
//! the keys it owns and leaves the rest of the file alone.

use crate::config::{Action, BellMode, Config, MAX_FONT_SIZE, MIN_FONT_SIZE, MIN_OPACITY};
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
    fn given_the_settings_rows_when_built_then_every_action_has_a_row() {
        let config = Config::default();
        let rows = rows(&config, None, None);
        assert_eq!(rows.len(), 16 + Action::ALL.len());
        assert_eq!(rows[0].label, "Font");
        assert_eq!(rows[1].label, "Window font");
        assert_eq!(rows[2].label, "Font size");
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
