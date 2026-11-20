//! Kitty keyboard protocol state: progressive enhancement flags and the
//! push/pop stack a program drives.
//!
//! `vt100` drops `CSI u` sequences, so the PTY reader's side channel parses
//! them here. The window and the CLI read the current flags from `ScreenMeta`
//! to encode key events, and the scanner answers `CSI ? u`.

use crate::csi::Csi;

/// Disambiguate escape codes: `CSI u` for Esc and modified keys.
pub const DISAMBIGUATE: u16 = 1 << 0;
/// Report key repeat and release events.
pub const REPORT_EVENTS: u16 = 1 << 1;
/// Report shifted and base-layout alternate keys.
pub const REPORT_ALTERNATES: u16 = 1 << 2;
/// Report every key as an escape code, including plain text and modifiers.
pub const REPORT_ALL: u16 = 1 << 3;
/// Report the text a key produced as code points.
pub const REPORT_TEXT: u16 = 1 << 4;

const ALL_FLAGS: u16 = DISAMBIGUATE | REPORT_EVENTS | REPORT_ALTERNATES | REPORT_ALL | REPORT_TEXT;

/// The progressive enhancement flags a program has enabled.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KittyFlags(u16);

impl KittyFlags {
    pub const NONE: Self = Self(0);

    pub fn from_bits(bits: u16) -> Self {
        Self(bits & ALL_FLAGS)
    }

    pub fn bits(self) -> u16 {
        self.0
    }

    pub fn is_enabled(self) -> bool {
        self.0 != 0
    }

    pub fn is_disambiguated(self) -> bool {
        self.0 & DISAMBIGUATE != 0
    }

    pub fn reports_events(self) -> bool {
        self.0 & REPORT_EVENTS != 0
    }

    pub fn reports_alternates(self) -> bool {
        self.0 & REPORT_ALTERNATES != 0
    }

    pub fn reports_all(self) -> bool {
        self.0 & REPORT_ALL != 0
    }

    pub fn reports_text(self) -> bool {
        self.0 & REPORT_TEXT != 0
    }
}

/// How `CSI = flags ; mode u` applies the flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetMode {
    Set,
    Or,
    Not,
}

impl SetMode {
    fn from_param(param: u16) -> Self {
        match param {
            2 => Self::Or,
            3 => Self::Not,
            _ => Self::Set,
        }
    }
}

/// The protocol asks terminals to cap the stack to avoid a denial of service.
const STACK_DEPTH: usize = 8;

#[derive(Debug)]
struct FlagStack {
    flags: [KittyFlags; STACK_DEPTH],
    index: usize,
}

impl Default for FlagStack {
    fn default() -> Self {
        Self {
            flags: [KittyFlags::NONE; STACK_DEPTH],
            index: 0,
        }
    }
}

impl FlagStack {
    fn current(&self) -> KittyFlags {
        self.flags[self.index]
    }

    fn push(&mut self, flags: KittyFlags) {
        self.index = (self.index + 1) % STACK_DEPTH;
        self.flags[self.index] = flags;
    }

    fn pop(&mut self, count: usize) {
        if count >= STACK_DEPTH {
            *self = Self::default();
            return;
        }
        for _ in 0..count {
            self.flags[self.index] = KittyFlags::NONE;
            self.index = (self.index + STACK_DEPTH - 1) % STACK_DEPTH;
        }
    }

    fn apply(&mut self, mode: SetMode, flags: KittyFlags) {
        let current = self.current().bits();
        let next = match mode {
            SetMode::Set => flags.bits(),
            SetMode::Or => current | flags.bits(),
            SetMode::Not => current & !flags.bits(),
        };
        self.flags[self.index] = KittyFlags::from_bits(next);
    }
}

/// The kitty keyboard state of one surface. The main and alternate screens
/// keep independent stacks, as the protocol requires.
#[derive(Debug, Default)]
pub struct KittyKeyboard {
    main: FlagStack,
    alternate: FlagStack,
    is_alternate_screen: bool,
}

impl KittyKeyboard {
    pub fn current(&self) -> KittyFlags {
        self.stack().current()
    }

    /// Switch stacks with the alternate screen. Modes 47, 1047 and 1049 all
    /// select it.
    pub fn set_alternate_screen(&mut self, is_alternate: bool) {
        self.is_alternate_screen = is_alternate;
    }

    /// Handle one CSI sequence. Returns the reply the program is owed, if any.
    pub fn observe(&mut self, sequence: &Csi) -> Option<String> {
        if sequence.final_byte != b'u' {
            return None;
        }
        let body = &sequence.body;
        match body.first().copied() {
            Some(b'?') if body.len() == 1 => Some(format!("\x1b[?{}u", self.current().bits())),
            Some(b'>') => {
                let flags = parse_flags(&body[1..]).unwrap_or(KittyFlags::NONE);
                self.stack_mut().push(flags);
                None
            }
            Some(b'=') => {
                let mut parts = body[1..].split(|byte| *byte == b';');
                let flags = parts
                    .next()
                    .and_then(parse_flags)
                    .unwrap_or(KittyFlags::NONE);
                let mode = parts
                    .next()
                    .and_then(parse_u16)
                    .map(SetMode::from_param)
                    .unwrap_or(SetMode::Set);
                self.stack_mut().apply(mode, flags);
                None
            }
            Some(b'<') => {
                let count = parse_usize(&body[1..]).unwrap_or(1);
                self.stack_mut().pop(count);
                None
            }
            _ => None,
        }
    }

    fn stack(&self) -> &FlagStack {
        if self.is_alternate_screen {
            &self.alternate
        } else {
            &self.main
        }
    }

    fn stack_mut(&mut self) -> &mut FlagStack {
        if self.is_alternate_screen {
            &mut self.alternate
        } else {
            &mut self.main
        }
    }
}

fn parse_flags(bytes: &[u8]) -> Option<KittyFlags> {
    if bytes.is_empty() {
        return Some(KittyFlags::NONE);
    }
    parse_u16(bytes).map(KittyFlags::from_bits)
}

fn parse_u16(bytes: &[u8]) -> Option<u16> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

fn parse_usize(bytes: &[u8]) -> Option<usize> {
    if bytes.is_empty() {
        return Some(1);
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn csi(body: &str, final_byte: u8) -> Csi {
        Csi {
            body: body.as_bytes().to_vec(),
            final_byte,
        }
    }

    #[test]
    fn given_flags_when_read_then_each_enhancement_is_a_bit() {
        let flags = KittyFlags::from_bits(0b10101);
        assert!(flags.is_disambiguated());
        assert!(flags.reports_alternates());
        assert!(flags.reports_text());
        assert!(!flags.reports_events());
        assert!(!flags.reports_all());
        assert_eq!(flags.bits(), 0b10101);
    }

    #[test]
    fn given_unknown_flag_bits_then_they_are_ignored() {
        assert_eq!(KittyFlags::from_bits(0xffff).bits(), ALL_FLAGS);
    }

    #[test]
    fn given_a_push_and_pop_then_the_previous_flags_return() {
        let mut keyboard = KittyKeyboard::default();
        keyboard.observe(&csi(">1", b'u'));
        assert_eq!(keyboard.current().bits(), 1);
        keyboard.observe(&csi(">5", b'u'));
        assert_eq!(keyboard.current().bits(), 5);
        keyboard.observe(&csi("<", b'u'));
        assert_eq!(keyboard.current().bits(), 1);
        keyboard.observe(&csi("<", b'u'));
        assert_eq!(keyboard.current().bits(), 0);
    }

    #[test]
    fn given_a_pop_past_the_stack_then_every_flag_is_reset() {
        let mut keyboard = KittyKeyboard::default();
        keyboard.observe(&csi(">5", b'u'));
        keyboard.observe(&csi("<100", b'u'));
        assert_eq!(keyboard.current().bits(), 0);
    }

    #[test]
    fn given_a_push_without_flags_then_it_pushes_zero() {
        let mut keyboard = KittyKeyboard::default();
        keyboard.observe(&csi(">5", b'u'));
        keyboard.observe(&csi(">", b'u'));
        assert_eq!(keyboard.current().bits(), 0);
    }

    #[test]
    fn given_set_modes_then_flags_are_replaced_ored_or_cleared() {
        let mut keyboard = KittyKeyboard::default();
        keyboard.observe(&csi("=5;1", b'u'));
        assert_eq!(keyboard.current().bits(), 5);
        keyboard.observe(&csi("=2;2", b'u'));
        assert_eq!(keyboard.current().bits(), 7);
        keyboard.observe(&csi("=4;3", b'u'));
        assert_eq!(keyboard.current().bits(), 3);
    }

    #[test]
    fn given_a_query_then_the_current_flags_are_reported() {
        let mut keyboard = KittyKeyboard::default();
        assert_eq!(
            keyboard.observe(&csi("?", b'u')).as_deref(),
            Some("\x1b[?0u")
        );
        keyboard.observe(&csi(">13", b'u'));
        assert_eq!(
            keyboard.observe(&csi("?", b'u')).as_deref(),
            Some("\x1b[?13u")
        );
    }

    #[test]
    fn given_main_and_alternate_screens_then_their_stacks_are_separate() {
        let mut keyboard = KittyKeyboard::default();
        keyboard.observe(&csi(">1", b'u'));
        keyboard.set_alternate_screen(true);
        assert_eq!(keyboard.current().bits(), 0);
        keyboard.observe(&csi(">2", b'u'));
        assert_eq!(keyboard.current().bits(), 2);
        keyboard.set_alternate_screen(false);
        assert_eq!(keyboard.current().bits(), 1);
    }

    #[test]
    fn given_a_sequence_that_is_not_a_keyboard_control_then_nothing_changes() {
        let mut keyboard = KittyKeyboard::default();
        assert_eq!(keyboard.observe(&csi(">1", b't')), None);
        assert_eq!(keyboard.observe(&csi("1", b'u')), None);
        assert_eq!(keyboard.current().bits(), 0);
    }
}
