//! The terminal-engine seam.
//!
//! `berth-headless` used to name `vt100` directly. Everything a shell needs is
//! now expressed by [`TerminalEngine`], so a second parser/grid (Alacritty) can
//! be selected from `config.json` without touching the GUI.
//!
//! The engine owns only the grid: bytes in, cells/modes/cursor out. The side
//! channel in the crate root still owns the sequences the parsers disagree on
//! (DECSET 2026, the kitty keyboard protocol, DECSCUSR, kitty graphics, sixel).

mod builtin;

#[cfg(feature = "alacritty-backend")]
mod alacritty;

pub use builtin::BuiltinEngine;

#[cfg(feature = "alacritty-backend")]
pub use alacritty::AlacrittyEngine;

use std::sync::{Arc, Mutex};

use crate::{ImageView, MouseEncoding, MouseMode, ScreenEvents, ScreenMeta};

/// One grid cell, reduced to what a renderer needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineCell {
    pub contents: String,
    pub fg: Option<[u8; 3]>,
    pub bg: Option<[u8; 3]>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub wide: bool,
    pub wide_continuation: bool,
}

/// What a surface's terminal does. One implementation per backend.
pub trait TerminalEngine: Send {
    /// Feed raw PTY bytes into the parser.
    fn process(&mut self, bytes: &[u8]);

    /// Apply a new grid size. `allow_reflow` is false while an inline image is
    /// anchored to the old layout; engines that reflow natively may ignore it.
    fn resize(&mut self, rows: u16, cols: u16, allow_reflow: bool);

    /// `(rows, cols)`.
    fn size(&self) -> (u16, u16);
    /// Visible cursor as `(row, col)`.
    fn cursor(&self) -> (u16, u16);
    fn cursor_hidden(&self) -> bool;
    fn alternate_screen(&self) -> bool;

    fn scrollback_len(&self) -> usize;
    fn scrollback(&self) -> usize;
    fn set_scrollback(&mut self, rows: usize);

    fn bracketed_paste(&self) -> bool;
    fn mouse_mode(&self) -> MouseMode;
    fn mouse_encoding(&self) -> MouseEncoding;

    /// A visible cell, or `None` past the grid.
    fn cell(&self, row: u16, col: u16) -> Option<EngineCell>;

    /// Plain text between two visible cells (selection readback).
    fn contents_between(
        &self,
        start_row: u16,
        start_col: u16,
        end_row: u16,
        end_col: u16,
    ) -> String;

    /// One visible row, trailing blanks removed.
    fn line_text(&self, row: u16) -> String;

    /// The visible screen as text, rows joined by newlines.
    fn screen_text(&self) -> String;
}

/// The parser/grid a shell selects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Backend {
    #[default]
    Builtin,
    Alacritty,
    Wezterm,
    Ghostty,
}

impl Backend {
    /// Parse a `terminalBackend` value. Unknown names return `None` so the
    /// caller can fall back to the default.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "builtin" | "vt100" => Some(Self::Builtin),
            "alacritty" => Some(Self::Alacritty),
            "wezterm" | "termwiz" => Some(Self::Wezterm),
            "ghostty" | "libghostty" => Some(Self::Ghostty),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Alacritty => "alacritty",
            Self::Wezterm => "wezterm",
            Self::Ghostty => "ghostty",
        }
    }

    /// Every value the settings row cycles through.
    pub const ALL: [Backend; 4] = [
        Backend::Builtin,
        Backend::Alacritty,
        Backend::Wezterm,
        Backend::Ghostty,
    ];

    /// Whether this build can actually run the backend.
    pub fn implemented(self) -> bool {
        match self {
            Self::Builtin => true,
            Self::Alacritty => cfg!(feature = "alacritty-backend"),
            Self::Wezterm | Self::Ghostty => false,
        }
    }
}

/// Build the selected engine for a new surface. An unavailable backend falls
/// back to the builtin engine, which is what the config layer logs.
pub fn create(
    backend: Backend,
    rows: u16,
    cols: u16,
    scrollback: usize,
    graphics: bool,
    meta: Arc<Mutex<ScreenMeta>>,
    images: Arc<Mutex<Vec<ImageView>>>,
) -> Box<dyn TerminalEngine> {
    match backend {
        #[cfg(feature = "alacritty-backend")]
        Backend::Alacritty => Box::new(AlacrittyEngine::new(
            rows, cols, scrollback, graphics, meta, images,
        )),
        _ => {
            let events = ScreenEvents::new(meta, images, graphics, scrollback);
            Box::new(BuiltinEngine::new(rows, cols, scrollback, events))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "alacritty-backend")]
    fn engine(backend: Backend) -> Box<dyn TerminalEngine> {
        let meta = Arc::new(Mutex::new(ScreenMeta::default()));
        let images = Arc::new(Mutex::new(Vec::new()));
        create(backend, 24, 80, 100, true, meta, images)
    }

    #[test]
    fn backend_names_parse_and_are_stable() {
        for backend in Backend::ALL {
            assert_eq!(Backend::parse(backend.name()), Some(backend));
        }
        assert_eq!(Backend::parse("VT100"), Some(Backend::Builtin));
        assert_eq!(Backend::parse("nope"), None);
    }

    #[cfg(feature = "alacritty-backend")]
    #[test]
    fn builtin_and_alacritty_agree_on_text_colours_and_wide_cells() {
        let bytes = b"hello\r\n\x1b[31mred\x1b[0m \xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e";
        let mut builtin = engine(Backend::Builtin);
        let mut alacritty = engine(Backend::Alacritty);
        builtin.process(bytes);
        alacritty.process(bytes);

        assert_eq!(builtin.size(), alacritty.size());
        assert_eq!(
            builtin.screen_text().trim_end(),
            alacritty.screen_text().trim_end()
        );
        assert_eq!(builtin.cell(0, 0), alacritty.cell(0, 0));
        assert_eq!(builtin.cell(1, 0), alacritty.cell(1, 0));

        let wide = builtin.cell(1, 4).expect("builtin wide cell");
        let other = alacritty.cell(1, 4).expect("alacritty wide cell");
        assert!(wide.wide && other.wide, "{wide:?} vs {other:?}");
        assert_eq!(
            builtin.cell(1, 5).map(|cell| cell.wide_continuation),
            alacritty.cell(1, 5).map(|cell| cell.wide_continuation)
        );
    }

    #[cfg(feature = "alacritty-backend")]
    #[test]
    fn alacritty_reflows_a_narrowed_grid() {
        let mut engine = engine(Backend::Alacritty);
        engine.process(b"one two three four five six seven");
        engine.resize(24, 12, true);
        let (_, cols) = engine.size();
        assert_eq!(cols, 12);
        assert!(engine.screen_text().contains("seven"));
    }
}
