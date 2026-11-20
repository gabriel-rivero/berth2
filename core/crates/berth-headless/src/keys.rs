//! Encode synthetic `surface.send_key` names as terminal input bytes.
//!
//! The macOS shell hands names to Ghostty, which computes the native keycode.
//! A PTY only accepts bytes, so this is the portable equivalent. Printable
//! single characters pass through; named keys become their xterm sequences.
//! Unknown names are sent literally, matching the macOS shell's fallback.
//!
//! When a program enables the kitty keyboard protocol, [`encode_with_flags`]
//! uses its `CSI u` form instead: the unshifted key code, modifiers, event
//! type, alternate keys and associated text, as the flags ask.

use crate::kitty::KittyFlags;

/// Whether a key event is a press, an auto-repeat or a release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Press,
    Repeat,
    Release,
}

/// One key event in the form the encoder needs, independent of the platform.
#[derive(Debug, Clone, Copy)]
pub struct KeyInput<'a> {
    /// A protocol key name (`"enter"`, `"left"`) or the logical character.
    pub key: &'a str,
    pub modifiers: &'a [String],
    pub action: KeyAction,
    /// The text the platform says this key produced, when it reports one.
    pub text: Option<&'a str>,
    /// The base PC-101 layout character for the physical key, when known.
    pub base_layout: Option<char>,
}

impl<'a> KeyInput<'a> {
    /// A plain press with no text or layout information, for CLI callers.
    pub fn press(key: &'a str, modifiers: &'a [String]) -> Self {
        Self {
            key,
            modifiers,
            action: KeyAction::Press,
            text: None,
            base_layout: None,
        }
    }
}

/// Encode a key event, using the kitty protocol when the program enabled it.
pub fn encode_with_flags(input: &KeyInput<'_>, flags: KittyFlags) -> Vec<u8> {
    if flags.is_enabled() {
        encode_kitty(input, flags)
    } else {
        encode_legacy(input)
    }
}

fn encode_legacy(input: &KeyInput<'_>) -> Vec<u8> {
    if input.action == KeyAction::Release {
        return Vec::new();
    }
    let ctrl = has(input.modifiers, &["ctrl", "control"]);
    let alt = has(input.modifiers, &["alt", "opt", "option"]);
    if !ctrl
        && !alt
        && named(input.key).is_none()
        && let Some(text) = input.text.filter(|text| !text.is_empty())
    {
        return text.as_bytes().to_vec();
    }
    encode(input.key, input.modifiers)
}

fn encode_kitty(input: &KeyInput<'_>, flags: KittyFlags) -> Vec<u8> {
    let shift = has(input.modifiers, &["shift"]);
    let alt = has(input.modifiers, &["alt", "opt", "option"]);
    let ctrl = has(input.modifiers, &["ctrl", "control"]);
    let super_key = has(input.modifiers, &["cmd", "super", "meta"]);
    let mods =
        1 + u16::from(shift) + u16::from(alt) * 2 + u16::from(ctrl) * 4 + u16::from(super_key) * 8;
    let is_control_key = matches!(
        input.key.to_ascii_lowercase().as_str(),
        "enter" | "return" | "tab" | "backspace" | "bs"
    );

    if input.action == KeyAction::Release {
        if !flags.reports_events() {
            return Vec::new();
        }
        if !flags.reports_all() && is_control_key {
            return Vec::new();
        }
    }

    let Some(entry) = kitty_entry(input, shift) else {
        return plain_text(input);
    };

    if !flags.reports_all() {
        if input.action != KeyAction::Release && mods == 1 && is_control_key {
            return legacy_control(input.key);
        }
        if input.action != KeyAction::Release
            && !ctrl
            && !alt
            && !super_key
            && !is_control_key
            && !(shift && flags.reports_alternates())
            && let Some(text) = printable_text(input.text)
        {
            return text;
        }
    }

    let shifted = if flags.reports_alternates() && shift {
        first_codepoint(input.text).filter(|codepoint| *codepoint != entry.code)
    } else {
        None
    };
    let base = if flags.reports_alternates() {
        input
            .base_layout
            .map(|character| character as u32)
            .filter(|codepoint| *codepoint != entry.code && Some(*codepoint) != shifted)
    } else {
        None
    };
    let text = if flags.reports_all()
        && flags.reports_text()
        && input.action != KeyAction::Release
        && !ctrl
        && !alt
        && !super_key
    {
        codepoints(input.text)
    } else {
        Vec::new()
    };

    let event = flags.reports_events().then_some(match input.action {
        KeyAction::Press => 1,
        KeyAction::Repeat => 2,
        KeyAction::Release => 3,
    });
    let mut out = String::from("\x1b[");

    if is_special_final(entry.final_byte) {
        if event.is_none() && mods == 1 {
            out.push(entry.final_byte as char);
        } else {
            out.push('1');
            out.push(';');
            out.push_str(&mods.to_string());
            if let Some(event) = event {
                out.push(':');
                out.push_str(&event.to_string());
            }
            out.push(entry.final_byte as char);
        }
        return out.into_bytes();
    }

    out.push_str(&entry.code.to_string());
    if let Some(shifted) = shifted {
        out.push(':');
        out.push_str(&shifted.to_string());
    }
    if let Some(base) = base {
        if shifted.is_none() {
            out.push_str("::");
        } else {
            out.push(':');
        }
        out.push_str(&base.to_string());
    }
    let emitted_params = event.is_some() || mods > 1;
    if let Some(event) = event {
        out.push(';');
        out.push_str(&mods.to_string());
        out.push(':');
        out.push_str(&event.to_string());
    } else if mods > 1 {
        out.push(';');
        out.push_str(&mods.to_string());
    }
    if !text.is_empty() {
        if !emitted_params {
            out.push(';');
        }
        out.push(';');
        for (index, codepoint) in text.iter().enumerate() {
            if index > 0 {
                out.push(':');
            }
            out.push_str(&codepoint.to_string());
        }
    }
    out.push(entry.final_byte as char);
    out.into_bytes()
}

struct KittyEntry {
    code: u32,
    final_byte: u8,
}

fn kitty_entry(input: &KeyInput<'_>, shift: bool) -> Option<KittyEntry> {
    if let Some(entry) = named_kitty(input.key) {
        return Some(entry);
    }
    let mut characters = input.key.chars();
    let character = characters.next()?;
    if characters.next().is_some() {
        return None;
    }
    Some(KittyEntry {
        code: unshifted_codepoint(character, shift),
        final_byte: b'u',
    })
}

fn named_kitty(key: &str) -> Option<KittyEntry> {
    let (code, final_byte) = match key.to_ascii_lowercase().as_str() {
        "escape" | "esc" => (27, b'u'),
        "enter" | "return" => (13, b'u'),
        "tab" => (9, b'u'),
        "backspace" | "bs" => (127, b'u'),
        "insert" | "ins" => (2, b'~'),
        "delete" | "del" => (3, b'~'),
        "left" => (1, b'D'),
        "right" => (1, b'C'),
        "up" => (1, b'A'),
        "down" => (1, b'B'),
        "pageup" | "pgup" => (5, b'~'),
        "pagedown" | "pgdn" => (6, b'~'),
        "home" => (1, b'H'),
        "end" => (1, b'F'),
        "space" => (32, b'u'),
        "f1" => (1, b'P'),
        "f2" => (1, b'Q'),
        "f3" => (13, b'~'),
        "f4" => (1, b'S'),
        "f5" => (15, b'~'),
        "f6" => (17, b'~'),
        "f7" => (18, b'~'),
        "f8" => (19, b'~'),
        "f9" => (20, b'~'),
        "f10" => (21, b'~'),
        "f11" => (23, b'~'),
        "f12" => (24, b'~'),
        _ => return None,
    };
    Some(KittyEntry { code, final_byte })
}

/// The CSI `u` form keys the unshifted (lower-case) code point, whatever the
/// layout reports for the shifted key.
fn unshifted_codepoint(character: char, shift: bool) -> u32 {
    let character = if shift {
        unshift(character).unwrap_or(character)
    } else {
        character
    };
    if character.is_ascii_uppercase() {
        u32::from(character.to_ascii_lowercase())
    } else {
        character as u32
    }
}

fn is_special_final(final_byte: u8) -> bool {
    final_byte.is_ascii_uppercase()
}

fn plain_text(input: &KeyInput<'_>) -> Vec<u8> {
    if input.action == KeyAction::Release {
        return Vec::new();
    }
    input
        .text
        .filter(|text| !text.is_empty())
        .map(|text| text.as_bytes().to_vec())
        .unwrap_or_default()
}

fn legacy_control(key: &str) -> Vec<u8> {
    match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => b"\r".to_vec(),
        "tab" => b"\t".to_vec(),
        "backspace" | "bs" => b"\x7f".to_vec(),
        _ => Vec::new(),
    }
}

fn printable_text(text: Option<&str>) -> Option<Vec<u8>> {
    let text = text.filter(|text| !text.is_empty())?;
    if text.chars().any(is_control) {
        return None;
    }
    Some(text.as_bytes().to_vec())
}

fn first_codepoint(text: Option<&str>) -> Option<u32> {
    text.and_then(|text| {
        text.chars()
            .find(|character| !is_control(*character))
            .map(u32::from)
    })
}

fn codepoints(text: Option<&str>) -> Vec<u32> {
    text.map(|text| {
        text.chars()
            .filter(|character| !is_control(*character))
            .map(u32::from)
            .collect()
    })
    .unwrap_or_default()
}

fn is_control(character: char) -> bool {
    character < '\u{20}' || ('\u{7f}'..='\u{9f}').contains(&character)
}

/// Bytes for `key` with the protocol's modifier names (`ctrl`, `alt`, `shift`,
/// `cmd`/`super`; the latter is ignored on Linux/Windows input paths).
pub fn encode(key: &str, mods: &[String]) -> Vec<u8> {
    let ctrl = has(mods, &["ctrl", "control"]);
    let alt = has(mods, &["alt", "opt", "option"]);
    let shift = has(mods, &["shift"]);
    let lower = key.to_ascii_lowercase();

    let mut out = if shift && lower == "tab" {
        b"\x1b[Z".to_vec()
    } else if shift && (lower == "enter" || lower == "return") {
        // Shift+Enter is ESC CR, the sequence Claude Code's own terminal setup
        // binds for a newline in the prompt.
        b"\x1b\r".to_vec()
    } else if let Some(sequence) = modified(&lower, shift, alt, ctrl) {
        return sequence;
    } else if let Some(sequence) = named(&lower) {
        sequence.to_vec()
    } else {
        let mut chars = key.chars();
        let Some(ch) = chars.next() else {
            return Vec::new();
        };
        if chars.next().is_some() {
            // Not a single character and not a known key: send the name.
            key.as_bytes().to_vec()
        } else if ctrl {
            control(ch)
        } else if shift && ch.is_ascii_alphabetic() {
            ch.to_ascii_uppercase().to_string().into_bytes()
        } else {
            ch.to_string().into_bytes()
        }
    };

    if alt {
        out.insert(0, 0x1b);
    }
    out
}

fn has(mods: &[String], names: &[&str]) -> bool {
    mods.iter()
        .any(|item| names.contains(&item.to_ascii_lowercase().as_str()))
}

/// Named keys with Shift, Alt or Ctrl become xterm modified sequences. The one
/// exception is Alt+Left/Right, which every readline prompt binds as ESC b and
/// ESC f for word movement; the xterm 1;3 form is not in the default keymaps.
fn modified(key: &str, shift: bool, alt: bool, ctrl: bool) -> Option<Vec<u8>> {
    let code = 1 + u8::from(shift) + u8::from(alt) * 2 + u8::from(ctrl) * 4;
    if code == 1 {
        return None;
    }
    let sequence = match key {
        "left" if alt && !ctrl && !shift => b"\x1bb".to_vec(),
        "right" if alt && !ctrl && !shift => b"\x1bf".to_vec(),
        "left" => format!("\x1b[1;{code}D").into_bytes(),
        "right" => format!("\x1b[1;{code}C").into_bytes(),
        "up" => format!("\x1b[1;{code}A").into_bytes(),
        "down" => format!("\x1b[1;{code}B").into_bytes(),
        "home" => format!("\x1b[1;{code}H").into_bytes(),
        "end" => format!("\x1b[1;{code}F").into_bytes(),
        "insert" | "ins" => format!("\x1b[2;{code}~").into_bytes(),
        "delete" | "del" => format!("\x1b[3;{code}~").into_bytes(),
        "pageup" | "pgup" => format!("\x1b[5;{code}~").into_bytes(),
        "pagedown" | "pgdn" => format!("\x1b[6;{code}~").into_bytes(),
        "f1" => format!("\x1b[1;{code}P").into_bytes(),
        "f2" => format!("\x1b[1;{code}Q").into_bytes(),
        "f3" => format!("\x1b[1;{code}R").into_bytes(),
        "f4" => format!("\x1b[1;{code}S").into_bytes(),
        "f5" => format!("\x1b[15;{code}~").into_bytes(),
        "f6" => format!("\x1b[17;{code}~").into_bytes(),
        "f7" => format!("\x1b[18;{code}~").into_bytes(),
        "f8" => format!("\x1b[19;{code}~").into_bytes(),
        "f9" => format!("\x1b[20;{code}~").into_bytes(),
        "f10" => format!("\x1b[21;{code}~").into_bytes(),
        "f11" => format!("\x1b[23;{code}~").into_bytes(),
        "f12" => format!("\x1b[24;{code}~").into_bytes(),
        _ => return None,
    };
    Some(sequence)
}

fn named(key: &str) -> Option<&'static [u8]> {
    Some(match key {
        "enter" | "return" => b"\r",
        "tab" => b"\t",
        "escape" | "esc" => b"\x1b",
        "space" => b" ",
        "backspace" | "bs" => b"\x7f",
        "delete" | "del" => b"\x1b[3~",
        "insert" | "ins" => b"\x1b[2~",
        "up" => b"\x1b[A",
        "down" => b"\x1b[B",
        "right" => b"\x1b[C",
        "left" => b"\x1b[D",
        "home" => b"\x1b[H",
        "end" => b"\x1b[F",
        "pageup" | "pgup" => b"\x1b[5~",
        "pagedown" | "pgdn" => b"\x1b[6~",
        "f1" => b"\x1bOP",
        "f2" => b"\x1bOQ",
        "f3" => b"\x1bOR",
        "f4" => b"\x1bOS",
        "f5" => b"\x1b[15~",
        "f6" => b"\x1b[17~",
        "f7" => b"\x1b[18~",
        "f8" => b"\x1b[19~",
        "f9" => b"\x1b[20~",
        "f10" => b"\x1b[21~",
        "f11" => b"\x1b[23~",
        "f12" => b"\x1b[24~",
        _ => return None,
    })
}

fn control(ch: char) -> Vec<u8> {
    let byte = match ch {
        'a'..='z' => ch as u8 - b'a' + 1,
        'A'..='Z' => ch as u8 - b'A' + 1,
        '@' | ' ' => 0,
        '[' => 27,
        '\\' => 28,
        ']' => 29,
        '^' => 30,
        '_' => 31,
        '?' => 127,
        _ => return ch.to_string().into_bytes(),
    };
    vec![byte]
}

/// The unshifted key for an ASCII symbol, for the US layout the defaults are
/// written for. Other layouts report a character that has no entry.
fn unshift(character: char) -> Option<char> {
    Some(match character {
        '!' => '1',
        '@' => '2',
        '#' => '3',
        '$' => '4',
        '%' => '5',
        '^' => '6',
        '&' => '7',
        '*' => '8',
        '(' => '9',
        ')' => '0',
        '_' => '-',
        '+' => '=',
        '{' => '[',
        '}' => ']',
        '|' => '\\',
        ':' => ';',
        '"' => '\'',
        '<' => ',',
        '>' => '.',
        '?' => '/',
        '~' => '`',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn named_keys_are_xterm_sequences() {
        assert_eq!(encode("enter", &[]), b"\r");
        assert_eq!(encode("up", &[]), b"\x1b[A");
        assert_eq!(encode("f5", &[]), b"\x1b[15~");
        assert_eq!(encode("pageDown", &[]), b"\x1b[6~");
    }

    #[test]
    fn ctrl_letters_become_control_bytes() {
        assert_eq!(encode("c", &mods(&["ctrl"])), vec![3]);
        assert_eq!(encode("C", &mods(&["Control"])), vec![3]);
        assert_eq!(encode("[", &mods(&["ctrl"])), vec![27]);
    }

    #[test]
    fn alt_prefixes_escape_and_shift_uppercases() {
        assert_eq!(encode("b", &mods(&["alt"])), b"\x1bb");
        assert_eq!(encode("a", &mods(&["shift"])), b"A");
        assert_eq!(encode("tab", &mods(&["shift"])), b"\x1b[Z");
    }

    #[test]
    fn alt_arrows_are_readline_word_motions() {
        assert_eq!(encode("left", &mods(&["alt"])), b"\x1bb");
        assert_eq!(encode("right", &mods(&["option"])), b"\x1bf");
    }

    #[test]
    fn modified_function_keys_use_xterm_codes() {
        assert_eq!(encode("left", &mods(&["ctrl"])), b"\x1b[1;5D");
        assert_eq!(encode("right", &mods(&["shift"])), b"\x1b[1;2C");
        assert_eq!(encode("up", &mods(&["alt", "shift"])), b"\x1b[1;4A");
        assert_eq!(encode("delete", &mods(&["ctrl"])), b"\x1b[3;5~");
        assert_eq!(encode("f5", &mods(&["alt"])), b"\x1b[15;3~");
        assert_eq!(encode("left", &[]), b"\x1b[D");
    }

    #[test]
    fn shift_enter_is_escape_cr_for_a_prompt_newline() {
        assert_eq!(encode("enter", &mods(&["shift"])), b"\x1b\r");
        assert_eq!(encode("Return", &mods(&["Shift"])), b"\x1b\r");
        assert_eq!(encode("enter", &[]), b"\r");
    }

    #[test]
    fn unknown_names_are_literal_text() {
        assert_eq!(encode("hello", &[]), b"hello");
    }

    // ---- kitty keyboard protocol -------------------------------------

    const DISAMBIGUATE: u16 = 1;
    const EVENTS: u16 = 2;
    const ALTERNATES: u16 = 4;
    const ALL: u16 = 8;
    const TEXT: u16 = 16;

    fn input<'a>(key: &'a str, mods: &'a [String]) -> KeyInput<'a> {
        KeyInput {
            key,
            modifiers: mods,
            action: KeyAction::Press,
            text: None,
            base_layout: None,
        }
    }

    fn encoded(input: &KeyInput<'_>, bits: u16) -> Vec<u8> {
        encode_with_flags(input, KittyFlags::from_bits(bits))
    }

    #[test]
    fn given_no_flags_then_the_legacy_encoding_is_unchanged() {
        assert_eq!(encoded(&input("a", &[]), 0), b"a");
        assert_eq!(
            encoded(&input("hello", &[]), 0),
            b"hello",
            "multi-character keys stay literal"
        );
        assert_eq!(encoded(&input("c", &mods(&["ctrl"])), 0), vec![3]);
        assert_eq!(encoded(&input("left", &[]), 0), b"\x1b[D");
    }

    #[test]
    fn given_plain_text_with_disambiguate_then_it_is_still_text() {
        let with_text = KeyInput {
            text: Some("a"),
            ..input("a", &[])
        };
        assert_eq!(encoded(&with_text, DISAMBIGUATE), b"a");

        let shift = mods(&["shift"]);
        let shifted = KeyInput {
            text: Some("A"),
            ..input("A", &shift)
        };
        assert_eq!(encoded(&shifted, DISAMBIGUATE), b"A");
    }

    #[test]
    fn given_escape_with_disambiguate_then_it_is_a_csi_u_sequence() {
        assert_eq!(encoded(&input("escape", &[]), DISAMBIGUATE), b"\x1b[27u");
    }

    #[test]
    fn given_ctrl_with_disambiguate_then_the_letter_keeps_its_own_code() {
        assert_eq!(
            encoded(&input("c", &mods(&["ctrl"])), DISAMBIGUATE),
            b"\x1b[99;5u"
        );
        assert_eq!(
            encoded(&input("[", &mods(&["ctrl"])), DISAMBIGUATE),
            b"\x1b[91;5u"
        );
        assert_eq!(
            encoded(&input("a", &mods(&["alt"])), DISAMBIGUATE),
            b"\x1b[97;3u"
        );
    }

    #[test]
    fn given_the_control_keys_then_their_legacy_bytes_are_kept_without_modifiers() {
        assert_eq!(encoded(&input("enter", &[]), DISAMBIGUATE), b"\r");
        assert_eq!(encoded(&input("tab", &[]), DISAMBIGUATE), b"\t");
        assert_eq!(encoded(&input("backspace", &[]), DISAMBIGUATE), b"\x7f");
        assert_eq!(
            encoded(&input("enter", &mods(&["shift"])), DISAMBIGUATE),
            b"\x1b[13;2u"
        );
        assert_eq!(
            encoded(&input("tab", &mods(&["shift"])), DISAMBIGUATE),
            b"\x1b[9;2u"
        );
    }

    #[test]
    fn given_report_events_then_a_release_carries_event_type_three() {
        let ctrl = mods(&["ctrl"]);
        let release = KeyInput {
            action: KeyAction::Release,
            ..input("a", &ctrl)
        };
        assert_eq!(encoded(&release, DISAMBIGUATE | EVENTS), b"\x1b[97;5:3u");
        let left = KeyInput {
            action: KeyAction::Release,
            ..input("left", &[])
        };
        assert_eq!(encoded(&left, DISAMBIGUATE | EVENTS), b"\x1b[1;1:3D");
    }

    #[test]
    fn given_report_events_without_report_all_then_control_releases_are_dropped() {
        let release = KeyInput {
            action: KeyAction::Release,
            ..input("enter", &[])
        };
        assert!(encoded(&release, DISAMBIGUATE | EVENTS).is_empty());
        assert_eq!(
            encoded(&release, DISAMBIGUATE | EVENTS | ALL),
            b"\x1b[13;1:3u"
        );
    }

    #[test]
    fn given_a_repeat_then_the_event_type_is_two() {
        let repeat = KeyInput {
            action: KeyAction::Repeat,
            ..input("left", &[])
        };
        assert_eq!(encoded(&repeat, DISAMBIGUATE | EVENTS), b"\x1b[1;1:2D");
    }

    #[test]
    fn given_report_all_then_plain_text_becomes_a_key_event() {
        let with_text = KeyInput {
            text: Some("a"),
            ..input("a", &[])
        };
        assert_eq!(encoded(&with_text, DISAMBIGUATE | ALL), b"\x1b[97u");
        assert_eq!(
            encoded(&with_text, DISAMBIGUATE | ALL | TEXT),
            b"\x1b[97;;97u"
        );
    }

    #[test]
    fn given_report_associated_text_then_the_shifted_codepoint_is_included() {
        let shift = mods(&["shift"]);
        let shifted = KeyInput {
            text: Some("A"),
            ..input("A", &shift)
        };
        assert_eq!(
            encoded(&shifted, DISAMBIGUATE | ALL | TEXT),
            b"\x1b[97;2;65u"
        );
    }

    #[test]
    fn given_report_alternates_then_the_shifted_key_is_included() {
        let shift = mods(&["shift"]);
        let shifted = KeyInput {
            text: Some("A"),
            ..input("A", &shift)
        };
        assert_eq!(
            encoded(&shifted, DISAMBIGUATE | ALTERNATES),
            b"\x1b[97:65;2u"
        );
    }

    #[test]
    fn given_a_base_layout_key_then_it_is_reported_with_an_empty_shift_slot() {
        let c_y_r_i_l_l_i_c = KeyInput {
            base_layout: Some('c'),
            ..input("с", &[])
        };
        assert_eq!(
            encoded(&c_y_r_i_l_l_i_c, DISAMBIGUATE | ALTERNATES),
            "\x1b[1089::99u".as_bytes()
        );
    }

    #[test]
    fn given_function_keys_then_the_tilde_and_special_forms_are_used() {
        assert_eq!(
            encoded(&input("f5", &mods(&["ctrl"])), DISAMBIGUATE),
            b"\x1b[15;5~"
        );
        assert_eq!(
            encoded(&input("f3", &mods(&["shift"])), DISAMBIGUATE),
            b"\x1b[13;2~"
        );
        assert_eq!(
            encoded(&input("left", &mods(&["shift"])), DISAMBIGUATE),
            b"\x1b[1;2D"
        );
        assert_eq!(encoded(&input("left", &[]), DISAMBIGUATE), b"\x1b[D");
    }
}
