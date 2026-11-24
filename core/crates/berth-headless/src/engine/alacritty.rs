//! Alacritty's terminal grid behind the [`TerminalEngine`] seam.
//!
//! Alacritty owns reflow, scrollback and selection, so this adapter is mostly
//! translation. The crate root's side channel still records the sequences
//! Alacritty does not expose (DECSET 2026, kitty keyboard, DECSCUSR).

use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{ClipboardType, Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};

use super::{EngineCell, TerminalEngine};
use crate::{ImageView, MouseEncoding, MouseMode, ScreenMeta};

struct TermSize {
    columns: usize,
    screen_lines: usize,
}

impl Dimensions for TermSize {
    fn columns(&self) -> usize {
        self.columns
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn total_lines(&self) -> usize {
        self.screen_lines
    }
}

/// Routes the events the grid cannot apply itself into `ScreenMeta`.
struct Events {
    meta: Arc<Mutex<ScreenMeta>>,
    size: Arc<Mutex<(u16, u16)>>,
}

impl EventListener for Events {
    fn send_event(&self, event: Event) {
        let mut meta = self.meta.lock().unwrap_or_else(|error| error.into_inner());
        match event {
            Event::PtyWrite(text) => meta.responses.push(text),
            Event::Title(title) => meta.title = Some(title),
            Event::ResetTitle => meta.title = None,
            Event::ClipboardStore(_, data) => meta.clipboard = Some(data),
            Event::ClipboardLoad(kind, _) => {
                meta.clipboard_read = Some(
                    match kind {
                        ClipboardType::Clipboard => "c",
                        ClipboardType::Selection => "p",
                    }
                    .to_string(),
                );
            }
            Event::Bell => meta.bells += 1,
            Event::TextAreaSizeRequest(format) => {
                let (rows, cols) = *self.size.lock().unwrap_or_else(|error| error.into_inner());
                if let Some((width, height)) = meta.cell_pixels
                    && rows > 0
                    && cols > 0
                {
                    meta.responses.push(format(WindowSize {
                        num_lines: rows,
                        num_cols: cols,
                        cell_width: width,
                        cell_height: height,
                    }));
                }
            }
            _ => {}
        }
    }
}

pub struct AlacrittyEngine {
    term: Term<Events>,
    parser: Processor,
    size: Arc<Mutex<(u16, u16)>>,
}

impl AlacrittyEngine {
    pub fn new(
        rows: u16,
        cols: u16,
        scrollback: usize,
        graphics: bool,
        meta: Arc<Mutex<ScreenMeta>>,
        _images: Arc<Mutex<Vec<ImageView>>>,
    ) -> Self {
        let _ = graphics;
        let size = Arc::new(Mutex::new((rows, cols)));
        let dimensions = TermSize {
            columns: usize::from(cols.max(1)),
            screen_lines: usize::from(rows.max(1)),
        };
        let config = Config {
            scrolling_history: scrollback,
            ..Config::default()
        };
        let events = Events {
            meta,
            size: Arc::clone(&size),
        };
        Self {
            term: Term::new(config, &dimensions, events),
            parser: Processor::new(),
            size,
        }
    }

    fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    fn cell_at(&self, row: u16, col: u16) -> Option<&Cell> {
        let (rows, cols) = self.size();
        if row >= rows || col >= cols {
            return None;
        }
        let line = Line(i32::from(row) - self.display_offset() as i32);
        Some(&self.term.grid()[Point::new(line, Column(usize::from(col)))])
    }
}

impl TerminalEngine for AlacrittyEngine {
    fn process(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    fn resize(&mut self, rows: u16, cols: u16, _allow_reflow: bool) {
        *self.size.lock().unwrap_or_else(|error| error.into_inner()) = (rows, cols);
        let size = TermSize {
            columns: usize::from(cols.max(1)),
            screen_lines: usize::from(rows.max(1)),
        };
        self.term.resize(size);
    }

    fn size(&self) -> (u16, u16) {
        (
            self.term.screen_lines().min(usize::from(u16::MAX)) as u16,
            self.term.columns().min(usize::from(u16::MAX)) as u16,
        )
    }

    fn cursor(&self) -> (u16, u16) {
        let point = self.term.grid().cursor.point;
        let row = (point.line.0 + self.display_offset() as i32).max(0) as u16;
        (row, point.column.0.min(usize::from(u16::MAX)) as u16)
    }

    fn cursor_hidden(&self) -> bool {
        !self.term.mode().contains(TermMode::SHOW_CURSOR)
    }

    fn alternate_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    fn scrollback_len(&self) -> usize {
        self.term.grid().history_size()
    }

    fn scrollback(&self) -> usize {
        self.display_offset()
    }

    fn set_scrollback(&mut self, rows: usize) {
        let current = self.display_offset();
        let target = rows.min(self.scrollback_len());
        let delta = current as i32 - target as i32;
        if delta != 0 {
            self.term.scroll_display(Scroll::Delta(delta));
        }
    }

    fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    fn mouse_mode(&self) -> MouseMode {
        let mode = self.term.mode();
        if mode.contains(TermMode::MOUSE_MOTION) {
            MouseMode::Motion
        } else if mode.contains(TermMode::MOUSE_DRAG) {
            MouseMode::Drag
        } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            MouseMode::PressRelease
        } else {
            MouseMode::Off
        }
    }

    fn mouse_encoding(&self) -> MouseEncoding {
        let mode = self.term.mode();
        if mode.contains(TermMode::SGR_MOUSE) {
            MouseEncoding::Sgr
        } else if mode.contains(TermMode::UTF8_MOUSE) {
            MouseEncoding::Utf8
        } else {
            MouseEncoding::X10
        }
    }

    fn cell(&self, row: u16, col: u16) -> Option<EngineCell> {
        let cell = self.cell_at(row, col)?;
        let mut contents = String::new();
        contents.push(cell.c);
        if let Some(extra) = cell.zerowidth() {
            contents.extend(extra.iter());
        }
        Some(EngineCell {
            contents,
            fg: alacritty_color(cell.fg),
            bg: alacritty_color(cell.bg),
            bold: cell.flags.contains(Flags::BOLD),
            dim: cell.flags.contains(Flags::DIM),
            italic: cell.flags.contains(Flags::ITALIC),
            underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
            inverse: cell.flags.contains(Flags::INVERSE),
            wide: cell.flags.contains(Flags::WIDE_CHAR),
            wide_continuation: cell.flags.contains(Flags::WIDE_CHAR_SPACER)
                || cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER),
        })
    }

    fn contents_between(
        &self,
        start_row: u16,
        start_col: u16,
        end_row: u16,
        end_col: u16,
    ) -> String {
        if start_row > end_row {
            return String::new();
        }
        let mut out = String::new();
        for row in start_row..=end_row {
            let first = if row == start_row { start_col } else { 0 };
            let last = if row == end_row {
                end_col
            } else {
                self.size().1
            };
            let mut line = String::new();
            for col in first..last {
                if let Some(cell) = self.cell(row, col)
                    && !cell.wide_continuation
                {
                    line.push_str(&cell.contents);
                }
            }
            out.push_str(line.trim_end());
            if row != end_row {
                out.push('\n');
            }
        }
        out
    }

    fn line_text(&self, row: u16) -> String {
        let cols = self.size().1;
        self.contents_between(row, 0, row, cols)
    }

    fn screen_text(&self) -> String {
        let (rows, cols) = self.size();
        let mut lines: Vec<String> = (0..rows)
            .map(|row| self.contents_between(row, 0, row, cols))
            .collect();
        while lines.last().is_some_and(|line| line.is_empty()) {
            lines.pop();
        }
        lines.join("\n")
    }
}

fn alacritty_color(color: Color) -> Option<[u8; 3]> {
    match color {
        Color::Spec(rgb) => Some([rgb.r, rgb.g, rgb.b]),
        Color::Indexed(index) => Some(crate::style::indexed_rgb(index)),
        Color::Named(named) => named_color(named),
    }
}

fn named_color(color: NamedColor) -> Option<[u8; 3]> {
    let index = match color {
        NamedColor::Black | NamedColor::DimBlack => 0,
        NamedColor::Red | NamedColor::DimRed => 1,
        NamedColor::Green | NamedColor::DimGreen => 2,
        NamedColor::Yellow | NamedColor::DimYellow => 3,
        NamedColor::Blue | NamedColor::DimBlue => 4,
        NamedColor::Magenta | NamedColor::DimMagenta => 5,
        NamedColor::Cyan | NamedColor::DimCyan => 6,
        NamedColor::White | NamedColor::DimWhite => 7,
        NamedColor::BrightBlack => 8,
        NamedColor::BrightRed => 9,
        NamedColor::BrightGreen => 10,
        NamedColor::BrightYellow => 11,
        NamedColor::BrightBlue => 12,
        NamedColor::BrightMagenta => 13,
        NamedColor::BrightCyan => 14,
        NamedColor::BrightWhite => 15,
        // Foreground/background/cursor inherit the theme.
        NamedColor::Foreground
        | NamedColor::Background
        | NamedColor::Cursor
        | NamedColor::BrightForeground
        | NamedColor::DimForeground => return None,
    };
    Some(crate::style::indexed_rgb(index))
}
