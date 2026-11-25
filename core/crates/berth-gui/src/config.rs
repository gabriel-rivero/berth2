//! The shared `config.json`, read the same way on every platform.
//!
//! The macOS shell owns the full schema; the portable window reads the keys it
//! can honor and ignores the rest, so one file serves both. Shortcut strings
//! use the macOS spelling (`cmd+d`); on Linux and Windows `cmd` means the
//! Super/Windows key and a separate default table keeps the chords usable.

use std::collections::HashMap;

use berth_headless::engine::Backend;
use serde::Deserialize;

pub const DEFAULT_FONT_SIZE: f32 = 14.0;
pub const MIN_FONT_SIZE: f32 = 8.0;
pub const MAX_FONT_SIZE: f32 = 72.0;
pub const MIN_OPACITY: f32 = 0.3;
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
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    NewWorkspace,
    NewWindow,
    NextWindow,
    TogglePaneZoom,
    ToggleBroadcast,
    DuplicateTab,
    RestartTab,
    SidebarFilter,
    Palette,
    JumpAttention,
    ShowAttention,
    ToggleSidebar,
    Settings,
    Search,
    PreviousPrompt,
    NextPrompt,
    CopyMode,
    PasteHistory,
    QuickTerminal,
    ZoomIn,
    ZoomOut,
    ZoomReset,
}

impl Action {
    pub const ALL: [Action; 34] = [
        Action::SplitRight,
        Action::SplitDown,
        Action::NewTab,
        Action::CloseTab,
        Action::NextTab,
        Action::PreviousTab,
        Action::NextPane,
        Action::PreviousPane,
        Action::FocusLeft,
        Action::FocusRight,
        Action::FocusUp,
        Action::FocusDown,
        Action::NewWorkspace,
        Action::NewWindow,
        Action::NextWindow,
        Action::TogglePaneZoom,
        Action::ToggleBroadcast,
        Action::DuplicateTab,
        Action::RestartTab,
        Action::SidebarFilter,
        Action::Palette,
        Action::JumpAttention,
        Action::ShowAttention,
        Action::ToggleSidebar,
        Action::Settings,
        Action::Search,
        Action::PreviousPrompt,
        Action::NextPrompt,
        Action::CopyMode,
        Action::PasteHistory,
        Action::QuickTerminal,
        Action::ZoomIn,
        Action::ZoomOut,
        Action::ZoomReset,
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
            Action::FocusLeft => "Focus Pane Left",
            Action::FocusRight => "Focus Pane Right",
            Action::FocusUp => "Focus Pane Up",
            Action::FocusDown => "Focus Pane Down",
            Action::NewWorkspace => "New Workspace",
            Action::NewWindow => "New Window",
            Action::NextWindow => "Next Window",
            Action::TogglePaneZoom => "Zoom Pane",
            Action::ToggleBroadcast => "Broadcast Input",
            Action::DuplicateTab => "Duplicate Tab",
            Action::RestartTab => "Restart Tab",
            Action::SidebarFilter => "Filter Sidebar",
            Action::Palette => "Command Palette",
            Action::JumpAttention => "Jump to Attention",
            Action::ShowAttention => "Attention List",
            Action::ToggleSidebar => "Toggle Sidebar",
            Action::Settings => "Settings",
            Action::Search => "Search Scrollback",
            Action::PreviousPrompt => "Previous Prompt",
            Action::NextPrompt => "Next Prompt",
            Action::CopyMode => "Copy Mode",
            Action::PasteHistory => "Paste History",
            Action::QuickTerminal => "Quick Terminal",
            Action::ZoomIn => "Zoom In",
            Action::ZoomOut => "Zoom Out",
            Action::ZoomReset => "Actual Size",
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
            Action::FocusLeft => "focusLeft",
            Action::FocusRight => "focusRight",
            Action::FocusUp => "focusUp",
            Action::FocusDown => "focusDown",
            Action::NewWorkspace => "newWorkspace",
            Action::NewWindow => "newWindow",
            Action::NextWindow => "nextWindow",
            Action::TogglePaneZoom => "togglePaneZoom",
            Action::ToggleBroadcast => "toggleBroadcast",
            Action::DuplicateTab => "duplicateTab",
            Action::RestartTab => "restartTab",
            Action::SidebarFilter => "sidebarFilter",
            Action::Palette => "palette",
            Action::JumpAttention => "jumpAttention",
            Action::ShowAttention => "showAttention",
            Action::ToggleSidebar => "toggleSidebar",
            Action::Settings => "settings",
            Action::Search => "search",
            Action::PreviousPrompt => "previousPrompt",
            Action::NextPrompt => "nextPrompt",
            Action::CopyMode => "copyMode",
            Action::PasteHistory => "pasteHistory",
            Action::QuickTerminal => "quickTerminal",
            Action::ZoomIn => "zoomIn",
            Action::ZoomOut => "zoomOut",
            Action::ZoomReset => "zoomReset",
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
            Action::FocusLeft => "cmd+opt+left",
            Action::FocusRight => "cmd+opt+right",
            Action::FocusUp => "cmd+opt+up",
            Action::FocusDown => "cmd+opt+down",
            Action::NewWorkspace => "cmd+n",
            Action::NewWindow => "cmd+shift+n",
            Action::NextWindow => "cmd+`",
            Action::TogglePaneZoom => "cmd+shift+enter",
            Action::ToggleBroadcast => "cmd+shift+b",
            Action::DuplicateTab => "cmd+shift+k",
            Action::RestartTab => "cmd+shift+r",
            Action::SidebarFilter => "cmd+opt+f",
            Action::Palette => "cmd+shift+p",
            Action::JumpAttention => "cmd+shift+u",
            Action::ShowAttention => "cmd+i",
            Action::ToggleSidebar => "cmd+opt+s",
            Action::Settings => "cmd+,",
            Action::Search => "cmd+f",
            Action::PreviousPrompt => "cmd+shift+up",
            Action::NextPrompt => "cmd+shift+down",
            Action::CopyMode => "cmd+shift+c",
            Action::PasteHistory => "cmd+shift+v",
            Action::QuickTerminal => "cmd+shift+`",
            Action::ZoomIn => "cmd+=",
            Action::ZoomOut => "cmd+-",
            Action::ZoomReset => "cmd+0",
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
            Action::FocusLeft => "ctrl+alt+left",
            Action::FocusRight => "ctrl+alt+right",
            Action::FocusUp => "ctrl+alt+up",
            Action::FocusDown => "ctrl+alt+down",
            Action::NewWorkspace => "ctrl+shift+n",
            Action::NewWindow => "ctrl+alt+n",
            Action::NextWindow => "ctrl+alt+`",
            Action::TogglePaneZoom => "ctrl+shift+enter",
            Action::ToggleBroadcast => "ctrl+shift+b",
            Action::DuplicateTab => "ctrl+shift+k",
            Action::RestartTab => "ctrl+shift+r",
            Action::SidebarFilter => "ctrl+alt+f",
            Action::Palette => "ctrl+shift+p",
            Action::JumpAttention => "ctrl+shift+u",
            Action::ShowAttention => "ctrl+shift+i",
            Action::ToggleSidebar => "ctrl+alt+s",
            Action::Settings => "ctrl+,",
            Action::Search => "ctrl+shift+f",
            Action::PreviousPrompt => "ctrl+shift+up",
            Action::NextPrompt => "ctrl+shift+down",
            Action::CopyMode => "ctrl+shift+space",
            Action::PasteHistory => "ctrl+alt+v",
            Action::QuickTerminal => "ctrl+shift+`",
            Action::ZoomIn => "ctrl+=",
            Action::ZoomOut => "ctrl+-",
            Action::ZoomReset => "ctrl+0",
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

/// Shortcut table with config overrides applied. The raw strings are kept so
/// the settings window can rewrite the file without inventing values.
#[derive(Clone, Debug)]
pub struct Shortcuts {
    bindings: HashMap<Action, Option<Binding>>,
    raw: HashMap<Action, String>,
}

impl Shortcuts {
    /// Defaults for this platform, then any `shortcuts` entry in the file. An
    /// empty string unbinds an action, matching the macOS settings window.
    pub fn build(overrides: &HashMap<String, String>) -> Self {
        let portable = cfg!(not(target_os = "macos"));
        let mut bindings = HashMap::new();
        let mut raw = HashMap::new();
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
            raw.insert(action, text.to_string());
        }
        Self { bindings, raw }
    }

    /// Replace one action's chord. An empty string unbinds it.
    pub fn set(&mut self, action: Action, text: &str) {
        self.bindings.insert(action, Binding::parse(text));
        self.raw.insert(action, text.to_string());
    }

    /// Every action as `(config key, chord)`, in palette order.
    pub fn entries(&self) -> Vec<(String, String)> {
        Action::ALL
            .iter()
            .map(|action| (action.key().to_string(), self.raw_text(*action)))
            .collect()
    }

    fn raw_text(&self, action: Action) -> String {
        self.raw
            .get(&action)
            .cloned()
            .unwrap_or_else(|| "unbound".to_string())
    }

    pub fn binding(&self, action: Action) -> Option<&Binding> {
        self.bindings.get(&action).and_then(Option::as_ref)
    }

    pub fn label(&self, action: Action) -> String {
        self.raw_text(action)
    }
}

/// The keys of `config.json` the portable window reads.
#[derive(Clone, Debug)]
pub struct Config {
    pub font_size: f32,
    /// `0` means automatic: `2 / backing scale`, clamped to `0.75..=2`.
    pub font_scale: f32,
    /// Preferred terminal faces, first installed one wins. Empty keeps the
    /// system monospace.
    pub font_family: Vec<String>,
    /// Base weight for normal text, 100..=900. Bold cells use at least 700.
    pub font_weight: u16,
    /// Line height as a multiple of the font size.
    pub line_height: f32,
    /// OpenType features, `"calt"` to enable or `"-liga"` to disable.
    pub font_features: Vec<String>,
    /// Per-family cell overrides, keyed by face name.
    pub font_metrics: HashMap<String, FaceMetrics>,
    /// Palette name; empty and unknown names use the dark default.
    pub theme: String,
    /// Window opacity, 0.3..=1.0. One is opaque.
    pub opacity: f32,
    /// What a bell does. The default flashes the pane border.
    pub bell: BellMode,
    pub compact_sidebar: bool,
    /// Close a pane when its process exits instead of keeping the final screen
    /// with a banner. On by default, like a terminal tab.
    pub close_exited_panes: bool,
    /// Replay a saved agent launch command's extra flags on resume, such as
    /// `--allow-dangerously-skip-permissions`. On by default.
    pub restore_launch_flags: bool,
    /// Start a restored Claude session with permissions bypassed. On by
    /// default; the injected mode is not written back into the saved payload.
    pub claude_bypass_permissions: bool,
    /// Copy a mouse selection when the button is released. On by default.
    pub copy_on_select: bool,
    /// Rows of history kept in memory per surface. Read at launch.
    pub scrollback_lines: usize,
    /// Decode terminal image protocols and paint them. Read at launch.
    pub graphics: bool,
    /// Program a new terminal runs, as argv. Empty means the platform default
    /// shell (`$SHELL` on Unix, `%ComSpec%` on Windows).
    pub shell: Vec<String>,
    /// Parser/grid for new surfaces. Read at launch; unavailable backends fall
    /// back to the builtin engine.
    pub terminal_backend: Backend,
    /// Extra arguments appended to a provider's resume command, by provider id.
    pub resume_args: std::collections::HashMap<String, Vec<String>>,
    /// Providers whose OS notifications are silenced; attention stays in-app.
    pub mute_providers: Vec<String>,
    pub sidebar_width: f64,
    pub socket: Option<String>,
    pub shortcuts: Shortcuts,
    /// The dropdown quick terminal's global hotkey, placement and height.
    pub quick: crate::quick::Options,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font_size: DEFAULT_FONT_SIZE,
            font_scale: 0.0,
            font_family: Vec::new(),
            font_weight: 400,
            line_height: 1.35,
            font_features: Vec::new(),
            font_metrics: HashMap::new(),
            theme: String::new(),
            opacity: 1.0,
            bell: BellMode::default(),
            compact_sidebar: false,
            close_exited_panes: true,
            restore_launch_flags: true,
            claude_bypass_permissions: true,
            copy_on_select: true,
            scrollback_lines: 4_000,
            graphics: true,
            shell: Vec::new(),
            terminal_backend: Backend::Builtin,
            resume_args: std::collections::HashMap::new(),
            mute_providers: Vec::new(),
            sidebar_width: 200.0,
            socket: None,
            shortcuts: Shortcuts::build(&HashMap::new()),
            quick: crate::quick::Options::default(),
        }
    }
}

/// What a bell does: nothing, the pane flash, or the system sound.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BellMode {
    None,
    #[default]
    Visual,
    Audio,
}

impl BellMode {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "visual" => Some(Self::Visual),
            "audio" => Some(Self::Audio),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Visual => "visual",
            Self::Audio => "audio",
        }
    }
}

/// `fontFamily` takes one name or a list, the same shape Ghostty accepts.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FamilyNames {
    One(String),
    Many(Vec<String>),
}

impl FamilyNames {
    fn into_names(self) -> Vec<String> {
        let names = match self {
            Self::One(name) => vec![name],
            Self::Many(names) => names,
        };
        names
            .into_iter()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect()
    }
}

/// `fontWeight` takes a numeric weight or a common name.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WeightValue {
    Number(f32),
    Name(String),
}

impl WeightValue {
    fn into_weight(self) -> u16 {
        match self {
            Self::Number(value) if value > 0.0 => (value.round() as u16).clamp(100, 900),
            Self::Number(_) => 400,
            Self::Name(name) => {
                let name: String = name
                    .chars()
                    .filter(|character| character.is_ascii_alphanumeric())
                    .collect::<String>()
                    .to_ascii_lowercase();
                match name.as_str() {
                    "thin" => 100,
                    "extralight" | "ultralight" => 200,
                    "light" => 300,
                    "medium" => 500,
                    "semibold" | "demibold" => 600,
                    "bold" => 700,
                    "extrabold" | "ultrabold" => 800,
                    "black" | "heavy" => 900,
                    _ => 400,
                }
            }
        }
    }
}

/// `fontMetrics` overrides the measured cell for one family.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FaceMetrics {
    pub width: Option<usize>,
    pub height: Option<usize>,
}

fn feature_tag(text: &str) -> Option<String> {
    let text = text.trim();
    let (enabled, tag) = match text.strip_prefix('-') {
        Some(tag) => (false, tag),
        None => (true, text.strip_prefix('+').unwrap_or(text)),
    };
    if tag.len() != 4 || !tag.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return None;
    }
    Some(if enabled {
        tag.to_string()
    } else {
        format!("-{tag}")
    })
}

/// `shell` takes a program path, or a program plus arguments. A single string
/// is the program alone, so a Windows path with spaces needs no quoting.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ShellSpec {
    Program(String),
    Argv(Vec<String>),
}

impl ShellSpec {
    fn into_argv(self) -> Vec<String> {
        let mut argv = match self {
            Self::Program(program) => vec![program.trim().to_string()],
            Self::Argv(argv) => argv,
        };
        argv.retain(|part| !part.is_empty());
        argv
    }
}

/// The raw file. Unknown keys are ignored by serde's default behavior.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct FileConfig {
    font_size: Option<f32>,
    font_scale: Option<f32>,
    font_family: Option<FamilyNames>,
    font_weight: Option<WeightValue>,
    line_height: Option<f32>,
    font_features: Option<Vec<String>>,
    font_metrics: Option<HashMap<String, FaceMetrics>>,
    theme: Option<String>,
    opacity: Option<f32>,
    bell: Option<String>,
    compact_sidebar: Option<bool>,
    close_exited_panes: Option<bool>,
    restore_launch_flags: Option<bool>,
    claude_bypass_permissions: Option<bool>,
    copy_on_select: Option<bool>,
    scrollback_lines: Option<usize>,
    graphics: Option<bool>,
    shell: Option<ShellSpec>,
    terminal_backend: Option<String>,
    resume_args: Option<std::collections::HashMap<String, Vec<String>>>,
    mute_providers: Option<Vec<String>>,
    sidebar_width: Option<f64>,
    socket: Option<String>,
    shortcuts: HashMap<String, String>,
    quick_hotkey: Option<String>,
    quick_placement: Option<String>,
    quick_height: Option<f64>,
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
            font_family: file
                .font_family
                .map(FamilyNames::into_names)
                .unwrap_or_default(),
            font_weight: file
                .font_weight
                .map(WeightValue::into_weight)
                .unwrap_or(defaults.font_weight),
            line_height: file
                .line_height
                .filter(|height| *height > 0.0)
                .unwrap_or(defaults.line_height)
                .clamp(0.8, 3.0),
            font_features: file
                .font_features
                .unwrap_or_default()
                .into_iter()
                .filter_map(|feature| feature_tag(&feature))
                .collect(),
            font_metrics: file
                .font_metrics
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(name, metrics)| {
                    let name = name.trim().to_string();
                    if name.is_empty() {
                        return None;
                    }
                    let metrics = FaceMetrics {
                        width: metrics.width.map(|width| width.clamp(1, 256)),
                        height: metrics.height.map(|height| height.clamp(1, 256)),
                    };
                    (metrics.width.is_some() || metrics.height.is_some()).then_some((name, metrics))
                })
                .collect(),
            theme: file.theme.unwrap_or_default().trim().to_string(),
            opacity: file
                .opacity
                .filter(|value| value.is_finite())
                .map(|value| value.clamp(MIN_OPACITY, 1.0))
                .unwrap_or(defaults.opacity),
            bell: file
                .bell
                .as_deref()
                .and_then(BellMode::parse)
                .unwrap_or(defaults.bell),
            compact_sidebar: file.compact_sidebar.unwrap_or(defaults.compact_sidebar),
            close_exited_panes: file
                .close_exited_panes
                .unwrap_or(defaults.close_exited_panes),
            restore_launch_flags: file
                .restore_launch_flags
                .unwrap_or(defaults.restore_launch_flags),
            claude_bypass_permissions: file
                .claude_bypass_permissions
                .unwrap_or(defaults.claude_bypass_permissions),
            copy_on_select: file.copy_on_select.unwrap_or(defaults.copy_on_select),
            scrollback_lines: file
                .scrollback_lines
                .unwrap_or(defaults.scrollback_lines)
                .min(100_000),
            graphics: file.graphics.unwrap_or(defaults.graphics),
            shell: file.shell.map(ShellSpec::into_argv).unwrap_or_default(),
            terminal_backend: file
                .terminal_backend
                .as_deref()
                .and_then(Backend::parse)
                .unwrap_or_default(),
            resume_args: file.resume_args.unwrap_or(defaults.resume_args),
            mute_providers: file.mute_providers.unwrap_or(defaults.mute_providers),
            sidebar_width: file
                .sidebar_width
                .unwrap_or(defaults.sidebar_width)
                .clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH),
            socket: file.socket,
            shortcuts: Shortcuts::build(&file.shortcuts),
            quick: crate::quick::Options::from_config(
                file.quick_hotkey.as_deref(),
                file.quick_placement.as_deref(),
                file.quick_height,
            ),
        }
    }

    /// Pick the primary face for the next font rebuild. `None` returns to the
    /// built-in default; a name moves to the front and keeps the fallbacks
    /// that were already configured after it.
    pub fn set_font_family(&mut self, primary: Option<String>) {
        match primary {
            None => self.font_family.clear(),
            Some(name) => {
                self.font_family
                    .retain(|existing| !existing.eq_ignore_ascii_case(&name));
                self.font_family.insert(0, name);
            }
        }
    }

    /// The palette this config selects. An unknown name keeps the dark theme.
    pub fn theme(&self) -> crate::theme::Theme {
        crate::theme::Theme::named(&self.theme).unwrap_or_default()
    }

    /// Pixel size for the terminal font on this display. Mirrors the macOS
    /// rule: an automatic scale of `2 / backing` keeps the physical glyph size
    /// stable between Retina and external monitors. `points` is the configured
    /// size plus any zoom for this session.
    pub fn font_pixels(&self, points: f32, backing_scale: f64) -> f32 {
        let backing = backing_scale.max(0.5);
        let multiplier = if self.font_scale > 0.0 {
            self.font_scale as f64
        } else {
            (2.0 / backing).clamp(0.75, 2.0)
        };
        (points as f64 * multiplier * backing) as f32
    }

    /// The renderer options this config selects.
    pub fn font_options(&self) -> crate::font::Options {
        crate::font::Options {
            weight: self.font_weight,
            line_height: self.line_height,
            features: self
                .font_features
                .iter()
                .filter_map(|feature| crate::font::parse_feature(feature))
                .collect(),
            metrics: self
                .font_metrics
                .iter()
                .map(|(name, metrics)| {
                    (
                        name.clone(),
                        crate::font::FaceMetrics {
                            width: metrics.width,
                            height: metrics.height,
                        },
                    )
                })
                .collect(),
        }
    }

    pub fn sidebar_pixels(&self) -> f64 {
        self.sidebar_width
    }

    pub fn socket_mode(&self) -> Option<&str> {
        self.socket.as_deref()
    }

    /// Write the keys this window owns, preserving anything else in the file.
    /// The macOS shell reads the same document, so unknown keys must survive.
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&berth_core::paths::config_path())
    }

    /// Save to an explicit path. Kept separate so the merge can be tested
    /// without touching the user's real config.
    pub fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        let mut document = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        let object = document.as_object_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "config is not an object")
        })?;
        object.insert("fontSize".into(), serde_json::json!(self.font_size));
        object.insert("fontScale".into(), serde_json::json!(self.font_scale));
        if !self.theme.is_empty() {
            object.insert("theme".into(), serde_json::json!(self.theme));
        }
        // One is the default and means "inherit"; writing it would tell the
        // AppKit shell to override the user's Ghostty transparency.
        if (self.opacity - 1.0).abs() > f32::EPSILON {
            object.insert("opacity".into(), serde_json::json!(self.opacity));
        } else {
            object.remove("opacity");
        }
        object.insert("bell".into(), serde_json::json!(self.bell.name()));
        if self.font_family.is_empty() {
            // Picking the default must clear a family a previous save wrote.
            object.remove("fontFamily");
        } else {
            object.insert("fontFamily".into(), serde_json::json!(self.font_family));
        }
        object.insert(
            "compactSidebar".into(),
            serde_json::json!(self.compact_sidebar),
        );
        object.insert(
            "closeExitedPanes".into(),
            serde_json::json!(self.close_exited_panes),
        );
        object.insert(
            "restoreLaunchFlags".into(),
            serde_json::json!(self.restore_launch_flags),
        );
        object.insert(
            "claudeBypassPermissions".into(),
            serde_json::json!(self.claude_bypass_permissions),
        );
        object.insert(
            "copyOnSelect".into(),
            serde_json::json!(self.copy_on_select),
        );
        object.insert(
            "scrollbackLines".into(),
            serde_json::json!(self.scrollback_lines),
        );
        object.insert("graphics".into(), serde_json::json!(self.graphics));
        if self.shell.is_empty() {
            // Cycling back to the default must clear a previously saved shell.
            object.remove("shell");
        } else {
            object.insert("shell".into(), serde_json::json!(self.shell));
        }
        if self.terminal_backend == Backend::Builtin {
            object.remove("terminalBackend");
        } else {
            object.insert(
                "terminalBackend".into(),
                serde_json::json!(self.terminal_backend.name()),
            );
        }
        if !self.mute_providers.is_empty() {
            object.insert(
                "muteProviders".into(),
                serde_json::json!(self.mute_providers),
            );
        }
        if !self.resume_args.is_empty() {
            object.insert("resumeArgs".into(), serde_json::json!(self.resume_args));
        }
        object.insert("sidebarWidth".into(), serde_json::json!(self.sidebar_width));
        if let Some(socket) = &self.socket {
            object.insert("socket".into(), serde_json::json!(socket));
        }
        let shortcuts: serde_json::Map<String, serde_json::Value> = self
            .shortcuts
            .entries()
            .into_iter()
            .map(|(key, value)| (key, serde_json::json!(value)))
            .collect();
        object.insert("shortcuts".into(), serde_json::Value::Object(shortcuts));
        let text = serde_json::to_string_pretty(&document)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, text.as_bytes())?;
        std::fs::rename(&temp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique path under the temp directory, so parallel tests cannot share
    /// a config file.
    fn temp_path(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "berth-config-{prefix}-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn given_a_missing_file_when_loaded_then_defaults_apply() {
        let config = Config::default();
        assert_eq!(config.font_size, DEFAULT_FONT_SIZE);
        assert_eq!(config.font_scale, 0.0);
        assert!(!config.compact_sidebar);
        assert!(config.close_exited_panes);
        assert!(config.restore_launch_flags);
        assert!(config.claude_bypass_permissions);
        assert!(config.copy_on_select);
        assert!(config.graphics);
        assert_eq!(config.scrollback_lines, 4000);
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
        let retina = config.font_pixels(config.font_size, 2.0);
        let external = config.font_pixels(config.font_size, 1.0);
        assert!((retina - external).abs() < 0.01);
    }

    #[test]
    fn given_a_fixed_scale_when_computed_then_it_overrides_the_automatic_rule() {
        let config = Config {
            font_scale: 1.0,
            ..Config::default()
        };
        assert!((config.font_pixels(config.font_size, 2.0) - 28.0).abs() < 0.01);
    }

    #[test]
    fn given_a_shortcut_string_when_parsed_then_modifiers_and_key_are_split() {
        let binding = Binding::parse("cmd+shift+d").unwrap();
        assert!(binding.cmd && binding.shift);
        assert!(!binding.ctrl && !binding.alt);
        assert_eq!(binding.key, "d");
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
    fn given_a_file_with_unknown_keys_when_saved_then_they_survive() {
        let path = temp_path("unknown");
        std::fs::write(&path, r#"{"fontSize": 10, "futureKey": {"a": 1}}"#).unwrap();
        let mut config = Config::parse(r#"{"fontSize": 10}"#).unwrap();
        config.font_size = 18.0;
        config.shortcuts.set(Action::NewTab, "ctrl+alt+t");
        config.save_to(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["fontSize"], 18.0);
        assert_eq!(saved["futureKey"]["a"], 1);
        assert_eq!(saved["shortcuts"]["newTab"], "ctrl+alt+t");
        assert!(saved["shortcuts"]["splitRight"].is_string());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn given_a_shell_as_a_program_when_parsed_then_it_is_a_one_element_argv() {
        let config =
            Config::parse(r#"{"shell": "C:\\Program Files\\Git\\bin\\bash.exe"}"#).unwrap();
        assert_eq!(
            config.shell,
            vec![r"C:\Program Files\Git\bin\bash.exe".to_string()]
        );
    }

    #[test]
    fn given_a_shell_as_an_argv_when_parsed_then_the_arguments_survive() {
        let config = Config::parse(r#"{"shell": ["wsl.exe", "-d", "Ubuntu"]}"#).unwrap();
        assert_eq!(config.shell, vec!["wsl.exe", "-d", "Ubuntu"]);
    }

    #[test]
    fn given_a_saved_shell_when_cleared_then_the_key_is_removed() {
        let path = temp_path("shell");
        let mut config = Config::parse(r#"{"shell": ["zsh", "-l"], "fontSize": 12}"#).unwrap();
        assert_eq!(config.shell, vec!["zsh", "-l"]);
        config.shell.clear();
        config.save_to(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(saved.get("shell").is_none());
        assert_eq!(saved["fontSize"], 12.0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn given_close_exited_panes_false_when_parsed_then_it_is_kept() {
        let config = Config::parse(r#"{"closeExitedPanes": false}"#).unwrap();
        assert!(!config.close_exited_panes);
        let saved_path = temp_path("close-exited");
        config.save_to(&saved_path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&saved_path).unwrap()).unwrap();
        assert_eq!(saved["closeExitedPanes"], false);
        assert!(
            !Config::parse(&std::fs::read_to_string(&saved_path).unwrap())
                .unwrap()
                .close_exited_panes
        );
        let _ = std::fs::remove_file(&saved_path);
    }

    #[test]
    fn given_restore_launch_flags_false_when_parsed_then_it_is_kept() {
        let config = Config::parse(r#"{"restoreLaunchFlags": false}"#).unwrap();
        assert!(!config.restore_launch_flags);
        let path = temp_path("launch-flags");
        config.save_to(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["restoreLaunchFlags"], false);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn given_claude_bypass_false_when_parsed_then_it_is_kept() {
        let config = Config::parse(r#"{"claudeBypassPermissions": false}"#).unwrap();
        assert!(!config.claude_bypass_permissions);
        let path = temp_path("claude-bypass");
        config.save_to(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["claudeBypassPermissions"], false);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn given_copy_on_select_false_when_parsed_then_it_is_kept() {
        let config = Config::parse(r#"{"copyOnSelect": false}"#).unwrap();
        assert!(!config.copy_on_select);
        let path = temp_path("copy-on-select");
        config.save_to(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["copyOnSelect"], false);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn given_scrollback_lines_when_parsed_then_they_are_clamped_and_saved() {
        assert_eq!(
            Config::parse(r#"{"scrollbackLines": 2500}"#)
                .unwrap()
                .scrollback_lines,
            2_500
        );
        assert_eq!(
            Config::parse(r#"{"scrollbackLines": 900000}"#)
                .unwrap()
                .scrollback_lines,
            100_000
        );
        let path = temp_path("scrollback");
        let config = Config {
            scrollback_lines: 4_096,
            ..Config::default()
        };
        config.save_to(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["scrollbackLines"], 4_096);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn given_resume_args_when_parsed_then_they_are_per_provider() {
        let config =
            Config::parse(r#"{"resumeArgs": {"codex": ["--sandbox", "read-only"]}}"#).unwrap();
        assert_eq!(
            config.resume_args.get("codex"),
            Some(&vec!["--sandbox".to_string(), "read-only".to_string()])
        );
        let path = temp_path("resume-args");
        config.save_to(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["resumeArgs"]["codex"][0], "--sandbox");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn given_mute_providers_when_parsed_then_they_are_kept() {
        let config = Config::parse(r#"{"muteProviders": ["codex"]}"#).unwrap();
        assert_eq!(config.mute_providers, vec!["codex".to_string()]);
        let path = temp_path("mute");
        config.save_to(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["muteProviders"][0], "codex");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn given_a_socket_key_when_parsed_then_it_is_exposed() {
        let config = Config::parse(r#"{"socket": "off"}"#).unwrap();
        assert_eq!(config.socket_mode(), Some("off"));
    }

    #[test]
    fn given_a_terminal_backend_when_parsed_then_it_round_trips() {
        use berth_headless::engine::Backend;
        assert_eq!(
            Config::parse(r#"{"terminalBackend": "alacritty"}"#)
                .unwrap()
                .terminal_backend,
            Backend::Alacritty
        );
        assert_eq!(
            Config::parse(r#"{"terminalBackend": "nonsense"}"#)
                .unwrap()
                .terminal_backend,
            Backend::Builtin
        );
        assert_eq!(Config::default().terminal_backend, Backend::Builtin);

        let path = temp_path("backend");
        let mut config = Config {
            terminal_backend: Backend::Alacritty,
            ..Config::default()
        };
        config.save_to(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["terminalBackend"], "alacritty");

        config.terminal_backend = Backend::Builtin;
        config.save_to(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(saved.get("terminalBackend").is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn given_one_font_family_when_parsed_then_it_is_a_single_entry_list() {
        let config = Config::parse(r#"{"fontFamily": "Victor Mono"}"#).unwrap();
        assert_eq!(config.font_family, vec!["Victor Mono".to_string()]);
    }

    #[test]
    fn given_a_font_family_list_when_parsed_then_the_order_is_kept_and_blanks_dropped() {
        let config =
            Config::parse(r#"{"fontFamily": ["MesloLGS NF", "  ", " JetBrains Mono "]}"#).unwrap();
        assert_eq!(
            config.font_family,
            vec!["MesloLGS NF".to_string(), "JetBrains Mono".to_string()]
        );
    }

    #[test]
    fn given_no_font_family_when_parsed_then_the_system_face_is_kept() {
        assert!(Config::parse("{}").unwrap().font_family.is_empty());
    }

    #[test]
    fn given_font_weight_forms_then_they_parse_and_clamp() {
        assert_eq!(
            Config::parse(r#"{"fontWeight": 650}"#).unwrap().font_weight,
            650
        );
        assert_eq!(
            Config::parse(r#"{"fontWeight": "semi-bold"}"#)
                .unwrap()
                .font_weight,
            600
        );
        assert_eq!(
            Config::parse(r#"{"fontWeight": 1000}"#)
                .unwrap()
                .font_weight,
            900
        );
        assert_eq!(
            Config::parse(r#"{"fontWeight": 10}"#).unwrap().font_weight,
            100
        );
    }

    #[test]
    fn given_line_height_then_it_is_clamped_and_zero_keeps_the_default() {
        let config = Config::parse(r#"{"lineHeight": 2.0}"#).unwrap();
        assert!((config.line_height - 2.0).abs() < f32::EPSILON);
        assert!(
            (Config::parse(r#"{"lineHeight": 9}"#).unwrap().line_height - 3.0).abs() < f32::EPSILON
        );
        assert!(
            (Config::parse(r#"{"lineHeight": 0}"#).unwrap().line_height - 1.35).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn given_font_features_then_only_four_character_tags_survive() {
        let config =
            Config::parse(r#"{"fontFeatures": ["calt", "-liga", "bad", "+ss01"]}"#).unwrap();
        assert_eq!(config.font_features, vec!["calt", "-liga", "ss01"]);
    }

    #[test]
    fn given_font_metrics_then_values_are_clamped_and_empty_entries_dropped() {
        let config = Config::parse(
            r#"{"fontMetrics": {"Fira Code": {"width": 9, "height": 900}, "Empty": {}, " ": {"width": 7}}}"#,
        )
        .unwrap();
        let metrics = config.font_metrics.get("Fira Code").expect("metrics");
        assert_eq!((metrics.width, metrics.height), (Some(9), Some(256)));
        assert!(!config.font_metrics.contains_key("Empty"));
        assert!(!config.font_metrics.contains_key(" "));
    }

    #[test]
    fn given_font_options_when_converted_then_weight_features_and_metrics_carry_over() {
        let config = Config::parse(
            r#"{"fontWeight": "medium", "fontFeatures": ["calt"], "fontMetrics": {"Mono": {"width": 9, "height": 21}}}"#,
        )
        .unwrap();
        let options = config.font_options();
        assert_eq!(options.weight, 500);
        assert_eq!(options.features, vec![(*b"calt", true)]);
        let metrics = options
            .metrics
            .iter()
            .find(|(name, _)| name == "Mono")
            .map(|(_, metrics)| *metrics)
            .expect("metric");
        assert_eq!((metrics.width, metrics.height), (Some(9), Some(21)));
    }

    #[test]
    fn given_a_font_family_when_saved_then_it_round_trips() {
        let path = temp_path("family");
        let config = Config {
            font_family: vec!["Fira Code".to_string()],
            ..Config::default()
        };
        config.save_to(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["fontFamily"][0], "Fira Code");
        assert_eq!(
            Config::parse(&std::fs::read_to_string(&path).unwrap())
                .unwrap()
                .font_family,
            vec!["Fira Code".to_string()]
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn given_no_font_family_when_saved_then_the_key_is_not_written() {
        let path = temp_path("nofamily");
        Config::default().save_to(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(saved.get("fontFamily").is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn given_a_picked_font_when_cleared_then_save_drops_the_previous_key() {
        let path = temp_path("clearfamily");
        let mut config = Config {
            font_family: vec!["Fira Code".to_string()],
            ..Config::default()
        };
        config.save_to(&path).unwrap();
        config.set_font_family(None);
        config.save_to(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(saved.get("fontFamily").is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn given_picked_fonts_when_replaced_then_the_new_face_leads_and_fallbacks_stay() {
        let mut config = Config {
            font_family: vec!["Monaco".to_string(), "VictorMono NFM".to_string()],
            ..Config::default()
        };
        config.set_font_family(Some("VictorMono NFM".to_string()));
        assert_eq!(
            config.font_family,
            vec!["VictorMono NFM".to_string(), "Monaco".to_string()]
        );
    }

    #[test]
    fn given_a_theme_and_opacity_when_parsed_then_they_clamp_and_resolve() {
        let config = Config::parse(r#"{"theme": " berth-light ", "opacity": 0.1}"#).unwrap();
        assert_eq!(config.theme, "berth-light");
        assert_eq!(config.opacity, MIN_OPACITY);
        assert_eq!(
            config.theme().terminal_bg,
            crate::theme::Theme::named("berth-light")
                .expect("light theme")
                .terminal_bg
        );
    }

    #[test]
    fn given_each_bell_name_when_parsed_then_it_maps_to_a_mode() {
        for (text, mode) in [
            ("none", BellMode::None),
            ("visual", BellMode::Visual),
            (" Audio ", BellMode::Audio),
        ] {
            assert_eq!(BellMode::parse(text), Some(mode));
            assert_eq!(mode.name(), text.trim().to_ascii_lowercase());
        }
        assert_eq!(BellMode::parse("bell"), None);
    }

    #[test]
    fn given_a_bell_key_when_parsed_and_saved_then_it_round_trips() {
        let config = Config::parse(r#"{"bell": "audio"}"#).unwrap();
        assert_eq!(config.bell, BellMode::Audio);
        assert_eq!(
            Config::parse(r#"{"bell": "loud"}"#).unwrap().bell,
            BellMode::Visual
        );
        let path = temp_path("bell");
        config.save_to(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["bell"], "audio");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn given_an_explicit_zero_opacity_then_it_clamps_to_the_floor() {
        let config = Config::parse(r#"{"opacity": 0}"#).unwrap();
        assert_eq!(config.opacity, MIN_OPACITY);
    }

    #[test]
    fn given_a_missing_opacity_then_the_window_is_opaque() {
        assert_eq!(Config::default().opacity, 1.0);
    }

    #[test]
    fn given_an_unknown_theme_then_the_dark_palette_is_used() {
        let config = Config::parse(r#"{"theme": "not-a-theme"}"#).unwrap();
        assert_eq!(
            config.theme().terminal_bg,
            crate::theme::Theme::default().terminal_bg
        );
        assert_eq!(config.opacity, 1.0);
    }

    #[test]
    fn given_a_theme_and_opacity_when_saved_then_they_round_trip() {
        let path = temp_path("theme");
        std::fs::write(&path, r#"{"futureKey": {"a": 1}}"#).unwrap();
        let mut config = Config::parse(r#"{}"#).unwrap();
        config.theme = "berth-light".into();
        config.opacity = 0.85;
        config.save_to(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["theme"], "berth-light");
        assert!((saved["opacity"].as_f64().expect("opacity") - 0.85).abs() < 1e-6);
        assert_eq!(saved["futureKey"]["a"], 1);

        config.opacity = 1.0;
        config.save_to(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(
            saved.get("opacity").is_none(),
            "opaque must not be written as an explicit override"
        );
        let _ = std::fs::remove_file(&path);
    }
}
