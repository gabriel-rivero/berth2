//! A byte-level scanner for CSI sequences the VT parser does not model.
//!
//! `vt100` owns the grid. Control sequences such as private modes
//! (`CSI ? 2026 h`), kitty's keyboard flags (`CSI > 1 u`) and extended SGR
//! parameters are outside its model and are dropped. The PTY reader feeds every
//! byte here as well as to `vt100`, so a sequence split across reads is
//! reassembled and observed without holding text back from the parser.

/// Longest parameter run kept. A sequence past this is dropped rather than
/// buffered without limit; a PTY can send arbitrary bytes.
const MAX_CSI_BODY: usize = 256;

/// One complete CSI sequence: the bytes between `ESC [` and the final byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Csi {
    pub body: Vec<u8>,
    pub final_byte: u8,
}

/// A private mode set (`CSI ? 25 h`) or reset (`CSI ? 25 l`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateModeChange {
    pub modes: Vec<u16>,
    /// `true` for `h`, `false` for `l`.
    pub is_enabled: bool,
}

impl Csi {
    /// The modes a private mode set or reset names, or `None` when the
    /// sequence is not one. Multiple modes keep their order.
    pub fn private_mode_change(&self) -> Option<PrivateModeChange> {
        let is_enabled = match self.final_byte {
            b'h' => true,
            b'l' => false,
            _ => return None,
        };
        let rest = self.body.strip_prefix(b"?")?;
        if rest.is_empty() {
            return None;
        }
        let modes = rest
            .split(|byte| *byte == b';')
            .map(|part| {
                if part.is_empty() || !part.iter().all(u8::is_ascii_digit) {
                    return None;
                }
                std::str::from_utf8(part).ok()?.parse().ok()
            })
            .collect::<Option<Vec<u16>>>()?;
        Some(PrivateModeChange { modes, is_enabled })
    }
}

/// Collects a CSI sequence one byte at a time. Call [`begin`](Self::begin)
/// after the `ESC [` that opens it, then [`feed`](Self::feed) every byte until
/// it returns the complete sequence.
#[derive(Debug, Default)]
pub struct CsiScanner {
    body: Vec<u8>,
    is_active: bool,
    is_overflowed: bool,
}

impl CsiScanner {
    /// Start collecting after the `ESC [` that opened a control sequence.
    pub fn begin(&mut self) {
        self.body.clear();
        self.is_active = true;
        self.is_overflowed = false;
    }

    /// Drop the sequence being collected, for a cancelled or malformed one.
    pub fn abort(&mut self) {
        self.body.clear();
        self.is_active = false;
        self.is_overflowed = false;
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    /// Consume one byte of an open sequence and return it once complete.
    pub fn feed(&mut self, byte: u8) -> Option<Csi> {
        if !self.is_active {
            return None;
        }
        if (0x40..=0x7e).contains(&byte) {
            self.is_active = false;
            if self.is_overflowed {
                self.body.clear();
                self.is_overflowed = false;
                return None;
            }
            return Some(Csi {
                body: std::mem::take(&mut self.body),
                final_byte: byte,
            });
        }
        if (0x20..=0x3f).contains(&byte) {
            if self.body.len() < MAX_CSI_BODY {
                self.body.push(byte);
            } else {
                self.body.clear();
                self.is_overflowed = true;
            }
        } else {
            // CAN, SUB or a stray control byte cancels the sequence.
            self.abort();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(bytes: &[u8]) -> Option<Csi> {
        let mut scanner = CsiScanner::default();
        scanner.begin();
        bytes.iter().filter_map(|byte| scanner.feed(*byte)).next()
    }

    #[test]
    fn given_a_private_mode_set_then_the_modes_are_parsed() {
        let sequence = scan(b"?25h").expect("sequence");
        let change = sequence.private_mode_change().expect("mode change");
        assert!(change.is_enabled);
        assert_eq!(change.modes, vec![25]);
    }

    #[test]
    fn given_a_private_mode_reset_then_it_is_a_disable() {
        let sequence = scan(b"?2026l").expect("sequence");
        let change = sequence.private_mode_change().expect("mode change");
        assert!(!change.is_enabled);
        assert_eq!(change.modes, vec![2026]);
    }

    #[test]
    fn given_several_modes_then_all_of_them_are_returned() {
        let sequence = scan(b"?1004;2026;25h").expect("sequence");
        assert_eq!(
            sequence.private_mode_change().expect("mode change").modes,
            vec![1004, 2026, 25]
        );
    }

    #[test]
    fn given_a_non_private_sequence_then_there_is_no_mode_change() {
        assert_eq!(scan(b"2J").expect("sequence").private_mode_change(), None);
    }

    #[test]
    fn given_a_sequence_that_is_not_a_mode_change_then_there_is_none() {
        assert_eq!(scan(b"?1u").expect("sequence").private_mode_change(), None);
        assert_eq!(
            scan(b"?2026$x").expect("sequence").private_mode_change(),
            None
        );
        assert_eq!(scan(b"?;h").expect("sequence").private_mode_change(), None);
    }

    #[test]
    fn given_a_sequence_split_across_feeds_then_it_completes() {
        let mut scanner = CsiScanner::default();
        scanner.begin();
        for byte in b"?2026" {
            assert_eq!(scanner.feed(*byte), None);
        }
        let sequence = scanner.feed(b'h').expect("complete");
        assert_eq!(
            sequence.private_mode_change().expect("mode change").modes,
            vec![2026]
        );
        assert!(!scanner.is_active());
    }

    #[test]
    fn given_an_overlong_sequence_then_it_is_dropped_and_the_next_one_works() {
        let mut scanner = CsiScanner::default();
        scanner.begin();
        for _ in 0..MAX_CSI_BODY + 1 {
            assert_eq!(scanner.feed(b'1'), None);
        }
        assert_eq!(scanner.feed(b'h'), None);

        scanner.begin();
        for byte in b"?2025" {
            assert_eq!(scanner.feed(*byte), None);
        }
        let sequence = scanner.feed(b'h').expect("next sequence");
        assert_eq!(
            sequence.private_mode_change().expect("mode change").modes,
            vec![2025]
        );
    }

    #[test]
    fn given_a_control_byte_then_the_sequence_is_cancelled() {
        let mut scanner = CsiScanner::default();
        scanner.begin();
        assert_eq!(scanner.feed(b'?'), None);
        assert_eq!(scanner.feed(0x18), None);
        assert!(!scanner.is_active());
        assert_eq!(scanner.feed(b'h'), None);
    }
}
