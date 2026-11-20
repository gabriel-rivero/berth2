//! A headless Berth shell: PTYs, a VT screen, and the socket server.
//!
//! This is the first half of the Linux/Windows shell spike. It owns real
//! terminal processes and implements [`SurfaceOps`], but paints nothing: a GUI
//! shell can later reuse the same engine and add a renderer on top.
//!
//! It is also useful on its own for CI: `berth-headless` speaks the exact
//! protocol, so the end-to-end criterion (create, type, read back) runs without
//! a display or Ghostty.

pub mod keys;

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use berth_core::model::NotifLevel;
use berth_core::store::Snapshot;
use berth_core::{
    AppState, CoreError, ErrorCode, Notification, PidInfo, ReadScope, Readback, SpawnRequest,
    Spawned, SurfaceOps,
};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtyPair, PtySize, native_pty_system};

/// Scrollback kept per surface. Enough for a day of agent output; the GUI can
/// render a window into it later.
pub const DEFAULT_SCROLLBACK: usize = 10_000;

/// One rendered character cell, with just enough styling for a GUI.
#[derive(Debug, Clone, Default)]
pub struct CellView {
    pub contents: String,
    /// `None` means the terminal default.
    pub fg: Option<[u8; 3]>,
    pub bg: Option<[u8; 3]>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    /// First half of a double-width character; the next cell is the tail.
    pub wide: bool,
    pub wide_continuation: bool,
}

/// A snapshot of the visible screen. Safe to hand to a renderer thread.
/// How an iTerm2 image asked to be sized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dimension {
    #[default]
    Auto,
    Cells(usize),
    Pixels(u32),
}

/// One decoded inline image, anchored to an absolute grid line.
#[derive(Clone, Debug)]
pub struct ImageView {
    pub anchor_line: usize,
    pub col: usize,
    pub width: u32,
    pub height: u32,
    pub target_width: Dimension,
    pub target_height: Dimension,
    pub pixels: Arc<Vec<u8>>,
}

/// A kitty image placed with Unicode placeholders (U+10EEEE cells).
#[derive(Clone, Debug)]
pub struct KittyImage {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<Vec<u8>>,
}

mod sixel;

#[derive(Debug, Clone, Default)]
pub struct ScreenView {
    pub cols: u16,
    pub rows: u16,
    /// Zero-based `(row, col)`.
    pub cursor: (u16, u16),
    pub cursor_hidden: bool,
    /// Rows of history above the top of the view.
    pub scrollback_len: usize,
    /// How many of those rows the view is scrolled up by.
    pub scrollback: usize,
    pub bracketed_paste: bool,
    /// Mouse tracking mode the program asked for.
    pub mouse_mode: MouseMode,
    /// Wire encoding for mouse reports.
    pub mouse_encoding: MouseEncoding,
    pub cells: Vec<CellView>,
    pub images: Vec<ImageView>,
    pub placeholders: Vec<KittyImage>,
}

/// How much pointer traffic the program wants. Mirrors `vt100`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseMode {
    #[default]
    Off,
    /// Report presses only, X10 style.
    Click,
    /// Report presses and releases.
    PressRelease,
    /// Report presses, releases and drags while a button is held.
    Drag,
    /// Report every pointer movement.
    Motion,
}

impl MouseMode {
    pub fn is_on(self) -> bool {
        self != Self::Off
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseEncoding {
    #[default]
    X10,
    Utf8,
    Sgr,
}

/// Terminal events that do not change the grid: title, cwd, bell, progress,
/// and clipboard requests. A shell publishes this so the UI can mirror what the
/// program in the terminal asked for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScreenMeta {
    /// Latest OSC 0/2 title.
    pub title: Option<String>,
    /// Latest OSC 7 working directory, as a local path.
    pub cwd: Option<String>,
    /// Monotonic count of bells (BEL, OSC body, visual bell).
    pub bells: u32,
    /// OSC 9;4 progress, `Some(None)` clears it.
    pub progress: Option<Option<f32>>,
    /// Latest OSC 52 clipboard write, already decoded.
    pub clipboard: Option<String>,
    /// Latest OSC 9 / OSC 777 desktop notification body.
    pub notification: Option<String>,
    /// Cell size in pixels, reported through XTWINOPS (`CSI 16 t`). The UI
    /// sets it on resize; zero means unknown.
    pub cell_pixels: Option<(u16, u16)>,
    /// Replies the terminal owes the program (cursor position, device
    /// attributes). The reader drains them back to the PTY.
    pub responses: Vec<String>,
}

/// Callbacks handed to `vt100`, writing into the surface's [`ScreenMeta`].
#[derive(Clone)]
pub struct ScreenEvents {
    meta: Arc<Mutex<ScreenMeta>>,
    images: Arc<Mutex<Vec<ImageView>>>,
    graphics: bool,
    scrollback: usize,
}

impl ScreenEvents {
    pub fn new(
        meta: Arc<Mutex<ScreenMeta>>,
        images: Arc<Mutex<Vec<ImageView>>>,
        graphics: bool,
        scrollback: usize,
    ) -> Self {
        Self {
            meta,
            images,
            graphics,
            scrollback,
        }
    }
}

impl vt100::Callbacks for ScreenEvents {
    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        let mut meta = lock(&self.meta);
        meta.bells = meta.bells.saturating_add(1);
    }

    fn visual_bell(&mut self, _: &mut vt100::Screen) {
        let mut meta = lock(&self.meta);
        meta.bells = meta.bells.saturating_add(1);
    }

    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        lock(&self.meta).title = Some(String::from_utf8_lossy(title).into_owned());
    }

    fn copy_to_clipboard(&mut self, _: &mut vt100::Screen, _: &[u8], data: &[u8]) {
        if let Some(text) = decode_base64(data) {
            lock(&self.meta).clipboard = Some(text);
        }
    }

    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        let first = params
            .first()
            .and_then(|param| param.first())
            .copied()
            .unwrap_or(0);
        let mut meta = lock(&self.meta);
        match (i1, c) {
            // Device status report: 5 is "ready", 6 is the cursor position.
            (None, 'n') => match first {
                5 => meta.responses.push("\x1b[0n".into()),
                6 => {
                    let (row, col) = screen.cursor_position();
                    meta.responses.push(format!(
                        "\x1b[{};{}R",
                        row.saturating_add(1),
                        col.saturating_add(1)
                    ));
                }
                _ => {}
            },
            // Primary and secondary device attributes.
            (None, 'c') => meta.responses.push("\x1b[?1;2c".into()),
            (Some(b'>'), 'c') => meta.responses.push("\x1b[>0;0;0c".into()),
            // XTWINOPS: 14 is the text area in pixels, 16 the cell size.
            (None, 't') => {
                if let Some((width, height)) = meta.cell_pixels {
                    let (rows, cols) = screen.size();
                    let (rows, cols) = (u32::from(rows.get()), u32::from(cols.get()));
                    match first {
                        14 => meta.responses.push(format!(
                            "\x1b[4;{};{}t",
                            u32::from(height) * rows,
                            u32::from(width) * cols
                        )),
                        16 => meta.responses.push(format!("\x1b[6;{height};{width}t")),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn unhandled_osc(&mut self, screen: &mut vt100::Screen, params: &[&[u8]]) {
        if self.graphics && params.first().is_some_and(|param| *param == b"1337") {
            // vte splits the OSC body on ';', and the iTerm2 metadata uses
            // those too, so put the body back together before parsing it.
            let mut payload = Vec::new();
            for (index, part) in params.iter().enumerate().skip(1) {
                if index > 1 {
                    payload.push(b';');
                }
                payload.extend_from_slice(part);
            }
            store_iterm_image(screen, &payload, &self.images, self.scrollback);
            return;
        }
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        let mut meta = lock(&self.meta);
        match params {
            // OSC 7: the shell reports its working directory as a file URL.
            [b"7", url] => meta.cwd = parse_file_url(&text(url)),
            // OSC 9;4: progress ("4;1;42", "4;0" clears).
            [b"9", b"4", b"0", ..] => meta.progress = Some(None),
            [b"9", b"4", _, progress] => {
                let value = text(progress).parse::<f32>().ok();
                meta.progress = Some(value.map(|value| (value / 100.0).clamp(0.0, 1.0)));
            }
            // OSC 9 and OSC 777 are desktop notifications.
            [b"9", body] => meta.notification = Some(text(body)),
            [b"777", _, body] => meta.notification = Some(text(body)),
            _ => {}
        }
    }
}

/// Largest image accepted in either axis, and the per-surface cache budget.
const MAX_IMAGE_AXIS: u32 = 4096;
const MAX_IMAGES: usize = 8;
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;

/// OSC 1337 File=... : iTerm2's inline image. The data is base64 PNG.
fn store_iterm_image(
    screen: &vt100::Screen,
    payload: &[u8],
    images: &Arc<Mutex<Vec<ImageView>>>,
    scrollback: usize,
) {
    let text = String::from_utf8_lossy(payload);
    let Some(rest) = text.strip_prefix("File=") else {
        return;
    };
    let Some((metadata, data)) = rest.split_once(':') else {
        return;
    };
    let mut inline = false;
    let mut target_width = Dimension::Auto;
    let mut target_height = Dimension::Auto;
    for pair in metadata.split(';') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "inline" => inline = value == "1",
            "width" => target_width = parse_dimension(value),
            "height" => target_height = parse_dimension(value),
            _ => {}
        }
    }
    if !inline {
        return;
    }
    let Some(bytes) = decode_base64_bytes(data.as_bytes()) else {
        return;
    };
    let Some((width, height, pixels)) = decode_png(&bytes) else {
        return;
    };
    let (row, col) = screen.cursor_position();
    push_image(
        images,
        scrollback,
        screen.scrollback_len() + usize::from(row),
        usize::from(col),
        width,
        height,
        target_width,
        target_height,
        pixels,
    );
}

/// Store an image anchored at a screen cell and enforce the cache budget.
#[allow(clippy::too_many_arguments)]
fn push_image(
    images: &Arc<Mutex<Vec<ImageView>>>,
    scrollback: usize,
    anchor_line: usize,
    col: usize,
    width: u32,
    height: u32,
    target_width: Dimension,
    target_height: Dimension,
    pixels: Vec<u8>,
) {
    let mut list = lock(images);
    list.push(ImageView {
        anchor_line,
        col,
        width,
        height,
        target_width,
        target_height,
        pixels: Arc::new(pixels),
    });
    let oldest = anchor_line.saturating_sub(scrollback);
    list.retain(|image| image.anchor_line >= oldest);
    while list.len() > MAX_IMAGES {
        list.remove(0);
    }
    let mut total: usize = list.iter().map(|image| image.pixels.len()).sum();
    while total > MAX_IMAGE_BYTES && list.len() > 1 {
        total -= list.remove(0).pixels.len();
    }
}

fn parse_dimension(value: &str) -> Dimension {
    if let Some(pixels) = value.strip_suffix("px") {
        return pixels.parse().map(Dimension::Pixels).unwrap_or_default();
    }
    value.parse().map(Dimension::Cells).unwrap_or_default()
}

fn decode_png(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let width = reader.info().width;
    let height = reader.info().height;
    if width == 0 || height == 0 || width > MAX_IMAGE_AXIS || height > MAX_IMAGE_AXIS {
        return None;
    }
    let mut buffer = vec![0u8; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buffer).ok()?;
    let data = &buffer[..frame.buffer_size()];
    let pixels = match reader.info().color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .chunks(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        png::ColorType::Grayscale => data
            .iter()
            .flat_map(|&gray| [gray, gray, gray, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .chunks(2)
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        png::ColorType::Indexed => return None,
    };
    if pixels.len() != width as usize * height as usize * 4 {
        return None;
    }
    Some((width, height, pixels))
}

fn parse_file_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    Some(percent_decode(path))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2]))
        {
            out.push(high * 16 + low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Decode the payload of an OSC 52 clipboard write.
fn decode_base64(data: &[u8]) -> Option<String> {
    String::from_utf8(decode_base64_bytes(data)?).ok()
}

fn decode_base64_bytes(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() * 3 / 4);
    let mut accumulator = 0u32;
    let mut bits = 0u8;
    for byte in data {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\r' | b'\n' | b' ' => continue,
            _ => return None,
        };
        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
            accumulator &= (1 << bits) - 1;
        }
    }
    Some(out)
}

type Parser = vt100::Parser<ScreenEvents>;

/// A live PTY-backed terminal and its parsed screen.
struct Surface {
    /// Kept for resize and foreground-process queries.
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    screen: Arc<Mutex<Parser>>,
    meta: Arc<Mutex<ScreenMeta>>,
    images: Arc<Mutex<Vec<ImageView>>>,
    kitty: Arc<Mutex<HashMap<u32, KittyImage>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    pid: u32,
    tty: String,
    alive: Arc<AtomicBool>,
}

/// Terminal host for a headless shell. Cheap to put behind an `Arc`; all state
/// is internally synchronised because the server calls from many threads.
/// A shell's repaint hook. Fired from the pty threads.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// The hook, shared with threads spawned before the shell installs it.
type SharedWaker = Arc<Mutex<Option<Waker>>>;

pub struct PtyHost {
    surfaces: Mutex<HashMap<String, Arc<Surface>>>,
    transport: Mutex<Transport>,
    scrollback: usize,
    graphics: bool,
    /// Program for a plain terminal surface (a spawn with no argv). Empty
    /// means the platform default. A GUI sets this from `config.json` so the
    /// user can run bash, zsh or WSL instead of PowerShell or `/bin/sh`.
    default_shell: Mutex<Vec<String>>,
    /// Fired by the read and exit threads. A GUI shell turns this into a
    /// repaint, so it never has to poll the surfaces for changes.
    waker: SharedWaker,
}

#[derive(Default)]
struct Transport {
    socket_path: Option<String>,
}

impl Default for PtyHost {
    fn default() -> Self {
        Self::new()
    }
}

impl PtyHost {
    pub fn new() -> Self {
        Self::with_scrollback(DEFAULT_SCROLLBACK)
    }

    pub fn with_scrollback(scrollback: usize) -> Self {
        Self::with_options(scrollback, true)
    }

    pub fn with_options(scrollback: usize, graphics: bool) -> Self {
        Self {
            surfaces: Mutex::new(HashMap::new()),
            transport: Mutex::new(Transport::default()),
            scrollback,
            graphics,
            default_shell: Mutex::new(Vec::new()),
            waker: Arc::new(Mutex::new(None)),
        }
    }

    /// Choose the program a new terminal runs. An empty list keeps the
    /// platform default. Takes effect for panes spawned after the call.
    pub fn set_default_shell(&self, argv: Vec<String>) {
        *lock(&self.default_shell) = argv;
    }

    /// Install the repaint hook. Surfaces spawned before this call still pick
    /// it up, because the threads share the slot rather than a copy of it.
    pub fn set_waker(&self, waker: Waker) {
        *lock(&self.waker) = Some(waker);
    }

    /// Inject connection details into every child so CLI and hooks can
    /// authenticate. Called once after the server picks its token.
    pub fn set_transport(&self, _token: Option<String>, socket_path: Option<String>) {
        let mut transport = lock(&self.transport);
        transport.socket_path = socket_path;
    }

    /// Number of live surfaces. Useful for diagnostics and tests.
    pub fn len(&self) -> usize {
        lock(&self.surfaces).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn surface(&self, surface_id: &str) -> Result<Arc<Surface>, CoreError> {
        lock(&self.surfaces)
            .get(surface_id)
            .cloned()
            .ok_or_else(|| CoreError::not_found(format!("no surface {surface_id}")))
    }

    /// Snapshot the visible screen for a renderer. The headless protocol only
    /// needs text, but a GUI wants per-cell colors and the cursor.
    pub fn screen_view(&self, surface_id: &str) -> Result<ScreenView, CoreError> {
        let surface = self.surface(surface_id)?;
        let parser = lock(&surface.screen);
        let screen = parser.screen();
        let (rows, cols) = screen.size();
        let rows = rows.get();
        let cols = cols.get();
        let cursor = screen.cursor_position();
        let mut cells = Vec::with_capacity(usize::from(rows) * usize::from(cols));
        for row in 0..rows {
            for col in 0..cols {
                let cell = screen.cell(row, col);
                cells.push(match cell {
                    Some(cell) => CellView {
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
                    },
                    None => CellView {
                        contents: " ".into(),
                        ..CellView::default()
                    },
                });
            }
        }
        let images = lock(&surface.images).clone();
        let placeholders = lock(&surface.kitty).values().cloned().collect();
        Ok(ScreenView {
            cols,
            rows,
            cursor,
            cursor_hidden: screen.hide_cursor(),
            scrollback_len: screen.scrollback_len(),
            scrollback: screen.scrollback(),
            bracketed_paste: screen.bracketed_paste(),
            mouse_mode: match screen.mouse_protocol_mode() {
                vt100::MouseProtocolMode::None => MouseMode::Off,
                vt100::MouseProtocolMode::Press => MouseMode::Click,
                vt100::MouseProtocolMode::PressRelease => MouseMode::PressRelease,
                vt100::MouseProtocolMode::ButtonMotion => MouseMode::Drag,
                vt100::MouseProtocolMode::AnyMotion => MouseMode::Motion,
            },
            mouse_encoding: match screen.mouse_protocol_encoding() {
                vt100::MouseProtocolEncoding::Default => MouseEncoding::X10,
                vt100::MouseProtocolEncoding::Utf8 => MouseEncoding::Utf8,
                vt100::MouseProtocolEncoding::Sgr => MouseEncoding::Sgr,
            },
            cells,
            images,
            placeholders,
        })
    }

    /// Terminal events the grid does not carry: title, cwd, bell count, OSC
    /// progress, clipboard writes and notification requests.
    pub fn screen_meta(&self, surface_id: &str) -> Result<ScreenMeta, CoreError> {
        let surface = self.surface(surface_id)?;
        Ok(lock(&surface.meta).clone())
    }

    /// Clear the pending clipboard and notification requests after handling.
    pub fn take_screen_meta_requests(&self, surface_id: &str) -> Result<ScreenMeta, CoreError> {
        let surface = self.surface(surface_id)?;
        let mut meta = lock(&surface.meta);
        let taken = ScreenMeta {
            title: meta.title.clone(),
            cwd: meta.cwd.clone(),
            bells: meta.bells,
            progress: meta.progress,
            clipboard: meta.clipboard.take(),
            notification: meta.notification.take(),
            cell_pixels: meta.cell_pixels,
            // Replies belong to the reader, which writes them to the PTY; the
            // window must not take them or the program waits forever.
            responses: Vec::new(),
        };
        Ok(taken)
    }

    /// Scroll the view up by `rows` history lines. Returns the offset actually
    /// applied, which the caller can clamp and mirror.
    pub fn set_scrollback(&self, surface_id: &str, rows: usize) -> Result<usize, CoreError> {
        let surface = self.surface(surface_id)?;
        let mut parser = lock(&surface.screen);
        parser.screen_mut().set_scrollback(rows);
        Ok(parser.screen().scrollback())
    }

    /// Scroll back to the live screen.
    pub fn reset_scrollback(&self, surface_id: &str) -> Result<(), CoreError> {
        let surface = self.surface(surface_id)?;
        lock(&surface.screen).screen_mut().set_scrollback(0);
        Ok(())
    }

    /// Whether the child process is still running. A dead PTY keeps its last
    /// screen so the window can show the final output.
    pub fn is_alive(&self, surface_id: &str) -> bool {
        lock(&self.surfaces)
            .get(surface_id)
            .map(|surface| surface.alive.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    /// Plain text between two cells of the visible view, used by the clipboard.
    #[allow(clippy::too_many_arguments)]
    pub fn contents_between(
        &self,
        surface_id: &str,
        start_row: u16,
        start_col: u16,
        end_row: u16,
        end_col: u16,
    ) -> Result<String, CoreError> {
        let surface = self.surface(surface_id)?;
        let parser = lock(&surface.screen);
        Ok(parser
            .screen()
            .contents_between(start_row, start_col, end_row, end_col))
    }

    /// The whole scrollback plus the live screen, oldest line first. Reading
    /// it walks the view once per screenful and restores the caller's offset.
    pub fn scrollback_lines(&self, surface_id: &str) -> Result<Vec<String>, CoreError> {
        let surface = self.surface(surface_id)?;
        let mut parser = lock(&surface.screen);
        let (rows, _) = parser.screen().size();
        let rows = rows.get() as usize;
        let len = parser.screen().scrollback_len();
        let offset = parser.screen().scrollback();
        let mut lines = vec![String::new(); len + rows];
        let mut at = 0;
        loop {
            parser.screen_mut().set_scrollback(at);
            let text = parser.screen().contents();
            let start = len - at.min(len);
            for (index, line) in text.lines().enumerate().take(rows) {
                if start + index < lines.len() {
                    lines[start + index] = line.to_string();
                }
            }
            if at >= len {
                break;
            }
            at = (at + rows).min(len);
        }
        parser.screen_mut().set_scrollback(offset);
        while lines.last().is_some_and(|line| line.is_empty()) {
            lines.pop();
        }
        Ok(lines)
    }

    /// One visible row as text, for word selection.
    pub fn line_text(&self, surface_id: &str, row: u16) -> Result<String, CoreError> {
        let surface = self.surface(surface_id)?;
        let parser = lock(&surface.screen);
        let (_, cols) = parser.screen().size();
        Ok(parser
            .screen()
            .contents_between(row, 0, row, cols.get())
            .trim_end()
            .to_string())
    }
}

impl SurfaceOps for PtyHost {
    fn spawn(&self, req: SpawnRequest) -> Result<Spawned, CoreError> {
        // The core mints the id and passes it in the environment, like the
        // macOS shell expects.
        let surface_id = req
            .env
            .iter()
            .find(|(key, _)| key == "BERTH_SURFACE_ID")
            .map(|(_, value)| value.clone())
            .ok_or_else(|| CoreError::internal("spawn request is missing BERTH_SURFACE_ID"))?;

        if let Some(existing) = lock(&self.surfaces).get(&surface_id).cloned() {
            let (cols, rows) = existing
                .master
                .lock()
                .ok()
                .and_then(|master| master.get_size().ok())
                .map(|size| (size.cols, size.rows))
                .unwrap_or((80, 24));
            return Ok(Spawned {
                pid: existing.pid,
                tty: existing.tty.clone(),
                cols,
                rows,
            });
        }

        let rows = req.rows.max(1);
        let cols = req.cols.max(1);
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| CoreError::internal(format!("openpty: {error}")))?;
        let PtyPair { slave, master } = pair;

        let shell = lock(&self.default_shell).clone();
        let mut command = if !req.argv.is_empty() {
            let mut command = CommandBuilder::new(&req.argv[0]);
            command.args(&req.argv[1..]);
            command
        } else if !shell.is_empty() {
            let mut command = CommandBuilder::new(&shell[0]);
            command.args(&shell[1..]);
            command
        } else {
            CommandBuilder::new_default_prog()
        };
        if let Some(cwd) = req.cwd.as_deref().filter(|cwd| !cwd.is_empty()) {
            command.cwd(cwd);
        }
        if command.get_env("TERM").is_none() {
            command.env("TERM", "xterm-256color");
        }
        for (key, value) in &req.env {
            command.env(key, value);
        }
        {
            let transport = lock(&self.transport);
            if let Some(path) = &transport.socket_path {
                command.env("BERTH_SOCKET_PATH", path);
            }
        }

        let child = slave
            .spawn_command(command)
            .map_err(|error| CoreError::internal(format!("spawn: {error}")))?;
        drop(slave);

        let pid = child.process_id().unwrap_or(0);
        #[cfg(unix)]
        let tty = master
            .tty_name()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        #[cfg(not(unix))]
        let tty = String::new();

        let reader = master
            .try_clone_reader()
            .map_err(|error| CoreError::internal(format!("pty reader: {error}")))?;
        let writer =
            Arc::new(Mutex::new(master.take_writer().map_err(|error| {
                CoreError::internal(format!("pty writer: {error}"))
            })?));
        let killer = child.clone_killer();

        let meta = Arc::new(Mutex::new(ScreenMeta::default()));
        let images = Arc::new(Mutex::new(Vec::new()));
        let kitty = Arc::new(Mutex::new(HashMap::new()));
        let events = ScreenEvents::new(
            Arc::clone(&meta),
            Arc::clone(&images),
            self.graphics,
            self.scrollback,
        );
        let screen = Arc::new(Mutex::new(Parser::new_with_callbacks(
            std::num::NonZeroU16::new(rows).expect("rows > 0"),
            std::num::NonZeroU16::new(cols).expect("cols > 0"),
            self.scrollback,
            events,
        )));
        let alive = Arc::new(AtomicBool::new(true));

        spawn_reader(
            reader,
            ReaderShare {
                screen: Arc::clone(&screen),
                alive: Arc::clone(&alive),
                writer: Arc::clone(&writer),
                meta: Arc::clone(&meta),
                images: Arc::clone(&images),
                kitty: Arc::clone(&kitty),
                graphics: self.graphics,
                scrollback: self.scrollback,
            },
            Arc::clone(&self.waker),
        );
        spawn_waiter(child, Arc::clone(&alive), Arc::clone(&self.waker));

        let surface = Arc::new(Surface {
            master: Mutex::new(master),
            writer,
            screen,
            meta,
            images,
            kitty,
            killer: Mutex::new(killer),
            pid,
            tty: tty.clone(),
            alive,
        });
        lock(&self.surfaces).insert(surface_id, surface);

        Ok(Spawned {
            pid,
            tty,
            cols,
            rows,
        })
    }

    fn send_text(&self, surface_id: &str, text: &str) -> Result<(), CoreError> {
        self.write_bytes(surface_id, text.as_bytes())
    }

    fn send_key(&self, surface_id: &str, key: &str, mods: &[String]) -> Result<(), CoreError> {
        self.write_bytes(surface_id, &keys::encode(key, mods))
    }

    fn read_text(
        &self,
        surface_id: &str,
        scope: ReadScope,
        max_bytes: usize,
    ) -> Result<Readback, CoreError> {
        let surface = self.surface(surface_id)?;
        let mut parser = lock(&surface.screen);
        let text = match scope {
            ReadScope::Screen => parser.screen().contents(),
            ReadScope::Scrollback => {
                let offset = parser.screen().scrollback();
                parser.screen_mut().set_scrollback(usize::MAX);
                let text = parser.screen().contents();
                parser.screen_mut().set_scrollback(offset);
                text
            }
        };
        Ok(truncate(text, max_bytes))
    }

    fn close(&self, surface_id: &str) -> Result<(), CoreError> {
        let Some(surface) = lock(&self.surfaces).remove(surface_id) else {
            return Ok(());
        };
        surface.alive.store(false, Ordering::Relaxed);
        if let Ok(mut killer) = surface.killer.lock() {
            let _ = killer.kill();
        }
        Ok(())
    }

    fn focus(&self, _surface_id: &str) -> Result<(), CoreError> {
        // Headless: focus is a UI concern.
        Ok(())
    }

    fn resize(&self, surface_id: &str, cols: u16, rows: u16) -> Result<(), CoreError> {
        self.resize_pixels(surface_id, cols, rows, None)
    }

    fn resize_pixels(
        &self,
        surface_id: &str,
        cols: u16,
        rows: u16,
        pixels: Option<(u32, u32)>,
    ) -> Result<(), CoreError> {
        if cols == 0 || rows == 0 {
            return Err(CoreError::new(
                ErrorCode::BadRequest,
                "terminal size must be at least 1x1",
            ));
        }
        let surface = self.surface(surface_id)?;
        let master = lock(&surface.master);
        let (pixel_width, pixel_height) = pixels.unwrap_or((0, 0));
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: pixel_width.min(u32::from(u16::MAX)) as u16,
                pixel_height: pixel_height.min(u32::from(u16::MAX)) as u16,
            })
            .map_err(|error| CoreError::internal(format!("resize: {error}")))?;
        lock(&surface.screen).set_size(
            std::num::NonZeroU16::new(rows).expect("rows > 0"),
            std::num::NonZeroU16::new(cols).expect("cols > 0"),
        );
        if let Some((pixel_width, pixel_height)) = pixels {
            let width = (pixel_width / u32::from(cols)).min(u32::from(u16::MAX)) as u16;
            let height = (pixel_height / u32::from(rows)).min(u32::from(u16::MAX)) as u16;
            lock(&surface.meta).cell_pixels = Some((width, height));
        }
        Ok(())
    }

    fn pid_info(&self, surface_id: &str) -> Result<PidInfo, CoreError> {
        let surface = self.surface(surface_id)?;
        if !surface.alive.load(Ordering::Relaxed) {
            return Ok(PidInfo::default());
        }

        #[cfg(unix)]
        let pid = {
            let master = lock(&surface.master);
            master
                .as_raw_fd()
                .and_then(|fd| {
                    // The pty master reports the foreground process group of
                    // the slave, which is what passive agent detection wants.
                    let pgrp = unsafe { libc::tcgetpgrp(fd) };
                    (pgrp > 0).then_some(pgrp as u32)
                })
                .unwrap_or(surface.pid)
        };
        #[cfg(not(unix))]
        let pid = surface.pid;

        if pid == 0 {
            return Ok(PidInfo::default());
        }
        Ok(PidInfo {
            pid: Some(pid),
            comm: process_comm(pid),
            argv: process_argv(pid),
        })
    }

    fn notify_os(&self, notification: &Notification) -> Result<(), CoreError> {
        // No UI to notify. A daemon logs; a GUI shell replaces this.
        let level = match notification.level {
            NotifLevel::Info => "info",
            NotifLevel::NeedsInput => "needs_input",
            NotifLevel::Error => "error",
        };
        eprintln!(
            "berth: notification [{level}] {}: {}",
            notification.title, notification.body
        );
        Ok(())
    }
}

impl PtyHost {
    /// Write raw bytes to a surface, for input the key encoder does not model
    /// (mouse reports).
    pub fn send_bytes(&self, surface_id: &str, bytes: &[u8]) -> Result<(), CoreError> {
        self.write_bytes(surface_id, bytes)
    }

    fn write_bytes(&self, surface_id: &str, bytes: &[u8]) -> Result<(), CoreError> {
        let surface = self.surface(surface_id)?;
        let mut writer = lock(&surface.writer);
        writer
            .write_all(bytes)
            .and_then(|_| writer.flush())
            .map_err(|error| CoreError::internal(format!("write: {error}")))
    }
}

/// Signal the shell that a surface changed. Called from the reader, so it runs
/// once per read and must stay cheap; the shell coalesces the burst.
fn notify(waker: &SharedWaker) {
    let waker = lock(waker).clone();
    if let Some(waker) = waker {
        waker();
    }
}

/// The per-surface state a reader touches: the grid, the PTY writer, the
/// screen metadata, the kitty image store and the liveness flag.
struct ReaderShare {
    screen: Arc<Mutex<Parser>>,
    alive: Arc<AtomicBool>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    meta: Arc<Mutex<ScreenMeta>>,
    images: Arc<Mutex<Vec<ImageView>>>,
    kitty: Arc<Mutex<HashMap<u32, KittyImage>>>,
    graphics: bool,
    scrollback: usize,
}

fn spawn_reader(mut reader: Box<dyn Read + Send>, share: ReaderShare, waker: SharedWaker) {
    let _ = thread::Builder::new()
        .name("berth-pty-reader".into())
        .spawn(move || {
            let mut buffer = [0u8; 16 * 1024];
            let mut scanner = GraphicsScanner::default();
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        scanner.feed(&buffer[..n], &share);
                        answer_queries(&share.meta, &share.writer);
                        notify(&waker);
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            share.alive.store(false, Ordering::Relaxed);
            notify(&waker);
        });
}

fn spawn_waiter(
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    alive: Arc<AtomicBool>,
    waker: SharedWaker,
) {
    let _ = thread::Builder::new()
        .name("berth-pty-wait".into())
        .spawn(move || {
            let _ = child.wait();
            alive.store(false, Ordering::Relaxed);
            notify(&waker);
        });
}

/// Send the replies the parser queued, so a program waiting for a cursor
/// position or a device-attributes answer can continue.
fn answer_queries(meta: &Arc<Mutex<ScreenMeta>>, writer: &Arc<Mutex<Box<dyn Write + Send>>>) {
    let replies = std::mem::take(&mut lock(meta).responses);
    if replies.is_empty() {
        return;
    }
    let mut writer = lock(writer);
    for reply in replies {
        let _ = writer.write_all(reply.as_bytes());
    }
    let _ = writer.flush();
}

fn truncate(text: String, max_bytes: usize) -> Readback {
    if max_bytes == 0 || text.len() <= max_bytes {
        return Readback {
            text,
            truncated: false,
        };
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Readback {
        text: text[..end].to_string(),
        truncated: true,
    }
}

#[cfg(target_os = "linux")]
fn process_comm(pid: u32) -> Option<String> {
    fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// macOS has no `/proc`. `libproc` answers without forking, which matters: the
/// passive detector asks once a second for every surface without an agent.
#[cfg(target_os = "macos")]
fn process_comm(pid: u32) -> Option<String> {
    let mut buffer = [0u8; 256];
    // SAFETY: the buffer is valid for its stated length and pid is an int.
    let length = unsafe {
        libc::proc_name(
            pid as libc::c_int,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
        )
    };
    if length <= 0 {
        return None;
    }
    let length = (length as usize).min(buffer.len());
    let text = String::from_utf8_lossy(&buffer[..length])
        .trim()
        .to_string();
    Some(text).filter(|text| !text.is_empty())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_comm(_pid: u32) -> Option<String> {
    None
}

#[cfg(target_os = "linux")]
fn process_argv(pid: u32) -> Vec<String> {
    fs::read(format!("/proc/{pid}/cmdline"))
        .map(|bytes| {
            bytes
                .split(|byte| *byte == 0)
                .filter(|part| !part.is_empty())
                .map(|part| String::from_utf8_lossy(part).into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// The kernel keeps the original argv after the executable path. One
/// `sysctl(KERN_PROCARGS2)` is much cheaper than forking `ps` per surface.
#[cfg(target_os = "macos")]
fn process_argv(pid: u32) -> Vec<String> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    let mut size: libc::size_t = 0;
    // SAFETY: `sysctl` writes only the size into `size` for a null buffer, and
    // the second call fills a buffer of exactly that size.
    unsafe {
        if libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        ) != 0
            || size == 0
            || size > 1 << 20
        {
            return Vec::new();
        }
    }
    let mut buffer = vec![0u8; size];
    // SAFETY: the buffer is valid for `size` bytes and `size` is updated in
    // place.
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as u32,
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 || size < 4 {
        return Vec::new();
    }
    buffer.truncate(size);
    let argc = i32::from_ne_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]).max(0) as usize;
    let mut index = 4;
    while index < buffer.len() && buffer[index] != 0 {
        index += 1;
    }
    while index < buffer.len() && buffer[index] == 0 {
        index += 1;
    }
    let mut args = Vec::new();
    while args.len() < argc && index < buffer.len() {
        let start = index;
        while index < buffer.len() && buffer[index] != 0 {
            index += 1;
        }
        if start == index {
            break;
        }
        args.push(String::from_utf8_lossy(&buffer[start..index]).into_owned());
        index += 1;
    }
    args
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_argv(_pid: u32) -> Vec<String> {
    Vec::new()
}

// ---- session persistence ------------------------------------------------

/// The snapshot path the headless shell owns.
pub fn session_path() -> PathBuf {
    berth_core::paths::snapshot_path()
}

/// Journal beside the session snapshot, the same file the macOS shell keeps.
pub fn journal_path() -> PathBuf {
    berth_core::paths::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("journal.ndjson")
}

/// Write the current session snapshot atomically, 0600 on Unix, and append it
/// to the crash journal.
pub fn save_session(state: &Arc<Mutex<AppState>>) -> io::Result<PathBuf> {
    let snapshot = lock(state).snapshot();
    let path = session_path();
    let bytes = serde_json::to_vec_pretty(&snapshot)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    write_atomic(&path, &bytes)?;
    if let Some(dir) = path.parent() {
        let _ = append_journal_in(dir, &snapshot);
    }
    Ok(path)
}

/// Journals kept before the oldest is dropped.
const JOURNAL_LINES: usize = 5;

/// Append one snapshot to `journal.ndjson` in `dir`, keeping the last few so a
/// crash can be recovered without unbounded growth.
pub fn append_journal_in(dir: &Path, snapshot: &Snapshot) -> io::Result<()> {
    let line = berth_core::store::JournalLine {
        ts_ms: berth_core::time::now_ms(),
        op: "snapshot".to_string(),
        data: serde_json::to_value(snapshot)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
    };
    let text = serde_json::to_string(&line)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let path = dir.join("journal.ndjson");
    {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        file.write_all(text.as_bytes())?;
        file.write_all(b"\n")?;
    }
    trim_journal(&path)
}

fn trim_journal(path: &Path) -> io::Result<()> {
    let text = fs::read_to_string(path)?;
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.len() <= JOURNAL_LINES {
        return Ok(());
    }
    let tail = lines[lines.len() - JOURNAL_LINES..].join("\n");
    write_atomic(path, format!("{tail}\n").as_bytes())
}

/// The newest valid snapshot in the journal, if any.
pub fn newest_journal_snapshot_in(dir: &Path) -> io::Result<Option<Snapshot>> {
    let path = dir.join("journal.ndjson");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut newest: Option<Snapshot> = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(entry) = serde_json::from_str::<berth_core::store::JournalLine>(line) else {
            continue;
        };
        if entry.op != "snapshot" {
            continue;
        }
        let Ok(snapshot) = serde_json::from_value::<Snapshot>(entry.data) else {
            continue;
        };
        if newest
            .as_ref()
            .is_none_or(|current| snapshot.saved_ms > current.saved_ms)
        {
            newest = Some(snapshot);
        }
    }
    Ok(newest)
}

/// Delete the journal after a clean quit, like the macOS shell.
pub fn clear_journal() -> io::Result<()> {
    match fs::remove_file(journal_path()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// How a shell resumes agents at boot.
#[derive(Clone, Debug, Default)]
pub struct RestorePolicy {
    /// Replay the saved launch command's extra flags.
    pub keep_launch_flags: bool,
    /// Start a restored Claude session with permissions bypassed.
    pub claude_bypass_permissions: bool,
    /// Extra arguments per provider, applied on every resume.
    pub resume_args: Vec<(String, Vec<String>)>,
}

/// Restore the newest session: the snapshot, or a newer journal entry left by a
/// crash. A corrupt snapshot is reported, not deleted.
pub fn restore_session(
    state: &Arc<Mutex<AppState>>,
    ops: &dyn SurfaceOps,
) -> Result<usize, CoreError> {
    restore_session_with(
        state,
        ops,
        RestorePolicy {
            keep_launch_flags: true,
            ..RestorePolicy::default()
        },
    )
}

/// Restore with the shell's resume policy. `keep_launch_flags` replays saved
/// flags, and `claude_bypass_permissions` appends the bypass mode to a Claude
/// resume without persisting it into the saved payload.
pub fn restore_session_with(
    state: &Arc<Mutex<AppState>>,
    ops: &dyn SurfaceOps,
    policy: RestorePolicy,
) -> Result<usize, CoreError> {
    let path = session_path();
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let from_file = match fs::read(&path) {
        Ok(bytes) => Some(serde_json::from_slice::<Snapshot>(&bytes).map_err(|error| {
            CoreError::new(
                ErrorCode::BadRequest,
                format!("parse {}: {error}", path.display()),
            )
        })?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(CoreError::internal(format!(
                "read {}: {error}",
                path.display()
            )));
        }
    };
    let from_journal = newest_journal_snapshot_in(directory).unwrap_or(None);
    let snapshot = match (from_file, from_journal) {
        (Some(file), Some(journal)) => {
            if journal.saved_ms > file.saved_ms {
                journal
            } else {
                file
            }
        }
        (Some(file), None) => file,
        (None, Some(journal)) => journal,
        (None, None) => return Ok(0),
    };
    let extra = |provider: &str| -> Vec<String> {
        let mut args = policy
            .resume_args
            .iter()
            .find(|(name, _)| name == provider)
            .map(|(_, args)| args.clone())
            .unwrap_or_default();
        if policy.claude_bypass_permissions && provider == "claude-code" {
            args.push("--permission-mode".into());
            args.push("bypassPermissions".into());
        }
        args
    };
    let mut state = lock(state);
    berth_core::store::restore_with(&mut state, snapshot, ops, policy.keep_launch_flags, &extra)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }

    // Windows' rename refuses to replace an existing file.
    #[cfg(windows)]
    let _ = fs::remove_file(path);
    fs::rename(&temp, path)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Map a vt100 color to the GUI's 8-bit RGB palette.
fn ansi_rgb(color: vt100::Color) -> Option<[u8; 3]> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Rgb(r, g, b) => Some([r, g, b]),
        vt100::Color::Idx(index) => Some(match index {
            0 => [0x1c, 0x1c, 0x22],
            1 => [0xd7, 0x5f, 0x5f],
            2 => [0x8f, 0xb3, 0x5f],
            3 => [0xd7, 0xa5, 0x5f],
            4 => [0x6c, 0x99, 0xc9],
            5 => [0xb0, 0x7c, 0xc9],
            6 => [0x5f, 0xb3, 0xb3],
            7 => [0xc9, 0xc9, 0xd0],
            8 => [0x5c, 0x5c, 0x66],
            9 => [0xff, 0x7a, 0x7a],
            10 => [0xa9, 0xd4, 0x6a],
            11 => [0xff, 0xd7, 0x66],
            12 => [0x82, 0xbb, 0xff],
            13 => [0xd7, 0x9a, 0xff],
            14 => [0x7a, 0xdb, 0xdb],
            15 => [0xff, 0xff, 0xff],
            n if n < 232 => {
                let n = n - 16;
                let r = n / 36;
                let g = (n % 36) / 6;
                let b = n % 6;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                [level(r), level(g), level(b)]
            }
            n => {
                let level = 8 + (n - 232) * 10;
                [level, level, level]
            }
        }),
    }
}

#[cfg(test)]
mod meta_tests {
    use super::*;

    fn parser() -> (Arc<Mutex<ScreenMeta>>, Parser) {
        let meta = Arc::new(Mutex::new(ScreenMeta::default()));
        let images = Arc::new(Mutex::new(Vec::new()));
        let events = ScreenEvents::new(Arc::clone(&meta), Arc::clone(&images), true, 10_000);
        let parser = Parser::new_with_callbacks(
            std::num::NonZeroU16::new(24).unwrap(),
            std::num::NonZeroU16::new(80).unwrap(),
            100,
            events,
        );
        (meta, parser)
    }

    fn parser_with_images() -> (Arc<Mutex<Vec<ImageView>>>, Parser) {
        let meta = Arc::new(Mutex::new(ScreenMeta::default()));
        let images = Arc::new(Mutex::new(Vec::new()));
        let events = ScreenEvents::new(
            Arc::clone(&meta),
            Arc::clone(&images),
            true,
            DEFAULT_SCROLLBACK,
        );
        let parser = Parser::new_with_callbacks(
            std::num::NonZeroU16::new(24).unwrap(),
            std::num::NonZeroU16::new(80).unwrap(),
            DEFAULT_SCROLLBACK,
            events,
        );
        (images, parser)
    }

    struct ScannerFixture {
        meta: Arc<Mutex<ScreenMeta>>,
        images: Arc<Mutex<Vec<ImageView>>>,
        screen: Arc<Mutex<Parser>>,
        kitty: Arc<Mutex<HashMap<u32, KittyImage>>>,
    }

    impl ScannerFixture {
        fn new() -> Self {
            let meta = Arc::new(Mutex::new(ScreenMeta::default()));
            let images = Arc::new(Mutex::new(Vec::new()));
            let events = ScreenEvents::new(
                Arc::clone(&meta),
                Arc::clone(&images),
                true,
                DEFAULT_SCROLLBACK,
            );
            let screen = Arc::new(Mutex::new(Parser::new_with_callbacks(
                std::num::NonZeroU16::new(24).unwrap(),
                std::num::NonZeroU16::new(80).unwrap(),
                DEFAULT_SCROLLBACK,
                events,
            )));
            Self {
                meta,
                images,
                screen,
                kitty: Arc::new(Mutex::new(HashMap::new())),
            }
        }

        fn feed(&self, scanner: &mut GraphicsScanner, bytes: &[u8]) {
            scanner.feed(bytes, &self.share());
        }

        fn share(&self) -> ReaderShare {
            ReaderShare {
                screen: Arc::clone(&self.screen),
                alive: Arc::new(AtomicBool::new(true)),
                writer: Arc::new(Mutex::new(Box::new(io::sink()))),
                meta: Arc::clone(&self.meta),
                images: Arc::clone(&self.images),
                kitty: Arc::clone(&self.kitty),
                graphics: true,
                scrollback: DEFAULT_SCROLLBACK,
            }
        }
    }

    #[test]
    fn given_a_kitty_apc_split_across_reads_then_the_image_is_stored() {
        let fixture = ScannerFixture::new();
        let data = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let mut scanner = GraphicsScanner::default();
        fixture.feed(&mut scanner, b"\x1b_Gq=2,a=T,U=1,f=100,i=7;");
        fixture.feed(&mut scanner, format!("{data}\x1b\\").as_bytes());
        let store = fixture.kitty.lock().unwrap();
        let image = store.get(&7).expect("image 7");
        assert_eq!((image.width, image.height), (1, 1));
        assert_eq!(image.pixels.len(), 4);
    }

    #[test]
    fn given_text_before_a_split_escape_then_the_text_is_not_held_back() {
        let fixture = ScannerFixture::new();
        let mut scanner = GraphicsScanner::default();
        fixture.feed(&mut scanner, b"before\x1b");
        let text = lock(&fixture.screen).screen().contents();
        assert!(text.starts_with("before"), "held back: {text}");
        fixture.feed(&mut scanner, b"[31mafter");
        let text = lock(&fixture.screen).screen().contents();
        assert!(text.starts_with("beforeafter"), "{text}");
    }

    #[test]
    fn given_a_kitty_capability_query_then_it_is_answered_ok() {
        let fixture = ScannerFixture::new();
        let mut scanner = GraphicsScanner::default();
        fixture.feed(
            &mut scanner,
            b"\x1b_Gi=278941603,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\",
        );
        assert_eq!(
            lock(&fixture.meta).responses,
            vec!["\x1b_Gi=278941603;OK\x1b\\".to_string()]
        );
    }

    #[test]
    fn given_a_shared_memory_query_then_it_is_reported_unsupported() {
        let fixture = ScannerFixture::new();
        let mut scanner = GraphicsScanner::default();
        fixture.feed(
            &mut scanner,
            b"\x1b_Gi=916472805,s=1,v=1,a=q,t=s,f=24,S=3;L3RtcC94\x1b\\",
        );
        assert_eq!(
            lock(&fixture.meta).responses,
            vec!["\x1b_Gi=916472805;ENOTSUPPORTED\x1b\\".to_string()]
        );
    }

    #[test]
    fn given_a_sixel_dcs_then_the_image_is_stored_at_the_cursor() {
        let fixture = ScannerFixture::new();
        let mut scanner = GraphicsScanner::default();
        fixture.feed(&mut scanner, b"\x1bP0;0;0q\"1;1;2;6#0;2;100;0;0#0~~\x1b\\");
        let images = fixture.images.lock().unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!((images[0].width, images[0].height), (2, 6));
        assert_eq!(&images[0].pixels[0..4], &[255, 0, 0, 255]);
        assert_eq!((images[0].anchor_line, images[0].col), (0, 0));
    }

    #[test]
    fn given_a_sixel_dcs_split_across_reads_then_the_image_is_stored() {
        let fixture = ScannerFixture::new();
        let mut scanner = GraphicsScanner::default();
        fixture.feed(&mut scanner, b"\x1bPq#0;2;0;0;100#0");
        assert!(fixture.images.lock().unwrap().is_empty());
        fixture.feed(&mut scanner, b"~~\x1b");
        fixture.feed(&mut scanner, b"\\");
        let images = fixture.images.lock().unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!((images[0].width, images[0].height), (2, 6));
    }

    #[test]
    fn given_a_non_sixel_dcs_then_no_image_is_stored() {
        let fixture = ScannerFixture::new();
        let mut scanner = GraphicsScanner::default();
        fixture.feed(&mut scanner, b"\x1bP$q q\x1b\\");
        assert!(fixture.images.lock().unwrap().is_empty());
    }

    #[test]
    fn given_a_known_cell_size_when_xtwinops_asks_then_it_is_reported() {
        let meta = Arc::new(Mutex::new(ScreenMeta::default()));
        let images = Arc::new(Mutex::new(Vec::new()));
        let events = ScreenEvents::new(Arc::clone(&meta), images, true, DEFAULT_SCROLLBACK);
        let mut parser = Parser::new_with_callbacks(
            std::num::NonZeroU16::new(24).unwrap(),
            std::num::NonZeroU16::new(80).unwrap(),
            DEFAULT_SCROLLBACK,
            events,
        );
        lock(&meta).cell_pixels = Some((10, 20));
        parser.process(b"\x1b[16t");
        assert_eq!(lock(&meta).responses, vec!["\x1b[6;20;10t".to_string()]);
    }

    #[test]
    fn given_an_iterm2_inline_png_then_the_image_is_stored_at_the_cursor() {
        // 1x1 PNG, base64.
        let data = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let (images, mut parser) = parser_with_images();
        parser.process(format!("\x1b]1337;File=inline=1;width=2:{data}\x07").as_bytes());
        let list = images.lock().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].width, list[0].height), (1, 1));
        assert_eq!(list[0].target_width, Dimension::Cells(2));
        assert_eq!((list[0].anchor_line, list[0].col), (0, 0));
        assert_eq!(list[0].pixels.len(), 4);
    }

    #[test]
    fn given_malformed_or_non_inline_iterm2_then_nothing_is_stored() {
        let (images, mut parser) = parser_with_images();
        parser.process(b"\x1b]1337;File=inline=0:AAAA\x07");
        parser.process(b"\x1b]1337;File=inline=1:not-base64!\x07");
        assert!(images.lock().unwrap().is_empty());
    }

    #[test]
    fn given_an_osc_title_when_parsed_then_meta_records_it() {
        let (meta, mut parser) = parser();
        parser.process(b"\x1b]2;build: cargo test\x07");
        assert_eq!(lock(&meta).title.as_deref(), Some("build: cargo test"));
    }

    #[test]
    fn given_an_osc_7_url_when_parsed_then_the_cwd_is_decoded() {
        let (meta, mut parser) = parser();
        parser.process(b"\x1b]7;file://host/home/me/proj%20x\x07");
        assert_eq!(lock(&meta).cwd.as_deref(), Some("/home/me/proj x"));
    }

    #[test]
    fn given_osc_progress_when_parsed_then_it_is_a_fraction_and_can_clear() {
        let (meta, mut parser) = parser();
        parser.process(b"\x1b]9;4;1;42\x07");
        assert_eq!(lock(&meta).progress, Some(Some(0.42)));
        parser.process(b"\x1b]9;4;0\x07");
        assert_eq!(lock(&meta).progress, Some(None));
    }

    #[test]
    fn given_a_bell_when_parsed_then_the_count_rises() {
        let (meta, mut parser) = parser();
        parser.process(b"\x07\x07");
        assert_eq!(lock(&meta).bells, 2);
    }

    #[test]
    fn given_an_osc_52_copy_when_parsed_then_the_text_is_decoded() {
        let (meta, mut parser) = parser();
        parser.process(b"\x1b]52;c;aGVsbG8=\x07");
        assert_eq!(lock(&meta).clipboard.as_deref(), Some("hello"));
    }

    #[test]
    fn given_a_desktop_notification_when_parsed_then_it_is_recorded() {
        let (meta, mut parser) = parser();
        parser.process(b"\x1b]9;build finished\x07");
        assert_eq!(lock(&meta).notification.as_deref(), Some("build finished"));
    }

    #[test]
    fn given_a_wide_character_when_rendered_then_it_is_marked_and_has_a_tail() {
        let (_, mut parser) = parser();
        parser.process("日本語".as_bytes());
        let screen = parser.screen();
        assert!(screen.cell(0, 0).unwrap().is_wide());
        assert!(screen.cell(0, 1).unwrap().is_wide_continuation());
    }

    #[test]
    fn given_a_cursor_position_query_when_parsed_then_a_report_is_queued() {
        let (meta, mut parser) = parser();
        parser.process(b"\x1b[6n");
        let responses = &lock(&meta).responses;
        assert_eq!(responses.len(), 1);
        assert!(responses[0].starts_with("\x1b["), "{}", responses[0]);
        assert!(responses[0].ends_with('R'), "{}", responses[0]);
        assert_eq!(responses[0], "\x1b[1;1R");
    }

    #[test]
    fn given_a_device_attributes_query_when_parsed_then_a_vt100_reply_is_queued() {
        let (meta, mut parser) = parser();
        parser.process(b"\x1b[c");
        assert_eq!(lock(&meta).responses, vec!["\x1b[?1;2c".to_string()]);
        parser.process(b"\x1b[>c");
        assert_eq!(
            lock(&meta).responses.last().map(String::as_str),
            Some("\x1b[>0;0;0c")
        );
    }

    #[test]
    fn given_a_percent_encoded_url_when_decoded_then_utf8_survives() {
        assert_eq!(percent_decode("/caf%C3%A9"), "/café");
    }
}

#[cfg(test)]
mod journal_tests {
    use super::*;
    use berth_core::NullOps;

    fn snapshot(state: &AppState) -> Snapshot {
        state.snapshot()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "berth-journal-{tag}-{}-{}",
            std::process::id(),
            berth_core::time::now_ms()
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn given_a_saved_snapshot_when_appended_then_the_journal_returns_it() {
        let dir = temp_dir("append");
        let state = Arc::new(Mutex::new(AppState::default()));
        lock(&state)
            .create_workspace(Some("one"), None, None, &NullOps)
            .unwrap();
        let snapshot = snapshot(&lock(&state));
        append_journal_in(&dir, &snapshot).unwrap();
        let newest = newest_journal_snapshot_in(&dir)
            .unwrap()
            .expect("journal entry");
        assert_eq!(newest.workspaces.len(), 1);
        assert_eq!(newest.saved_ms, snapshot.saved_ms);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn given_more_than_five_entries_when_appended_then_only_the_last_five_survive() {
        let dir = temp_dir("trim");
        let state = Arc::new(Mutex::new(AppState::default()));
        lock(&state)
            .create_workspace(Some("one"), None, None, &NullOps)
            .unwrap();
        let snapshot = snapshot(&lock(&state));
        for _ in 0..8 {
            append_journal_in(&dir, &snapshot).unwrap();
        }
        let text = fs::read_to_string(dir.join("journal.ndjson")).unwrap();
        assert_eq!(text.lines().count(), JOURNAL_LINES);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn given_a_corrupt_line_when_read_then_it_is_skipped() {
        let dir = temp_dir("corrupt");
        let state = Arc::new(Mutex::new(AppState::default()));
        lock(&state)
            .create_workspace(Some("one"), None, None, &NullOps)
            .unwrap();
        append_journal_in(&dir, &snapshot(&lock(&state))).unwrap();
        let path = dir.join("journal.ndjson");
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("not json\n");
        fs::write(&path, text).unwrap();
        assert!(newest_journal_snapshot_in(&dir).unwrap().is_some());
        let _ = fs::remove_dir_all(&dir);
    }
}

/// Splits a PTY read into text (fed to vt100), kitty APC sequences and sixel
/// DCS sequences (fed to the image store). The state survives chunk boundaries.
#[derive(Default)]
struct GraphicsScanner {
    normal: Vec<u8>,
    capture: Option<Capture>,
    capture_esc: bool,
    capture_overflow: bool,
    esc: bool,
    pending: KittyPending,
}

enum Capture {
    Apc(Vec<u8>),
    Dcs(Vec<u8>),
}

/// Images are bounded by the same byte budget as the store; a sequence past
/// this is dropped rather than buffered without limit.
const MAX_SEQUENCE_BYTES: usize = 8 * 1024 * 1024;

/// A kitty transmission split across APC chunks: the control data of the first
/// chunk and the base64 payload accumulated so far.
#[derive(Default)]
struct KittyPending {
    control: Option<String>,
    data: Vec<u8>,
}

impl GraphicsScanner {
    fn feed(&mut self, chunk: &[u8], share: &ReaderShare) {
        for &byte in chunk {
            if self.capture.is_some() {
                let mut complete = false;
                {
                    let buffer = match self.capture.as_mut().expect("capture") {
                        Capture::Apc(buffer) => buffer,
                        Capture::Dcs(buffer) => buffer,
                    };
                    if self.capture_esc {
                        self.capture_esc = false;
                        if byte == b'\\' {
                            complete = true;
                        } else {
                            buffer.push(0x1b);
                            buffer.push(byte);
                        }
                    } else if byte == 0x1b {
                        self.capture_esc = true;
                    } else {
                        buffer.push(byte);
                    }
                    if buffer.len() > MAX_SEQUENCE_BYTES {
                        self.capture_overflow = true;
                        buffer.clear();
                    }
                }
                if complete {
                    let capture = self.capture.take().expect("capture");
                    let overflow = std::mem::take(&mut self.capture_overflow);
                    match capture {
                        Capture::Apc(payload) if !overflow => handle_kitty(
                            &payload,
                            &share.meta,
                            &share.kitty,
                            share.graphics,
                            &mut self.pending,
                        ),
                        Capture::Dcs(payload) if !overflow => handle_dcs(
                            &payload,
                            &share.screen,
                            &share.images,
                            share.graphics,
                            share.scrollback,
                        ),
                        _ => {}
                    }
                }
                continue;
            }
            if self.esc {
                self.esc = false;
                match byte {
                    b'_' => {
                        flush_normal(&share.screen, &mut self.normal);
                        self.capture = Some(Capture::Apc(Vec::new()));
                    }
                    b'P' => {
                        flush_normal(&share.screen, &mut self.normal);
                        self.capture = Some(Capture::Dcs(Vec::new()));
                    }
                    _ => {
                        self.normal.push(0x1b);
                        self.normal.push(byte);
                    }
                }
                continue;
            }
            if byte == 0x1b {
                self.esc = true;
            } else {
                self.normal.push(byte);
            }
        }
        if self.capture.is_none() {
            flush_normal(&share.screen, &mut self.normal);
        }
    }
}

fn flush_normal(screen: &Arc<Mutex<Parser>>, normal: &mut Vec<u8>) {
    if !normal.is_empty() {
        lock(screen).process(normal);
        normal.clear();
    }
}

/// A DCS is sixel when the part before `q` is only its numeric parameters.
fn sixel_payload(payload: &[u8]) -> Option<&[u8]> {
    let position = payload.iter().position(|byte| *byte == b'q')?;
    let params = &payload[..position];
    if params
        .iter()
        .all(|byte| byte.is_ascii_digit() || *byte == b';')
    {
        Some(&payload[position + 1..])
    } else {
        None
    }
}

fn handle_dcs(
    payload: &[u8],
    screen: &Arc<Mutex<Parser>>,
    images: &Arc<Mutex<Vec<ImageView>>>,
    graphics: bool,
    scrollback: usize,
) {
    let Some(data) = sixel_payload(payload) else {
        // Not sixel: hand the sequence back to vt100, which ignored it before.
        let mut sequence = Vec::with_capacity(payload.len() + 4);
        sequence.extend_from_slice(b"\x1bP");
        sequence.extend_from_slice(payload);
        sequence.extend_from_slice(b"\x1b\\");
        lock(screen).process(&sequence);
        return;
    };
    if !graphics {
        return;
    }
    let Some(image) = sixel::decode(data) else {
        return;
    };
    let (row, col, scrollback_len) = {
        let parser = lock(screen);
        let screen = parser.screen();
        let (row, col) = screen.cursor_position();
        (row, col, screen.scrollback_len())
    };
    push_image(
        images,
        scrollback,
        scrollback_len + usize::from(row),
        usize::from(col),
        image.width,
        image.height,
        Dimension::Auto,
        Dimension::Auto,
        image.pixels,
    );
}

fn handle_kitty(
    payload: &[u8],
    meta: &Arc<Mutex<ScreenMeta>>,
    kitty: &Arc<Mutex<HashMap<u32, KittyImage>>>,
    graphics: bool,
    pending: &mut KittyPending,
) {
    let Some(rest) = payload.strip_prefix(b"G") else {
        return;
    };
    let text = String::from_utf8_lossy(rest);
    let (control, data) = text.split_once(';').unwrap_or((&text, ""));
    let mut more = false;
    for pair in control.split(',').filter(|pair| !pair.is_empty()) {
        if let Some(value) = pair.strip_prefix("m=") {
            more = value != "0";
        }
    }
    if more {
        if pending.control.is_none() {
            pending.control = Some(control.to_string());
        }
        pending.data.extend_from_slice(data.as_bytes());
        return;
    }
    let control = pending
        .control
        .take()
        .unwrap_or_else(|| control.to_string());
    let mut bytes = data.as_bytes().to_vec();
    if !pending.data.is_empty() {
        bytes.splice(0..0, std::mem::take(&mut pending.data));
    }
    let mut map = std::collections::HashMap::new();
    for pair in control.split(',').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(key, value);
    }
    let action = map.get("a").copied().unwrap_or("t");
    let id: u32 = map
        .get("i")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    if action == "q" {
        // Capability probe. Shared memory is never supported, and graphics may
        // be switched off; either way the program falls back to text.
        let is_supported = graphics && id != 0 && map.get("t").copied() != Some("s");
        let status = if is_supported { "OK" } else { "ENOTSUPPORTED" };
        lock(meta)
            .responses
            .push(format!("\x1b_Gi={id};{status}\x1b\\"));
        return;
    }
    let mut store = lock(kitty);
    if action == "d" {
        match map.get("d").copied() {
            Some("A") => store.clear(),
            _ => {
                if id != 0 {
                    store.remove(&id);
                }
            }
        }
        return;
    }
    if !graphics || id == 0 {
        return;
    }
    if action == "p" {
        return;
    }
    let Some(raw) = decode_base64_bytes(&bytes) else {
        return;
    };
    let format = map.get("f").copied().unwrap_or("32");
    let (width, height, pixels) = match format {
        "100" => match decode_png(&raw) {
            Some(value) => value,
            None => return,
        },
        "24" | "32" => {
            let width: u32 = map.get("s").and_then(|v| v.parse().ok()).unwrap_or(0);
            let height: u32 = map.get("v").and_then(|v| v.parse().ok()).unwrap_or(0);
            if width == 0 || height == 0 || width > MAX_IMAGE_AXIS || height > MAX_IMAGE_AXIS {
                return;
            }
            if format == "24" {
                if raw.len() != width as usize * height as usize * 3 {
                    return;
                }
                let pixels = raw
                    .chunks(3)
                    .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
                    .collect();
                (width, height, pixels)
            } else {
                if raw.len() != width as usize * height as usize * 4 {
                    return;
                }
                (width, height, raw)
            }
        }
        _ => return,
    };
    store.insert(
        id,
        KittyImage {
            id,
            width,
            height,
            pixels: Arc::new(pixels),
        },
    );
    while store.len() > MAX_IMAGES {
        if let Some(oldest) = store.keys().next().copied() {
            store.remove(&oldest);
        }
    }
}
