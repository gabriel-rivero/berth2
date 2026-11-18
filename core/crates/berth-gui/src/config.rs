//! The shared `config.json`, read the same way on every platform.
//!
//! The macOS shell owns the full schema; the portable window reads the keys it
//! can honor and ignores the rest, so one file serves both. Shortcut strings
//! use the macOS spelling (`cmd+d`); on Linux and Windows `cmd` means the
//! Super/Windows key and a separate default table keeps the chords usable.

use std::collections::HashMap;

use serde::Deserialize;

pub const DEFAULT_FONT_SIZE: f32 = 14.0;
pub const MIN_FONT_SIZE: f32 = 8.0;
pub const MAX_FONT_SIZE: f32 = 72.0;
pub const MIN_SIDEBAR_WIDTH: f64 = 180.0;
pub const MAX_SIDEBAR_WIDTH: f64 = 360.0;

/// Actions the portable window can bind. The names match the macOS config.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    SplitRight,
    SplitDown,
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    NextPane,
    PreviousPane,
    NewWorkspace,
    Palette,
    JumpAttention,
    ShowAttention,
    ToggleSidebar,
    Settings,
}

impl Action {
    pub const ALL: [Action; 14] = [
        Action::SplitRight,
        Action::SplitDown,
        Action::NewTab,
        Action::CloseTab,
        Action::NextTab,
        Action::PreviousTab,
        Action::NextPane,
        Action::PreviousPane,
        Action::NewWorkspace,
        Action::Palette,
        Action::JumpAttention,
        Action::ShowAttention,
        Action::ToggleSidebar,
        Action::Settings,
    ];

    /// How the palette spells the action.
    pub fn label(self) -> &'static str {
        match self {
            Action::SplitRight => "Split Right",
            Action::SplitDown => "Split Down",
            Action::NewTab => "New Tab",
            Action::CloseTab => "Close Tab",
            Action::NextTab => "Next Tab",
            Action::PreviousTab => "Previous Tab",
            Action::NextPane => "Next Pane",
            Action::PreviousPane => "Previous Pane",
            Action::NewWorkspace => "New Workspace",
            Action::Palette => "Command Palette",
            Action::JumpAttention => "Jump to Attention",
            Action::ShowAttention => "Attention List",
            Action::ToggleSidebar => "Toggle Sidebar",
            Action::Settings => "Settings",
        }
    }

    /// The `shortcuts` key this action is stored under.
    pub fn key(self) -> &'static str {
        match self {
            Action::SplitRight => "splitRight",
            Action::SplitDown => "splitDown",
            Action::NewTab => "newTab",
            Action::CloseTab => "closeSurface",
            Action::NextTab => "nextTab",
            Action::PreviousTab => "previousTab",
            Action::NextPane => "nextPane",
            Action::PreviousPane => "previousPane",
            Action::NewWorkspace => "newWorkspace",
            Action::Palette => "palette",
            Action::JumpAttention => "jumpAttention",
            Action::ShowAttention => "showAttention",
            Action::ToggleSidebar => "toggleSidebar",
            Action::Settings => "settings",
        }
    }

    /// The macOS default. These are the chords the Swift shell ships with.
    pub fn default_macos(self) -> &'static str {
        match self {
            Action::SplitRight => "cmd+d",
            Action::SplitDown => "cmd+shift+d",
            Action::NewTab => "cmd+t",
            Action::CloseTab => "cmd+w",
            Action::NextTab => "cmd+shift+]",
            Action::PreviousTab => "cmd+shift+[",
            Action::NextPane => "cmd+]",
            Action::PreviousPane => "cmd+[",
            Action::NewWorkspace => "cmd+n",
            Action::Palette => "cmd+shift+p",
            Action::JumpAttention => "cmd+shift+u",
            Action::ShowAttention => "cmd+i",
            Action::ToggleSidebar => "cmd+opt+s",
            Action::Settings => "cmd+,",
        }
    }

    /// The default outside macOS. `cmd` would fall on the Windows key, which
    /// the desktop captures, so the portable window uses Ctrl+Shift chords the
    /// way other Linux and Windows terminals do.
    pub fn default_portable(self) -> &'static str {
        match self {
            Action::SplitRight => "ctrl+shift+d",
            Action::SplitDown => "ctrl+shift+e",
            Action::NewTab => "ctrl+shift+t",
            Action::CloseTab => "ctrl+shift+w",
            Action::NextTab => "ctrl+shift+]",
            Action::PreviousTab => "ctrl+shift+[",
            Action::NextPane => "ctrl+alt+]",
            Action::PreviousPane => "ctrl+alt+[",
            Action::NewWorkspace => "ctrl+shift+n",
            Action::Palette => "ctrl+shift+p",
            Action::JumpAttention => "ctrl+shift+u",
            Action::ShowAttention => "ctrl+shift+i",
            Action::ToggleSidebar => "ctrl+alt+s",
            Action::Settings => "ctrl+,",
        }
    }
}

/// One parsed chord: modifiers plus a key as printed on the keyboard.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Binding {
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Lowercase for letters, otherwise the character as written.
    pub key: String,
}

impl Binding {
    /// Parse `cmd+shift+d`, `ctrl+]`, `alt+enter`. Returns `None` for an empty
    /// string or an unknown modifier name.
    pub fn parse(text: &str) -> Option<Self> {
        if text.trim_end().ends_with('+') {
            return None;
        }
        let mut binding = Self {
            cmd: false,
            ctrl: false,
            alt: false,
            shift: false,
            key: String::new(),
        };
        let parts: Vec<&str> = text
            .split('+')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();
        let (key, modifiers) = parts.split_last()?;
        for modifier in modifiers {
            match modifier.to_ascii_lowercase().as_str() {
                "cmd" | "command" | "super" | "meta" => binding.cmd = true,
                "ctrl" | "control" => binding.ctrl = true,
                "alt" | "opt" | "option" => binding.alt = true,
                "shift" => binding.shift = true,
                _ => return None,
            }
        }
        if key.is_empty() {
            return None;
        }
        binding.key = key.to_lowercase();
        Some(binding)
    }
}

impl Binding {
    /// Human-readable form, used by the palette and the settings list.
    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.cmd {
            parts.push("cmd".to_string());
        }
        if self.ctrl {
            parts.push("ctrl".to_string());
        }
        if self.alt {
            parts.push("alt".to_string());
        }
        if self.shift {
            parts.push("shift".to_string());
        }
        parts.push(self.key.clone());
        parts.join("+")
    }
}

/// Shortcut table with config overrides applied.
#[derive(Clone, Debug)]
pub struct Shortcuts {
    bindings: HashMap<Action, Option<Binding>>,
}

impl Shortcuts {
    /// Defaults for this platform, then any `shortcuts` entry in the file. An
    /// empty string unbinds an action, matching the macOS settings window.
    pub fn build(overrides: &HashMap<String, String>) -> Self {
        let portable = cfg!(not(target_os = "macos"));
        let mut bindings = HashMap::new();
        for action in Action::ALL {
            let default = if portable {
                action.default_portable()
            } else {
                action.default_macos()
            };
            let text = overrides
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(action.key()))
                .map(|(_, value)| value.as_str())
                .unwrap_or(default);
            bindings.insert(action, Binding::parse(text));
        }
        Self { bindings }
    }

    pub fn binding(&self, action: Action) -> Option<&Binding> {
        self.bindings.get(&action).and_then(Option::as_ref)
    }

    pub fn label(&self, action: Action) -> String {
        self.binding(action)
            .map(Binding::label)
            .unwrap_or_else(|| "unbound".to_string())
    }
}

/// The keys of `config.json` the portable window reads.
#[derive(Clone, Debug)]
pub struct Config {
    pub font_size: f32,
    /// `0` means automatic: `2 / backing scale`, clamped to `0.75..=2`.
    pub font_scale: f32,
    pub compact_sidebar: bool,
    pub sidebar_width: f64,
    pub socket: Option<String>,
    pub shortcuts: Shortcuts,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_FONT_SIZE,
            font_scale: 0.0,
            compact_sidebar: false,
            sidebar_width: 200.0,
            socket: None,
            shortcuts: Shortcuts::build(&HashMap::new()),
        }
    }
}

/// The raw file. Unknown keys are ignored by serde's default behavior.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct FileConfig {
    font_size: Option<f32>,
    font_scale: Option<f32>,
    compact_sidebar: Option<bool>,
    sidebar_width: Option<f64>,
    socket: Option<String>,
    shortcuts: HashMap<String, String>,
}

impl Config {
    /// Read `paths::config_path()`. A missing file is the default config; a
    /// malformed one is reported and ignored, never fatal.
    pub fn load() -> (Self, Option<String>) {
        let path = berth_core::paths::config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => match Self::parse(&text) {
                Ok(config) => (config, None),
                Err(error) => (
                    Self::default(),
                    Some(format!("{}: {error}", path.display())),
                ),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(error) => (
                Self::default(),
                Some(format!("{}: {error}", path.display())),
            ),
        }
    }

    pub fn parse(text: &str) -> Result<Self, serde_json::Error> {
        let file: FileConfig = serde_json::from_str(text)?;
        Ok(Self::from_file(file))
    }

    fn from_file(file: FileConfig) -> Self {
        let defaults = Self::default();
        Self {
            font_size: file
                .font_size
                .unwrap_or(defaults.font_size)
                .clamp(MIN_FONT_SIZE, MAX_FONT_SIZE),
            font_scale: file
                .font_scale
                .unwrap_or(defaults.font_scale)
                .clamp(0.0, 3.0),
            compact_sidebar: file.compact_sidebar.unwrap_or(defaults.compact_sidebar),
            sidebar_width: file
                .sidebar_width
                .unwrap_or(defaults.sidebar_width)
                .clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH),
            socket: file.socket,
            shortcuts: Shortcuts::build(&file.shortcuts),
        }
    }

    /// Pixel size for the terminal font on this display. Mirrors the macOS
    /// rule: an automatic scale of `2 / backing` keeps the physical glyph size
    /// stable between Retina and external monitors.
    pub fn font_pixels(&self, backing_scale: f64) -> f32 {
        let backing = backing_scale.max(0.5);
        let multiplier = if self.font_scale > 0.0 {
            self.font_scale as f64
        } else {
            (2.0 / backing).clamp(0.75, 2.0)
        };
        (self.font_size as f64 * multiplier * backing) as f32
    }

    pub fn sidebar_pixels(&self) -> f64 {
        self.sidebar_width
    }

    pub fn socket_mode(&self) -> Option<&str> {
        self.socket.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_a_missing_file_when_loaded_then_defaults_apply() {
        let config = Config::default();
        assert_eq!(config.font_size, DEFAULT_FONT_SIZE);
        assert_eq!(config.font_scale, 0.0);
        assert!(!config.compact_sidebar);
    }

    #[test]
    fn given_a_partial_file_when_parsed_then_other_keys_keep_defaults() {
        let config = Config::parse(r#"{"fontSize": 18, "compactSidebar": true}"#).unwrap();
        assert_eq!(config.font_size, 18.0);
        assert!(config.compact_sidebar);
        assert_eq!(config.sidebar_width, Config::default().sidebar_width);
    }

    #[test]
    fn given_unknown_keys_when_parsed_then_they_are_ignored() {
        let config = Config::parse(r#"{"fontSize": 12, "futureKey": {"a": 1}}"#).unwrap();
        assert_eq!(config.font_size, 12.0);
    }

    #[test]
    fn given_out_of_range_font_when_parsed_then_it_is_clamped() {
        assert_eq!(Config::parse(r#"{"fontSize": 4}"#).unwrap().font_size, 8.0);
        assert_eq!(
            Config::parse(r#"{"fontSize": 200}"#).unwrap().font_size,
            72.0
        );
    }

    #[test]
    fn given_an_automatic_scale_when_computed_then_retina_and_external_match() {
        let config = Config::default();
        let retina = config.font_pixels(2.0);
        let external = config.font_pixels(1.0);
        assert!((retina - external).abs() < 0.01);
    }

    #[test]
    fn given_a_fixed_scale_when_computed_then_it_overrides_the_automatic_rule() {
        let config = Config {
            font_scale: 1.0,
            ..Config::default()
        };
        assert!((config.font_pixels(2.0) - 28.0).abs() < 0.01);
    }

    #[test]
    fn given_a_shortcut_string_when_parsed_then_modifiers_and_key_are_split() {
        let binding = Binding::parse("cmd+shift+d").unwrap();
        assert!(binding.cmd && binding.shift);
        assert!(!binding.ctrl && !binding.alt);
        assert_eq!(binding.key, "d");
        assert_eq!(binding.label(), "cmd+shift+d");
    }

    #[test]
    fn given_every_action_when_labelled_then_the_palette_has_a_name() {
        for action in Action::ALL {
            assert!(!action.label().is_empty());
        }
        assert_eq!(Action::JumpAttention.label(), "Jump to Attention");
    }

    #[test]
    fn given_a_bracket_shortcut_when_parsed_then_the_key_survives() {
        let binding = Binding::parse("ctrl+alt+]").unwrap();
        assert_eq!(binding.key, "]");
        assert!(binding.ctrl && binding.alt);
    }

    #[test]
    fn given_garbage_when_parsed_then_no_binding_is_produced() {
        assert!(Binding::parse("").is_none());
        assert!(Binding::parse("hyper+d").is_none());
        assert!(Binding::parse("cmd+").is_none());
    }

    #[test]
    fn given_a_config_override_when_built_then_it_replaces_the_default() {
        let mut overrides = HashMap::new();
        overrides.insert("splitRight".to_string(), "ctrl+alt+1".to_string());
        overrides.insert("newTab".to_string(), String::new());
        let shortcuts = Shortcuts::build(&overrides);
        let split = shortcuts.binding(Action::SplitRight).unwrap();
        assert!(split.ctrl && split.alt);
        assert_eq!(split.key, "1");
        assert!(shortcuts.binding(Action::NewTab).is_none());
        assert!(shortcuts.binding(Action::CloseTab).is_some());
    }

    #[test]
    fn given_the_platform_defaults_when_built_then_every_action_is_bound() {
        let shortcuts = Shortcuts::build(&HashMap::new());
        for action in Action::ALL {
            assert!(
                shortcuts.binding(action).is_some(),
                "{:?} has no default",
                action
            );
        }
    }

    #[test]
    fn given_a_socket_key_when_parsed_then_it_is_exposed() {
        let config = Config::parse(r#"{"socket": "off"}"#).unwrap();
        assert_eq!(config.socket_mode(), Some("off"));
    }
}
