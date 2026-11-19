//! Encode synthetic `surface.send_key` names as terminal input bytes.
//!
//! The macOS shell hands names to Ghostty, which computes the native keycode.
//! A PTY only accepts bytes, so this is the portable equivalent. Printable
//! single characters pass through; named keys become their xterm sequences.
//! Unknown names are sent literally, matching the macOS shell's fallback.

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
}
