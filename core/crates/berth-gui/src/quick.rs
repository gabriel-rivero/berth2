//! The quick terminal: a dropdown window opened by a global hotkey.
//!
//! The window is an ordinary core window whose `kind` is `quick`, so it
//! restores, hosts surfaces and speaks the protocol like any other. What makes
//! it a dropdown is presentation: no sidebar, always on top, and hidden instead
//! of closed when it loses focus.
//!
//! The hotkey is global, so it works while another application is frontmost.
//! That is the whole point of the feature and the reason it is a separate
//! module: [`parse`] turns the config string into the one key the platform
//! layer registers, and everything that can be decided without a window is
//! decided and tested here.

use global_hotkey::hotkey::{Code, HotKey, Modifiers};

/// Where the dropdown sits on its display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Full width at the top of the screen, like a Quake console.
    Top,
    /// Full width at the bottom, above the dock.
    Bottom,
    /// Centred, for a large display.
    Center,
}

impl Placement {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "top" => Some(Self::Top),
            "bottom" => Some(Self::Bottom),
            "center" | "centre" => Some(Self::Center),
            _ => None,
        }
    }

    /// The top-left corner and size of the dropdown in a display's **logical**
    /// work area, given the fraction of the height it should cover.
    ///
    /// Everything here is in one unit space on purpose. The caller converts the
    /// monitor's geometry into logical points before this call and passes the
    /// result straight to winit as logical values, because winit's monitor
    /// geometry is platform-inconsistent: on macOS `size()` is scaled twice
    /// while `position()` is physical. Mixing the two spaces put the drop-down
    /// off-screen on a Retina display.
    pub fn rect(self, area: (f64, f64, f64, f64), height_fraction: f64) -> (f64, f64, f64, f64) {
        let (x, y, width, height) = area;
        // A minimum of 10 logical rows' worth, so a tiny fraction still shows a
        // usable strip, and never taller than the display.
        let strip = (height * height_fraction.clamp(0.15, 1.0))
            .max(MIN_HEIGHT)
            .min(height);
        let top = match self {
            Placement::Top => y,
            Placement::Bottom => y + height - strip,
            Placement::Center => y + (height - strip) / 2.0,
        };
        (x, top, width, strip)
    }
}

/// The smallest dropdown, in logical points.
pub const MIN_HEIGHT: f64 = 200.0;

/// Convert a monitor's reported geometry into one unit space: logical points.
///
/// winit disagrees across platforms. On macOS `MonitorHandle::size()` is
/// `PhysicalSize::from_logical(pixels, scale)` — the pixel count is treated as
/// logical and multiplied again — so it reports `scale² ×` the real size, while
/// `position()` is genuine physical pixels. Elsewhere both are physical.
/// Taking a single `scale` and a flag keeps the correction in one place and
/// testable, rather than buried in the caller.
pub fn logical_monitor(
    position: (i32, i32),
    size: (u32, u32),
    scale: f64,
    size_is_prescaled: bool,
) -> (f64, f64, f64, f64) {
    let scale = scale.max(0.1);
    let x = f64::from(position.0) / scale;
    let y = f64::from(position.1) / scale;
    // `size_is_prescaled` marks the macOS shape, where the logical size is one
    // extra factor of `scale` smaller than the physical size implies.
    let divisor = if size_is_prescaled {
        scale * scale
    } else {
        scale
    };
    let width = f64::from(size.0) / divisor;
    let height = f64::from(size.1) / divisor;
    (x, y, width, height)
}

/// The quick-terminal settings, resolved from `config.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// The global chord, in the same spelling the `shortcuts` table uses. An
    /// empty string disables the global hotkey, like the shortcut table.
    pub hotkey: String,
    pub placement: Placement,
    /// Fraction of the display height the dropdown covers.
    pub height: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            hotkey: "cmd+shift+`".to_string(),
            placement: Placement::Top,
            height: 0.45,
        }
    }
}

impl Options {
    /// Read the keys, clamping the height and falling back on a bad placement
    /// so a typo cannot leave the terminal with no way to open it.
    pub fn from_config(hotkey: Option<&str>, placement: Option<&str>, height: Option<f64>) -> Self {
        let defaults = Self::default();
        // An empty value disables the global hotkey instead of resetting it to
        // the default, which is how an empty entry behaves in `shortcuts`.
        let hotkey = match hotkey {
            Some(value) => value.trim().to_string(),
            None => defaults.hotkey,
        };
        Self {
            hotkey,
            placement: placement
                .and_then(Placement::parse)
                .unwrap_or(defaults.placement),
            height: height
                .map(|value| value.clamp(0.15, 1.0))
                .unwrap_or(defaults.height),
        }
    }
}

/// Parse a chord in config spelling (`cmd+shift+space`) into the platform key.
/// Returns `None` for a chord this build cannot register, so the caller can
/// log it and leave the terminal reachable through the menu instead.
pub fn parse(chord: &str) -> Option<HotKey> {
    let parts: Vec<&str> = chord
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    let key = parts.last()?;
    let mut modifiers = Modifiers::empty();
    for part in &parts[..parts.len().saturating_sub(1)] {
        match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "super" => modifiers |= Modifiers::META,
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "alt" | "opt" | "option" => modifiers |= Modifiers::ALT,
            "shift" => modifiers |= Modifiers::SHIFT,
            _ => return None,
        }
    }
    Some(HotKey::new(Some(modifiers), code(key)?))
}

/// The key half of a chord, in the same spelling the shortcut table uses.
fn code(key: &str) -> Option<Code> {
    let lower = key.to_ascii_lowercase();
    Some(match lower.as_str() {
        "space" => Code::Space,
        "enter" | "return" => Code::Enter,
        "tab" => Code::Tab,
        "escape" | "esc" => Code::Escape,
        "backquote" | "`" | "~" => Code::Backquote,
        "minus" | "-" => Code::Minus,
        "equal" | "=" => Code::Equal,
        "left" => Code::ArrowLeft,
        "right" => Code::ArrowRight,
        "up" => Code::ArrowUp,
        "down" => Code::ArrowDown,
        "slash" | "/" => Code::Slash,
        _ => {
            if let Some(digit) = lower.strip_prefix("f").and_then(|n| n.parse::<u8>().ok()) {
                return f_key(digit);
            }
            // A single letter is its physical key.
            let mut chars = key.chars();
            let ch = chars.next()?;
            if chars.next().is_some() || !ch.is_ascii_alphabetic() {
                return None;
            }
            letter_code(ch.to_ascii_uppercase())?
        }
    })
}

fn f_key(n: u8) -> Option<Code> {
    Some(match n {
        1 => Code::F1,
        2 => Code::F2,
        3 => Code::F3,
        4 => Code::F4,
        5 => Code::F5,
        6 => Code::F6,
        7 => Code::F7,
        8 => Code::F8,
        9 => Code::F9,
        10 => Code::F10,
        11 => Code::F11,
        12 => Code::F12,
        _ => return None,
    })
}

fn letter_code(upper: char) -> Option<Code> {
    Some(match upper {
        'A' => Code::KeyA,
        'B' => Code::KeyB,
        'C' => Code::KeyC,
        'D' => Code::KeyD,
        'E' => Code::KeyE,
        'F' => Code::KeyF,
        'G' => Code::KeyG,
        'H' => Code::KeyH,
        'I' => Code::KeyI,
        'J' => Code::KeyJ,
        'K' => Code::KeyK,
        'L' => Code::KeyL,
        'M' => Code::KeyM,
        'N' => Code::KeyN,
        'O' => Code::KeyO,
        'P' => Code::KeyP,
        'Q' => Code::KeyQ,
        'R' => Code::KeyR,
        'S' => Code::KeyS,
        'T' => Code::KeyT,
        'U' => Code::KeyU,
        'V' => Code::KeyV,
        'W' => Code::KeyW,
        'X' => Code::KeyX,
        'Y' => Code::KeyY,
        'Z' => Code::KeyZ,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_a_modifier_chord_then_it_parses_to_the_platform_key() {
        let hotkey = parse("cmd+shift+space").expect("a chord");
        assert_eq!(hotkey.key, Code::Space);
        assert!(hotkey.mods.contains(Modifiers::META));
        assert!(hotkey.mods.contains(Modifiers::SHIFT));
    }

    #[test]
    fn given_the_default_chord_then_it_parses_and_does_not_collide_with_next_window() {
        let hotkey = parse(&Options::default().hotkey).expect("the default chord");
        assert_eq!(hotkey.key, Code::Backquote);
        assert!(hotkey.mods.contains(Modifiers::META));
        assert!(hotkey.mods.contains(Modifiers::SHIFT));
        // `NextWindow` owns `cmd+``; a global chord there would take the key
        // from every application, so the dropdown leans on a different one.
        assert_ne!(Options::default().hotkey, "cmd+`");
    }

    #[test]
    fn given_a_prescaled_macos_monitor_then_the_logical_size_is_corrected() {
        // A 1440x900-point display at 2x has 2880x1800 physical pixels. winit's
        // macOS `size()` is `from_logical((2880, 1800), 2.0)`, i.e. 5760x3600,
        // which is `logical * scale^2`; dividing by `scale^2` recovers 1440x900.
        let (x, y, width, height) = logical_monitor((0, 0), (5760, 3600), 2.0, true);
        assert_eq!((x, y), (0.0, 0.0));
        assert_eq!((width, height), (1440.0, 900.0));
    }

    #[test]
    fn given_a_plain_monitor_then_the_logical_size_is_physical_over_scale() {
        let (_, _, width, height) = logical_monitor((0, 0), (2880, 1800), 2.0, false);
        assert_eq!((width, height), (1440.0, 900.0));
        let (_, _, width, height) = logical_monitor((0, 0), (1920, 1080), 1.0, false);
        assert_eq!((width, height), (1920.0, 1080.0));
    }

    #[test]
    fn given_a_full_height_dropdown_on_a_two_x_display_then_it_covers_the_screen() {
        // The failing case in review: a 45% strip of a 1440x900-point display
        // must be 405 points, not 90% of the screen.
        let (x, y, width, height) = logical_monitor((0, 0), (5760, 3600), 2.0, true);
        let (_, top, w, h) = Placement::Top.rect((x, y, width, height), 0.45);
        assert_eq!((top, w, h), (0.0, 1440.0, 405.0));
        assert!(
            h < height / 2.0,
            "the strip took more than half the display"
        );
    }

    #[test]
    fn given_an_empty_hotkey_then_the_global_chord_is_disabled() {
        let options = Options::from_config(Some(""), None, None);
        assert!(options.hotkey.is_empty());
    }

    #[test]
    fn given_a_letter_chord_then_it_is_case_insensitive() {
        assert_eq!(parse("ctrl+J").expect("a letter").key, Code::KeyJ);
    }

    #[test]
    fn given_a_function_key_then_it_parses() {
        assert_eq!(parse("ctrl+f12").expect("f12").key, Code::F12);
    }

    #[test]
    fn given_a_key_with_no_modifier_then_it_still_parses() {
        let hotkey = parse("f1").expect("f1 alone");
        assert!(hotkey.mods.is_empty());
        assert_eq!(hotkey.key, Code::F1);
    }

    #[test]
    fn given_an_unusable_chord_then_the_parser_declines() {
        assert!(parse("cmd+").is_none());
        assert!(parse("cmd+hyper+x").is_none());
        assert!(parse("cmd+£").is_none());
        assert!(parse("cmd+13").is_none());
    }

    #[test]
    fn given_a_placement_then_it_parses() {
        assert_eq!(Placement::parse("top"), Some(Placement::Top));
        assert_eq!(Placement::parse("Bottom"), Some(Placement::Bottom));
        assert_eq!(Placement::parse("centre"), Some(Placement::Center));
        assert_eq!(Placement::parse("sideways"), None);
    }

    #[test]
    fn given_a_placement_then_the_rect_sits_where_it_says() {
        let area = (0.0, 0.0, 1920.0, 1000.0);
        let (x, y, width, height) = Placement::Top.rect(area, 0.5);
        assert_eq!((x, y, width, height), (0.0, 0.0, 1920.0, 500.0));

        let (_, y, _, _) = Placement::Bottom.rect(area, 0.5);
        assert_eq!(y, 500.0);

        let (_, y, _, _) = Placement::Center.rect(area, 0.5);
        assert_eq!(y, 250.0);
    }

    #[test]
    fn given_an_absurd_height_then_the_rect_is_clamped_to_the_display() {
        let area = (0.0, 0.0, 1000.0, 800.0);
        let (_, _, _, height) = Placement::Top.rect(area, 5.0);
        assert_eq!(height, 800.0);
        let (_, _, _, tiny) = Placement::Top.rect(area, 0.01);
        assert!(tiny >= 120.0);
    }

    #[test]
    fn given_no_config_then_the_defaults_are_used() {
        let options = Options::from_config(None, None, None);
        assert_eq!(options, Options::default());
    }

    #[test]
    fn given_a_tiny_fraction_then_the_minimum_height_applies() {
        let (_, _, _, height) = Placement::Top.rect((0.0, 0.0, 1000.0, 4000.0), 0.15);
        assert_eq!(height, MIN_HEIGHT.max(0.15 * 4000.0));
    }

    #[test]
    fn given_a_bad_placement_and_height_then_they_fall_back_and_clamp() {
        let options = Options::from_config(Some("ctrl+space"), Some("sideways"), Some(2.0));
        assert_eq!(options.hotkey, "ctrl+space");
        assert_eq!(options.placement, Placement::Top);
        assert_eq!(options.height, 1.0);
    }
}
