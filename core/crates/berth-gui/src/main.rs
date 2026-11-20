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

// A double-click must not open a console window; `console::attach_parent_console`
// reconnects to a terminal when the binary is started from one.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use berth_core::model::Orientation;
use berth_core::{AppState, CoreError, ReadScope, Request, Response, SurfaceOps, paths};
use berth_headless::{
    Dimension, MouseEncoding, MouseMode, PtyHost, RestorePolicy, restore_session_with, save_session,
};
use berth_server::{Server, ServerConfig, ServerHandle, SocketMode, auth};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Window, WindowId};

mod a11y;
mod config;
mod console;
mod desktop;
mod font;
mod keys;
mod layout;
mod links;
mod log;
mod mouse;
mod paint;
mod panel;
mod settings;
mod shells;
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

/// Print a startup phase time when `BERTH_TRACE` is set. The first-frame line
/// stays unconditional; these phases exist to find what delayed it.
fn trace_phase(started: Instant, label: &str) {
    if std::env::var_os("BERTH_TRACE").is_some() {
        eprintln!(
            "berth-gui: phase {label} at {} ms",
            started.elapsed().as_millis()
        );
    }
}

/// The next moment the cursor blink phase flips, in `started`-relative steps.
/// Always strictly after `now`.
fn next_blink_edge(started: Instant, now: Instant) -> Instant {
    let step = BLINK_INTERVAL.as_millis().max(1) as u64;
    let elapsed = now.saturating_duration_since(started).as_millis() as u64;
    started + Duration::from_millis((elapsed / step + 1) * step)
}
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
    console::attach_parent_console();
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
    log::init();
    // Read the system font database while the window and shells are being set
    // up; the first frame blocks on it otherwise.
    font::preload();
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

    if mode == SocketMode::Trusted {
        let message = "trusted mode: any local process can drive this instance";
        log::line(message);
        eprintln!("berth-gui: {message}");
    }
    let host = Arc::new(PtyHost::with_options(
        config.scrollback_lines,
        config.graphics,
    ));
    host.set_transport(token.clone(), Some(paths::display(&socket)));
    host.set_default_shell(config.shell.clone());

    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            eprintln!("berth-gui: event loop: {error}");
            std::process::exit(1);
        }
    };
    let proxy = event_loop.create_proxy();
    let wake = Arc::new(Wake::new({
        let proxy = proxy.clone();
        move || {
            let _ = proxy.send_event(UserEvent::Redraw);
        }
    }));
    host.set_waker({
        let wake = Arc::clone(&wake);
        Arc::new(move || wake.wake())
    });

    let state = Arc::new(Mutex::new(AppState::default()));
    lock(&state).set_muted_providers(config.mute_providers.clone());
    if !args.no_restore {
        match restore_session_with(
            &state,
            host.as_ref(),
            RestorePolicy {
                keep_launch_flags: config.restore_launch_flags,
                claude_bypass_permissions: config.claude_bypass_permissions,
                resume_args: config
                    .resume_args
                    .iter()
                    .map(|(provider, args)| (provider.clone(), args.clone()))
                    .collect(),
            },
        ) {
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
            .on_request({
                let wake = Arc::clone(&wake);
                Arc::new(move || wake.wake())
            })
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
                log::line(&format!("endpoint busy: {error}"));
                eprintln!("berth-gui: {error}");
                eprintln!("berth-gui: close the other instance, or pass --socket with a new path");
                std::process::exit(1);
            }
            Err(error) => eprintln!("berth-gui: server not started: {error}"),
        }
    }

    {
        let proxy = proxy.clone();
        if let Err(error) = ctrlc::set_handler(move || {
            let _ = proxy.send_event(UserEvent::Quit);
        }) {
            eprintln!("berth-gui: signal handler: {error}");
        }
    }
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = Gui {
        state,
        host,
        server,
        theme: Theme::default(),
        config,
        font: None,
        chrome_font: None,
        font_job: None,
        chrome_job: None,
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
        sidebar_scroll: 0,
        drag: Drag::None,
        selection: HashMap::new(),
        last_selection: None,
        search_highlight: None,
        hooks_notice: None,
        scroll: HashMap::new(),
        last_meta: HashMap::new(),
        flashes: HashMap::new(),
        last_click: None,
        exit_announced: HashMap::new(),
        window_focused: true,
        overlay: None,
        mouse_down: None,
        preedit: None,
        a11y: None,
        a11y_active: false,
        a11y_text: String::new(),
        proxy: proxy.clone(),
        wake: Arc::clone(&wake),
        blink_shown: true,
        ime_cursor: None,
        autoscroll: false,
        cursor_icon: CursorIcon::Default,
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

/// Ask the event loop for a repaint from any thread, coalescing a burst (a
/// child printing a screenful) into one wake.
struct Wake {
    pending: AtomicBool,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl Wake {
    fn new(notify: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            pending: AtomicBool::new(false),
            notify: Arc::new(notify),
        }
    }

    fn wake(&self) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            (self.notify)();
        }
    }

    /// Called once the loop has picked the wake up, so the next change wakes
    /// it again.
    fn clear(&self) {
        self.pending.store(false, Ordering::Release);
    }
}

/// Events the window sends itself: a repaint request from a pty thread or the
/// socket server, accessibility requests, and a clean-quit request from the
/// signal handler.
enum UserEvent {
    /// Something outside the event loop changed: pty output, an exit, or a
    /// request handled by the socket server.
    Redraw,
    A11y(accesskit_winit::Event),
    Quit,
}

impl From<accesskit_winit::Event> for UserEvent {
    fn from(event: accesskit_winit::Event) -> Self {
        Self::A11y(event)
    }
}

#[derive(Clone, Debug)]
struct SearchMatch {
    line: usize,
    col: usize,
    len: usize,
}

#[derive(Clone, Debug)]
struct SearchHighlight {
    surface_id: String,
    line: usize,
    col: usize,
    len: usize,
}

/// A modal panel over the window. Both variants are lists driven by the core.
enum Overlay {
    Palette {
        query: String,
        selected: usize,
    },
    Attention {
        selected: usize,
    },
    Settings {
        selected: usize,
        /// The action whose next chord is being captured.
        recording: Option<config::Action>,
    },
    /// Multi-line paste preview. The text is sent only on confirmation.
    Confirm {
        text: String,
    },
    /// A tab, workspace or group name being edited.
    Rename {
        target: RenameTarget,
        value: String,
    },
    /// Case-insensitive search through one pane's scrollback.
    Search {
        surface_id: String,
        lines: Vec<String>,
        query: String,
        matches: Vec<SearchMatch>,
        selected: usize,
    },
}

#[derive(Clone, Debug)]
enum RenameTarget {
    Tab(String),
    Workspace(String),
    Group(String),
}

impl Overlay {
    fn selected(&self) -> usize {
        match self {
            Overlay::Palette { selected, .. }
            | Overlay::Attention { selected }
            | Overlay::Settings { selected, .. } => *selected,
            Overlay::Rename { .. } | Overlay::Confirm { .. } => 0,
            Overlay::Search { selected, .. } => *selected,
        }
    }

    fn select(&mut self, index: usize) {
        match self {
            Overlay::Palette { selected, .. }
            | Overlay::Attention { selected }
            | Overlay::Settings { selected, .. }
            | Overlay::Search { selected, .. } => *selected = index,
            Overlay::Rename { .. } | Overlay::Confirm { .. } => {}
        }
    }
}

/// What the pointer is holding, if anything.
enum Drag {
    None,
    Divider(Divider),
    /// A workspace row being dragged to a new position.
    Workspace {
        id: String,
        last_target: Option<String>,
    },
    SidebarWidth {
        start_x: f64,
        start_width: f64,
    },
    /// A tab being dragged along its strip.
    Tab {
        surface_id: String,
        pane_id: String,
        start_x: usize,
    },
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
    WorkspaceRow(String),
    GroupRow(String),
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
    font_job: Option<font::FontSlot>,
    chrome_job: Option<font::FontSlot>,
    clipboard: Option<arboard::Clipboard>,
    smoke: Option<Duration>,
    smoke_png: Option<PathBuf>,
    started: Instant,
    window: Option<Arc<Window>>,
    context: Option<softbuffer::Context<Arc<Window>>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    sized: HashMap<String, (u16, u16, u32, u32)>,
    pointer: (f64, f64),
    modifiers: ModifiersState,
    title: String,
    ready: bool,
    exit_code: i32,
    scale: f64,
    sidebar_visible: bool,
    sidebar_width: f64,
    sidebar_scroll: usize,
    drag: Drag,
    selection: HashMap<String, Selection>,
    /// The surface that last held a selection, so copy still works when focus
    /// and the selection disagree.
    last_selection: Option<String>,
    /// The search match to highlight in the painted pane.
    search_highlight: Option<SearchHighlight>,
    /// Last result of the Settings "reinstall hooks" row.
    hooks_notice: Option<String>,
    scroll: HashMap<String, usize>,
    last_meta: HashMap<String, AppliedMeta>,
    flashes: HashMap<String, Instant>,
    last_click: Option<Click>,
    exit_announced: HashMap<String, bool>,
    window_focused: bool,
    overlay: Option<Overlay>,
    last_detect: Instant,
    last_sweep: Instant,
    /// Button currently held, for drag reports and local selection.
    mouse_down: Option<MouseButton>,
    /// Composing text from an input method, shown at the cursor until commit.
    preedit: Option<String>,
    a11y: Option<accesskit_winit::Adapter>,
    a11y_active: bool,
    a11y_text: String,
    proxy: EventLoopProxy<UserEvent>,
    wake: Arc<Wake>,
    /// Blink phase of the last frame, so a phase flip is what schedules a
    /// repaint instead of a fixed frame timer.
    blink_shown: bool,
    /// Cursor cell and pixel origin the IME window was last placed at.
    ime_cursor: Option<(String, u16, u16, i32, i32)>,
    /// Set by `tick_selection_autoscroll` while a drag sits past a pane edge,
    /// so the loop keeps a frame cadence only while scrolling.
    autoscroll: bool,
    cursor_icon: CursorIcon,
    save_due: Option<Instant>,
    save_seq: u64,
    /// Set when a one-frame run (PNG smoke) has written its frame.
    quit: bool,
    captured: bool,
}

impl ApplicationHandler<UserEvent> for Gui {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("Berth")
                .with_inner_size(LogicalSize::new(1180.0, 720.0))
                .with_min_inner_size(LogicalSize::new(640.0, 320.0))
                // The accessibility adapter must exist before the window is
                // shown, so it starts hidden and is revealed below.
                .with_visible(false),
        ) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("berth-gui: window: {error}");
                self.exit_code = 1;
                event_loop.exit();
                return;
            }
        };
        trace_phase(self.started, "window");
        // Build both fonts while softbuffer, the accessibility adapter and the
        // pane layout are set up; the first frame joins them.
        self.scale = window.scale_factor();
        self.font_job = Some(font::FontSlot::spawn(
            self.config.font_pixels(self.scale),
            self.config.font_family.clone(),
        ));
        self.chrome_job = Some(font::FontSlot::spawn(
            CHROME_FONT_SIZE * self.scale as f32,
            self.config.font_family.clone(),
        ));
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
        trace_phase(self.started, "surface");
        window.set_ime_allowed(true);
        self.a11y = Some(accesskit_winit::Adapter::with_event_loop_proxy(
            &window,
            self.proxy.clone(),
        ));
        window.set_visible(true);
        self.sidebar_width = self.config.sidebar_pixels().max(config::MIN_SIDEBAR_WIDTH);
        self.context = Some(context);
        self.surface = Some(surface);
        trace_phase(self.started, "adapter");
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
        if let Some(window) = self.window.clone()
            && let Some(adapter) = self.a11y.as_mut()
        {
            adapter.process_event(&window, &event);
        }
        match event {
            WindowEvent::CloseRequested => {
                self.save_and_close();
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
            WindowEvent::Ime(ime) => self.ime(ime),
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = (position.x, position.y);
                if self.mouse_down.is_some() && self.report_motion() {
                    return;
                }
                self.drag_to_pointer();
                self.update_cursor_icon();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => self.mouse_pressed(button),
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button,
                ..
            } => {
                if self.report_button(button, true) {
                    self.mouse_down = None;
                    self.request_redraw();
                    return;
                }
                self.mouse_down = None;
                if button != MouseButton::Left {
                    self.request_redraw();
                    return;
                }
                if let Drag::Tab {
                    surface_id,
                    pane_id,
                    start_x,
                } = std::mem::replace(&mut self.drag, Drag::None)
                {
                    let end_x = self.pointer.0.max(0.0) as usize;
                    self.finish_tab_drag(&surface_id, &pane_id, start_x, end_x);
                }
                if let Some(surface_id) = self.active_surface()
                    && let Some(selection) = self.selection.get_mut(&surface_id)
                {
                    selection.dragging = false;
                }
                self.copy_on_select();
                self.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => self.wheel(delta),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.key(event);
            }
            _ => {}
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Redraw => {
                self.wake.clear();
                self.request_redraw();
            }
            UserEvent::Quit => {
                self.save_and_close();
                event_loop.exit();
            }
            UserEvent::A11y(event) => match event.window_event {
                accesskit_winit::WindowEvent::InitialTreeRequested => {
                    self.a11y_active = true;
                    self.refresh_a11y();
                }
                accesskit_winit::WindowEvent::ActionRequested(request) => {
                    self.a11y_action(request);
                }
                accesskit_winit::WindowEvent::AccessibilityDeactivated => {
                    self.a11y_active = false;
                }
            },
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
            self.save_and_close();
            event_loop.exit();
            return;
        }
        self.tick_agents();
        self.tick_save();
        self.tick_selection_autoscroll();
        if self.a11y_active {
            self.refresh_a11y();
        }
        if self.quit {
            self.save_and_close();
            event_loop.exit();
            return;
        }
        // Repaint only when something changed. A pty read, a child exit or a
        // socket request arrives as `UserEvent::Redraw`; everything the window
        // does itself already called `request_redraw`.
        let blink_on = self.blink_on();
        if blink_on != self.blink_shown {
            self.blink_shown = blink_on;
            self.request_redraw();
        }
        self.expire_flashes();
        // The agent poll always schedules a deadline, so `Wait` is only a
        // fallback for a build with the poll compiled out.
        event_loop.set_control_flow(match self.next_deadline(Instant::now()) {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.server.take();
    }
}

impl Gui {
    /// Cursor blink phase: `true` draws the cursor. The loop wakes on the phase
    /// change, not on a frame timer.
    fn blink_on(&self) -> bool {
        let step = BLINK_INTERVAL.as_millis() as u64;
        (self.started.elapsed().as_millis() as u64 / step.max(1)).is_multiple_of(2)
    }

    fn cursor_blinks(&self) -> bool {
        self.window_focused && !lock(&self.state).surfaces.is_empty()
    }

    /// Drop bells whose flash has finished, repainting once so the pane border
    /// returns to normal.
    fn expire_flashes(&mut self) {
        let before = self.flashes.len();
        self.flashes.retain(|_, at| at.elapsed() < FLASH_DURATION);
        if self.flashes.len() != before {
            self.request_redraw();
        }
    }

    /// When the loop next has to run with nothing else waking it: the blink
    /// edge, a flash ending, the autosave debounce, an agent sweep, or a drag
    /// that keeps auto-scrolling.
    fn next_deadline(&self, now: Instant) -> Option<Instant> {
        let mut next: Option<Instant> = None;
        let mut consider = |at: Instant| {
            next = Some(match next {
                Some(current) => current.min(at),
                None => at,
            });
        };
        if self.cursor_blinks() {
            consider(next_blink_edge(self.started, now));
        }
        for at in self.flashes.values() {
            let end = *at + FLASH_DURATION;
            if end > now {
                consider(end);
            }
        }
        if let Some(due) = self.save_due
            && due > now
        {
            consider(due);
        }
        consider(self.last_detect + Duration::from_secs(1));
        consider(self.last_sweep + Duration::from_secs(30));
        if self.smoke.is_some() || (self.smoke_png.is_some() && !self.captured) {
            // A smoke run wakes on a frame cadence instead of spinning, and
            // exits once the frame is captured.
            consider(now + FRAME_INTERVAL);
        }
        if self.autoscroll {
            consider(now + FRAME_INTERVAL);
        }
        next
    }

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
            match self.dispatch("agent.detect", params) {
                // The core answers `{"agent": null}` when it recognised
                // nothing, so the inner field is what says a row appeared.
                Ok(result) if result.get("agent").is_some_and(|agent| !agent.is_null()) => {
                    self.request_redraw();
                }
                Ok(_) => {}
                Err(error) => eprintln!("berth-gui: agent.detect: {error}"),
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
                self.request_redraw();
            }
        }
    }

    /// Drag past the top or bottom of a pane and the view scrolls, extending the
    /// selection with it.
    fn tick_selection_autoscroll(&mut self) {
        self.autoscroll = false;
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        if !self
            .selection
            .get(&surface_id)
            .is_some_and(|selection| selection.dragging)
        {
            return;
        }
        let nav = self.nav();
        let pane = {
            let state = lock(&self.state);
            ui::build_panes(&state, nav.area, nav.cell.height, nav.tab_height, None)
                .0
                .into_iter()
                .find(|pane| pane.surface.as_deref() == Some(surface_id.as_str()))
        };
        let Some(pane) = pane else {
            return;
        };
        let y = self.pointer.1.max(0.0) as usize;
        let rows = (pane.content.h / nav.cell.height.max(1)).max(1);
        let (older, row) = if y < pane.content.y {
            (true, 0u16)
        } else if y >= pane.content.bottom() {
            (false, (rows - 1) as u16)
        } else {
            return;
        };
        let current = self.scroll.get(&surface_id).copied().unwrap_or(0);
        let target = if older {
            current + 1
        } else {
            current.saturating_sub(1)
        };
        if target == current {
            return;
        }
        self.autoscroll = true;
        match self.host.set_scrollback(&surface_id, target) {
            Ok(applied) => {
                if applied == current {
                    return;
                }
                let shift = applied as i32 - current as i32;
                self.scroll.insert(surface_id.clone(), applied);
                if let Some(selection) = self.selection.get_mut(&surface_id) {
                    selection.shift(shift, u16::MAX);
                    selection.head = Cell {
                        row,
                        col: selection.head.col,
                    };
                }
                self.request_redraw();
            }
            Err(error) => eprintln!("berth-gui: autoscroll: {error}"),
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
                log::line(&format!("autosave failed: {error}"));
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
        if self.font.is_none() {
            self.font = self.font_job.take().and_then(|mut job| job.take());
        }
        if self.font.is_none() {
            let pixels = self.config.font_pixels(self.scale);
            self.font = Some(FontRenderer::new(pixels, &self.config.font_family));
        }
        self.font.as_mut().expect("font just created")
    }

    fn cell(&mut self) -> CellSize {
        self.font().cell()
    }

    fn chrome_font(&mut self) -> &mut FontRenderer {
        if self.chrome_font.is_none() {
            self.chrome_font = self.chrome_job.take().and_then(|mut job| job.take());
        }
        if self.chrome_font.is_none() {
            self.chrome_font = Some(FontRenderer::new(
                CHROME_FONT_SIZE * self.scale as f32,
                &self.config.font_family,
            ));
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
        ui::active_surface(&lock(&self.state))
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
                if let Err(error) = desktop::notify("Berth", &body) {
                    log::line(&format!("desktop notification failed: {error}"));
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
            if self.config.close_exited_panes {
                self.close_surface(&surface_id);
                self.ensure_workspace();
                continue;
            }
            let params = serde_json::json!({
                "surface_id": surface_id,
                "exited": true,
            });
            if let Err(error) = self.dispatch("surface.update", params) {
                eprintln!("berth-gui: surface.update exited: {error}");
            }
        }
    }

    /// Keep the window usable when the last pane exits and its workspace closes.
    fn ensure_workspace(&mut self) {
        if !lock(&self.state).workspaces.is_empty() {
            return;
        }
        if let Err(error) = self.dispatch("workspace.create", serde_json::json!({"name": "1"})) {
            eprintln!("berth-gui: workspace.create after exit: {error}");
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
            let pixel_width = pane.content.w.min(u32::MAX as usize) as u32;
            let pixel_height = pane.content.h.min(u32::MAX as usize) as u32;
            if self.sized.get(surface_id) == Some(&(cols, rows, pixel_width, pixel_height)) {
                continue;
            }
            let params = serde_json::json!({
                "surface_id": surface_id,
                "cols": cols,
                "rows": rows,
                "pixel_width": pixel_width,
                "pixel_height": pixel_height,
            });
            match self.dispatch("surface.resize", params) {
                Ok(_) => {
                    self.sized
                        .insert(surface_id.clone(), (cols, rows, pixel_width, pixel_height));
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
        let trace_started = Instant::now();
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
                self.sidebar_scroll,
            )
            .rows
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
            let blink_on = self.blink_on();
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
                if let Drag::Tab {
                    surface_id,
                    pane_id,
                    ..
                } = &self.drag
                    && &pane.pane_id == pane_id
                {
                    if let Some(tab) = pane.tabs.iter().find(|tab| &tab.id == surface_id) {
                        painter.stroke(tab.rect, theme.accent);
                    }
                    let x = self.pointer.0.max(0.0) as usize;
                    let target = ui::tab_index_at(&pane.tabs, x);
                    let marker = pane
                        .tabs
                        .last()
                        .filter(|tab| x >= tab.rect.x + tab.rect.w)
                        .map(|tab| tab.rect.right())
                        .or_else(|| pane.tabs.get(target).map(|tab| tab.rect.x))
                        .unwrap_or_else(|| pane.tab_bar.right());
                    painter.fill(
                        Rect::new(marker.saturating_sub(1), pane.tab_bar.y, 2, pane.tab_bar.h),
                        theme.accent,
                    );
                }
                let Some(surface_id) = &pane.surface else {
                    continue;
                };
                let Ok(screen) = self.host.screen_view(surface_id) else {
                    continue;
                };
                let selection = self.selection.get(surface_id).copied();
                let exited = !self.host.is_alive(surface_id);
                let search = self
                    .search_highlight
                    .as_ref()
                    .filter(|hit| &hit.surface_id == surface_id)
                    .map(|hit| (hit.line, hit.col, hit.len));
                let options = ScreenOptions {
                    theme: &theme,
                    focused: pane.focused,
                    cursor_visible: pane.focused && self.window_focused && blink_on,
                    selection: selection.as_ref(),
                    exited,
                    search,
                };
                let font = self.font();
                painter.screen(font, &screen, pane.content, &options);
                let cell = font.cell();
                let first_line = screen.scrollback_len.saturating_sub(screen.scrollback);
                for image in &screen.images {
                    let row = image.anchor_line as isize - first_line as isize;
                    let x = pane.content.x as isize + (image.col * cell.width) as isize;
                    let y = pane.content.y as isize + row * cell.height as isize;
                    let mut width = image.width as usize;
                    let mut height = image.height as usize;
                    let mut sized_width = false;
                    let mut sized_height = false;
                    match image.target_width {
                        Dimension::Cells(cells) => {
                            width = cells * cell.width;
                            sized_width = true;
                        }
                        Dimension::Pixels(pixels) => {
                            width = pixels as usize;
                            sized_width = true;
                        }
                        Dimension::Auto => {}
                    }
                    match image.target_height {
                        Dimension::Cells(cells) => {
                            height = cells * cell.height;
                            sized_height = true;
                        }
                        Dimension::Pixels(pixels) => {
                            height = pixels as usize;
                            sized_height = true;
                        }
                        Dimension::Auto => {}
                    }
                    if sized_width && !sized_height {
                        height =
                            (width * image.height as usize / image.width.max(1) as usize).max(1);
                    } else if sized_height && !sized_width {
                        width =
                            (height * image.width as usize / image.height.max(1) as usize).max(1);
                    }
                    painter.image(
                        x as i32,
                        y as i32,
                        width,
                        height,
                        image.pixels.as_slice(),
                        image.width,
                        image.height,
                        pane.content,
                    );
                }
                if pane.focused
                    && let Some(preedit) = self.preedit.clone()
                {
                    let (row, col) = screen.cursor;
                    let origin = (
                        pane.content.x + usize::from(col) * cell.width,
                        pane.content.y + usize::from(row) * cell.height,
                    );
                    let text_width = preedit.chars().count() * cell.width;
                    painter.fill(
                        Rect::new(origin.0, origin.1, text_width.max(cell.width), cell.height),
                        theme.terminal_bg,
                    );
                    let font = self.font();
                    painter.text(
                        font,
                        origin,
                        &preedit,
                        font::TextStyle::NORMAL,
                        theme.terminal_fg,
                        pane.content.right().saturating_sub(origin.0),
                    );
                    painter.fill(
                        Rect::new(
                            origin.0,
                            origin.1 + cell.height.saturating_sub(4),
                            text_width.max(cell.width),
                            1,
                        ),
                        theme.accent,
                    );
                }
            }

            if let Some(content) = self.panel_data() {
                let geometry = panel::geometry(
                    Rect::new(0, 0, width, height),
                    self.scale,
                    content.query.is_some(),
                );
                let font = self.chrome_font();
                let row_height = font.cell().height + 8;
                let visible = panel::visible_rows(geometry.rows, row_height);
                let first = panel::first_visible(content.selected, content.rows.len(), visible);
                painter.panel(
                    font,
                    &paint::PanelStyle {
                        theme: &theme,
                        geometry: &geometry,
                        title: &content.title,
                        query: content.query.as_deref(),
                        rows: &content.rows,
                        selected: content.selected,
                        first,
                        hint: &content.hint,
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

        self.update_ime_cursor(width, height);
        if !self.ready {
            trace_phase(self.started, "first paint");
        }
        buffer
            .present()
            .map_err(|error| format!("softbuffer present: {error}"))?;
        if std::env::var_os("BERTH_TRACE").is_some() {
            eprintln!(
                "berth-gui: paint {} ms",
                trace_started.elapsed().as_millis()
            );
        }
        if !self.ready {
            self.ready = true;
            println!(
                "berth-gui: window ready {width}x{height} ({} pane(s), {}x{} cell) in {} ms",
                panes.len(),
                cell.width,
                cell.height,
                self.started.elapsed().as_millis()
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
        let button = ui::new_workspace_button(area);
        painter.text(
            font,
            (
                button.x + button.w.saturating_sub(cell.width) / 2,
                button.y + button.h.saturating_sub(cell.height) / 2,
            ),
            "+",
            font::TextStyle::NORMAL,
            theme.text_secondary,
            cell.width,
        );
        let group_button = ui::new_group_button(area);
        painter.text(
            font,
            (
                group_button.x + 2,
                group_button.y + group_button.h.saturating_sub(cell.height) / 2,
            ),
            "+G",
            font::TextStyle::NORMAL,
            theme.text_tertiary,
            group_button.w.saturating_sub(2),
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

    /// The pane and grid cell under a point, with its mouse-tracking settings.
    fn mouse_target(&mut self, point: (usize, usize)) -> Option<MouseTarget> {
        let nav = self.nav();
        let (panes, _) = {
            let state = lock(&self.state);
            ui::build_panes(&state, nav.area, nav.cell.height, nav.tab_height, None)
        };
        let pane = ui::pane_at(&panes, point.0, point.1)?;
        let surface_id = pane.surface.clone()?;
        if !pane.content.contains(point.0, point.1) {
            return None;
        }
        let view = self.host.screen_view(&surface_id).ok()?;
        Some(MouseTarget {
            surface_id,
            col: ((point.0.saturating_sub(pane.content.x)) / nav.cell.width.max(1)) as u16,
            row: ((point.1.saturating_sub(pane.content.y)) / nav.cell.height.max(1)) as u16,
            mode: view.mouse_mode,
            encoding: view.mouse_encoding,
        })
    }

    fn send_mouse(&self, target: &MouseTarget, report: mouse::Report) {
        if let Some(bytes) = mouse::encode(report, target.encoding)
            && let Err(error) = self.host.send_bytes(&target.surface_id, &bytes)
        {
            eprintln!("berth-gui: mouse: {error}");
        }
    }

    fn modifiers_bits(&self) -> mouse::Modifiers {
        mouse::Modifiers {
            shift: self.modifiers.shift_key(),
            alt: self.modifiers.alt_key(),
            ctrl: self.modifiers.control_key(),
        }
    }

    fn report_button(&mut self, button: MouseButton, released: bool) -> bool {
        let Some(button) = mouse_button(button) else {
            return false;
        };
        let point = (
            self.pointer.0.max(0.0) as usize,
            self.pointer.1.max(0.0) as usize,
        );
        let Some(target) = self.mouse_target(point) else {
            return false;
        };
        if !target.mode.is_on() || self.modifiers.shift_key() {
            return false;
        }
        if !mouse::should_report(target.mode, !released) {
            return false;
        }
        // A program that tracks the mouse consumes the press, so focus has to
        // move here or the pane never becomes the keyboard target.
        if needs_focus_before_report(
            released,
            self.active_surface().as_deref(),
            &target.surface_id,
        ) {
            self.activate(&target.surface_id);
        }
        self.send_mouse(
            &target,
            mouse::Report {
                col: target.col,
                row: target.row,
                button,
                release: released,
                motion: false,
                modifiers: self.modifiers_bits(),
            },
        );
        true
    }

    fn report_wheel(&mut self, up: bool) -> bool {
        let point = (
            self.pointer.0.max(0.0) as usize,
            self.pointer.1.max(0.0) as usize,
        );
        let Some(target) = self.mouse_target(point) else {
            return false;
        };
        if !target.mode.is_on() || self.modifiers.shift_key() {
            return false;
        }
        self.send_mouse(
            &target,
            mouse::Report {
                col: target.col,
                row: target.row,
                button: if up {
                    mouse::Button::WheelUp
                } else {
                    mouse::Button::WheelDown
                },
                release: false,
                motion: false,
                modifiers: self.modifiers_bits(),
            },
        );
        true
    }

    /// Report a horizontal wheel event (xterm buttons 6 and 7). Programs that
    /// do not track the mouse never see it; nothing scrolls sideways here.
    fn report_wheel_horizontal(&mut self, right: bool) -> bool {
        let point = (
            self.pointer.0.max(0.0) as usize,
            self.pointer.1.max(0.0) as usize,
        );
        let Some(target) = self.mouse_target(point) else {
            return false;
        };
        if !target.mode.is_on() {
            return false;
        }
        self.send_mouse(
            &target,
            mouse::Report {
                col: target.col,
                row: target.row,
                button: if right {
                    mouse::Button::WheelRight
                } else {
                    mouse::Button::WheelLeft
                },
                release: false,
                motion: false,
                modifiers: self.modifiers_bits(),
            },
        );
        true
    }

    /// The link under the pointer, if any, with the surface and cell it lives in.
    fn link_under_pointer(&mut self) -> Option<String> {
        if self.overlay.is_some() {
            return None;
        }
        let point = (
            self.pointer.0.max(0.0) as usize,
            self.pointer.1.max(0.0) as usize,
        );
        let target = self.mouse_target(point)?;
        let row = self.host.line_text(&target.surface_id, target.row).ok()?;
        links::link_at(&row, target.col as usize).map(|(_, _, link)| link)
    }

    fn open_link_under_pointer(&mut self) {
        if let Some(link) = self.link_under_pointer()
            && let Err(error) = links::open(&link)
        {
            eprintln!("berth-gui: open {link}: {error}");
        }
    }

    /// Pointer shape for the element under it.
    fn update_cursor_icon(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let point = (
            self.pointer.0.max(0.0) as usize,
            self.pointer.1.max(0.0) as usize,
        );
        let icon = self.cursor_icon_at(point);
        if icon != self.cursor_icon {
            self.cursor_icon = icon;
            window.set_cursor(icon);
        }
    }

    fn cursor_icon_at(&mut self, point: (usize, usize)) -> CursorIcon {
        if self.overlay.is_some() {
            return CursorIcon::Default;
        }
        let nav = self.nav();
        let (panes, dividers) = {
            let state = lock(&self.state);
            ui::build_panes(&state, nav.area, nav.cell.height, nav.tab_height, None)
        };
        if let Some(divider) = dividers
            .iter()
            .find(|divider| divider.grab.contains(point.0, point.1))
        {
            return match divider.orientation {
                Orientation::Horizontal => CursorIcon::ColResize,
                Orientation::Vertical => CursorIcon::RowResize,
            };
        }
        if self.sidebar_visible
            && point.0 < nav.area.x
            && (point.0 as f64 - self.sidebar_pixels() as f64).abs() <= DIVIDER_EDGE as f64
        {
            return CursorIcon::ColResize;
        }
        let Some(pane) = ui::pane_at(&panes, point.0, point.1) else {
            return CursorIcon::Default;
        };
        if pane.tab_bar.contains(point.0, point.1) {
            return CursorIcon::Pointer;
        }
        if pane.content.contains(point.0, point.1) {
            if self.reporting_to_pty(point) {
                return CursorIcon::Default;
            }
            if self.link_under_pointer().is_some() {
                return CursorIcon::Pointer;
            }
            return CursorIcon::Text;
        }
        CursorIcon::Default
    }

    fn reporting_to_pty(&mut self, point: (usize, usize)) -> bool {
        self.mouse_target(point)
            .map(|target| target.mode.is_on())
            .unwrap_or(false)
    }

    fn report_motion(&mut self) -> bool {
        let Some(held) = self.mouse_down else {
            return false;
        };
        let Some(button) = mouse_button(held) else {
            return false;
        };
        let point = (
            self.pointer.0.max(0.0) as usize,
            self.pointer.1.max(0.0) as usize,
        );
        let Some(target) = self.mouse_target(point) else {
            return false;
        };
        let mode = target.mode;
        if !matches!(mode, MouseMode::Drag | MouseMode::Motion) {
            return false;
        }
        self.send_mouse(
            &target,
            mouse::Report {
                col: target.col,
                row: target.row,
                button,
                release: false,
                motion: true,
                modifiers: self.modifiers_bits(),
            },
        );
        true
    }

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
        let primary = self.modifiers.super_key()
            || (!cfg!(target_os = "macos") && self.modifiers.control_key());
        if button == MouseButton::Left && primary {
            self.open_link_under_pointer();
            return;
        }
        if self.report_button(button, false) {
            self.mouse_down = Some(button);
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
            self.sidebar_scroll,
        )
        .rows;
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
                    let value = lock(&self.state)
                        .surface(&tab.id)
                        .and_then(|surface| surface.user_title.clone())
                        .unwrap_or_default();
                    self.open_rename(RenameTarget::Tab(tab.id.clone()), value);
                    return;
                }
                self.record_click(point, ClickTarget::Tab(tab.id.clone()));
                self.activate(&tab.id);
                self.drag = Drag::Tab {
                    surface_id: tab.id.clone(),
                    pane_id: pane.pane_id.clone(),
                    start_x: point.0,
                };
                return;
            }
            if pane.tab_bar.contains(point.0, point.1) {
                if self.same_click(point, ClickTarget::TabStrip(pane.pane_id.clone())) {
                    self.new_tab_in(&pane.pane_id);
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
            self.last_selection = Some(surface_id.clone());
            return;
        }
        if self.same_click(point, ClickTarget::Pane(surface_id.clone())) {
            let Ok(line) = self.host.line_text(surface_id, target.row) else {
                return;
            };
            let selection = Selection::word_at(target, &line);
            self.selection.insert(surface_id.clone(), selection);
            self.last_selection = Some(surface_id.clone());
            return;
        }
        self.record_click(point, ClickTarget::Pane(surface_id.clone()));
        self.selection
            .insert(surface_id.clone(), Selection::new(target));
        self.last_selection = Some(surface_id.clone());
    }

    fn sidebar_pressed(&mut self, point: (usize, usize), nav: Nav) {
        if ui::new_group_button(nav.sidebar).contains(point.0, point.1) {
            self.new_group();
            return;
        }
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
                self.sidebar_scroll,
            )
            .rows
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
            ui::RowKind::Group {
                id,
                collapsed,
                color,
                ..
            } => {
                if self.modifiers.shift_key() {
                    let next = next_row_color(color.as_deref());
                    if let Err(error) = self.dispatch(
                        "group.update",
                        serde_json::json!({"group_id": id, "color": next}),
                    ) {
                        eprintln!("berth-gui: group colour: {error}");
                    }
                    return;
                }
                if self.same_click(point, ClickTarget::GroupRow(id.clone())) {
                    let name = lock(&self.state)
                        .groups
                        .iter()
                        .find(|group| &group.id == id)
                        .map(|group| group.name.clone())
                        .unwrap_or_default();
                    self.open_rename(RenameTarget::Group(id.clone()), name);
                    return;
                }
                self.record_click(point, ClickTarget::GroupRow(id.clone()));
                let params = serde_json::json!({
                    "group_id": id,
                    "collapsed": !*collapsed,
                });
                if let Err(error) = self.dispatch("group.update", params) {
                    eprintln!("berth-gui: group.update: {error}");
                }
            }
            ui::RowKind::Workspace(workspace) => {
                if self.modifiers.shift_key() {
                    let next = next_row_color(workspace.color.as_deref());
                    if let Err(error) = self.dispatch(
                        "workspace.update",
                        serde_json::json!({"workspace_id": workspace.id, "color": next}),
                    ) {
                        eprintln!("berth-gui: workspace colour: {error}");
                    }
                    return;
                }
                if self.same_click(point, ClickTarget::WorkspaceRow(workspace.id.clone())) {
                    let name = lock(&self.state)
                        .workspace(&workspace.id)
                        .map(|workspace| workspace.name.clone())
                        .unwrap_or_default();
                    self.open_rename(RenameTarget::Workspace(workspace.id.clone()), name);
                    return;
                }
                self.record_click(point, ClickTarget::WorkspaceRow(workspace.id.clone()));
                self.select_workspace(&workspace.id);
                self.drag = Drag::Workspace {
                    id: workspace.id.clone(),
                    last_target: None,
                };
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

    /// Wheel scrolling for the sidebar, clamped so the last row can be reached.
    fn scroll_sidebar(&mut self, lines: i32) {
        let (width, height) = self
            .window
            .as_ref()
            .map(|window| {
                let size = window.inner_size();
                (size.width as usize, size.height as usize)
            })
            .unwrap_or((0, 0));
        let area = Rect::new(0, 0, self.sidebar_pixels(), height);
        let row_height = self.workspace_row_height() as usize;
        let capacity = area
            .h
            .saturating_sub(ui::SIDEBAR_HEADER)
            .checked_div(row_height.max(1))
            .unwrap_or(1)
            .max(1);
        let total = {
            let state = lock(&self.state);
            ui::build_sidebar(&state, area, self.workspace_row_height(), &self.theme, 0).total
        };
        let max = total.saturating_sub(capacity);
        let current = self.sidebar_scroll as i32;
        let _ = width;
        self.sidebar_scroll = (current + lines).clamp(0, max as i32) as usize;
        self.request_redraw();
    }

    /// Live workspace reorder while a sidebar row is dragged.
    fn drag_workspace(&mut self, point: (usize, usize)) {
        let (id, previous) = match &self.drag {
            Drag::Workspace { id, last_target } => (id.clone(), last_target.clone()),
            _ => return,
        };
        let nav = self.nav();
        if nav.area.x == 0 {
            return;
        }
        let rows = {
            let state = lock(&self.state);
            ui::build_sidebar(
                &state,
                Rect::new(0, 0, nav.area.x, nav.height),
                self.workspace_row_height(),
                &self.theme,
                self.sidebar_scroll,
            )
            .rows
        };
        let Some(target) = rows.iter().find_map(|row| match &row.kind {
            ui::RowKind::Workspace(workspace)
                if row.rect.contains(point.0, point.1) && workspace.id != id =>
            {
                Some(workspace.id.clone())
            }
            _ => None,
        }) else {
            return;
        };
        if previous.as_deref() == Some(target.as_str()) {
            return;
        }
        let index = {
            let state = lock(&self.state);
            state
                .workspaces
                .iter()
                .position(|workspace| workspace.id == target)
        };
        let Some(index) = index else {
            return;
        };
        if let Drag::Workspace { last_target, .. } = &mut self.drag {
            *last_target = Some(target);
        }
        let params = serde_json::json!({
            "workspace_id": id,
            "index": index,
        });
        if let Err(error) = self.dispatch("workspace.move", params) {
            eprintln!("berth-gui: workspace.move: {error}");
        }
        self.request_redraw();
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
                    self.last_selection = Some(surface_id.clone());
                    self.request_redraw();
                }
            }
            Drag::Tab { .. } => {}
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
            Drag::Workspace { .. } => self.drag_workspace(point),
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
        let (x, y) = match delta {
            MouseScrollDelta::LineDelta(x, y) => (f64::from(x), f64::from(y)),
            MouseScrollDelta::PixelDelta(position) => (position.x, position.y),
        };
        // A trackpad reports both axes. Horizontal gestures go to the program
        // as buttons 6/7; there is no horizontal scrollback to fall back on.
        if let Some(right) = mouse::horizontal_gesture(x, y) {
            self.report_wheel_horizontal(right);
            return;
        }
        if y == 0.0 {
            return;
        }
        if self.report_wheel(y > 0.0) {
            return;
        }
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
        if nav.area.x > 0 && point.0 < nav.area.x {
            self.scroll_sidebar(lines);
            return;
        }
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
                self.sized.remove(surface_id);
                self.last_meta.remove(surface_id);
                self.flashes.remove(surface_id);
                self.exit_announced.remove(surface_id);
            }
            Err(error) => eprintln!("berth-gui: close {surface_id}: {error}"),
        }
    }

    fn new_tab(&mut self) {
        // One lock: `ui::active_surface` reads the state by reference, so this
        // holds the guard while it asks, instead of locking again inside.
        let pane_id = {
            let state = lock(&self.state);
            let Some(surface_id) = ui::active_surface(&state) else {
                return;
            };
            state
                .surface(&surface_id)
                .map(|surface| surface.pane_id.clone())
        };
        if let Some(pane_id) = pane_id {
            self.new_tab_in(&pane_id);
        }
    }

    /// Add a tab to one pane: the strip that was double-clicked, or the focused
    /// pane for `⌘T`.
    fn new_tab_in(&mut self, pane_id: &str) {
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

    /// Create a group and put the selected workspace in it.
    fn new_group(&mut self) {
        let name = format!("Group {}", lock(&self.state).groups.len() + 1);
        match self.dispatch("group.create", serde_json::json!({"name": name})) {
            Ok(result) => {
                let id = result["group"]["id"].as_str().map(str::to_string);
                if let (Some(id), Some(workspace)) = (
                    id,
                    self.active_surface().and_then(|surface| {
                        lock(&self.state)
                            .surface(&surface)
                            .map(|surface| surface.workspace_id.clone())
                    }),
                ) && let Err(error) = self.dispatch(
                    "workspace.update",
                    serde_json::json!({"workspace_id": workspace, "group_id": id}),
                ) {
                    eprintln!("berth-gui: workspace group: {error}");
                }
            }
            Err(error) => eprintln!("berth-gui: group.create: {error}"),
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

    /// The surface with a selection to copy: the focused one when it has one,
    /// else the last surface that held a selection.
    fn selection_surface(&self) -> Option<String> {
        if let Some(id) = self.active_surface()
            && self.selection.contains_key(&id)
        {
            return Some(id);
        }
        self.last_selection
            .as_ref()
            .filter(|id| self.selection.contains_key(*id))
            .cloned()
    }

    fn copy_selection(&mut self) {
        let Some(surface_id) = self.selection_surface() else {
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

    /// Copy the finished selection without a chord, like a terminal with
    /// copy-on-select. A single cell (a plain click) never replaces the
    /// clipboard.
    fn copy_on_select(&mut self) {
        if !self.config.copy_on_select {
            return;
        }
        let Some(surface_id) = self.selection_surface() else {
            return;
        };
        let Some(selection) = self.selection.get(&surface_id) else {
            return;
        };
        let (start, end) = selection.span();
        if start == end {
            return;
        }
        self.copy_selection();
    }

    fn paste(&mut self) {
        if self.active_surface().is_none() {
            return;
        }
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
        if text.lines().count() > 1 {
            self.overlay = Some(Overlay::Confirm { text });
            return;
        }
        self.send_paste(&text);
    }

    /// Send pasted text to the active surface, wrapped when the program asked
    /// for bracketed paste.
    fn send_paste(&mut self, text: &str) {
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        let bracketed = self
            .host
            .screen_view(&surface_id)
            .map(|view| view.bracketed_paste)
            .unwrap_or(false);
        let payload = if bracketed {
            format!("\x1b[200~{text}\x1b[201~")
        } else {
            text.to_string()
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
                self.overlay = Some(Overlay::Settings {
                    selected: 0,
                    recording: None,
                });
            }
            Command::Search => self.open_search(),
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
        self.overlay
            .as_ref()
            .map_or(0, |overlay| self.overlay_rows_len(overlay))
    }

    /// Row count for one panel. `overlay_key` takes the overlay out of `self`,
    /// so it must measure the value it holds, not `self.overlay`.
    fn overlay_rows_len(&self, overlay: &Overlay) -> usize {
        match overlay {
            Overlay::Palette { query, .. } => Self::palette_entries(query).len(),
            Overlay::Attention { .. } => self.attention_entries().len(),
            Overlay::Settings { recording, .. } => {
                settings::rows(&self.config, *recording, self.hooks_notice.as_deref()).len()
            }
            Overlay::Confirm { text } => confirm_lines(text).len(),
            Overlay::Rename { .. } => 0,
            Overlay::Search { matches, .. } => matches.len(),
        }
    }

    fn overlay_key(&mut self, event: &KeyEvent) {
        let Some(mut overlay) = self.overlay.take() else {
            return;
        };
        let current = overlay.selected();
        let mut next_selected: Option<usize> = None;
        let mut close = false;
        let mut activate = false;
        let mut rename_commit: Option<(RenameTarget, String)> = None;
        match &mut overlay {
            Overlay::Palette { query, .. } => match &event.logical_key {
                Key::Named(NamedKey::Escape) => close = true,
                Key::Named(NamedKey::Enter) => activate = true,
                Key::Named(NamedKey::ArrowDown) => {
                    next_selected = Some(panel::move_selection(
                        current,
                        1,
                        Self::palette_entries(query).len(),
                    ));
                }
                Key::Named(NamedKey::ArrowUp) => {
                    next_selected = Some(panel::move_selection(
                        current,
                        -1,
                        Self::palette_entries(query).len(),
                    ));
                }
                Key::Named(NamedKey::Backspace) => {
                    query.pop();
                    next_selected = Some(0);
                }
                Key::Character(text)
                    if !self.modifiers.control_key()
                        && !self.modifiers.super_key()
                        && !self.modifiers.alt_key() =>
                {
                    query.push_str(text);
                    next_selected = Some(0);
                }
                _ => {}
            },
            Overlay::Attention { .. } => match &event.logical_key {
                Key::Named(NamedKey::Escape) => close = true,
                Key::Named(NamedKey::Enter) => activate = true,
                Key::Named(NamedKey::ArrowDown) => {
                    next_selected = Some(panel::move_selection(
                        current,
                        1,
                        self.attention_entries().len(),
                    ));
                }
                Key::Named(NamedKey::ArrowUp) => {
                    next_selected = Some(panel::move_selection(
                        current,
                        -1,
                        self.attention_entries().len(),
                    ));
                }
                _ => {}
            },
            Overlay::Settings {
                selected,
                recording,
            } => {
                self.settings_key(event, &mut close, &mut activate, selected, recording);
            }
            Overlay::Search {
                surface_id,
                lines,
                query,
                matches,
                selected,
            } => {
                let mut changed = false;
                let mut jump = false;
                match &event.logical_key {
                    Key::Named(NamedKey::Escape) => {
                        self.search_highlight = None;
                        close = true;
                    }
                    Key::Named(NamedKey::Enter) => {
                        if !matches.is_empty() {
                            let delta = if self.modifiers.shift_key() { -1 } else { 1 };
                            let next = (*selected as i32 + delta).rem_euclid(matches.len() as i32)
                                as usize;
                            *selected = next;
                            jump = true;
                        }
                    }
                    Key::Named(NamedKey::ArrowDown) => {
                        if !matches.is_empty() {
                            *selected = (*selected + 1) % matches.len();
                            jump = true;
                        }
                    }
                    Key::Named(NamedKey::ArrowUp) => {
                        if !matches.is_empty() {
                            *selected = (*selected + matches.len() - 1) % matches.len();
                            jump = true;
                        }
                    }
                    Key::Named(NamedKey::Backspace) => {
                        query.pop();
                        changed = true;
                    }
                    Key::Character(text)
                        if !self.modifiers.control_key()
                            && !self.modifiers.super_key()
                            && !self.modifiers.alt_key() =>
                    {
                        query.push_str(text);
                        changed = true;
                    }
                    _ => {}
                }
                if changed {
                    *matches = Self::search_matches(lines, query);
                    *selected = 0;
                    jump = true;
                }
                if jump && let Some(hit) = matches.get(*selected) {
                    self.jump_to_match(surface_id, hit);
                }
            }
            Overlay::Rename { target, value } => match &event.logical_key {
                Key::Named(NamedKey::Escape) => close = true,
                Key::Named(NamedKey::Enter) => {
                    rename_commit = Some((target.clone(), value.clone()));
                    close = true;
                }
                Key::Named(NamedKey::Backspace) => {
                    value.pop();
                }
                Key::Character(text)
                    if !self.modifiers.control_key()
                        && !self.modifiers.super_key()
                        && !self.modifiers.alt_key() =>
                {
                    value.push_str(text);
                }
                _ => {}
            },
            Overlay::Confirm { .. } => match &event.logical_key {
                Key::Named(NamedKey::Escape) => close = true,
                Key::Named(NamedKey::Enter) => activate = true,
                _ => close = true,
            },
        }
        if let Some((target, value)) = rename_commit {
            self.commit_rename(&target, &value);
        }
        if let Some(index) = next_selected {
            overlay.select(index);
        }
        let clamped = panel::clamp_selection(overlay.selected(), self.overlay_rows_len(&overlay));
        overlay.select(clamped);
        if close {
            return;
        }
        self.overlay = Some(overlay);
        if activate {
            self.overlay_activate();
        } else {
            self.request_redraw();
        }
    }

    /// Settings keys: arrows adjust, Enter toggles or records a chord. Recording
    /// swallows the next key so a chord that is already a shortcut still works.
    fn settings_key(
        &mut self,
        event: &KeyEvent,
        close: &mut bool,
        activate: &mut bool,
        selected: &mut usize,
        recording: &mut Option<config::Action>,
    ) {
        let rows = settings::rows(&self.config, *recording, self.hooks_notice.as_deref());
        let len = rows.len();
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                if recording.is_some() {
                    *recording = None;
                } else {
                    *close = true;
                }
            }
            Key::Named(NamedKey::ArrowUp) => {
                *selected = panel::move_selection(*selected, -1, len);
            }
            Key::Named(NamedKey::ArrowDown) => {
                *selected = panel::move_selection(*selected, 1, len);
            }
            Key::Named(NamedKey::ArrowLeft) | Key::Named(NamedKey::ArrowRight) => {
                if recording.is_some() {
                    return;
                }
                let delta = if matches!(event.logical_key, Key::Named(NamedKey::ArrowLeft)) {
                    -1
                } else {
                    1
                };
                if let Some(row) = rows.get(*selected) {
                    let change = settings::adjust(&mut self.config, row.field, delta);
                    self.apply_setting(change);
                }
            }
            Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Space) => {
                if let Some(row) = rows.get(*selected) {
                    match row.field {
                        settings::Field::Shortcut(action) => *recording = Some(action),
                        field => {
                            let change = settings::toggle(&mut self.config, field);
                            self.apply_setting(change);
                        }
                    }
                }
            }
            Key::Named(NamedKey::Backspace) => {
                if let Some(row) = rows.get(*selected)
                    && let settings::Field::Shortcut(action) = row.field
                {
                    let change = settings::set_shortcut(&mut self.config, action, "");
                    self.apply_setting(change);
                    *recording = None;
                }
            }
            _ => {
                if let Some(action) = *recording
                    && let Some(chord) = keys::chord_for(&event.logical_key, self.modifiers)
                {
                    let change = settings::set_shortcut(&mut self.config, action, &chord);
                    self.apply_setting(change);
                    *recording = None;
                }
            }
        }
        let _ = activate;
    }

    /// Run the CLI shipped beside the window to refresh the agent hooks.
    fn reinstall_hooks(&mut self) {
        let sibling = std::env::current_exe()
            .ok()
            .and_then(|exe| {
                exe.parent()
                    .map(|dir| dir.join(if cfg!(windows) { "berth.exe" } else { "berth" }))
            })
            .filter(|path| path.is_file());
        let program = sibling.unwrap_or_else(|| "berth".into());
        let result = std::process::Command::new(&program)
            .args(["hooks", "install", "all"])
            .output();
        self.hooks_notice = Some(match result {
            Ok(output) if output.status.success() => "installed".to_string(),
            Ok(output) => format!("failed: {}", String::from_utf8_lossy(&output.stderr).trim()),
            Err(error) => format!("failed: {error}"),
        });
        log::line(&format!(
            "hooks: {} ({})",
            self.hooks_notice.as_deref().unwrap_or(""),
            program.display()
        ));
    }

    fn apply_setting(&mut self, change: settings::Change) {
        if change == settings::Change::None {
            return;
        }
        if change == settings::Change::Font {
            self.font = None;
            self.chrome_font = None;
            self.sized.clear();
        }
        if change == settings::Change::Shell {
            // Panes already running keep their program; the next one uses it.
            self.host.set_default_shell(self.config.shell.clone());
        }
        if let Err(error) = settings::save(&self.config) {
            log::line(&format!("settings save failed: {error}"));
            eprintln!("berth-gui: settings: {error}");
        }
        self.request_redraw();
    }

    fn open_search(&mut self) {
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        let lines = match self.host.scrollback_lines(&surface_id) {
            Ok(lines) => lines,
            Err(error) => {
                eprintln!("berth-gui: scrollback: {error}");
                return;
            }
        };
        self.overlay = Some(Overlay::Search {
            surface_id,
            lines,
            query: String::new(),
            matches: Vec::new(),
            selected: 0,
        });
        self.request_redraw();
    }

    fn search_matches(lines: &[String], query: &str) -> Vec<SearchMatch> {
        if query.is_empty() {
            return Vec::new();
        }
        let needle = query.to_lowercase();
        let mut matches = Vec::new();
        for (line, text) in lines.iter().enumerate() {
            let haystack = text.to_lowercase();
            let mut from = 0;
            while let Some(found) = haystack[from..].find(&needle) {
                let col = from + found;
                matches.push(SearchMatch {
                    line,
                    col: haystack[..col].chars().count(),
                    len: needle.chars().count(),
                });
                from = col + needle.len().max(1);
                if matches.len() >= 500 {
                    return matches;
                }
            }
        }
        matches
    }

    fn jump_to_match(&mut self, surface_id: &str, hit: &SearchMatch) {
        let Ok(view) = self.host.screen_view(surface_id) else {
            return;
        };
        let target = view.scrollback_len.saturating_sub(hit.line);
        if let Ok(applied) = self.host.set_scrollback(surface_id, target) {
            self.scroll.insert(surface_id.to_string(), applied);
        }
        self.search_highlight = Some(SearchHighlight {
            surface_id: surface_id.to_string(),
            line: hit.line,
            col: hit.col,
            len: hit.len,
        });
        self.request_redraw();
    }

    fn open_rename(&mut self, target: RenameTarget, value: String) {
        self.overlay = Some(Overlay::Rename { target, value });
        self.request_redraw();
    }

    fn commit_rename(&mut self, target: &RenameTarget, value: &str) {
        let name = value.trim().to_string();
        let result = match target {
            RenameTarget::Tab(surface_id) => self.dispatch(
                "surface.rename",
                serde_json::json!({"surface_id": surface_id, "title": name}),
            ),
            RenameTarget::Workspace(workspace_id) => self.dispatch(
                "workspace.update",
                serde_json::json!({"workspace_id": workspace_id, "name": name}),
            ),
            RenameTarget::Group(group_id) => self.dispatch(
                "group.update",
                serde_json::json!({"group_id": group_id, "name": name}),
            ),
        };
        if let Err(error) = result {
            eprintln!("berth-gui: rename: {error}");
        }
    }

    /// Finish a tab drag: move the tab to the slot under the pointer.
    fn finish_tab_drag(&mut self, surface_id: &str, pane_id: &str, start_x: usize, end_x: usize) {
        if start_x.abs_diff(end_x) < 6 {
            return;
        }
        let nav = self.nav();
        let (panes, _) = {
            let state = lock(&self.state);
            ui::build_panes(&state, nav.area, nav.cell.height, nav.tab_height, None)
        };
        let Some(pane) = panes.iter().find(|pane| pane.pane_id == pane_id) else {
            return;
        };
        let target = ui::tab_index_at(&pane.tabs, end_x);
        if let Err(error) = self.dispatch(
            "surface.move",
            serde_json::json!({"surface_id": surface_id, "index": target}),
        ) {
            eprintln!("berth-gui: surface.move: {error}");
        }
    }

    /// Title, optional query, rows, selection and footer hint for the panel.
    fn panel_data(&self) -> Option<PanelContent> {
        match self.overlay.as_ref()? {
            Overlay::Palette { query, selected } => Some(PanelContent {
                title: "Command Palette".to_string(),
                query: Some(query.clone()),
                rows: Self::palette_rows(query, &self.config.shortcuts),
                selected: *selected,
                hint: "↑↓ move · enter run · esc close".to_string(),
            }),
            Overlay::Attention { selected } => Some(PanelContent {
                title: "Attention".to_string(),
                query: None,
                rows: self
                    .attention_entries()
                    .into_iter()
                    .map(|(_, _, _, row)| row)
                    .collect(),
                selected: *selected,
                hint: "↑↓ move · enter jump and acknowledge · esc close".to_string(),
            }),
            Overlay::Search {
                lines,
                query,
                matches,
                selected,
                ..
            } => Some(PanelContent {
                title: if matches.is_empty() {
                    "Search".to_string()
                } else {
                    format!("Search {}/{}", selected + 1, matches.len())
                },
                query: Some(query.clone()),
                rows: matches
                    .iter()
                    .map(|hit| panel::PanelRow {
                        label: lines
                            .get(hit.line)
                            .map(|line| line.trim().to_string())
                            .unwrap_or_default(),
                        detail: format!("line {}", hit.line + 1),
                        color: None,
                    })
                    .collect(),
                selected: *selected,
                hint: "enter next · shift+enter prev · esc close".to_string(),
            }),
            Overlay::Rename { value, .. } => Some(PanelContent {
                title: "Rename".to_string(),
                query: Some(value.clone()),
                rows: Vec::new(),
                selected: 0,
                hint: "enter save · esc cancel".to_string(),
            }),
            Overlay::Confirm { text } => Some(PanelContent {
                title: format!("Paste {} lines?", confirm_lines(text).len()),
                query: None,
                rows: confirm_lines(text)
                    .into_iter()
                    .map(|line| panel::PanelRow {
                        label: line,
                        detail: String::new(),
                        color: None,
                    })
                    .collect(),
                selected: 0,
                hint: "enter paste · esc cancel".to_string(),
            }),
            Overlay::Settings {
                selected,
                recording,
            } => Some(PanelContent {
                title: "Settings".to_string(),
                query: None,
                rows: settings::rows(&self.config, *recording, self.hooks_notice.as_deref())
                    .into_iter()
                    .map(|row| panel::PanelRow {
                        label: row.label,
                        detail: row.value,
                        color: None,
                    })
                    .collect(),
                selected: *selected,
                hint: "↑↓ move · ←→ adjust · enter toggle or record · esc close".to_string(),
            }),
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
        let has_query = matches!(
            self.overlay,
            Some(Overlay::Palette { .. } | Overlay::Rename { .. } | Overlay::Search { .. })
        );
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
        let row_height = cell.height + 8;
        let visible = panel::visible_rows(geometry.rows, row_height);
        let selected = self.overlay.as_ref().map(Overlay::selected).unwrap_or(0);
        let first = panel::first_visible(selected, len, visible);
        let rects = panel::row_rects(geometry.rows, len, row_height);
        let Some(index) = rects
            .iter()
            .position(|rect| rect.contains(point.0, point.1))
        else {
            return;
        };
        let index = first + index;
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
            Some(Overlay::Rename { .. }) | Some(Overlay::Search { .. }) => {}
            Some(Overlay::Confirm { text }) => {
                self.send_paste(&text);
            }
            Some(Overlay::Settings {
                selected,
                recording,
            }) => {
                let rows = settings::rows(&self.config, recording, self.hooks_notice.as_deref());
                let mut next = Overlay::Settings {
                    selected,
                    recording,
                };
                if let Some(row) = rows.get(selected) {
                    match row.field {
                        settings::Field::Shortcut(action) => {
                            if let Overlay::Settings { recording, .. } = &mut next {
                                *recording = Some(action);
                            }
                        }
                        settings::Field::ReinstallHooks => self.reinstall_hooks(),
                        field => {
                            let change = settings::toggle(&mut self.config, field);
                            self.apply_setting(change);
                        }
                    }
                }
                self.overlay = Some(next);
            }
            None => {}
        }
        self.request_redraw();
    }

    /// Publish the focused pane's text to the platform accessibility API when it
    /// changed. Called once a second, not per frame.
    fn refresh_a11y(&mut self) {
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        let Ok(read) = self.host.read_text(&surface_id, ReadScope::Screen, 1 << 20) else {
            return;
        };
        if read.text == self.a11y_text {
            return;
        }
        self.a11y_text = read.text.clone();
        let title = {
            let state = lock(&self.state);
            ui::active_workspace(&state)
                .and_then(|id| state.workspace(&id).map(|workspace| workspace.name.clone()))
                .map(|name| format!("Berth — {name}"))
                .unwrap_or_else(|| "Berth".to_string())
        };
        let update = a11y::tree(&title, &read.text);
        if let Some(adapter) = self.a11y.as_mut() {
            adapter.update_if_active(|| update);
        }
    }

    fn a11y_action(&mut self, request: accesskit::ActionRequest) {
        match request.action {
            accesskit::Action::Focus | accesskit::Action::Click => {
                if let Some(surface_id) = self.active_surface() {
                    self.activate(&surface_id);
                }
            }
            accesskit::Action::ReplaceSelectedText => {
                if let Some(accesskit::ActionData::Value(text)) = request.data
                    && let Some(surface_id) = self.active_surface()
                {
                    if let Err(error) = self.host.send_text(&surface_id, &text) {
                        eprintln!("berth-gui: a11y text: {error}");
                    }
                    self.reset_scroll(&surface_id);
                }
            }
            _ => {}
        }
    }

    /// Save the session and drop the journal, so a clean exit never looks like
    /// a crash.
    fn save_and_close(&mut self) {
        if let Err(error) = save_session(&self.state) {
            eprintln!("berth-gui: session.save: {error}");
            log::line(&format!("session save failed on exit: {error}"));
        }
        if let Err(error) = berth_headless::clear_journal() {
            log::line(&format!("journal clear failed: {error}"));
        }
    }

    /// Input-method events. Commits are sent as text; composition is drawn at
    /// the cursor until then.
    fn ime(&mut self, ime: Ime) {
        match ime {
            Ime::Enabled | Ime::Disabled => self.preedit = None,
            Ime::Preedit(text, _) => {
                self.preedit = if text.is_empty() { None } else { Some(text) };
            }
            Ime::Commit(text) => {
                self.preedit = None;
                if let Some(surface_id) = self.active_surface() {
                    if let Err(error) = self.host.send_text(&surface_id, &text) {
                        eprintln!("berth-gui: ime: {error}");
                    }
                    self.reset_scroll(&surface_id);
                }
            }
        }
        self.request_redraw();
    }

    /// Move the IME candidate window to the focused cursor cell.
    fn update_ime_cursor(&mut self, width: usize, height: usize) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let cell = self.cell();
        let area = self.content_area(width, height);
        let panes = {
            let state = lock(&self.state);
            ui::build_panes(&state, area, cell.height, self.tab_height(), None).0
        };
        let Some(pane) = panes.iter().find(|pane| pane.focused) else {
            return;
        };
        let Some(surface_id) = &pane.surface else {
            return;
        };
        let Ok(view) = self.host.screen_view(surface_id) else {
            return;
        };
        let (row, col) = view.cursor;
        let x = (pane.content.x + usize::from(col) * cell.width) as i32;
        let y = (pane.content.y + usize::from(row) * cell.height) as i32;
        let key = (surface_id.clone(), row, col, x, y);
        if self.ime_cursor.as_ref() == Some(&key) {
            return;
        }
        self.ime_cursor = Some(key);
        let position = PhysicalPosition::new(x, y);
        window.set_ime_cursor_area(
            position,
            PhysicalSize::new(cell.width as u32, cell.height as u32),
        );
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
        // Cmd/Super belongs to the platform shell; the window only claims the
        // line motions macOS terminals provide.
        if self.modifiers.super_key() {
            if !self.modifiers.control_key()
                && !self.modifiers.alt_key()
                && !self.modifiers.shift_key()
                && let Some(name) = super_arrow_key(&event.logical_key)
            {
                let _ = self.host.send_key(&surface_id, name, &[]);
            }
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

/// The pane under the pointer and its mouse-tracking settings.
struct MouseTarget {
    surface_id: String,
    col: u16,
    row: u16,
    mode: MouseMode,
    encoding: MouseEncoding,
}

/// Everything one panel needs to paint.
struct PanelContent {
    title: String,
    query: Option<String>,
    rows: Vec<panel::PanelRow>,
    selected: usize,
    hint: String,
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

/// The lines a paste preview shows, capped so the panel stays readable.
fn confirm_lines(text: &str) -> Vec<String> {
    const PREVIEW_LINES: usize = 12;
    let mut lines: Vec<String> = text
        .lines()
        .take(PREVIEW_LINES)
        .map(|line| {
            let mut line = line.to_string();
            if line.chars().count() > 96 {
                line = line.chars().take(95).collect::<String>() + "…";
            }
            line
        })
        .collect();
    if text.lines().count() > PREVIEW_LINES {
        lines.push("…".to_string());
    }
    lines
}

/// A press in a pane the program will consume still has to move focus.
fn needs_focus_before_report(released: bool, active: Option<&str>, target: &str) -> bool {
    !released && active != Some(target)
}

/// The named key a Super+arrow maps to, like Terminal and iTerm2: beginning and
/// end of the prompt line.
fn super_arrow_key(key: &Key) -> Option<&'static str> {
    match key {
        Key::Named(NamedKey::ArrowLeft) => Some("home"),
        Key::Named(NamedKey::ArrowRight) => Some("end"),
        _ => None,
    }
}

/// The palette Shift-click cycles a sidebar row through.
const ROW_COLORS: [&str; 5] = ["#e8b06a", "#8bd5a6", "#7fb4ff", "#e06c75", "#c792ea"];

fn next_row_color(current: Option<&str>) -> &'static str {
    let index = current
        .and_then(|color| {
            ROW_COLORS
                .iter()
                .position(|candidate| candidate.eq_ignore_ascii_case(color))
        })
        .map(|index| (index + 1) % ROW_COLORS.len())
        .unwrap_or(0);
    ROW_COLORS[index]
}

fn mouse_button(button: MouseButton) -> Option<mouse::Button> {
    Some(match button {
        MouseButton::Left => mouse::Button::Left,
        MouseButton::Middle => mouse::Button::Middle,
        MouseButton::Right => mouse::Button::Right,
        _ => return None,
    })
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

#[cfg(test)]
mod search_tests {
    use super::Gui;

    #[test]
    fn given_a_query_then_matches_are_case_insensitive_and_keep_order() {
        let lines = vec![
            "alpha beta".to_string(),
            "BETA gamma".to_string(),
            "nothing".to_string(),
        ];
        let hits = Gui::search_matches(&lines, "beta");
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].line, hits[0].col, hits[0].len), (0, 6, 4));
        assert_eq!((hits[1].line, hits[1].col), (1, 0));
    }

    #[test]
    fn given_an_empty_query_then_there_are_no_matches() {
        assert!(Gui::search_matches(&["alpha".to_string()], "").is_empty());
    }
}

#[cfg(test)]
mod press_tests {
    use super::{needs_focus_before_report, super_arrow_key};
    use winit::keyboard::{Key, NamedKey};

    #[test]
    fn given_a_press_in_another_pane_when_it_tracks_the_mouse_then_focus_moves_first() {
        assert!(needs_focus_before_report(
            false,
            Some("srf-right"),
            "srf-left"
        ));
    }

    #[test]
    fn given_a_press_in_the_focused_pane_then_focus_is_left_alone() {
        assert!(!needs_focus_before_report(
            false,
            Some("srf-left"),
            "srf-left"
        ));
    }

    #[test]
    fn given_a_row_colour_then_shift_click_cycles_through_the_palette() {
        assert_eq!(super::next_row_color(None), "#e8b06a");
        assert_eq!(super::next_row_color(Some("#e8b06a")), "#8bd5a6");
        assert_eq!(super::next_row_color(Some("#c792ea")), "#e8b06a");
    }

    #[test]
    fn given_super_with_the_horizontal_arrows_then_it_is_a_line_motion() {
        assert_eq!(
            super_arrow_key(&Key::Named(NamedKey::ArrowLeft)),
            Some("home")
        );
        assert_eq!(
            super_arrow_key(&Key::Named(NamedKey::ArrowRight)),
            Some("end")
        );
        assert_eq!(super_arrow_key(&Key::Named(NamedKey::ArrowUp)), None);
        assert_eq!(super_arrow_key(&Key::Character("a".into())), None);
    }

    #[test]
    fn given_a_release_then_focus_is_not_moved_again() {
        assert!(!needs_focus_before_report(
            true,
            Some("srf-right"),
            "srf-left"
        ));
    }
}

#[cfg(test)]
mod loop_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn given_two_changes_in_a_row_when_the_loop_has_not_woken_then_one_wake_is_sent() {
        let sent = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&sent);
        let wake = Wake::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        wake.wake();
        wake.wake();
        wake.wake();
        assert_eq!(sent.load(Ordering::SeqCst), 1);
        wake.clear();
        wake.wake();
        assert_eq!(sent.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn given_a_moment_in_a_blink_phase_when_asking_for_the_next_edge_then_it_is_the_next_multiple()
    {
        let started = Instant::now();
        let step = BLINK_INTERVAL.as_millis() as u64;
        let now = started + Duration::from_millis(step / 2);
        assert_eq!(
            next_blink_edge(started, now),
            started + Duration::from_millis(step)
        );
        let now = started + Duration::from_millis(step + step / 3);
        assert_eq!(
            next_blink_edge(started, now),
            started + Duration::from_millis(step * 2)
        );
    }

    #[test]
    fn given_any_moment_when_asking_for_the_next_blink_edge_then_it_is_in_the_future() {
        let started = Instant::now();
        for offset_ms in [0u64, 1, 529, 530, 531, 60_000] {
            let now = started + Duration::from_millis(offset_ms);
            assert!(next_blink_edge(started, now) > now, "offset {offset_ms}");
        }
    }
}
