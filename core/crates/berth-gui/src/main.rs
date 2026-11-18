//! Berth's portable window: Linux, Windows and macOS on one shell.
//!
//! `winit` owns the window and input, `softbuffer` presents a CPU buffer, and
//! [`font`] rasterises real system fonts. The terminal engine, session and
//! protocol are `berth-headless` and `berth-server`, so the CLI and hooks keep
//! working against this window exactly as they do against the macOS app.
//!
//! The core is the only source of truth: panes, tabs, workspaces, agents and
//! notifications all come from `AppState`, and every user action goes back
//! through `berth_core::dispatch` so events are emitted on one path.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use berth_core::model::Orientation;
use berth_core::{AppState, CoreError, Request, Response, SurfaceOps, paths};
use berth_headless::{PtyHost, restore_session, save_session};
use berth_server::{Server, ServerConfig, ServerHandle, SocketMode, auth};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

mod config;
mod font;
mod keys;
mod layout;
mod paint;
mod panel;
mod theme;
mod ui;

use config::Config;
use font::{CellSize, FontRenderer};
use keys::Command;
use layout::{Divider, Rect};
use paint::{Painter, ScreenOptions};
use theme::Theme;
use ui::{Cell, PaneView, Selection};

const FRAME_INTERVAL: Duration = Duration::from_millis(33);
const FLASH_DURATION: Duration = Duration::from_millis(1200);
const BLINK_INTERVAL: Duration = Duration::from_millis(530);
const DIVIDER_EDGE: usize = 4;
/// Let the first shell prompt (or a restore) arrive before a PNG capture.
const PNG_DELAY: Duration = Duration::from_millis(700);
/// Chrome is drawn smaller than the terminal, like the macOS window's 12pt
/// sidebar and tab labels. Points, before the display scale.
const CHROME_FONT_SIZE: f32 = 13.0;

#[derive(Debug)]
struct Args {
    socket: Option<PathBuf>,
    mode: Option<SocketMode>,
    token: Option<String>,
    no_restore: bool,
    smoke_ms: Option<u64>,
    smoke_png: Option<PathBuf>,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut args = std::env::args().skip(1);
        let mut parsed = Self {
            socket: None,
            mode: None,
            token: None,
            no_restore: false,
            smoke_ms: None,
            smoke_png: None,
        };
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--socket" => {
                    parsed.socket = Some(
                        args.next()
                            .ok_or_else(|| "--socket needs a path".to_string())?
                            .into(),
                    );
                }
                "--mode" => {
                    let mode = args
                        .next()
                        .ok_or_else(|| "--mode needs off, local or trusted".to_string())?;
                    parsed.mode = Some(match mode.as_str() {
                        "off" => SocketMode::Off,
                        "local" => SocketMode::Local,
                        "trusted" => SocketMode::Trusted,
                        other => return Err(format!("unknown mode {other:?}")),
                    });
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
                "--smoke-png" => {
                    parsed.smoke_png = Some(
                        args.next()
                            .ok_or_else(|| "--smoke-png needs a path".to_string())?
                            .into(),
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
         Portable Berth window for Linux, Windows and macOS.\n\
         \n\
         Usage: berth-gui [options]\n\
         \n\
           --socket <path>     socket or named pipe (default: platform path)\n\
           --mode <mode>       off | local | trusted (default: config, else local)\n\
           --token <token>     token for local mode\n\
           --no-restore        ignore the saved session\n\
           --smoke-ms <ms>     render, then exit (headless check)\n\
           --smoke-png <path>  render one frame to PNG, then exit\n\
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

    let (config, warning) = Config::load();
    if let Some(warning) = warning {
        eprintln!("berth-gui: config skipped: {warning}");
    }
    let socket = args.socket.clone().unwrap_or_else(paths::socket_path);
    let mode = args
        .mode
        .or_else(|| match config.socket_mode() {
            Some("off") => Some(SocketMode::Off),
            Some("trusted") => Some(SocketMode::Trusted),
            Some("local") => Some(SocketMode::Local),
            _ => None,
        })
        .unwrap_or(SocketMode::Local);

    let token = match mode {
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
    host.set_transport(token.clone(), Some(paths::display(&socket)));

    let state = Arc::new(Mutex::new(AppState::default()));
    if !args.no_restore {
        match restore_session(&state, host.as_ref()) {
            Ok(0) => {}
            Ok(count) => println!("berth-gui: restored {count} session(s)"),
            Err(error) => eprintln!("berth-gui: restore skipped: {error}"),
        }
    }
    if lock(&state).workspaces.is_empty()
        && let Err(error) = lock(&state).create_workspace(Some("1"), None, None, host.as_ref())
    {
        eprintln!("berth-gui: could not create workspace: {error}");
        std::process::exit(1);
    }

    let mut server: Option<ServerHandle> = None;
    if mode != SocketMode::Off {
        let save_state = Arc::clone(&state);
        let server_config = ServerConfig::new(socket.clone(), mode);
        let server_config = match &token {
            Some(token) => server_config.token(token.clone()),
            None => server_config,
        };
        match Server::new(server_config, Arc::clone(&state), host.clone())
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
                    mode.as_str()
                );
                server = Some(handle);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                eprintln!("berth-gui: {error}");
                eprintln!("berth-gui: close the other instance, or pass --socket with a new path");
                std::process::exit(1);
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
        theme: Theme::default(),
        config,
        font: None,
        chrome_font: None,
        clipboard: arboard::Clipboard::new().ok(),
        smoke: args.smoke_ms.map(Duration::from_millis),
        smoke_png: args.smoke_png,
        started: Instant::now(),
        window: None,
        context: None,
        surface: None,
        sized: HashMap::new(),
        pointer: (0.0, 0.0),
        modifiers: ModifiersState::empty(),
        title: String::new(),
        ready: false,
        exit_code: 0,
        scale: 1.0,
        sidebar_visible: true,
        sidebar_width: 0.0,
        drag: Drag::None,
        selection: HashMap::new(),
        scroll: HashMap::new(),
        last_meta: HashMap::new(),
        flashes: HashMap::new(),
        last_click: None,
        exit_announced: HashMap::new(),
        window_focused: true,
        overlay: None,
        last_detect: Instant::now(),
        last_sweep: Instant::now(),
        save_due: None,
        save_seq: 0,
        quit: false,
        captured: false,
    };

    if let Err(error) = event_loop.run_app(&mut app) {
        eprintln!("berth-gui: {error}");
        std::process::exit(1);
    }
    if app.exit_code != 0 {
        std::process::exit(app.exit_code);
    }
}

/// A modal panel over the window. Both variants are lists driven by the core.
enum Overlay {
    Palette { query: String, selected: usize },
    Attention { selected: usize },
}

impl Overlay {
    fn selected(&self) -> usize {
        match self {
            Overlay::Palette { selected, .. } | Overlay::Attention { selected } => *selected,
        }
    }

    fn select(&mut self, index: usize) {
        match self {
            Overlay::Palette { selected, .. } | Overlay::Attention { selected } => {
                *selected = index;
            }
        }
    }
}

/// What the pointer is holding, if anything.
enum Drag {
    None,
    Divider(Divider),
    SidebarWidth { start_x: f64, start_width: f64 },
}

struct Click {
    at: Instant,
    point: (usize, usize),
    target: ClickTarget,
}

#[derive(Clone, PartialEq, Eq)]
enum ClickTarget {
    Pane(String),
    Tab(String),
    TabStrip(String),
    SidebarEmpty,
}

/// The metadata last applied to the core, so a frame only emits changes.
#[derive(PartialEq, Default)]
struct AppliedMeta {
    title: Option<String>,
    cwd: Option<String>,
    bells: u32,
    progress: Option<Option<f32>>,
}

struct Gui {
    state: Arc<Mutex<AppState>>,
    host: Arc<PtyHost>,
    server: Option<ServerHandle>,
    theme: Theme,
    config: Config,
    font: Option<FontRenderer>,
    chrome_font: Option<FontRenderer>,
    clipboard: Option<arboard::Clipboard>,
    smoke: Option<Duration>,
    smoke_png: Option<PathBuf>,
    started: Instant,
    window: Option<Arc<Window>>,
    context: Option<softbuffer::Context<Arc<Window>>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    sized: HashMap<String, (u16, u16)>,
    pointer: (f64, f64),
    modifiers: ModifiersState,
    title: String,
    ready: bool,
    exit_code: i32,
    scale: f64,
    sidebar_visible: bool,
    sidebar_width: f64,
    drag: Drag,
    selection: HashMap<String, Selection>,
    scroll: HashMap<String, usize>,
    last_meta: HashMap<String, AppliedMeta>,
    flashes: HashMap<String, Instant>,
    last_click: Option<Click>,
    exit_announced: HashMap<String, bool>,
    window_focused: bool,
    overlay: Option<Overlay>,
    last_detect: Instant,
    last_sweep: Instant,
    save_due: Option<Instant>,
    save_seq: u64,
    /// Set when a one-frame run (PNG smoke) has written its frame.
    quit: bool,
    captured: bool,
}

impl ApplicationHandler for Gui {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("Berth")
                .with_inner_size(LogicalSize::new(1180.0, 720.0))
                .with_min_inner_size(LogicalSize::new(640.0, 320.0)),
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
        self.scale = window.scale_factor();
        self.sidebar_width = self.config.sidebar_pixels().max(config::MIN_SIDEBAR_WIDTH);
        self.context = Some(context);
        self.surface = Some(surface);
        self.window = Some(window);
        self.request_redraw();
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
                self.sync_scale();
                self.request_redraw();
            }
            WindowEvent::Focused(focused) => {
                self.window_focused = focused;
                if !focused {
                    self.overlay = None;
                }
                if focused && let Some(surface_id) = self.active_surface() {
                    let _ = self.host.focus(&surface_id);
                }
                self.request_redraw();
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = (position.x, position.y);
                self.drag_to_pointer();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => self.mouse_pressed(button),
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                self.drag = Drag::None;
                if let Some(surface_id) = self.active_surface()
                    && let Some(selection) = self.selection.get_mut(&surface_id)
                {
                    selection.dragging = false;
                }
                self.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => self.wheel(delta),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.key(event);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let smoke_done = self
            .smoke
            .is_some_and(|smoke| self.started.elapsed() >= smoke);
        let capture_pending = self.smoke_png.is_some() && !self.captured;
        if smoke_done && !capture_pending {
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
        self.tick_agents();
        self.tick_save();
        if self.quit {
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
    /// Passive detection, the same cadence as the macOS shell: once a second for
    /// surfaces without an agent, then a stale sweep every 30 seconds.
    fn tick_agents(&mut self) {
        if self.last_detect.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_detect = Instant::now();
        let candidates: Vec<String> = {
            let state = lock(&self.state);
            state
                .surfaces
                .values()
                .filter(|surface| surface.pid.is_some())
                .filter(|surface| state.agent_for_surface(&surface.id).is_none())
                .map(|surface| surface.id.clone())
                .collect()
        };
        for surface_id in candidates {
            if !self.host.is_alive(&surface_id) {
                continue;
            }
            let params = serde_json::json!({"surface_id": surface_id});
            if let Err(error) = self.dispatch("agent.detect", params) {
                eprintln!("berth-gui: agent.detect: {error}");
            }
        }
        if self.last_sweep.elapsed() >= Duration::from_secs(30) {
            self.last_sweep = Instant::now();
            let now = berth_core::time::now_ms();
            let changed = {
                let mut state = lock(&self.state);
                state.sweep_stale(now, 180_000, self.host.as_ref())
            };
            if !changed.is_empty() {
                self.flush_events();
            }
        }
    }

    /// Write the snapshot half a second after the last core event, like the
    /// macOS autosave. Watching the event sequence covers changes made by the
    /// server thread, not only the ones this window dispatched.
    fn tick_save(&mut self) {
        let seq = lock(&self.state).last_seq();
        if seq != self.save_seq {
            self.save_seq = seq;
            self.save_due = Some(Instant::now() + Duration::from_millis(500));
        }
        if let Some(due) = self.save_due
            && Instant::now() >= due
        {
            self.save_due = None;
            if let Err(error) = save_session(&self.state) {
                eprintln!("berth-gui: autosave: {error}");
            }
        }
    }

    fn sync_scale(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        let scale = window.scale_factor();
        if (scale - self.scale).abs() > f64::EPSILON {
            self.scale = scale;
            self.font = None;
            self.chrome_font = None;
            self.sized.clear();
        }
    }

    fn font(&mut self) -> &mut FontRenderer {
        let pixels = self.config.font_pixels(self.scale);
        if self.font.is_none() {
            self.font = Some(FontRenderer::new(pixels));
        }
        self.font.as_mut().expect("font just created")
    }

    fn cell(&mut self) -> CellSize {
        self.font().cell()
    }

    fn chrome_font(&mut self) -> &mut FontRenderer {
        if self.chrome_font.is_none() {
            self.chrome_font = Some(FontRenderer::new(CHROME_FONT_SIZE * self.scale as f32));
        }
        self.chrome_font.as_mut().expect("chrome font just created")
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// The surface the selected workspace wants shown.
    fn active_surface(&self) -> Option<String> {
        let state = lock(&self.state);
        let workspace_id = ui::active_workspace(&state)?;
        state
            .active_surface_of_workspace(&workspace_id)
            .or_else(|| state.selected_surface.clone())
    }

    fn dispatch(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, CoreError> {
        let response = {
            let mut state = lock(&self.state);
            let request = Request {
                id: serde_json::Value::Null,
                method: method.to_string(),
                params,
            };
            berth_core::dispatch(&mut state, self.host.as_ref(), &request)
        };
        self.flush_events();
        match response {
            Response {
                ok: true, result, ..
            } => Ok(result.unwrap_or(serde_json::Value::Null)),
            Response {
                error: Some(error), ..
            } => Err(CoreError::new(error.code, error.message)),
            _ => Err(CoreError::internal("dispatch returned no result")),
        }
    }

    /// Publish core events produced by actions the window took directly.
    fn flush_events(&mut self) {
        let events = lock(&self.state).drain_events();
        if events.is_empty() {
            return;
        }
        if let Some(server) = &self.server {
            for event in events {
                if let Ok(line) = serde_json::to_string(&event) {
                    server.broadcast(&line);
                }
            }
        }
    }

    fn sidebar_pixels(&self) -> usize {
        if !self.sidebar_visible {
            return 0;
        }
        (self.sidebar_width * self.scale) as usize
    }

    fn content_area(&self, width: usize, height: usize) -> Rect {
        let sidebar = self.sidebar_pixels().min(width.saturating_sub(120));
        Rect::new(sidebar, 0, width.saturating_sub(sidebar), height)
    }

    fn tab_height(&self) -> usize {
        (ui::TAB_HEIGHT * self.scale) as usize
    }

    fn workspace_row_height(&self) -> f64 {
        let logical = if self.config.compact_sidebar {
            ui::WORKSPACE_ROW_COMPACT
        } else {
            ui::WORKSPACE_ROW_HEIGHT
        };
        logical * self.scale
    }

    fn nav(&mut self) -> Nav {
        let cell = self.cell();
        let (width, height) = self
            .window
            .as_ref()
            .map(|window| {
                let size = window.inner_size();
                (size.width as usize, size.height as usize)
            })
            .unwrap_or((0, 0));
        let area = self.content_area(width, height);
        Nav {
            cell,
            height,
            area,
            sidebar: Rect::new(0, 0, area.x, height),
            tab_height: self.tab_height(),
        }
    }

    /// Apply OSC title, cwd, bell, progress, clipboard and notification
    /// requests from every surface. Only changes reach the core.
    fn poll_meta(&mut self) {
        let surfaces: Vec<String> = lock(&self.state).surfaces.keys().cloned().collect();
        for surface_id in surfaces {
            let Ok(meta) = self.host.take_screen_meta_requests(&surface_id) else {
                continue;
            };
            let mut params = serde_json::json!({"surface_id": surface_id});
            let mut changed = false;
            {
                let applied = self.last_meta.entry(surface_id.clone()).or_default();
                if meta.title.is_some() && meta.title != applied.title {
                    params["title"] = serde_json::json!(meta.title);
                    applied.title = meta.title.clone();
                    changed = true;
                }
                if meta.cwd.is_some() && meta.cwd != applied.cwd {
                    params["cwd"] = serde_json::json!(meta.cwd);
                    applied.cwd = meta.cwd.clone();
                    changed = true;
                }
                if meta.progress.is_some() && meta.progress != applied.progress {
                    match meta.progress {
                        Some(Some(value)) => {
                            params["progress"] = serde_json::json!(value);
                        }
                        _ => params["clear_progress"] = serde_json::json!(true),
                    }
                    applied.progress = meta.progress;
                    changed = true;
                }
                if meta.bells > applied.bells {
                    params["bell"] = serde_json::json!(true);
                    applied.bells = meta.bells;
                    changed = true;
                }
            }
            if changed {
                if meta.bells > 0 {
                    self.flashes.insert(surface_id.clone(), Instant::now());
                }
                if let Err(error) = self.dispatch("surface.update", params) {
                    eprintln!("berth-gui: surface.update: {error}");
                }
            }
            if let Some(text) = meta.clipboard
                && let Some(clipboard) = self.clipboard.as_mut()
            {
                let _ = clipboard.set_text(text);
            }
            if let Some(body) = meta.notification {
                let workspace_id = lock(&self.state)
                    .surface(&surface_id)
                    .map(|surface| surface.workspace_id.clone());
                let params = serde_json::json!({
                    "level": "info",
                    "title": body,
                    "surface_id": surface_id,
                    "workspace_id": workspace_id,
                });
                if let Err(error) = self.dispatch("notification.create", params) {
                    eprintln!("berth-gui: notification.create: {error}");
                }
            }
        }
    }

    /// Tell the core when a child process exits, once per surface.
    fn poll_exits(&mut self) {
        let surfaces: Vec<String> = lock(&self.state)
            .surfaces
            .values()
            .filter(|surface| surface.pid.is_some())
            .map(|surface| surface.id.clone())
            .collect();
        for surface_id in surfaces {
            let alive = self.host.is_alive(&surface_id);
            let announced = *self
                .exit_announced
                .entry(surface_id.clone())
                .or_insert(!alive);
            if alive || announced {
                continue;
            }
            self.exit_announced.insert(surface_id.clone(), true);
            let params = serde_json::json!({
                "surface_id": surface_id,
                "exited": true,
            });
            if let Err(error) = self.dispatch("surface.update", params) {
                eprintln!("berth-gui: surface.update exited: {error}");
            }
        }
    }

    /// Give every visible surface the grid that fits its pane.
    fn sync_sizes(&mut self, panes: &[PaneView], cell: CellSize) {
        for pane in panes {
            let Some(surface_id) = &pane.surface else {
                continue;
            };
            let cols = (pane.content.w / cell.width).max(1).min(u16::MAX as usize) as u16;
            let rows = (pane.content.h / cell.height).max(1).min(u16::MAX as usize) as u16;
            if self.sized.get(surface_id) == Some(&(cols, rows)) {
                continue;
            }
            let params = serde_json::json!({
                "surface_id": surface_id,
                "cols": cols,
                "rows": rows,
            });
            match self.dispatch("surface.resize", params) {
                Ok(_) => {
                    self.sized.insert(surface_id.clone(), (cols, rows));
                }
                Err(error) => eprintln!("berth-gui: surface.resize: {error}"),
            }
        }
    }

    fn sync_title(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        let state = lock(&self.state);
        let Some(workspace_id) = ui::active_workspace(&state) else {
            return;
        };
        let Some(workspace) = state.workspace(&workspace_id) else {
            return;
        };
        let title = format!("Berth — {}", workspace.name);
        if title != self.title {
            window.set_title(&title);
            self.title = title;
        }
    }

    fn redraw(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        self.poll_meta();
        self.poll_exits();
        let PhysicalSize { width, height } = window.inner_size();
        let (width, height) = (width as usize, height as usize);
        if width == 0 || height == 0 {
            return;
        }
        // Take the presenter out of `self` so painting can borrow the rest of
        // the window state while it owns the pixel buffer.
        let Some(mut soft) = self.surface.take() else {
            return;
        };
        if let Err(error) = self.paint(&mut soft, width, height) {
            eprintln!("berth-gui: {error}");
            self.exit_code = 1;
        }
        self.surface = Some(soft);
    }

    fn paint(
        &mut self,
        soft: &mut softbuffer::Surface<Arc<Window>, Arc<Window>>,
        width: usize,
        height: usize,
    ) -> Result<(), String> {
        let cell = self.cell();
        let area = self.content_area(width, height);
        let theme = self.theme;
        let tab_height = self.tab_height();
        let flash = self
            .flashes
            .iter()
            .find(|(_, at)| at.elapsed() < FLASH_DURATION)
            .map(|(surface, _)| surface.clone());

        let (panes, _dividers) = {
            let state = lock(&self.state);
            ui::build_panes(&state, area, cell.height, tab_height, flash.as_deref())
        };
        let rows = if area.x > 0 {
            let state = lock(&self.state);
            ui::build_sidebar(
                &state,
                Rect::new(0, 0, area.x, height),
                self.workspace_row_height(),
                &theme,
            )
        } else {
            Vec::new()
        };
        self.sync_sizes(&panes, cell);
        self.sync_title();

        let (Some(nonzero_w), Some(nonzero_h)) = (
            std::num::NonZeroU32::new(width as u32),
            std::num::NonZeroU32::new(height as u32),
        ) else {
            return Ok(());
        };
        soft.resize(nonzero_w, nonzero_h)
            .map_err(|error| format!("softbuffer resize: {error}"))?;
        let mut buffer = soft
            .buffer_mut()
            .map_err(|error| format!("softbuffer buffer: {error}"))?;

        {
            let mut painter = Painter::new(&mut buffer, width, height);
            painter.fill(Rect::new(0, 0, width, height), theme.terminal_bg);
            if area.x > 0 {
                self.paint_sidebar(&mut painter, &rows, Rect::new(0, 0, area.x, height), theme);
            }
            if panes.is_empty() {
                let font = self.chrome_font();
                painter.text(
                    font,
                    (area.x + 12, 12),
                    "berth: no live surface",
                    font::TextStyle::NORMAL,
                    theme.text_tertiary,
                    area.w.saturating_sub(24),
                );
            }
            let blink_on =
                (self.started.elapsed().as_millis() / BLINK_INTERVAL.as_millis()).is_multiple_of(2);
            for pane in &panes {
                let border = if pane.focused {
                    theme.pane_border_focused
                } else {
                    theme.pane_border
                };
                painter.stroke(pane.rect, border);
                if pane.attention_flash {
                    painter.stroke(pane.rect, theme.attention_input);
                }
                painter.fill(pane.content, theme.terminal_bg);
                let font = self.chrome_font();
                painter.tab_strip(font, pane, theme);
                let Some(surface_id) = &pane.surface else {
                    continue;
                };
                let Ok(screen) = self.host.screen_view(surface_id) else {
                    continue;
                };
                let selection = self.selection.get(surface_id).copied();
                let exited = !self.host.is_alive(surface_id);
                let options = ScreenOptions {
                    theme: &theme,
                    focused: pane.focused,
                    cursor_visible: pane.focused && self.window_focused && blink_on,
                    selection: selection.as_ref(),
                    exited,
                };
                let font = self.font();
                painter.screen(font, &screen, pane.content, &options);
            }

            if let Some((title, query, rows, selected)) = self.panel_data() {
                let geometry =
                    panel::geometry(Rect::new(0, 0, width, height), self.scale, query.is_some());
                let font = self.chrome_font();
                painter.panel(
                    font,
                    &paint::PanelStyle {
                        theme: &theme,
                        geometry: &geometry,
                        title: &title,
                        query: query.as_deref(),
                        rows: &rows,
                        selected,
                    },
                );
            }
        }

        let capture_at = self.smoke.unwrap_or(PNG_DELAY);
        if !self.captured
            && self.started.elapsed() >= capture_at
            && let Some(path) = self.smoke_png.clone()
        {
            write_png(&path, &buffer, width, height)
                .map_err(|error| format!("png {}: {error}", path.display()))?;
            self.captured = true;
            if self.smoke.is_none() {
                self.quit = true;
            }
        }

        buffer
            .present()
            .map_err(|error| format!("softbuffer present: {error}"))?;
        if !self.ready {
            self.ready = true;
            println!(
                "berth-gui: window ready {width}x{height} ({} pane(s), {}x{} cell)",
                panes.len(),
                cell.width,
                cell.height
            );
        }
        Ok(())
    }

    fn paint_sidebar(
        &mut self,
        painter: &mut Painter<'_>,
        rows: &[ui::SidebarRow],
        area: Rect,
        theme: Theme,
    ) {
        painter.fill(area, theme.sidebar_bg);
        let compact = self.config.compact_sidebar;
        let font = self.chrome_font();
        let cell = font.cell();
        painter.text(
            font,
            (12, 8),
            "WORKSPACES",
            font::TextStyle::BOLD,
            theme.text_tertiary,
            area.w.saturating_sub(24),
        );
        painter.text(
            font,
            (area.w.saturating_sub(cell.width + 12), 8),
            "+",
            font::TextStyle::NORMAL,
            theme.text_secondary,
            cell.width,
        );
        for row in rows {
            painter.sidebar_row(font, row, theme, compact);
        }
        painter.fill(
            Rect::new(area.w.saturating_sub(1), 0, 1, area.h),
            theme.pane_border,
        );
    }

    // ---- mouse -------------------------------------------------------

    fn mouse_pressed(&mut self, button: MouseButton) {
        let point = (
            self.pointer.0.max(0.0) as usize,
            self.pointer.1.max(0.0) as usize,
        );
        if self.overlay.is_some() {
            if button == MouseButton::Left {
                self.overlay_click(point);
            }
            self.request_redraw();
            return;
        }
        match button {
            MouseButton::Left => self.left_pressed(point),
            MouseButton::Middle => self.middle_pressed(point),
            _ => {}
        }
        self.request_redraw();
    }

    fn middle_pressed(&mut self, point: (usize, usize)) {
        let nav = self.nav();
        let state = lock(&self.state);
        let rows = ui::build_sidebar(
            &state,
            Rect::new(0, 0, nav.area.x, nav.height),
            self.workspace_row_height(),
            &self.theme,
        );
        drop(state);
        if let Some(row) = rows.iter().find(|row| row.rect.contains(point.0, point.1))
            && let ui::RowKind::Workspace(workspace) = &row.kind
        {
            let params = serde_json::json!({"workspace_id": workspace.id});
            if let Err(error) = self.dispatch("workspace.close", params) {
                eprintln!("berth-gui: workspace.close: {error}");
            }
            return;
        }
        let (panes, _) = {
            let state = lock(&self.state);
            ui::build_panes(&state, nav.area, nav.cell.height, nav.tab_height, None)
        };
        for pane in &panes {
            if let Some(tab) = pane
                .tabs
                .iter()
                .find(|tab| tab.rect.contains(point.0, point.1))
            {
                self.close_surface(&tab.id);
                return;
            }
        }
    }

    fn left_pressed(&mut self, point: (usize, usize)) {
        let nav = self.nav();
        if nav.area.x > 0 && point.0 < nav.area.x {
            self.sidebar_pressed(point, nav);
            return;
        }
        let (panes, dividers) = {
            let state = lock(&self.state);
            ui::build_panes(&state, nav.area, nav.cell.height, nav.tab_height, None)
        };
        if let Some(divider) = dividers
            .iter()
            .find(|divider| divider.grab.contains(point.0, point.1))
        {
            self.drag = Drag::Divider(divider.clone());
            return;
        }
        for pane in &panes {
            if !pane.rect.contains(point.0, point.1) {
                continue;
            }
            if let Some(tab) = pane
                .tabs
                .iter()
                .find(|tab| tab.close.contains(point.0, point.1))
            {
                self.close_surface(&tab.id);
                return;
            }
            if let Some(tab) = pane
                .tabs
                .iter()
                .find(|tab| tab.rect.contains(point.0, point.1))
            {
                if self.same_click(point, ClickTarget::Tab(tab.id.clone())) {
                    self.close_surface(&tab.id);
                    return;
                }
                self.record_click(point, ClickTarget::Tab(tab.id.clone()));
                self.activate(&tab.id);
                return;
            }
            if pane.tab_bar.contains(point.0, point.1) {
                if self.same_click(point, ClickTarget::TabStrip(pane.pane_id.clone())) {
                    self.new_tab();
                } else {
                    self.record_click(point, ClickTarget::TabStrip(pane.pane_id.clone()));
                }
                return;
            }
            if pane.content.contains(point.0, point.1) {
                self.pane_pressed(pane, point, nav.cell);
                return;
            }
        }
    }

    fn pane_pressed(&mut self, pane: &PaneView, point: (usize, usize), cell: CellSize) {
        let Some(surface_id) = &pane.surface else {
            return;
        };
        let column = (point.0.saturating_sub(pane.content.x)) / cell.width.max(1);
        let row = (point.1.saturating_sub(pane.content.y)) / cell.height.max(1);
        let target = Cell {
            row: row.min(u16::MAX as usize) as u16,
            col: column.min(u16::MAX as usize) as u16,
        };
        let shift = self.modifiers.shift_key();
        let already_focused = pane.focused;
        if !already_focused {
            self.activate(surface_id);
        }
        if shift
            && already_focused
            && let Some(selection) = self.selection.get_mut(surface_id)
        {
            selection.extend(target);
            selection.dragging = true;
            return;
        }
        if self.same_click(point, ClickTarget::Pane(surface_id.clone())) {
            let Ok(line) = self.host.line_text(surface_id, target.row) else {
                return;
            };
            let selection = Selection::word_at(target, &line);
            self.selection.insert(surface_id.clone(), selection);
            return;
        }
        self.record_click(point, ClickTarget::Pane(surface_id.clone()));
        self.selection
            .insert(surface_id.clone(), Selection::new(target));
    }

    fn sidebar_pressed(&mut self, point: (usize, usize), nav: Nav) {
        if ui::new_workspace_button(nav.sidebar).contains(point.0, point.1) {
            if let Err(error) = self.dispatch("workspace.create", serde_json::json!({})) {
                eprintln!("berth-gui: workspace.create: {error}");
            }
            return;
        }
        if (point.0 as f64 - self.sidebar_pixels() as f64).abs() <= DIVIDER_EDGE as f64 {
            self.drag = Drag::SidebarWidth {
                start_x: point.0 as f64,
                start_width: self.sidebar_width,
            };
            return;
        }
        let rows = {
            let state = lock(&self.state);
            ui::build_sidebar(
                &state,
                Rect::new(0, 0, nav.area.x, nav.height),
                self.workspace_row_height(),
                &self.theme,
            )
        };
        let Some(row) = rows.iter().find(|row| row.rect.contains(point.0, point.1)) else {
            if self.same_click(point, ClickTarget::SidebarEmpty) {
                self.new_workspace();
            } else {
                self.record_click(point, ClickTarget::SidebarEmpty);
            }
            return;
        };
        match &row.kind {
            ui::RowKind::Group { id, collapsed, .. } => {
                let params = serde_json::json!({
                    "group_id": id,
                    "collapsed": !*collapsed,
                });
                if let Err(error) = self.dispatch("group.update", params) {
                    eprintln!("berth-gui: group.update: {error}");
                }
            }
            ui::RowKind::Workspace(workspace) => {
                self.select_workspace(&workspace.id);
            }
        }
    }

    fn same_click(&self, point: (usize, usize), target: ClickTarget) -> bool {
        self.last_click.as_ref().is_some_and(|click| {
            click.target == target
                && click.at.elapsed() < Duration::from_millis(400)
                && click.point.0.abs_diff(point.0) < 4
                && click.point.1.abs_diff(point.1) < 4
        })
    }

    fn record_click(&mut self, point: (usize, usize), target: ClickTarget) {
        self.last_click = Some(Click {
            at: Instant::now(),
            point,
            target,
        });
    }

    fn drag_to_pointer(&mut self) {
        let point = (
            self.pointer.0.max(0.0) as usize,
            self.pointer.1.max(0.0) as usize,
        );
        match &self.drag {
            Drag::None => {
                if self.selection.values().any(|selection| selection.dragging) {
                    let nav = self.nav();
                    let Some(surface_id) = self.active_surface() else {
                        return;
                    };
                    let (panes, _) = {
                        let state = lock(&self.state);
                        ui::build_panes(&state, nav.area, nav.cell.height, nav.tab_height, None)
                    };
                    let Some(pane) = ui::pane_at(&panes, point.0, point.1) else {
                        return;
                    };
                    if pane.surface.as_deref() != Some(surface_id.as_str()) {
                        return;
                    }
                    let cell = Cell {
                        row: ((point.1.saturating_sub(pane.content.y)) / nav.cell.height.max(1))
                            as u16,
                        col: ((point.0.saturating_sub(pane.content.x)) / nav.cell.width.max(1))
                            as u16,
                    };
                    if let Some(selection) = self.selection.get_mut(&surface_id) {
                        selection.extend(cell);
                    }
                    self.request_redraw();
                }
            }
            Drag::Divider(divider) => {
                let axis = match divider.orientation {
                    Orientation::Horizontal => point.0,
                    Orientation::Vertical => point.1,
                };
                let weight = layout::dragged_weight(divider, axis);
                let params = serde_json::json!({
                    "split_id": divider.split_id,
                    "index": divider.child,
                    "weight": weight,
                });
                let split_id = divider.split_id.clone();
                let child = divider.child;
                if let Err(error) = self.dispatch("split.resize", params) {
                    eprintln!("berth-gui: split.resize {split_id}/{child}: {error}");
                }
                self.request_redraw();
            }
            Drag::SidebarWidth {
                start_x,
                start_width,
            } => {
                let delta = (point.0 as f64 - start_x) / self.scale;
                self.sidebar_width = (start_width + delta)
                    .clamp(config::MIN_SIDEBAR_WIDTH, config::MAX_SIDEBAR_WIDTH);
                self.request_redraw();
            }
        }
    }

    fn wheel(&mut self, delta: MouseScrollDelta) {
        let cell = self.cell();
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, y) => (y * keys::SCROLL_STEP as f32).round() as i32,
            MouseScrollDelta::PixelDelta(position) => {
                let per = cell.height.max(1) as f64;
                let raw = position.y / per;
                if raw.abs() < 1.0 {
                    if raw == 0.0 {
                        0
                    } else if raw > 0.0 {
                        1
                    } else {
                        -1
                    }
                } else {
                    raw.round() as i32
                }
            }
        };
        if lines == 0 {
            return;
        }
        let point = (
            self.pointer.0.max(0.0) as usize,
            self.pointer.1.max(0.0) as usize,
        );
        let nav = self.nav();
        let (panes, _) = {
            let state = lock(&self.state);
            ui::build_panes(&state, nav.area, cell.height, nav.tab_height, None)
        };
        let Some(pane) = ui::pane_at(&panes, point.0, point.1) else {
            return;
        };
        let Some(surface_id) = pane.surface.clone() else {
            return;
        };
        let current = self.scroll.get(&surface_id).copied().unwrap_or(0) as i32;
        let target = (current + lines).max(0) as usize;
        match self.host.set_scrollback(&surface_id, target) {
            Ok(applied) => {
                let shift = applied as i32 - current;
                self.scroll.insert(surface_id.clone(), applied);
                if let Some(selection) = self.selection.get_mut(&surface_id) {
                    selection.shift(shift, cell.height as u16);
                }
            }
            Err(error) => eprintln!("berth-gui: scroll {surface_id}: {error}"),
        }
        self.request_redraw();
    }

    // ---- actions -----------------------------------------------------

    fn activate(&mut self, surface_id: &str) {
        let params = serde_json::json!({
            "surface_id": surface_id,
            "focus": true,
        });
        if let Err(error) = self.dispatch("surface.activate", params) {
            eprintln!("berth-gui: activate {surface_id}: {error}");
        }
    }

    fn close_surface(&mut self, surface_id: &str) {
        let params = serde_json::json!({"surface_id": surface_id});
        match self.dispatch("surface.close", params) {
            Ok(_) => {
                self.selection.remove(surface_id);
                self.scroll.remove(surface_id);
            }
            Err(error) => eprintln!("berth-gui: close {surface_id}: {error}"),
        }
    }

    fn new_tab(&mut self) {
        let pane_id = {
            let state = lock(&self.state);
            let Some(surface_id) = self.active_surface() else {
                return;
            };
            state
                .surface(&surface_id)
                .map(|surface| surface.pane_id.clone())
        };
        let Some(pane_id) = pane_id else {
            return;
        };
        let params = serde_json::json!({
            "pane_id": pane_id,
            "focus": true,
        });
        if let Err(error) = self.dispatch("surface.create", params) {
            eprintln!("berth-gui: new tab: {error}");
        }
    }

    fn split(&mut self, direction: &str) {
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        let params = serde_json::json!({
            "surface_id": surface_id,
            "direction": direction,
            "focus": true,
        });
        if let Err(error) = self.dispatch("surface.split", params) {
            eprintln!("berth-gui: split {direction}: {error}");
        }
    }

    fn cycle_tab(&mut self, delta: i32) {
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        let params = serde_json::json!({
            "surface_id": surface_id,
            "delta": delta,
        });
        if let Err(error) = self.dispatch("surface.cycle", params) {
            eprintln!("berth-gui: cycle tab: {error}");
        }
    }

    fn cycle_pane(&mut self, delta: i32) {
        let state = lock(&self.state);
        let Some(workspace_id) = ui::active_workspace(&state) else {
            return;
        };
        let Some(workspace) = state.workspace(&workspace_id) else {
            return;
        };
        let leaves: Vec<String> = workspace
            .root
            .leaves()
            .iter()
            .map(|id| id.to_string())
            .collect();
        let current = state
            .selected_surface
            .as_deref()
            .and_then(|surface_id| state.surface(surface_id))
            .map(|surface| surface.pane_id.clone());
        drop(state);
        if leaves.len() < 2 {
            return;
        }
        let index = current
            .and_then(|pane_id| leaves.iter().position(|leaf| *leaf == pane_id))
            .unwrap_or(0) as i32;
        let next = (index + delta).rem_euclid(leaves.len() as i32) as usize;
        let target_pane = leaves[next].clone();
        let surface = lock(&self.state)
            .panes
            .get(&target_pane)
            .and_then(|pane| pane.active_surface().map(str::to_owned));
        if let Some(surface_id) = surface {
            self.activate(&surface_id);
        }
    }

    fn new_workspace(&mut self) {
        if self.last_click.as_ref().is_some_and(|click| {
            click.target == ClickTarget::SidebarEmpty
                && click.at.elapsed() < Duration::from_millis(400)
        }) {
            self.last_click = None;
            if let Err(error) = self.dispatch("workspace.create", serde_json::json!({})) {
                eprintln!("berth-gui: workspace.create: {error}");
            }
        }
    }

    fn select_workspace(&mut self, workspace_id: &str) {
        let params = serde_json::json!({"workspace_id": workspace_id});
        if let Err(error) = self.dispatch("workspace.select", params) {
            eprintln!("berth-gui: workspace.select: {error}");
        }
    }

    fn select_workspace_index(&mut self, index: usize) {
        let id = {
            let state = lock(&self.state);
            state
                .workspaces
                .get(index.saturating_sub(1))
                .map(|w| w.id.clone())
        };
        if let Some(id) = id {
            self.select_workspace(&id);
        }
    }

    fn jump_attention(&mut self) {
        let target = {
            let state = lock(&self.state);
            let mut candidates: Vec<_> = state
                .agents
                .values()
                .filter(|agent| {
                    agent.attention.is_unread() || agent.attention == berth_core::Attention::Info
                })
                .filter(|agent| {
                    state.selected_surface.as_deref() != Some(agent.surface_id.as_str())
                })
                .collect();
            candidates.sort_by_key(|agent| std::cmp::Reverse(agent.since_ms));
            candidates
                .first()
                .map(|agent| (agent.id.clone(), agent.surface_id.clone()))
        };
        let Some((agent_id, surface_id)) = target else {
            return;
        };
        self.select_workspace(&{
            let state = lock(&self.state);
            state
                .surface(&surface_id)
                .map(|surface| surface.workspace_id.clone())
                .unwrap_or_default()
        });
        self.activate(&surface_id);
        let params = serde_json::json!({"agent_id": agent_id});
        if let Err(error) = self.dispatch("agent.ack", params) {
            eprintln!("berth-gui: agent.ack: {error}");
        }
    }

    fn copy_selection(&mut self) {
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        let Some(selection) = self.selection.get(&surface_id).copied() else {
            return;
        };
        let (start, end) = selection.span();
        let Ok(text) =
            self.host
                .contents_between(&surface_id, start.row, start.col, end.row, end.col)
        else {
            return;
        };
        let text = text
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n");
        if text.is_empty() {
            return;
        }
        if let Some(clipboard) = self.clipboard.as_mut()
            && let Err(error) = clipboard.set_text(text)
        {
            eprintln!("berth-gui: clipboard: {error}");
        }
    }

    fn paste(&mut self) {
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        let Some(clipboard) = self.clipboard.as_mut() else {
            eprintln!("berth-gui: paste: no clipboard available");
            return;
        };
        let text = match clipboard.get_text() {
            Ok(text) => text,
            Err(error) => {
                eprintln!("berth-gui: paste: {error}");
                return;
            }
        };
        if text.is_empty() {
            return;
        }
        let bracketed = self
            .host
            .screen_view(&surface_id)
            .map(|view| view.bracketed_paste)
            .unwrap_or(false);
        let payload = if bracketed {
            format!("\x1b[200~{text}\x1b[201~")
        } else {
            text
        };
        if let Err(error) = self.host.send_text(&surface_id, &payload) {
            eprintln!("berth-gui: paste: {error}");
        }
        self.reset_scroll(&surface_id);
    }

    fn reset_scroll(&mut self, surface_id: &str) {
        let offset = self.scroll.remove(surface_id).unwrap_or(0);
        if offset > 0 {
            let _ = self.host.reset_scrollback(surface_id);
            if let Some(selection) = self.selection.get_mut(surface_id) {
                selection.shift(-(offset as i32), u16::MAX);
            }
        }
    }

    fn scroll_view(&mut self, lines: usize) {
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        let current = self.scroll.get(&surface_id).copied().unwrap_or(0);
        let target = current.saturating_add(lines);
        match self.host.set_scrollback(&surface_id, target) {
            Ok(applied) => {
                let shift = applied as i32 - current as i32;
                self.scroll.insert(surface_id.clone(), applied);
                if let Some(selection) = self.selection.get_mut(&surface_id) {
                    selection.shift(shift, u16::MAX);
                }
            }
            Err(error) => eprintln!("berth-gui: scroll: {error}"),
        }
    }

    fn run_command(&mut self, command: Command) {
        match command {
            Command::SplitRight => self.split("right"),
            Command::SplitDown => self.split("down"),
            Command::NewTab => self.new_tab(),
            Command::CloseTab => {
                if let Some(surface_id) = self.active_surface() {
                    self.close_surface(&surface_id);
                }
            }
            Command::NextTab => self.cycle_tab(1),
            Command::PreviousTab => self.cycle_tab(-1),
            Command::NextPane => self.cycle_pane(1),
            Command::PreviousPane => self.cycle_pane(-1),
            Command::NewWorkspace => {
                if let Err(error) = self.dispatch("workspace.create", serde_json::json!({})) {
                    eprintln!("berth-gui: workspace.create: {error}");
                }
            }
            Command::ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                self.sized.clear();
            }
            Command::Workspace(index) => self.select_workspace_index(index),
            Command::JumpAttention => self.jump_attention(),
            Command::Copy => self.copy_selection(),
            Command::Paste => self.paste(),
            Command::ScrollUp(lines) => self.scroll_view(lines),
            Command::ScrollDown(lines) => {
                let Some(surface_id) = self.active_surface() else {
                    return;
                };
                let current = self.scroll.get(&surface_id).copied().unwrap_or(0);
                let target = current.saturating_sub(lines);
                match self.host.set_scrollback(&surface_id, target) {
                    Ok(applied) => {
                        let shift = applied as i32 - current as i32;
                        self.scroll.insert(surface_id.clone(), applied);
                        if let Some(selection) = self.selection.get_mut(&surface_id) {
                            selection.shift(shift, u16::MAX);
                        }
                    }
                    Err(error) => eprintln!("berth-gui: scroll: {error}"),
                }
            }
            Command::ScrollTop => self.scroll_view(usize::MAX / 2),
            Command::ScrollBottom => {
                if let Some(surface_id) = self.active_surface() {
                    self.reset_scroll(&surface_id);
                }
            }
            Command::Palette => {
                self.overlay = Some(Overlay::Palette {
                    query: String::new(),
                    selected: 0,
                })
            }
            Command::ShowAttention => self.overlay = Some(Overlay::Attention { selected: 0 }),
            Command::Settings => {
                eprintln!("berth-gui: settings has no window yet (parity work in progress)");
            }
        }
        self.request_redraw();
    }

    // ---- panels ------------------------------------------------------

    fn palette_entries(query: &str) -> Vec<config::Action> {
        config::Action::ALL
            .iter()
            .copied()
            .filter(|action| panel::fuzzy_match(query, action.label()))
            .collect()
    }

    fn palette_rows(query: &str, shortcuts: &config::Shortcuts) -> Vec<panel::PanelRow> {
        Self::palette_entries(query)
            .into_iter()
            .map(|action| panel::PanelRow {
                label: action.label().to_string(),
                detail: shortcuts.label(action),
                color: None,
            })
            .collect()
    }

    /// Unacknowledged notifications, newest first, with the workspace they came
    /// from so the panel can jump there.
    fn attention_entries(&self) -> Vec<(String, Option<String>, Option<String>, panel::PanelRow)> {
        let state = lock(&self.state);
        state
            .notifications
            .iter()
            .rev()
            .filter(|notification| !notification.acked)
            .map(|notification| {
                let workspace = notification
                    .workspace_id
                    .as_deref()
                    .and_then(|id| state.workspace(id))
                    .map(|workspace| workspace.name.clone());
                let mut detail = workspace.clone().unwrap_or_default();
                if !notification.body.is_empty() {
                    if !detail.is_empty() {
                        detail.push_str("  ");
                    }
                    detail.push_str(&notification.body);
                }
                let color = match notification.level {
                    berth_core::model::NotifLevel::Error => Some(self.theme.attention_error),
                    berth_core::model::NotifLevel::NeedsInput => Some(self.theme.attention_input),
                    berth_core::model::NotifLevel::Info => None,
                };
                (
                    notification.id.clone(),
                    notification.workspace_id.clone(),
                    notification.surface_id.clone(),
                    panel::PanelRow {
                        label: notification.title.clone(),
                        detail,
                        color,
                    },
                )
            })
            .collect()
    }

    fn overlay_len(&self) -> usize {
        match &self.overlay {
            Some(Overlay::Palette { query, .. }) => Self::palette_entries(query).len(),
            Some(Overlay::Attention { .. }) => self.attention_entries().len(),
            None => 0,
        }
    }

    fn overlay_key(&mut self, event: &KeyEvent) {
        let len = self.overlay_len();
        let mut close = false;
        let mut activate = false;
        if let Some(overlay) = self.overlay.as_mut() {
            match &event.logical_key {
                Key::Named(NamedKey::Escape) => close = true,
                Key::Named(NamedKey::Enter) => activate = true,
                Key::Named(NamedKey::ArrowDown) => {
                    overlay.select(panel::move_selection(overlay.selected(), 1, len));
                }
                Key::Named(NamedKey::ArrowUp) => {
                    overlay.select(panel::move_selection(overlay.selected(), -1, len));
                }
                Key::Named(NamedKey::Backspace) => {
                    if let Overlay::Palette { query, .. } = overlay {
                        query.pop();
                        overlay.select(0);
                    }
                }
                Key::Character(text)
                    if !self.modifiers.control_key()
                        && !self.modifiers.super_key()
                        && !self.modifiers.alt_key() =>
                {
                    if let Overlay::Palette { query, .. } = overlay {
                        query.push_str(text);
                        overlay.select(0);
                    }
                }
                _ => {}
            }
            let selected = panel::clamp_selection(overlay.selected(), len);
            overlay.select(selected);
        }
        if close {
            self.overlay = None;
            self.request_redraw();
        } else if activate {
            self.overlay_activate();
        } else {
            self.request_redraw();
        }
    }

    /// Title, optional query, rows and selection for the open panel.
    fn panel_data(&self) -> Option<(String, Option<String>, Vec<panel::PanelRow>, usize)> {
        match self.overlay.as_ref()? {
            Overlay::Palette { query, selected } => Some((
                "Command Palette".to_string(),
                Some(query.clone()),
                Self::palette_rows(query, &self.config.shortcuts),
                *selected,
            )),
            Overlay::Attention { selected } => Some((
                "Attention".to_string(),
                None,
                self.attention_entries()
                    .into_iter()
                    .map(|(_, _, _, row)| row)
                    .collect(),
                *selected,
            )),
        }
    }

    fn overlay_geometry(&mut self) -> panel::PanelGeometry {
        let (width, height) = self
            .window
            .as_ref()
            .map(|window| {
                let size = window.inner_size();
                (size.width as usize, size.height as usize)
            })
            .unwrap_or((0, 0));
        let has_query = matches!(self.overlay, Some(Overlay::Palette { .. }));
        panel::geometry(Rect::new(0, 0, width, height), self.scale, has_query)
    }

    fn overlay_click(&mut self, point: (usize, usize)) {
        let geometry = self.overlay_geometry();
        if !geometry.panel.contains(point.0, point.1) {
            self.overlay = None;
            self.request_redraw();
            return;
        }
        let cell = self.chrome_font().cell();
        let len = self.overlay_len();
        let rects = panel::row_rects(geometry.rows, len, cell.height + 8);
        let Some(index) = rects
            .iter()
            .position(|rect| rect.contains(point.0, point.1))
        else {
            return;
        };
        if let Some(overlay) = self.overlay.as_mut() {
            overlay.select(index);
        }
        self.overlay_activate();
    }

    fn overlay_activate(&mut self) {
        match self.overlay.take() {
            Some(Overlay::Palette { query, selected }) => {
                let entries = Self::palette_entries(&query);
                if let Some(action) = entries.get(selected).copied() {
                    self.run_command(Command::for_action(action));
                }
            }
            Some(Overlay::Attention { selected }) => {
                let entries = self.attention_entries();
                if let Some((id, workspace_id, surface_id, _)) = entries.get(selected) {
                    let params = serde_json::json!({"notification_id": id});
                    if let Err(error) = self.dispatch("notification.ack", params) {
                        eprintln!("berth-gui: notification.ack: {error}");
                    }
                    if let Some(workspace_id) = workspace_id {
                        self.select_workspace(workspace_id);
                    }
                    if let Some(surface_id) = surface_id {
                        self.activate(surface_id);
                    }
                }
            }
            None => {}
        }
        self.request_redraw();
    }

    // ---- keyboard ----------------------------------------------------

    fn key(&mut self, event: KeyEvent) {
        if self.overlay.is_some() {
            self.overlay_key(&event);
            return;
        }
        if let Some(command) = keys::resolve(
            &event.logical_key,
            self.modifiers,
            event.repeat,
            &self.config.shortcuts,
        ) {
            self.run_command(command);
            return;
        }
        if let Some(surface_id) = self.active_surface() {
            self.reset_scroll(&surface_id);
        }
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        let mods = self.modifier_names();
        // Cmd/Super belongs to the platform shell; don't leak it into the PTY.
        if self.modifiers.super_key() {
            return;
        }

        if let Key::Named(named) = &event.logical_key
            && let Some(key) = named_key(*named)
        {
            let _ = self.host.send_key(&surface_id, key, &mods);
            return;
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
}

/// Geometry for one input event.
struct Nav {
    cell: CellSize,
    height: usize,
    area: Rect,
    sidebar: Rect,
    tab_height: usize,
}

fn write_png(
    path: &std::path::Path,
    buffer: &[u32],
    width: usize,
    height: usize,
) -> std::io::Result<()> {
    let mut rgba = Vec::with_capacity(width * height * 4);
    for pixel in buffer.iter().take(width * height) {
        rgba.push(((pixel >> 16) & 0xff) as u8);
        rgba.push(((pixel >> 8) & 0xff) as u8);
        rgba.push((pixel & 0xff) as u8);
        rgba.push(0xff);
    }
    let file = std::fs::File::create(path)?;
    let writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&rgba)?;
    Ok(())
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
