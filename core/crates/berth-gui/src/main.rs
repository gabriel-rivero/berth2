//! Minimal Linux/Windows window for Berth.
//!
//! One window, one terminal, no GPU: `softbuffer` presents an 8×16 bitmap-font
//! grid and `winit` routes keys. It embeds the same `berth-server` and `PtyHost`
//! as the headless shell, so the CLI and hooks keep working.
//!
//! This is the first visible slice, not the final shell: no splits, tabs,
//! sidebar, selection, mouse or clipboard yet. `--smoke-ms` renders one frame
//! under Xvfb and exits, which is how CI checks the window path.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use berth_core::{AppState, SurfaceOps, paths};
use berth_headless::{CellView, PtyHost, ScreenView, restore_session, save_session};
use berth_server::{Server, ServerConfig, ServerHandle, SocketMode, auth};
use font8x8::{BASIC_FONTS, UnicodeFonts};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

const FRAME_INTERVAL: Duration = Duration::from_millis(33);
const BG: u32 = 0x0010_1014;
const FG: u32 = 0x00e6_e6ee;
const MIN_COLS: u16 = 20;
const MIN_ROWS: u16 = 5;

#[derive(Debug)]
struct Args {
    socket: PathBuf,
    mode: SocketMode,
    token: Option<String>,
    no_restore: bool,
    smoke_ms: Option<u64>,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut args = std::env::args().skip(1);
        let mut parsed = Self {
            socket: paths::socket_path(),
            mode: SocketMode::Local,
            token: None,
            no_restore: false,
            smoke_ms: None,
        };
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--socket" => {
                    parsed.socket = args
                        .next()
                        .ok_or_else(|| "--socket needs a path".to_string())?
                        .into();
                }
                "--mode" => {
                    let mode = args
                        .next()
                        .ok_or_else(|| "--mode needs off, local or trusted".to_string())?;
                    parsed.mode = match mode.as_str() {
                        "off" => SocketMode::Off,
                        "local" => SocketMode::Local,
                        "trusted" => SocketMode::Trusted,
                        other => return Err(format!("unknown mode {other:?}")),
                    };
                }
                "--token" => {
                    parsed.token = Some(
                        args.next()
                            .ok_or_else(|| "--token needs a value".to_string())?,
                    );
                }
                "--no-restore" => parsed.no_restore = true,
                "--smoke-ms" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--smoke-ms needs milliseconds".to_string())?;
                    parsed.smoke_ms = Some(
                        value
                            .parse()
                            .map_err(|_| format!("invalid --smoke-ms {value:?}"))?,
                    );
                }
                "-h" | "--help" => {
                    print_help();
                    std::process::exit(0);
                }
                "-V" | "--version" => {
                    println!("berth-gui {}", env!("CARGO_PKG_VERSION"));
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument {other:?}")),
            }
        }
        Ok(parsed)
    }
}

fn print_help() {
    println!(
        "berth-gui {}\n\
         \n\
         Minimal Linux/Windows shell: one window, one terminal.\n\
         \n\
         Usage: berth-gui [options]\n\
         \n\
           --socket <path>     socket or named pipe (default: platform path)\n\
           --mode <mode>       off | local | trusted (default: local)\n\
           --token <token>     token for local mode\n\
           --no-restore        ignore the saved session\n\
           --smoke-ms <ms>     render, then exit (CI/headless check)\n\
           -h, --help          show this help\n\
           -V, --version       show the version",
        env!("CARGO_PKG_VERSION")
    );
}

fn main() {
    let args = match Args::parse() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("berth-gui: {error}");
            std::process::exit(2);
        }
    };

    let token = match args.mode {
        SocketMode::Local => match args.token.clone() {
            Some(token) => Some(token),
            None => match auth::load_or_create_token(&paths::token_path()) {
                Ok(token) => Some(token),
                Err(error) => {
                    eprintln!("berth-gui: token: {error}");
                    std::process::exit(1);
                }
            },
        },
        SocketMode::Trusted | SocketMode::Off => args.token.clone(),
    };

    let host = Arc::new(PtyHost::new());
    host.set_transport(token.clone(), Some(paths::display(&args.socket)));

    let state = Arc::new(Mutex::new(AppState::default()));
    if !args.no_restore {
        match restore_session(&state, host.as_ref()) {
            Ok(0) => {}
            Ok(count) => println!("berth-gui: restored {count} session(s)"),
            Err(error) => eprintln!("berth-gui: restore skipped: {error}"),
        }
    }
    if lock(&state).workspaces.is_empty() {
        if let Err(error) = lock(&state).create_workspace(Some("1"), None, None, host.as_ref()) {
            eprintln!("berth-gui: could not create workspace: {error}");
            std::process::exit(1);
        }
    }

    let mut server: Option<ServerHandle> = None;
    if args.mode != SocketMode::Off {
        let save_state = Arc::clone(&state);
        let config = ServerConfig::new(args.socket.clone(), args.mode);
        let config = match &token {
            Some(token) => config.token(token.clone()),
            None => config,
        };
        match Server::new(config, Arc::clone(&state), host.clone())
            .on_session_save(Arc::new(move || {
                if let Err(error) = save_session(&save_state) {
                    eprintln!("berth-gui: session.save: {error}");
                }
            }))
            .start()
        {
            Ok(handle) => {
                println!(
                    "berth-gui {} · socket {} · auth {}",
                    env!("CARGO_PKG_VERSION"),
                    paths::display(handle.path()),
                    args.mode.as_str()
                );
                server = Some(handle);
            }
            Err(error) => eprintln!("berth-gui: server not started: {error}"),
        }
    }

    let event_loop = match EventLoop::new() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            eprintln!("berth-gui: event loop: {error}");
            std::process::exit(1);
        }
    };
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = Gui {
        state,
        host,
        server,
        smoke: args.smoke_ms.map(Duration::from_millis),
        started: Instant::now(),
        window: None,
        context: None,
        surface: None,
        cols: 80,
        rows: 24,
        sized_surface: None,
        modifiers: ModifiersState::empty(),
        title: String::new(),
        ready: false,
        exit_code: 0,
    };

    if let Err(error) = event_loop.run_app(&mut app) {
        eprintln!("berth-gui: {error}");
        std::process::exit(1);
    }
    if app.exit_code != 0 {
        std::process::exit(app.exit_code);
    }
}

struct Gui {
    state: Arc<Mutex<AppState>>,
    host: Arc<PtyHost>,
    server: Option<ServerHandle>,
    smoke: Option<Duration>,
    started: Instant,
    window: Option<Arc<Window>>,
    context: Option<softbuffer::Context<Arc<Window>>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    cols: u16,
    rows: u16,
    sized_surface: Option<String>,
    modifiers: ModifiersState,
    title: String,
    ready: bool,
    exit_code: i32,
}

impl ApplicationHandler for Gui {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("Berth")
                .with_inner_size(LogicalSize::new(960.0, 600.0))
                .with_min_inner_size(LogicalSize::new(320.0, 200.0)),
        ) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("berth-gui: window: {error}");
                self.exit_code = 1;
                event_loop.exit();
                return;
            }
        };
        let context = match softbuffer::Context::new(window.clone()) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("berth-gui: softbuffer context: {error}");
                self.exit_code = 1;
                event_loop.exit();
                return;
            }
        };
        let surface = match softbuffer::Surface::new(&context, window.clone()) {
            Ok(surface) => surface,
            Err(error) => {
                eprintln!("berth-gui: softbuffer surface: {error}");
                self.exit_code = 1;
                event_loop.exit();
                return;
            }
        };
        self.context = Some(context);
        self.surface = Some(surface);
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window.as_ref().map(|window| window.id()) != Some(window_id) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => {
                if let Err(error) = save_session(&self.state) {
                    eprintln!("berth-gui: session.save: {error}");
                }
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.sync_size();
                self.request_redraw();
            }
            WindowEvent::Focused(true) => {
                if let Some(surface_id) = self.active_surface() {
                    let _ = self.host.focus(&surface_id);
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.key(event);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(smoke) = self.smoke
            && self.started.elapsed() >= smoke
        {
            if !self.ready {
                eprintln!("berth-gui: smoke finished before the first frame");
                self.exit_code = 1;
            }
            if let Err(error) = save_session(&self.state) {
                eprintln!("berth-gui: session.save: {error}");
            }
            event_loop.exit();
            return;
        }
        self.request_redraw();
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + FRAME_INTERVAL));
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.server.take();
    }
}

impl Gui {
    /// The surface the selected workspace wants shown. Falls back to the first
    /// workspace when a client closed or moved the selection.
    fn active_surface(&self) -> Option<String> {
        let state = lock(&self.state);
        let workspace_id = state.selected_workspace.clone().or_else(|| {
            state
                .workspaces
                .first()
                .map(|workspace| workspace.id.clone())
        })?;
        state
            .active_surface_of_workspace(&workspace_id)
            .or_else(|| state.selected_surface.clone())
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn cell_size(&self) -> (usize, usize) {
        let scale = self
            .window
            .as_ref()
            .map(|window| window.scale_factor())
            .unwrap_or(1.0)
            .max(1.0) as usize;
        (8 * scale, 16 * scale)
    }

    fn sync_size(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        let PhysicalSize { width, height } = window.inner_size();
        if width == 0 || height == 0 {
            return;
        }
        let (cell_w, cell_h) = self.cell_size();
        let cols = (width as usize / cell_w).max(MIN_COLS as usize) as u16;
        let rows = (height as usize / cell_h).max(MIN_ROWS as usize) as u16;
        let active = self.active_surface();

        if let Some(surface_id) = &active {
            if self.sized_surface.as_deref() != Some(surface_id)
                || (cols, rows) != (self.cols, self.rows)
            {
                if let Some(window) = &self.window {
                    let state = lock(&self.state);
                    if let Some(workspace_id) = state.workspace_of_surface(surface_id) {
                        if let Some(workspace) = state.workspace(workspace_id) {
                            let title = format!("Berth — {}", workspace.name);
                            if title != self.title {
                                window.set_title(&title);
                                self.title = title;
                            }
                        }
                    }
                }
                let _ = self.host.resize(surface_id, cols, rows);
                if let Ok(mut state) = self.state.lock()
                    && let Some(surface) = state.surface_mut(surface_id)
                {
                    surface.cols = cols;
                    surface.rows = rows;
                }
                self.cols = cols;
                self.rows = rows;
                self.sized_surface = Some(surface_id.clone());
            }
        }
    }

    fn key(&mut self, event: winit::event::KeyEvent) {
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        let mods = self.modifier_names();
        // Cmd/Super belongs to the platform shell; don't leak it into the PTY.
        if self.modifiers.super_key() {
            return;
        }

        if let Key::Named(named) = &event.logical_key {
            if let Some(key) = named_key(*named) {
                let _ = self.host.send_key(&surface_id, key, &mods);
                return;
            }
        }

        match &event.logical_key {
            Key::Character(character) => {
                if self.modifiers.control_key() || self.modifiers.alt_key() {
                    let _ = self.host.send_key(&surface_id, character, &mods);
                } else if let Some(text) = &event.text {
                    let _ = self.host.send_text(&surface_id, text);
                }
            }
            _ => {
                if let Some(text) = &event.text {
                    let _ = self.host.send_text(&surface_id, text);
                }
            }
        }
    }

    fn modifier_names(&self) -> Vec<String> {
        let mut mods = Vec::new();
        if self.modifiers.control_key() {
            mods.push("ctrl".into());
        }
        if self.modifiers.alt_key() {
            mods.push("alt".into());
        }
        if self.modifiers.shift_key() {
            mods.push("shift".into());
        }
        mods
    }

    fn redraw(&mut self) {
        self.sync_size();
        let Some(window) = self.window.clone() else {
            return;
        };
        let PhysicalSize { width, height } = window.inner_size();
        let (width, height) = (width as usize, height as usize);
        if width == 0 || height == 0 {
            return;
        }
        let (cell_w, cell_h) = self.cell_size();
        let active = self.active_surface();
        let view = active
            .as_deref()
            .and_then(|surface_id| self.host.screen_view(surface_id).ok());
        let Some(soft_surface) = self.surface.as_mut() else {
            return;
        };
        let (Some(nonzero_w), Some(nonzero_h)) = (
            std::num::NonZeroU32::new(width as u32),
            std::num::NonZeroU32::new(height as u32),
        ) else {
            return;
        };
        if soft_surface.resize(nonzero_w, nonzero_h).is_err() {
            return;
        }
        let Ok(mut buffer) = soft_surface.buffer_mut() else {
            return;
        };
        buffer.fill(BG);

        if let Some(view) = view {
            paint_screen(&mut buffer, &view, width, height, cell_w, cell_h);
        } else {
            let message = "berth: no live surface";
            for (index, character) in message.chars().enumerate() {
                draw_glyph(
                    &mut buffer,
                    width,
                    8 + index * cell_w,
                    8,
                    character,
                    FG,
                    cell_w,
                    cell_h,
                    false,
                );
            }
        }

        if buffer.present().is_err() {
            return;
        }
        if !self.ready {
            self.ready = true;
            println!(
                "berth-gui: window ready {width}x{height} ({}x{} cells)",
                self.cols, self.rows
            );
        }
    }
}

fn paint_screen(
    buffer: &mut [u32],
    view: &ScreenView,
    width: usize,
    height: usize,
    cell_w: usize,
    cell_h: usize,
) {
    let cols = view.cols.min((width / cell_w) as u16);
    let rows = view.rows.min((height / cell_h) as u16);
    for row in 0..rows {
        for col in 0..cols {
            let index = usize::from(row) * usize::from(view.cols) + usize::from(col);
            let Some(cell) = view.cells.get(index) else {
                continue;
            };
            let cursor = view.cursor == (row, col);
            paint_cell(buffer, width, col, row, cell, cursor, cell_w, cell_h);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_cell(
    buffer: &mut [u32],
    stride: usize,
    col: u16,
    row: u16,
    cell: &CellView,
    cursor: bool,
    cell_w: usize,
    cell_h: usize,
) {
    let mut fg = cell.fg.map(rgb).unwrap_or(FG);
    let mut bg = cell.bg.map(rgb).unwrap_or(BG);
    if cell.inverse {
        std::mem::swap(&mut fg, &mut bg);
    }
    if cursor {
        std::mem::swap(&mut fg, &mut bg);
    }
    let x = usize::from(col) * cell_w;
    let y = usize::from(row) * cell_h;
    fill_rect(buffer, stride, x, y, cell_w, cell_h, bg);

    let character = cell.contents.chars().next().unwrap_or(' ');
    if character != ' ' {
        draw_glyph(
            buffer, stride, x, y, character, fg, cell_w, cell_h, cell.bold,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_glyph(
    buffer: &mut [u32],
    stride: usize,
    x: usize,
    y: usize,
    character: char,
    color: u32,
    cell_w: usize,
    cell_h: usize,
    bold: bool,
) {
    let Some(glyph) = BASIC_FONTS.get(character).or_else(|| BASIC_FONTS.get('?')) else {
        return;
    };
    let scale_x = (cell_w / 8).max(1);
    let scale_y = (cell_h / 8).max(1);
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..8usize {
            if bits & (1 << col) == 0 {
                continue;
            }
            let px = x + col * scale_x;
            let py = y + row * scale_y;
            fill_rect(buffer, stride, px, py, scale_x, scale_y, color);
            if bold {
                fill_rect(buffer, stride, px + 1, py, scale_x, scale_y, color);
            }
        }
    }
}

fn fill_rect(
    buffer: &mut [u32],
    stride: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    color: u32,
) {
    let stride_pixels = buffer.len() / stride.max(1);
    for row in y..y.saturating_add(height) {
        if row >= stride_pixels {
            break;
        }
        let start = row * stride + x;
        let end = (start + width).min((row + 1) * stride).min(buffer.len());
        if start < end {
            buffer[start..end].fill(color);
        }
    }
}

fn rgb(color: [u8; 3]) -> u32 {
    (u32::from(color[0]) << 16) | (u32::from(color[1]) << 8) | u32::from(color[2])
}

fn named_key(named: NamedKey) -> Option<&'static str> {
    Some(match named {
        NamedKey::Enter => "enter",
        NamedKey::Tab => "tab",
        NamedKey::Escape => "escape",
        NamedKey::Backspace => "backspace",
        NamedKey::Delete => "delete",
        NamedKey::Insert => "insert",
        NamedKey::ArrowUp => "up",
        NamedKey::ArrowDown => "down",
        NamedKey::ArrowLeft => "left",
        NamedKey::ArrowRight => "right",
        NamedKey::Home => "home",
        NamedKey::End => "end",
        NamedKey::PageUp => "pageup",
        NamedKey::PageDown => "pagedown",
        NamedKey::Space => "space",
        NamedKey::F1 => "f1",
        NamedKey::F2 => "f2",
        NamedKey::F3 => "f3",
        NamedKey::F4 => "f4",
        NamedKey::F5 => "f5",
        NamedKey::F6 => "f6",
        NamedKey::F7 => "f7",
        NamedKey::F8 => "f8",
        NamedKey::F9 => "f9",
        NamedKey::F10 => "f10",
        NamedKey::F11 => "f11",
        NamedKey::F12 => "f12",
        _ => return None,
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
