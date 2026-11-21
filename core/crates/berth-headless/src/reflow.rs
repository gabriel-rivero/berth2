//! Width-change reflow for the portable screen.
//!
//! `vt100` resizes by truncating and padding rows, which cuts every wrapped
//! line short. This module captures the grid as text with a newline only where
//! the program broke the line, replays it at the new width, and restores the
//! cursor and the input modes the program had set.

use std::num::NonZeroU16;

use vt100::Parser;

/// Rebuild `parser` at `rows` by `cols`, re-wrapping lines that wrapped.
///
/// Returns `None` on the alternate screen: there is no history to reflow and a
/// full-screen program redraws itself on SIGWINCH, so the plain resize is
/// safer.
pub fn reflowed<CB: vt100::Callbacks + Clone>(
    parser: &Parser<CB>,
    rows: NonZeroU16,
    cols: NonZeroU16,
) -> Option<Parser<CB>> {
    let screen = parser.screen();
    if screen.alternate_screen() {
        return None;
    }
    let (cursor_row, cursor_col) = screen.cursor_position();
    let old_rows = screen.size().0.get();
    let scroll = screen.scrollback();
    let at_end = cursor_at_content_end(screen);
    let mut fresh = Parser::new_with_callbacks(
        rows,
        cols,
        screen.scrollback_capacity(),
        parser.callbacks().clone(),
    );
    fresh.process(captured_text(screen).as_bytes());
    if at_end {
        let (row, col) = content_end(fresh.screen());
        fresh.process(format!("\x1b[{};{}H", row + 1, col + 1).as_bytes());
    } else {
        // Keep the distance from the bottom edge; the column cannot be mapped
        // through a re-wrap, so it is clamped instead.
        let from_bottom = old_rows.saturating_sub(1).saturating_sub(cursor_row);
        let row = rows.get().saturating_sub(1).saturating_sub(from_bottom);
        let col = cursor_col.min(cols.get() - 1);
        fresh.process(format!("\x1b[{};{}H", row + 1, col + 1).as_bytes());
    }
    fresh.process(restored_modes(screen).as_bytes());
    let offset = scroll.min(fresh.screen().scrollback_len());
    if offset > 0 {
        fresh.screen_mut().set_scrollback(offset);
    }
    Some(fresh)
}

/// The whole grid as SGR text. `vt100` writes a newline only where a row did
/// not wrap and leaves empty cells unwritten, so replaying it re-wraps each
/// logical line without adding padding.
fn captured_text(screen: &vt100::Screen) -> String {
    let mut state = vt100::capture::BasicFormattedCaptureState::new();
    let mut text = String::new();
    let _ = screen.write_contents_formatted_basic(
        &mut text,
        vt100::capture::BasicFormattedCaptureRange::Full(&mut state),
    );
    // `vt100` treats a bare line feed as a row move only, so every row break
    // needs its carriage return or the next row starts mid-column.
    text.replace('\n', "\r\n")
}

/// Whether the cursor sits after the last written character, which is also
/// where a replay leaves it.
fn cursor_at_content_end(screen: &vt100::Screen) -> bool {
    let (rows, cols) = screen.size();
    let (cursor_row, cursor_col) = screen.cursor_position();
    for row in (0..rows.get()).rev() {
        for col in (0..cols.get()).rev() {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.contents().trim().is_empty() {
                continue;
            }
            return row == cursor_row && col < cursor_col;
        }
    }
    true
}

/// Where a cursor that sat after the text lands: after the last written
/// character in the replayed grid.
fn content_end(screen: &vt100::Screen) -> (u16, u16) {
    let (rows, cols) = screen.size();
    for row in (0..rows.get()).rev() {
        for col in (0..cols.get()).rev() {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.contents().trim().is_empty() {
                continue;
            }
            // A cursor after text in the last column cannot be expressed with
            // CUP. The clamp keeps it on that row, so a shell's CR redraw lands
            // where it would have with the pending wrap.
            return (row, (col + 1).min(cols.get() - 1));
        }
    }
    (0, 0)
}

/// Re-emit the modes the program set.
fn restored_modes(screen: &vt100::Screen) -> String {
    let mut out = String::new();
    if screen.hide_cursor() {
        out.push_str("\x1b[?25l");
    }
    if screen.application_cursor() {
        out.push_str("\x1b[?1h");
    }
    if screen.application_keypad() {
        out.push_str("\x1b=");
    }
    if screen.bracketed_paste() {
        out.push_str("\x1b[?2004h");
    }
    // `vt100` maps the protocol numbers its own way: 9 is Press, 1000 is
    // PressRelease, 1002 is ButtonMotion and 1003 is AnyMotion.
    out.push_str(match screen.mouse_protocol_mode() {
        vt100::MouseProtocolMode::None => "",
        vt100::MouseProtocolMode::Press => "\x1b[?9h",
        vt100::MouseProtocolMode::PressRelease => "\x1b[?1000h",
        vt100::MouseProtocolMode::ButtonMotion => "\x1b[?1002h",
        vt100::MouseProtocolMode::AnyMotion => "\x1b[?1003h",
    });
    out.push_str(match screen.mouse_protocol_encoding() {
        vt100::MouseProtocolEncoding::Default => "",
        vt100::MouseProtocolEncoding::Utf8 => "\x1b[?1005h",
        vt100::MouseProtocolEncoding::Sgr => "\x1b[?1006h",
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nz(value: u16) -> NonZeroU16 {
        NonZeroU16::new(value).expect("non-zero")
    }

    fn parser(rows: u16, cols: u16) -> Parser {
        Parser::new(nz(rows), nz(cols), 500)
    }

    /// Every printable character in the grid and its scrollback.
    fn full_text(parser: &Parser) -> String {
        let mut state = vt100::capture::BasicFormattedCaptureState::new();
        let mut text = String::new();
        parser
            .screen()
            .write_contents_formatted_basic(
                &mut text,
                vt100::capture::BasicFormattedCaptureRange::Full(&mut state),
            )
            .expect("capture");
        vt100::capture::basic_formatted_to_plain(&text)
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn given_a_wrapped_line_when_the_width_shrinks_then_every_character_survives() {
        let mut parser = parser(24, 40);
        let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";
        parser.process(text.as_bytes());
        let before = full_text(&parser);
        let narrow = reflowed(&parser, nz(24), nz(20)).expect("primary screen");
        let after = full_text(&narrow);
        assert_eq!(after, before);
        assert!(after.contains("lambda"));
    }

    #[test]
    fn given_wrapped_history_when_the_width_shrinks_then_no_line_is_lost() {
        let mut parser = parser(10, 40);
        for line in 0..80 {
            parser.process(format!("line{line:03} ").as_bytes());
            parser.process(b"abcdefghijklmnopqrstuvwxyz0123456789\r\n");
        }
        let narrow = reflowed(&parser, nz(10), nz(20)).expect("primary screen");
        let text = full_text(&narrow);
        for line in 0..80 {
            assert!(
                text.contains(&format!("line{line:03}")),
                "line{line:03} lost"
            );
        }
        assert!(text.contains("abcdefghijklmnopqrstuvwxyz"));
    }

    #[test]
    fn given_the_alternate_screen_then_nothing_is_reflowed() {
        let mut parser = parser(24, 40);
        parser.process(b"\x1b[?1049h");
        assert!(reflowed(&parser, nz(24), nz(20)).is_none());
    }

    #[test]
    fn given_the_cursor_after_the_text_then_a_rewrap_keeps_it_there() {
        let mut parser = parser(10, 10);
        parser.process(b"hello world");
        assert_eq!(parser.screen().cursor_position(), (1, 1));
        let narrow = reflowed(&parser, nz(10), nz(6)).expect("primary screen");
        assert_eq!(narrow.screen().cursor_position(), (1, 5));
    }

    #[test]
    fn given_a_cursor_off_the_text_then_it_keeps_its_distance_from_the_bottom() {
        let mut parser = parser(10, 40);
        parser.process(b"one\r\ntwo\r\nthree\r\n\r\n\r\nfour");
        parser.process(b"\x1b[7;1H");
        let wide = reflowed(&parser, nz(10), nz(20)).expect("primary screen");
        assert_eq!(wide.screen().cursor_position().0, 6);
    }

    #[test]
    fn given_input_modes_when_reflowed_then_they_survive() {
        let mut parser = parser(24, 40);
        parser.process(b"\x1b[?2004h\x1b[?1h\x1b[?1002h\x1b[?1006h");
        let narrow = reflowed(&parser, nz(24), nz(20)).expect("primary screen");
        let screen = narrow.screen();
        assert!(screen.bracketed_paste());
        assert!(screen.application_cursor());
        // `?1002h` is ButtonMotion in `vt100`'s mapping, not PressRelease.
        assert_eq!(
            screen.mouse_protocol_mode(),
            vt100::MouseProtocolMode::ButtonMotion
        );
        assert_eq!(
            screen.mouse_protocol_encoding(),
            vt100::MouseProtocolEncoding::Sgr
        );
    }

    #[test]
    fn given_a_coloured_blank_line_then_its_background_survives() {
        let mut parser = parser(24, 40);
        parser.process(b"\x1b[44m          \r\nworld");
        let narrow = reflowed(&parser, nz(24), nz(20)).expect("primary screen");
        assert_eq!(
            narrow.screen().cell(0, 0).expect("cell").bgcolor(),
            vt100::Color::Idx(4)
        );
        assert_eq!(
            narrow.screen().cell(1, 0).expect("cell").bgcolor(),
            vt100::Color::Idx(4)
        );
        assert!(narrow.screen().contents().contains("world"));
    }

    #[test]
    fn given_a_colour_left_on_then_the_next_line_keeps_it_after_reflow() {
        let mut parser = parser(24, 40);
        parser.process(b"\x1b[41mfirst\r\nsecond");
        let narrow = reflowed(&parser, nz(24), nz(20)).expect("primary screen");
        assert_eq!(
            narrow.screen().cell(1, 0).expect("cell").bgcolor(),
            vt100::Color::Idx(1)
        );
    }

    #[test]
    fn given_application_keypad_then_it_survives() {
        let mut parser = parser(24, 40);
        parser.process(b"\x1b=");
        let narrow = reflowed(&parser, nz(24), nz(20)).expect("primary screen");
        assert!(narrow.screen().application_keypad());
    }

    #[test]
    fn given_a_scrolled_back_view_then_the_offset_survives() {
        let mut parser = parser(10, 40);
        for line in 0..80 {
            parser.process(format!("line{line:03}\r\n").as_bytes());
        }
        parser.screen_mut().set_scrollback(5);
        let narrow = reflowed(&parser, nz(10), nz(20)).expect("primary screen");
        assert_eq!(narrow.screen().scrollback(), 5);
    }
}
