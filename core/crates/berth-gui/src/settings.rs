//! Settings panel model: rows, values, and edits that persist to config.json.
//!
//! The panel edits the same document the macOS shell reads, so it writes only
//! the keys it owns and leaves the rest of the file alone.

use crate::config::{Action, Config, MAX_FONT_SIZE, MIN_FONT_SIZE};

/// Values the display-scale row cycles through. `0` means automatic.
const SCALES: [f32; 9] = [0.0, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 2.5, 3.0];

const SOCKET_MODES: [Option<&str>; 3] = [Some("local"), Some("trusted"), Some("off")];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    FontSize,
    FontScale,
    CompactSidebar,
    CloseExitedPanes,
    RestoreLaunchFlags,
    ClaudeBypass,
    CopyOnSelect,
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
    /// Font metrics changed; the renderer must be rebuilt.
    Font,
    /// Layout or bindings changed; only a redraw is needed.
    View,
}

/// The rows of the settings panel, in display order.
pub fn rows(config: &Config, recording: Option<Action>) -> Vec<Row> {
    let mut rows = vec![
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

/// Move a numeric or cycling value by one step.
pub fn adjust(config: &mut Config, field: Field, delta: i32) -> Change {
    match field {
        Field::FontSize => {
            let size = (config.font_size + delta as f32).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
            if (size - config.font_size).abs() < f32::EPSILON {
                return Change::None;
            }
            config.font_size = size;
            Change::Font
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
        Field::CompactSidebar
        | Field::CloseExitedPanes
        | Field::RestoreLaunchFlags
        | Field::ClaudeBypass
        | Field::CopyOnSelect
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
        let rows = rows(&config, None);
        assert_eq!(rows.len(), 8 + Action::ALL.len());
        assert_eq!(rows[0].label, "Font size");
    }

    #[test]
    fn given_a_recording_row_when_built_then_it_asks_for_a_chord() {
        let config = Config::default();
        let rows = rows(&config, Some(Action::NewTab));
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
