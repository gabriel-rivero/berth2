//! Mouse reports for programs that enable tracking.
//!
//! Terminals speak three encodings; the program picks one with a private
//! sequence and `vt100` reports the choice. Coordinates are one-based on the
//! wire and zero-based here.

use berth_headless::{MouseEncoding, MouseMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    /// Horizontal wheel (trackpad swipe or tilt wheel), xterm buttons 6 and 7.
    WheelLeft,
    WheelRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl Modifiers {
    fn bits(self) -> u8 {
        u8::from(self.shift) * 4 + u8::from(self.alt) * 8 + u8::from(self.ctrl) * 16
    }
}

/// One pointer event to report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    /// Zero-based column inside the pane's grid.
    pub col: u16,
    /// Zero-based row inside the pane's grid.
    pub row: u16,
    pub button: Button,
    pub release: bool,
    /// A drag or move with a button held (adds the 32 motion bit).
    pub motion: bool,
    pub modifiers: Modifiers,
}

/// Does this mode report a pointer move? `motion` is only sent when a button is
/// held (Drag) or for every move (Motion).
pub fn should_report(mode: MouseMode, pressed: bool) -> bool {
    match mode {
        MouseMode::Off => false,
        MouseMode::Click => pressed,
        MouseMode::PressRelease => true,
        MouseMode::Drag => true,
        MouseMode::Motion => true,
    }
}

fn button_code(button: Button) -> u8 {
    match button {
        Button::Left => 0,
        Button::Middle => 1,
        Button::Right => 2,
        Button::WheelUp => 64,
        Button::WheelDown => 65,
        Button::WheelLeft => 66,
        Button::WheelRight => 67,
    }
}

/// Classify a two-axis scroll as horizontal, returning `true` for right.
/// A gesture counts as horizontal only when that axis dominates, so a diagonal
/// trackpad swipe keeps scrolling vertically.
pub fn horizontal_gesture(x: f64, y: f64) -> Option<bool> {
    if x != 0.0 && x.abs() > y.abs() {
        Some(x > 0.0)
    } else {
        None
    }
}

/// Encode one report. Returns `None` for encodings that cannot express it:
/// X10 has no release or motion codes and no coordinates past 223.
pub fn encode(report: Report, encoding: MouseEncoding) -> Option<Vec<u8>> {
    let mut code = button_code(report.button) + report.modifiers.bits();
    if report.motion {
        code += 32;
    }
    let x = report.col as u32 + 1;
    let y = report.row as u32 + 1;
    match encoding {
        MouseEncoding::Sgr => {
            let final_byte = if report.release { 'm' } else { 'M' };
            Some(format!("\x1b[<{code};{x};{y}{final_byte}").into_bytes())
        }
        MouseEncoding::Utf8 => {
            if x > 2015 || y > 2015 {
                return None;
            }
            let code = if report.release { 3 } else { code };
            let mut out = b"\x1b[M".to_vec();
            out.push(32 + code);
            encode_utf8(&mut out, 32 + x);
            encode_utf8(&mut out, 32 + y);
            Some(out)
        }
        MouseEncoding::X10 => {
            if report.release || report.motion || x > 223 || y > 223 {
                return None;
            }
            Some(vec![
                0x1b,
                b'[',
                b'M',
                32 + code,
                (32 + x) as u8,
                (32 + y) as u8,
            ])
        }
    }
}

fn encode_utf8(out: &mut Vec<u8>, value: u32) {
    if value < 0x80 {
        out.push(value as u8);
    } else if value < 0x800 {
        out.push(0xc0 | (value >> 6) as u8);
        out.push(0x80 | (value & 0x3f) as u8);
    } else {
        out.push(0xe0 | (value >> 12) as u8);
        out.push(0x80 | ((value >> 6) & 0x3f) as u8);
        out.push(0x80 | (value & 0x3f) as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(button: Button) -> Report {
        Report {
            col: 4,
            row: 2,
            button,
            release: false,
            motion: false,
            modifiers: Modifiers::default(),
        }
    }

    #[test]
    fn given_sgr_when_a_left_press_is_encoded_then_coordinates_are_one_based() {
        let bytes = encode(report(Button::Left), MouseEncoding::Sgr).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), "\x1b[<0;5;3M");
    }

    #[test]
    fn given_sgr_when_a_release_is_encoded_then_it_uses_the_lowercase_m() {
        let mut event = report(Button::Left);
        event.release = true;
        let bytes = encode(event, MouseEncoding::Sgr).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), "\x1b[<0;5;3m");
    }

    #[test]
    fn given_modifiers_when_encoded_then_the_bits_match_xterm() {
        let mut event = report(Button::Left);
        event.modifiers = Modifiers {
            shift: true,
            alt: true,
            ctrl: true,
        };
        let bytes = encode(event, MouseEncoding::Sgr).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), "\x1b[<28;5;3M");
    }

    #[test]
    fn given_a_wheel_event_when_encoded_then_it_uses_the_wheel_codes() {
        let up = encode(report(Button::WheelUp), MouseEncoding::X10).unwrap();
        assert_eq!(up, vec![0x1b, b'[', b'M', 32 + 64, 32 + 5, 32 + 3]);
        let down = encode(report(Button::WheelDown), MouseEncoding::X10).unwrap();
        assert_eq!(down[3], 32 + 65);
    }

    #[test]
    fn given_motion_when_encoded_then_the_motion_bit_is_set() {
        let mut event = report(Button::Left);
        event.motion = true;
        let bytes = encode(event, MouseEncoding::Sgr).unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), "\x1b[<32;5;3M");
    }

    #[test]
    fn given_x10_when_a_release_or_far_coordinate_is_reported_then_it_is_dropped() {
        let mut release = report(Button::Left);
        release.release = true;
        assert!(encode(release, MouseEncoding::X10).is_none());
        let mut far = report(Button::Left);
        far.col = 400;
        assert!(encode(far, MouseEncoding::X10).is_none());
        assert!(encode(far, MouseEncoding::Sgr).is_some());
    }

    #[test]
    fn given_utf8_when_a_coordinate_is_over_127_then_it_is_multibyte() {
        let mut event = report(Button::Left);
        event.col = 300;
        let bytes = encode(event, MouseEncoding::Utf8).unwrap();
        assert_eq!(&bytes[..3], b"\x1b[M");
        assert!(bytes.len() > 6);
    }

    #[test]
    fn given_each_mode_when_pressed_and_released_then_reporting_matches_the_mode() {
        assert!(!should_report(MouseMode::Off, true));
        assert!(should_report(MouseMode::Click, true));
        assert!(!should_report(MouseMode::Click, false));
        assert!(should_report(MouseMode::PressRelease, false));
        assert!(should_report(MouseMode::Drag, false));
        assert!(should_report(MouseMode::Motion, false));
    }

    #[test]
    fn given_a_horizontal_scroll_then_the_gesture_is_horizontal() {
        assert_eq!(horizontal_gesture(3.0, 0.0), Some(true));
        assert_eq!(horizontal_gesture(-2.0, 0.5), Some(false));
        assert_eq!(horizontal_gesture(0.4, 0.1), Some(true));
    }

    #[test]
    fn given_a_vertical_or_absent_scroll_then_no_horizontal_gesture() {
        assert_eq!(horizontal_gesture(0.0, 2.0), None);
        assert_eq!(horizontal_gesture(1.0, 1.0), None);
        assert_eq!(horizontal_gesture(0.0, 0.0), None);
    }

    #[test]
    fn given_sgr_when_a_sideways_wheel_is_encoded_then_it_uses_buttons_six_and_seven() {
        let left = encode(report(Button::WheelLeft), MouseEncoding::Sgr).unwrap();
        let right = encode(report(Button::WheelRight), MouseEncoding::Sgr).unwrap();
        assert_eq!(String::from_utf8(left).unwrap(), "\x1b[<66;5;3M");
        assert_eq!(String::from_utf8(right).unwrap(), "\x1b[<67;5;3M");
    }

    #[test]
    fn given_utf8_when_a_sideways_wheel_is_encoded_then_the_byte_follows_the_protocol() {
        let bytes = encode(report(Button::WheelRight), MouseEncoding::Utf8).unwrap();
        assert_eq!(bytes, vec![0x1b, b'[', b'M', 32 + 67, 32 + 5, 32 + 3]);
    }
}
