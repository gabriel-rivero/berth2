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
}

/// Callbacks handed to `vt100`, writing into the surface's [`ScreenMeta`].
#[derive(Clone, Default)]
pub struct ScreenEvents {
    meta: Arc<Mutex<ScreenMeta>>,
}

impl ScreenEvents {
    pub fn new(meta: Arc<Mutex<ScreenMeta>>) -> Self {
        Self { meta }
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

    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
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
    String::from_utf8(out).ok()
}

type Parser = vt100::Parser<ScreenEvents>;

/// A live PTY-backed terminal and its parsed screen.
struct Surface {
    /// Kept for resize and foreground-process queries.
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    screen: Arc<Mutex<Parser>>,
    meta: Arc<Mutex<ScreenMeta>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    pid: u32,
    tty: String,
    alive: Arc<AtomicBool>,
}

/// Terminal host for a headless shell. Cheap to put behind an `Arc`; all state
/// is internally synchronised because the server calls from many threads.
pub struct PtyHost {
    surfaces: Mutex<HashMap<String, Arc<Surface>>>,
    transport: Mutex<Transport>,
    scrollback: usize,
}

#[derive(Default)]
struct Transport {
    token: Option<String>,
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
        Self {
            surfaces: Mutex::new(HashMap::new()),
            transport: Mutex::new(Transport::default()),
            scrollback,
        }
    }

    /// Inject connection details into every child so CLI and hooks can
    /// authenticate. Called once after the server picks its token.
    pub fn set_transport(&self, token: Option<String>, socket_path: Option<String>) {
        let mut transport = lock(&self.transport);
        transport.token = token;
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

        let mut command = if req.argv.is_empty() {
            CommandBuilder::new_default_prog()
        } else {
            let mut command = CommandBuilder::new(&req.argv[0]);
            command.args(&req.argv[1..]);
            command
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
            if let Some(token) = &transport.token {
                command.env("BERTH_TOKEN", token);
            }
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
        let writer = master
            .take_writer()
            .map_err(|error| CoreError::internal(format!("pty writer: {error}")))?;
        let killer = child.clone_killer();

        let meta = Arc::new(Mutex::new(ScreenMeta::default()));
        let events = ScreenEvents::new(Arc::clone(&meta));
        let screen = Arc::new(Mutex::new(Parser::new_with_callbacks(
            std::num::NonZeroU16::new(rows).expect("rows > 0"),
            std::num::NonZeroU16::new(cols).expect("cols > 0"),
            self.scrollback,
            events,
        )));
        let alive = Arc::new(AtomicBool::new(true));

        spawn_reader(reader, Arc::clone(&screen), Arc::clone(&alive));
        spawn_waiter(child, Arc::clone(&alive));

        let surface = Arc::new(Surface {
            master: Mutex::new(master),
            writer: Mutex::new(writer),
            screen,
            meta,
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
        if cols == 0 || rows == 0 {
            return Err(CoreError::new(
                ErrorCode::BadRequest,
                "terminal size must be at least 1x1",
            ));
        }
        let surface = self.surface(surface_id)?;
        let master = lock(&surface.master);
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| CoreError::internal(format!("resize: {error}")))?;
        lock(&surface.screen).set_size(
            std::num::NonZeroU16::new(rows).expect("rows > 0"),
            std::num::NonZeroU16::new(cols).expect("cols > 0"),
        );
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

fn spawn_reader(
    mut reader: Box<dyn Read + Send>,
    screen: Arc<Mutex<Parser>>,
    alive: Arc<AtomicBool>,
) {
    let _ = thread::Builder::new()
        .name("berth-pty-reader".into())
        .spawn(move || {
            let mut buffer = [0u8; 16 * 1024];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        lock(&screen).process(&buffer[..n]);
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            alive.store(false, Ordering::Relaxed);
        });
}

fn spawn_waiter(mut child: Box<dyn portable_pty::Child + Send + Sync>, alive: Arc<AtomicBool>) {
    let _ = thread::Builder::new()
        .name("berth-pty-wait".into())
        .spawn(move || {
            let _ = child.wait();
            alive.store(false, Ordering::Relaxed);
        });
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

/// macOS has no `/proc`; `ps` is the portable answer and is what the AppKit
/// shell uses for passive detection too.
#[cfg(target_os = "macos")]
fn process_comm(pid: u32) -> Option<String> {
    let output = std::process::Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Some(text)
        .filter(|text| !text.is_empty())
        .map(|text| text.rsplit('/').next().unwrap_or(&text).to_string())
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

#[cfg(target_os = "macos")]
fn process_argv(pid: u32) -> Vec<String> {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-o", "args=", "-p", &pid.to_string()])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .map(str::to_string)
        .collect()
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

/// Write the current session snapshot atomically, 0600 on Unix.
pub fn save_session(state: &Arc<Mutex<AppState>>) -> io::Result<PathBuf> {
    let snapshot = lock(state).snapshot();
    let path = session_path();
    let bytes = serde_json::to_vec_pretty(&snapshot)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    write_atomic(&path, &bytes)?;
    Ok(path)
}

/// Restore the snapshot if one exists. A corrupt snapshot is reported, not
/// deleted: the next `session.save` will replace it.
pub fn restore_session(
    state: &Arc<Mutex<AppState>>,
    ops: &dyn SurfaceOps,
) -> Result<usize, CoreError> {
    let path = session_path();
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(CoreError::internal(format!(
                "read {}: {error}",
                path.display()
            )));
        }
    };
    let snapshot: Snapshot = serde_json::from_slice(&bytes).map_err(|error| {
        CoreError::new(
            ErrorCode::BadRequest,
            format!("parse {}: {error}", path.display()),
        )
    })?;
    let mut state = lock(state);
    berth_core::store::restore(&mut state, snapshot, ops)
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
        let events = ScreenEvents::new(Arc::clone(&meta));
        let parser = Parser::new_with_callbacks(
            std::num::NonZeroU16::new(24).unwrap(),
            std::num::NonZeroU16::new(80).unwrap(),
            100,
            events,
        );
        (meta, parser)
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
    fn given_a_percent_encoded_url_when_decoded_then_utf8_survives() {
        assert_eq!(percent_decode("/caf%C3%A9"), "/café");
    }
}
