//! The original `vt100` engine, wrapped so the rest of the crate no longer
//! names it. Behaviour is unchanged, including the capture-and-replay reflow.

use std::num::NonZeroU16;

use vt100::Screen;

use super::{EngineCell, TerminalEngine};
use crate::{MouseEncoding, MouseMode, ScreenEvents, ansi_rgb};

type Parser = vt100::Parser<ScreenEvents>;

pub struct BuiltinEngine {
    parser: Parser,
}

impl BuiltinEngine {
    pub fn new(rows: u16, cols: u16, scrollback: usize, events: ScreenEvents) -> Self {
        Self {
            parser: Parser::new_with_callbacks(
                NonZeroU16::new(rows.max(1)).expect("rows > 0"),
                NonZeroU16::new(cols.max(1)).expect("cols > 0"),
                scrollback,
                events,
            ),
        }
    }

    fn screen(&self) -> &Screen {
        self.parser.screen()
    }
}

impl TerminalEngine for BuiltinEngine {
    fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    fn resize(&mut self, rows: u16, cols: u16, allow_reflow: bool) {
        let target_rows = NonZeroU16::new(rows.max(1)).expect("rows > 0");
        let target_cols = NonZeroU16::new(cols.max(1)).expect("cols > 0");
        let (_, old_cols) = self.screen().size();
        let reflowed = (allow_reflow && old_cols != target_cols)
            .then(|| crate::reflow::reflowed(&self.parser, target_rows, target_cols))
            .flatten();
        match reflowed {
            Some(fresh) => self.parser = fresh,
            None => self.parser.set_size(target_rows, target_cols),
        }
    }

    fn size(&self) -> (u16, u16) {
        let (rows, cols) = self.screen().size();
        (rows.get(), cols.get())
    }

    fn cursor(&self) -> (u16, u16) {
        self.screen().cursor_position()
    }

    fn cursor_hidden(&self) -> bool {
        self.screen().hide_cursor()
    }

    fn alternate_screen(&self) -> bool {
        self.screen().alternate_screen()
    }

    fn scrollback_len(&self) -> usize {
        self.screen().scrollback_len()
    }

    fn scrollback(&self) -> usize {
        self.screen().scrollback()
    }

    fn set_scrollback(&mut self, rows: usize) {
        self.parser.screen_mut().set_scrollback(rows);
    }

    fn bracketed_paste(&self) -> bool {
        self.screen().bracketed_paste()
    }

    fn mouse_mode(&self) -> MouseMode {
        match self.screen().mouse_protocol_mode() {
            vt100::MouseProtocolMode::None => MouseMode::Off,
            vt100::MouseProtocolMode::Press => MouseMode::Click,
            vt100::MouseProtocolMode::PressRelease => MouseMode::PressRelease,
            vt100::MouseProtocolMode::ButtonMotion => MouseMode::Drag,
            vt100::MouseProtocolMode::AnyMotion => MouseMode::Motion,
        }
    }

    fn mouse_encoding(&self) -> MouseEncoding {
        match self.screen().mouse_protocol_encoding() {
            vt100::MouseProtocolEncoding::Default => MouseEncoding::X10,
            vt100::MouseProtocolEncoding::Utf8 => MouseEncoding::Utf8,
            vt100::MouseProtocolEncoding::Sgr => MouseEncoding::Sgr,
        }
    }

    fn cell(&self, row: u16, col: u16) -> Option<EngineCell> {
        let cell = self.screen().cell(row, col)?;
        Some(EngineCell {
            contents: cell.contents().to_string(),
            fg: ansi_rgb(cell.fgcolor()),
            bg: ansi_rgb(cell.bgcolor()),
            bold: cell.bold(),
            dim: cell.dim(),
            italic: cell.italic(),
            underline: cell.underline(),
            inverse: cell.inverse(),
            wide: cell.is_wide(),
            wide_continuation: cell.is_wide_continuation(),
        })
    }

    fn contents_between(
        &self,
        start_row: u16,
        start_col: u16,
        end_row: u16,
        end_col: u16,
    ) -> String {
        self.screen()
            .contents_between(start_row, start_col, end_row, end_col)
    }

    fn line_text(&self, row: u16) -> String {
        let (_, cols) = self.screen().size();
        self.screen()
            .contents_between(row, 0, row, cols.get())
            .trim_end()
            .to_string()
    }

    fn screen_text(&self) -> String {
        self.screen().contents()
    }
}
