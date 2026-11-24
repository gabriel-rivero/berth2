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

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use berth_core::model::Orientation;
use berth_core::{AppState, CoreError, ReadScope, Request, Response, SurfaceOps, paths};
use berth_headless::keys::{KeyAction, KeyInput};
use berth_headless::prompt;
use berth_headless::{
    Dimension, MouseEncoding, MouseMode, PtyHost, RestorePolicy, restore_session_with, save_session,
};
use berth_server::{Server, ServerConfig, ServerHandle, SocketMode, auth};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{CursorIcon, Window, WindowId};

mod a11y;
mod clipboard;
mod config;
mod console;
mod copymode;
mod desktop;
mod font;
mod git;
mod keys;
mod layout;
mod links;
mod log;
mod mouse;
mod paint;
mod panel;
mod paste_history;
mod search;
mod settings;
mod shells;
mod sync_output;
mod theme;
mod ui;
mod window;

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
    let host = Arc::new(PtyHost::with_engine(
        config.terminal_backend,
        config.scrollback_lines,
        config.graphics,
    ));
    if host.backend() != config.terminal_backend {
        let message = format!(
            "terminalBackend {} is not in this build; using {}",
            config.terminal_backend.name(),
            host.backend().name()
        );
        log::line(&message);
        eprintln!("berth-gui: {message}");
    }
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
        theme: config.theme(),
        font_zoom: 0.0,
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
        search_marks: SearchMarks::default(),
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
        has_blinking_content: false,
        ime_cursor: None,
        autoscroll: false,
        cursor_icon: CursorIcon::Default,
        last_detect: Instant::now(),
        last_sweep: Instant::now(),
        save_due: None,
        save_seq: 0,
        quit: false,
        captured: false,
        frames: sync_output::FrameHold::default(),
        focus_reported: None,
        pending_clipboard: VecDeque::new(),
        stash: HashMap::new(),
        core_of: HashMap::new(),
        tab_scroll: HashMap::new(),
        sidebar_filter: String::new(),
        sidebar_filter_active: false,
        geometry_due: None,
        branch: RefCell::new(BranchCache::default()),
        prompt_surfaces: HashSet::new(),
        copy_mode: None,
        paste_history: paste_history::PasteHistory::new(),
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
struct SearchHighlight {
    surface_id: String,
    line: usize,
    col: usize,
    len: usize,
}

/// The results of a closed search panel, kept so the hits stay tinted after
/// the list is dismissed. `current` is the one the panel was on.
#[derive(Default)]
struct SearchMarks {
    /// Hits per surface, so each pane tints only its own rows.
    by_surface: HashMap<String, Vec<search::Span>>,
    current: Option<SearchHighlight>,
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
    /// Search every pane of the workspace: literal or regex, whole-word
    /// optional, case-insensitive.
    Search {
        /// One buffer per pane, captured when the panel opened.
        panes: Vec<search::PaneBuffer>,
        query: String,
        options: search::Options,
        /// The compiled query, so a keystroke does not recompile per pane.
        compiled: search::Query,
        hits: Vec<search::Hit>,
        selected: usize,
    },
    /// A program asked to read the clipboard; allow or deny.
    ClipboardRead {
        surface_id: String,
        selection: String,
        selected: usize,
    },
    /// The last few clipboard entries, newest first, to paste one of them.
    PasteHistory {
        selected: usize,
    },
    /// The tabs a pane's strip cannot show.
    TabMenu {
        pane_id: String,
        selected: usize,
    },
    /// Right-click actions for a tab or pane.
    Context {
        surface_id: Option<String>,
        pane_id: Option<String>,
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
            Overlay::ClipboardRead { selected, .. } | Overlay::PasteHistory { selected } => {
                *selected
            }
            Overlay::TabMenu { selected, .. } | Overlay::Context { selected, .. } => *selected,
            Overlay::Rename { .. } | Overlay::Confirm { .. } => 0,
            Overlay::Search { selected, .. } => *selected,
        }
    }

    fn select(&mut self, index: usize) {
        match self {
            Overlay::Palette { selected, .. }
            | Overlay::Attention { selected }
            | Overlay::Settings { selected, .. }
            | Overlay::Search { selected, .. }
            | Overlay::ClipboardRead { selected, .. }
            | Overlay::PasteHistory { selected }
            | Overlay::TabMenu { selected, .. }
            | Overlay::Context { selected, .. } => *selected = index,
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
    /// Session offset from `config.font_size` that the zoom chords set.
    font_zoom: f32,
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
    /// The search hits to tint, per pane, plus the one the panel was on.
    search_marks: SearchMarks,
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
    /// Set at paint time when visible content blinks, so the loop wakes on the
    /// phase edge only then.
    has_blinking_content: bool,
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
    /// The frame each surface last showed, held while a program has DECSET
    /// 2026 set.
    frames: sync_output::FrameHold,
    /// The surface we last told the program has focus.
    focus_reported: Option<String>,
    /// OSC 52 read requests waiting for the modal confirmation.
    pending_clipboard: VecDeque<(String, String)>,
    /// Windows that are not the one currently taking events. The active
    /// window's state stays in the fields above so every existing helper keeps
    /// working; switching swaps a `WinStash` in and out.
    stash: HashMap<WindowId, WinStash>,
    /// Native window id to the core window that owns it.
    core_of: HashMap<WindowId, String>,
    /// First visible tab of a pane whose tabs overflow the strip.
    tab_scroll: HashMap<String, usize>,
    sidebar_filter: String,
    /// The sidebar filter field is taking keystrokes.
    sidebar_filter_active: bool,
    /// Debounced geometry write after a move or resize.
    geometry_due: Option<Instant>,
    /// The focused pane's git branch, cached so the status line does not walk
    /// the filesystem every frame.
    branch: RefCell<BranchCache>,
    /// Surfaces whose program emits OSC 133 prompt marks. Only these panes
    /// spend columns on the gutter.
    prompt_surfaces: HashSet<String>,
    /// The copy-mode session, while one is open. It captures every key before
    /// the shell sees it.
    copy_mode: Option<copymode::CopyMode>,
    /// The last few clipboards, in memory only.
    paste_history: paste_history::PasteHistory,
}

/// Last branch read for a cwd, with the time it was read. A cwd change or a
/// stale entry triggers a fresh read.
#[derive(Default)]
struct BranchCache {
    cwd: String,
    branch: Option<String>,
    at: Option<Instant>,
}

/// Per-window presentation state stored while another window is active.
struct WinStash {
    window: Arc<Window>,
    context: softbuffer::Context<Arc<Window>>,
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    pointer: (f64, f64),
    modifiers: ModifiersState,
    title: String,
    scale: f64,
    sidebar_visible: bool,
    sidebar_width: f64,
    sidebar_scroll: usize,
    drag: Drag,
    overlay: Option<Overlay>,
    window_focused: bool,
    a11y: Option<accesskit_winit::Adapter>,
    a11y_active: bool,
    a11y_text: String,
    tab_scroll: HashMap<String, usize>,
    sidebar_filter: String,
    sidebar_filter_active: bool,
    copy_mode: Option<copymode::CopyMode>,
}

impl ApplicationHandler<UserEvent> for Gui {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() || !self.stash.is_empty() {
            return;
        }
        // One native window per core window. A fresh state has none, so mint
        // the first one through the protocol and let reconcile build from it.
        let (windows, selected) = {
            let mut state = lock(&self.state);
            if state.windows.is_empty() {
                let request = Request {
                    id: serde_json::Value::Null,
                    method: "window.create".to_string(),
                    params: serde_json::json!({"name": "main"}),
                };
                let _ = berth_core::dispatch(&mut state, self.host.as_ref(), &request);
            }
            (state.windows.clone(), state.selected_window.clone())
        };
        self.flush_events();

        for core in &windows {
            if let Err(error) = self.open_window(event_loop, core) {
                eprintln!("berth-gui: window: {error}");
                self.exit_code = 1;
                event_loop.exit();
                return;
            }
            self.stash_active();
        }
        let target = selected
            .as_deref()
            .and_then(|id| self.core_of.iter().find(|(_, core)| *core == id))
            .map(|(native, _)| *native)
            .or_else(|| self.stash.keys().next().copied());
        if let Some(id) = target {
            self.load_window(id);
        }
        trace_phase(self.started, "adapter");
        self.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.current_window_id() != Some(window_id) && !self.load_window(window_id) {
            return;
        }
        if let Some(window) = self.window.clone()
            && let Some(adapter) = self.a11y.as_mut()
        {
            adapter.process_event(&window, &event);
        }
        match event {
            WindowEvent::CloseRequested => {
                if self.window_count() <= 1 {
                    self.save_and_close();
                    event_loop.exit();
                    return;
                }
                if let Some(core_id) = self.current_core_window() {
                    let _ =
                        self.dispatch("window.close", serde_json::json!({"window_id": core_id}));
                }
                self.core_of.remove(&window_id);
                self.window = None;
                self.context = None;
                self.surface = None;
                self.a11y = None;
                self.overlay = None;
                self.drag = Drag::None;
                if let Some(next) = self.stash.keys().next().copied() {
                    self.load_window(next);
                }
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.sync_scale();
                self.geometry_due = Some(Instant::now());
                self.request_redraw();
            }
            WindowEvent::Moved(_) => {
                self.geometry_due = Some(Instant::now());
            }
            WindowEvent::Focused(focused) => {
                self.window_focused = focused;
                if !focused
                    && let Some(Overlay::ClipboardRead {
                        surface_id,
                        selection,
                        ..
                    }) = self.overlay.take()
                {
                    self.send_clipboard_response(&surface_id, &selection, None);
                }
                if focused {
                    if let Some(core_id) = self.current_core_window() {
                        let _ = self
                            .dispatch("window.select", serde_json::json!({"window_id": core_id}));
                    }
                    if let Some(surface_id) = self.active_surface() {
                        let _ = self.host.focus(&surface_id);
                    }
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
            WindowEvent::KeyboardInput { event, .. } => match event.state {
                ElementState::Pressed => self.key(event),
                ElementState::Released => self.key_release(event),
            },
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
        self.sync_focus_events();
        self.open_pending_clipboard();
        if self
            .geometry_due
            .is_some_and(|at| at.elapsed() >= Duration::from_millis(250))
        {
            self.geometry_due = None;
            self.persist_window_state();
        }
        self.sync_windows(event_loop);
        let selected_core = lock(&self.state).selected_window.clone();
        if let Some(core_id) = selected_core
            && self.current_core_window().as_deref() != Some(core_id.as_str())
            && let Some(native) = self
                .core_of
                .iter()
                .find(|(_, core)| **core == core_id)
                .map(|(native, _)| *native)
        {
            self.switch_window(native);
            if let Some(window) = &self.window {
                window.focus_window();
            }
            self.request_redraw();
        }
        if self.frames.release_expired(Instant::now()) {
            self.request_redraw();
        }
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

/// Blinking text keeps the timer awake in any pane; a blinking cursor only
/// counts where it is drawn, which needs focus and no scrollback.
fn blinks_needing_timer(window_focused: bool, cursor_blinks: bool, cell_blinks: bool) -> bool {
    window_focused && (cursor_blinks || cell_blinks)
}

impl Gui {
    /// Cursor blink phase: `true` draws the cursor. The loop wakes on the phase
    /// change, not on a frame timer.
    fn blink_on(&self) -> bool {
        let step = BLINK_INTERVAL.as_millis() as u64;
        (self.started.elapsed().as_millis() as u64 / step.max(1)).is_multiple_of(2)
    }

    fn needs_blink(&self) -> bool {
        self.has_blinking_content
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
        if self.needs_blink() {
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
        if let Some(at) = self.frames.deadline() {
            // A wake can land a hair before the deadline; retry shortly rather
            // than leaving the pane held.
            consider(if at > now {
                at
            } else {
                now + Duration::from_millis(1)
            });
        }
        next
    }

    /// Tell the focused program when focus enters or leaves. A program opts in
    /// with CSI ?1004; the old surface gets FocusOut before the new gets FocusIn.
    fn sync_focus_events(&mut self) {
        if let Some(surface_id) = self.focus_reported.clone()
            && !self.host.is_focus_reporting(&surface_id).unwrap_or(false)
        {
            self.focus_reported = None;
        }
        let current = if self.window_focused {
            self.active_surface()
        } else {
            None
        };
        if self.focus_reported.as_deref() == current.as_deref() {
            return;
        }
        if let Some(old) = self.focus_reported.take() {
            let _ = self.host.send_bytes(&old, b"\x1b[O");
        }
        if let Some(new) = current
            && self.host.is_focus_reporting(&new).unwrap_or(false)
        {
            let _ = self.host.send_bytes(&new, b"\x1b[I");
            self.focus_reported = Some(new);
        }
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
            ui::build_panes(
                &state,
                nav.area,
                nav.cell.height,
                nav.tab_height,
                None,
                self.pane_options(),
            )
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
            self.invalidate_fonts();
        }
    }

    fn font(&mut self) -> &mut FontRenderer {
        if self.font.is_none() {
            self.font = self.font_job.take().and_then(|mut job| job.take());
        }
        if self.font.is_none() {
            let pixels = self.config.font_pixels(self.font_points(), self.scale);
            self.font = Some(FontRenderer::new(
                pixels,
                &self.config.font_family,
                self.config.font_options(),
            ));
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
                self.config.font_options(),
            ));
        }
        self.chrome_font.as_mut().expect("chrome font just created")
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    // ---- native windows ------------------------------------------------

    fn current_window_id(&self) -> Option<WindowId> {
        self.window.as_ref().map(|window| window.id())
    }

    fn current_core_window(&self) -> Option<String> {
        self.current_window_id()
            .and_then(|id| self.core_of.get(&id).cloned())
    }

    fn window_count(&self) -> usize {
        self.stash.len() + usize::from(self.window.is_some())
    }

    /// Build the presentation state for one core window and leave it active.
    fn open_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        core: &berth_core::model::Window,
    ) -> Result<(), String> {
        let mut attributes = Window::default_attributes()
            .with_title(if core.name.is_empty() {
                "Berth"
            } else {
                &core.name
            })
            .with_min_inner_size(LogicalSize::new(640.0, 320.0))
            // The accessibility adapter must exist before the window is shown,
            // so it starts hidden and is revealed below.
            .with_visible(false);
        match core.bounds {
            Some(bounds) => {
                attributes = attributes
                    .with_position(winit::dpi::LogicalPosition::new(
                        bounds.x as f64,
                        bounds.y as f64,
                    ))
                    .with_inner_size(LogicalSize::new(
                        bounds.width.max(320) as f64,
                        bounds.height.max(240) as f64,
                    ));
            }
            None => {
                attributes = attributes.with_inner_size(LogicalSize::new(1180.0, 720.0));
            }
        }
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|error| error.to_string())?,
        );
        self.scale = window.scale_factor();
        self.font_job = Some(font::FontSlot::spawn(
            self.config.font_pixels(self.font_points(), self.scale),
            self.config.font_family.clone(),
            self.config.font_options(),
        ));
        self.chrome_job = Some(font::FontSlot::spawn(
            CHROME_FONT_SIZE * self.scale as f32,
            self.config.font_family.clone(),
            self.config.font_options(),
        ));
        let context = softbuffer::Context::new(window.clone())
            .map_err(|error| format!("softbuffer context: {error}"))?;
        let surface = softbuffer::Surface::new(&context, window.clone())
            .map_err(|error| format!("softbuffer surface: {error}"))?;
        window.set_ime_allowed(true);
        self.a11y = Some(accesskit_winit::Adapter::with_event_loop_proxy(
            &window,
            self.proxy.clone(),
        ));
        window.set_visible(true);
        crate::window::set_opacity(&window, self.config.opacity);

        self.sidebar_width = core
            .sidebar_width
            .unwrap_or(self.config.sidebar_pixels())
            .clamp(config::MIN_SIDEBAR_WIDTH, config::MAX_SIDEBAR_WIDTH);
        self.sidebar_visible = true;
        self.sidebar_scroll = 0;
        self.sidebar_filter.clear();
        self.sidebar_filter_active = false;
        self.pointer = (0.0, 0.0);
        self.modifiers = ModifiersState::empty();
        self.title = core.name.clone();
        self.drag = Drag::None;
        self.overlay = None;
        self.window_focused = false;
        self.a11y_active = false;
        self.a11y_text.clear();
        self.tab_scroll.clear();
        self.context = Some(context);
        self.surface = Some(surface);
        self.core_of.insert(window.id(), core.id.clone());
        self.window = Some(window);
        Ok(())
    }

    fn stash_active(&mut self) {
        let Some(window) = self.window.take() else {
            return;
        };
        let id = window.id();
        let (Some(context), Some(surface)) = (self.context.take(), self.surface.take()) else {
            self.window = Some(window);
            return;
        };
        let stash = WinStash {
            window,
            context,
            surface,
            pointer: self.pointer,
            modifiers: self.modifiers,
            title: std::mem::take(&mut self.title),
            scale: self.scale,
            sidebar_visible: self.sidebar_visible,
            sidebar_width: self.sidebar_width,
            sidebar_scroll: self.sidebar_scroll,
            drag: std::mem::replace(&mut self.drag, Drag::None),
            overlay: self.overlay.take(),
            window_focused: self.window_focused,
            a11y: self.a11y.take(),
            a11y_active: self.a11y_active,
            a11y_text: std::mem::take(&mut self.a11y_text),
            tab_scroll: std::mem::take(&mut self.tab_scroll),
            sidebar_filter: std::mem::take(&mut self.sidebar_filter),
            sidebar_filter_active: self.sidebar_filter_active,
            copy_mode: self.copy_mode.take(),
        };
        self.stash.insert(id, stash);
    }

    fn load_window(&mut self, id: WindowId) -> bool {
        let Some(stash) = self.stash.remove(&id) else {
            return false;
        };
        self.window = Some(stash.window);
        self.context = Some(stash.context);
        self.surface = Some(stash.surface);
        self.pointer = stash.pointer;
        self.modifiers = stash.modifiers;
        self.title = stash.title;
        self.scale = stash.scale;
        self.sidebar_visible = stash.sidebar_visible;
        self.sidebar_width = stash.sidebar_width;
        self.sidebar_scroll = stash.sidebar_scroll;
        self.drag = stash.drag;
        self.overlay = stash.overlay;
        self.window_focused = stash.window_focused;
        self.a11y = stash.a11y;
        self.a11y_active = stash.a11y_active;
        self.a11y_text = stash.a11y_text;
        self.tab_scroll = stash.tab_scroll;
        self.sidebar_filter = stash.sidebar_filter;
        self.sidebar_filter_active = stash.sidebar_filter_active;
        self.copy_mode = stash.copy_mode;
        // Transient input state must not follow the pointer between windows.
        self.preedit = None;
        self.mouse_down = None;
        true
    }

    fn switch_window(&mut self, id: WindowId) {
        if self.current_window_id() == Some(id) {
            return;
        }
        self.stash_active();
        if !self.load_window(id) {
            // The target vanished; keep whatever window is still available.
            if let Some(id) = self.stash.keys().next().copied() {
                self.load_window(id);
            }
        }
    }

    /// Persist geometry and sidebar width for the active core window.
    fn persist_window_state(&mut self) {
        let Some(core_id) = self.current_core_window() else {
            return;
        };
        let Some(window) = self.window.clone() else {
            return;
        };
        let size = window.inner_size();
        let position = window
            .outer_position()
            .unwrap_or(PhysicalPosition::new(0, 0));
        let scale = window.scale_factor();
        let monitor = window.current_monitor().and_then(|monitor| monitor.name());
        let bounds = serde_json::json!({
            "x": position.x,
            "y": position.y,
            "width": size.width,
            "height": size.height,
            "maximized": false,
            "scale": scale,
        });
        let mut params = serde_json::json!({
            "window_id": core_id,
            "bounds": bounds,
            "sidebar_width": self.sidebar_width,
        });
        if let Some(monitor) = monitor {
            params["monitor"] = serde_json::json!(monitor);
        }
        let _ = self.dispatch("window.update", params);
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
        let top = self.menu_bar_height();
        let bottom = self.status_height();
        Rect::new(
            sidebar,
            top,
            width.saturating_sub(sidebar),
            height.saturating_sub(top + bottom),
        )
    }

    fn menu_bar_height(&self) -> usize {
        (24.0 * self.scale) as usize
    }

    fn status_height(&self) -> usize {
        (22.0 * self.scale) as usize
    }

    /// The branch checked out at `cwd`, re-read when the pane moves or the
    /// cached answer is more than a couple of seconds old.
    fn branch_text(&self, cwd: &str) -> Option<String> {
        let mut cache = self.branch.borrow_mut();
        let now = Instant::now();
        let stale = cache.cwd != cwd
            || cache
                .at
                .is_none_or(|at| now.duration_since(at) >= Duration::from_secs(2));
        if stale {
            cache.cwd.clear();
            cache.cwd.push_str(cwd);
            cache.branch = git::branch(cwd);
            cache.at = Some(now);
        }
        cache.branch.clone()
    }

    /// One line for the status bar: mode chips, the workspace and the window.
    fn status_text(&self) -> String {
        let (broadcast, window_index, workspace) = {
            let state = lock(&self.state);
            let current = self.current_core_window();
            let broadcast = current
                .as_deref()
                .and_then(|id| state.window(id))
                .map(|window| window.broadcast)
                .unwrap_or(false);
            let index = current
                .as_deref()
                .and_then(|id| state.windows.iter().position(|window| window.id == id))
                .map(|index| (index + 1, state.windows.len()))
                .unwrap_or((0, 0));
            let workspace = ui::active_workspace(&state).and_then(|id| {
                let workspace = state.workspace(&id)?;
                let zoomed = workspace.zoomed_pane.is_some();
                let cwd = workspace
                    .active_surface
                    .as_deref()
                    .and_then(|surface| state.surface(surface))
                    .map(|surface| surface.cwd.clone())
                    .unwrap_or_default();
                Some((workspace.name.clone(), cwd, zoomed))
            });
            (broadcast, index, workspace)
        };
        let mut parts = Vec::new();
        let mut modifiers = String::new();
        if self.modifiers.control_key() {
            modifiers.push('⌃');
        }
        if self.modifiers.alt_key() {
            modifiers.push('⌥');
        }
        if self.modifiers.shift_key() {
            modifiers.push('⇧');
        }
        if self.modifiers.super_key() {
            modifiers.push('⌘');
        }
        if !modifiers.is_empty() {
            parts.push(modifiers);
        }
        if broadcast {
            parts.push("BROADCAST".to_string());
        }
        if let Some(mode) = self.copy_mode.as_ref() {
            // The chip tells the user the shell is not receiving their keys.
            parts.push(if mode.is_visual() {
                "COPY-V".to_string()
            } else {
                "COPY".to_string()
            });
        }
        if let Some((name, cwd, zoomed)) = workspace {
            if zoomed {
                parts.push("ZOOM".to_string());
            }
            parts.push(name);
            if let Some(base) = cwd.trim_end_matches('/').rsplit('/').next()
                && !base.is_empty()
            {
                parts.push(base.to_string());
            }
            if let Some(branch) = self.branch_text(&cwd) {
                parts.push(format!("⑂ {branch}"));
            }
        }
        if window_index.1 > 0 {
            parts.push(format!("{}/{}", window_index.0, window_index.1));
        }
        parts.join("  ·  ")
    }

    fn tab_height(&self) -> usize {
        (ui::TAB_HEIGHT * self.scale) as usize
    }

    fn pane_options(&self) -> ui::PaneOptions<'_> {
        ui::PaneOptions {
            zoom: None,
            tab_scroll: Some(&self.tab_scroll),
            prompt_marks: Some(&self.prompt_surfaces),
        }
    }

    fn sidebar_filter_arg(&self) -> Option<&str> {
        if self.sidebar_filter.is_empty() {
            None
        } else {
            Some(self.sidebar_filter.as_str())
        }
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
        self.prompt_surfaces.retain(|id| surfaces.contains(id));
        for surface_id in surfaces {
            // Whether the program emits prompt marks decides the layout, so it
            // is checked here rather than every frame of the paint loop. The
            // answer is latched: showing the gutter narrows the grid, and a
            // grid change re-wraps the history and drops the marks, so deriving
            // the gutter from the current marks would make it blink on and off.
            if self.host.has_prompt_marks(&surface_id).unwrap_or(false) {
                self.prompt_surfaces.insert(surface_id.clone());
            }
            let Ok(meta) = self.host.take_screen_meta_requests(&surface_id) else {
                continue;
            };
            let mut params = serde_json::json!({"surface_id": surface_id});
            let mut changed = false;
            let mut bell_rang = false;
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
                    applied.bells = meta.bells;
                    bell_rang = true;
                    if self.config.bell != config::BellMode::None {
                        params["bell"] = serde_json::json!(true);
                        changed = true;
                    }
                }
            }
            if changed {
                if bell_rang {
                    match self.config.bell {
                        config::BellMode::None => {}
                        config::BellMode::Visual => {
                            self.flashes.insert(surface_id.clone(), Instant::now());
                        }
                        config::BellMode::Audio => {
                            if let Err(error) = desktop::ring() {
                                log::line(&format!("bell failed: {error}"));
                            }
                        }
                    }
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
            if let Some(selection) = meta.clipboard_read {
                self.pending_clipboard
                    .push_back((surface_id.clone(), selection));
                self.request_redraw();
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
        self.has_blinking_content = false;
        let tab_height = self.tab_height();
        let flash = self
            .flashes
            .iter()
            .find(|(_, at)| at.elapsed() < FLASH_DURATION)
            .map(|(surface, _)| surface.clone());

        let (panes, _dividers) = {
            let state = lock(&self.state);
            ui::build_panes(
                &state,
                area,
                cell.height,
                tab_height,
                flash.as_deref(),
                self.pane_options(),
            )
        };
        let rows = if area.x > 0 {
            let state = lock(&self.state);
            ui::build_sidebar_filtered(
                &state,
                Rect::new(0, 0, area.x, height),
                self.workspace_row_height(),
                &theme,
                self.sidebar_scroll,
                self.sidebar_filter_arg(),
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
            // Menu and status strips belong to the content column, so the
            // sidebar keeps full height for its own rows.
            let menu_height = self.menu_bar_height();
            let status_height = self.status_height();
            let menu_rect = Rect::new(
                area.x,
                0,
                width.saturating_sub(area.x),
                menu_height.min(height),
            );
            let status_rect = Rect::new(
                area.x,
                height.saturating_sub(status_height),
                width.saturating_sub(area.x),
                status_height.min(height),
            );
            let status_text = self.status_text();
            let status_bg = if status_text.contains("BROADCAST") {
                theme.attention_input
            } else {
                theme.tab_bar_bg
            };
            let chrome_cell = self.chrome_font().cell();
            painter.fill(menu_rect, theme.tab_bar_bg);
            let menu_font = self.chrome_font();
            painter.text(
                menu_font,
                (
                    menu_rect.x + 10,
                    menu_rect.y + menu_rect.h.saturating_sub(chrome_cell.height) / 2,
                ),
                "Berth   Terminal   View   Window",
                font::TextStyle::NORMAL,
                theme.text_secondary,
                menu_rect.w.saturating_sub(20),
            );
            painter.fill(status_rect, status_bg);
            let status_font = self.chrome_font();
            painter.text_elided(
                status_font,
                (
                    status_rect.x + 10,
                    status_rect.y + status_rect.h.saturating_sub(chrome_cell.height) / 2,
                ),
                &status_text,
                font::TextStyle::NORMAL,
                theme.text_secondary,
                status_rect.w.saturating_sub(20),
            );
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
            let now = Instant::now();
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
                let Ok(fresh) = self.host.screen_view(surface_id) else {
                    continue;
                };
                let screen = self.frames.frame_to_paint(surface_id, fresh, now);
                let cursor_blinks = pane.focused
                    && !screen.cursor_hidden
                    && screen.scrollback == 0
                    && screen.cursor_style.blinks;
                self.has_blinking_content |= blinks_needing_timer(
                    self.window_focused,
                    cursor_blinks,
                    screen.cells.iter().any(|cell| cell.blink),
                );
                let selection = self.selection.get(surface_id).copied();
                let exited = !self.host.is_alive(surface_id);
                // Every hit in this pane tints; the one the panel is on tints
                // in the accent so it reads as the current result.
                let (search_hits, search_current) = self.search_marks_for(surface_id);
                let pointer = (
                    self.pointer.0.max(0.0) as usize,
                    self.pointer.1.max(0.0) as usize,
                );
                let hover_link = if pane.content.contains(pointer.0, pointer.1) {
                    let col = (pointer.0 - pane.content.x) / cell.width.max(1);
                    let row = (pointer.1 - pane.content.y) / cell.height.max(1);
                    screen
                        .cells
                        .get(row * usize::from(screen.cols) + col)
                        .and_then(|cell| cell.hyperlink.as_deref())
                } else {
                    None
                };
                let options = ScreenOptions {
                    theme: &theme,
                    focused: pane.focused,
                    cursor_visible: pane.focused
                        && self.window_focused
                        && (!screen.cursor_style.blinks || blink_on),
                    selection: selection.as_ref(),
                    exited,
                    search: &search_hits,
                    search_current,
                    hover_link,
                    // Blinking text stays solid while the window is unfocused,
                    // since no timer runs to flip the phase.
                    blink_on: blink_on || !self.window_focused,
                };
                let font = self.font();
                painter.screen(font, &screen, pane.content, &options);
                let cell = font.cell();
                painter.prompt_gutter(pane, &screen, theme, cell.height);
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
        let filter_text = if self.sidebar_filter_active || !self.sidebar_filter.is_empty() {
            Some((
                format!(
                    "{}{}",
                    if self.sidebar_filter_active {
                        "/"
                    } else {
                        "filter: "
                    },
                    self.sidebar_filter
                ),
                self.sidebar_filter_active,
            ))
        } else {
            None
        };
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
        if let Some((text, active)) = &filter_text {
            painter.text(
                font,
                (12 + cell.width * 11, 8),
                text,
                font::TextStyle::NORMAL,
                if *active {
                    theme.accent
                } else {
                    theme.text_tertiary
                },
                area.w.saturating_sub(12 + cell.width * 11 + 70),
            );
        }
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
            ui::build_panes(
                &state,
                nav.area,
                nav.cell.height,
                nav.tab_height,
                None,
                self.pane_options(),
            )
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
        if let Ok(view) = self.host.screen_view(&target.surface_id) {
            let index = usize::from(target.row) * usize::from(view.cols) + usize::from(target.col);
            if let Some(link) = view
                .cells
                .get(index)
                .and_then(|cell| cell.hyperlink.as_deref())
            {
                return Some(link.to_string());
            }
        }
        let row = self.host.line_text(&target.surface_id, target.row).ok()?;
        links::link_at(&row, target.col as usize).map(|(_, _, link)| link)
    }

    /// Open the tab/pane action menu for a right-click. Returns false when the
    /// press landed somewhere with no pane.
    fn open_context(&mut self, point: (usize, usize)) -> bool {
        let nav = self.nav();
        if nav.area.x > 0 && point.0 < nav.area.x {
            return false;
        }
        let panes = {
            let state = lock(&self.state);
            ui::build_panes(
                &state,
                nav.area,
                nav.cell.height,
                nav.tab_height,
                None,
                self.pane_options(),
            )
            .0
        };
        let Some(pane) = ui::pane_at(&panes, point.0, point.1) else {
            return false;
        };
        let surface_id = pane
            .tabs
            .iter()
            .find(|tab| tab.rect.contains(point.0, point.1))
            .map(|tab| tab.id.clone())
            .or_else(|| pane.surface.clone());
        self.overlay = Some(Overlay::Context {
            surface_id,
            pane_id: Some(pane.pane_id.clone()),
            selected: 0,
        });
        true
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
            ui::build_panes(
                &state,
                nav.area,
                nav.cell.height,
                nav.tab_height,
                None,
                self.pane_options(),
            )
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
        if pane.gutter.w > 0 && pane.gutter.contains(point.0, point.1) {
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
        if button == MouseButton::Right && self.open_context(point) {
            self.request_redraw();
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
        let rows = ui::build_sidebar_filtered(
            &state,
            Rect::new(0, 0, nav.area.x, nav.height),
            self.workspace_row_height(),
            &self.theme,
            self.sidebar_scroll,
            self.sidebar_filter_arg(),
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
            ui::build_panes(
                &state,
                nav.area,
                nav.cell.height,
                nav.tab_height,
                None,
                self.pane_options(),
            )
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
            ui::build_panes(
                &state,
                nav.area,
                nav.cell.height,
                nav.tab_height,
                None,
                self.pane_options(),
            )
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
            if let Some(overflow) = pane.overflow
                && overflow.contains(point.0, point.1)
            {
                let selected = lock(&self.state)
                    .panes
                    .get(&pane.pane_id)
                    .map(|pane| pane.active)
                    .unwrap_or(0);
                self.overlay = Some(Overlay::TabMenu {
                    pane_id: pane.pane_id.clone(),
                    selected,
                });
                return;
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
        if pane.gutter.w > 0 && pane.gutter.contains(point.0, point.1) {
            self.gutter_pressed(surface_id, pane, point, cell);
            return;
        }
        let column = (point.0.saturating_sub(pane.content.x)) / cell.width.max(1);
        let row = (point.1.saturating_sub(pane.content.y)) / cell.height.max(1);
        let target = Cell {
            row: row.min(u16::MAX as usize) as u16,
            col: column.min(u16::MAX as usize) as u16,
        };
        // A click in copy mode moves its cursor and extends a visual selection.
        // Starting a mouse selection instead would abandon the mode's cursor.
        if self.copy_mode.is_some() {
            self.copy_mode_click(surface_id, target);
            return;
        }
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

    /// Click in the gutter: scroll that prompt to the top of the pane, so the
    /// command and its output read from the start. A click on a row with no
    /// prompt mark does nothing.
    fn gutter_pressed(
        &mut self,
        surface_id: &str,
        pane: &PaneView,
        point: (usize, usize),
        cell: CellSize,
    ) {
        if cell.height == 0 {
            return;
        }
        let Ok(view) = self.host.screen_view(surface_id) else {
            return;
        };
        let row = point.1.saturating_sub(pane.gutter.y) / cell.height;
        let first_line = view.scrollback_len.saturating_sub(view.scrollback);
        let line = first_line.saturating_add(row);
        let Some(mark) = view
            .prompt_marks
            .iter()
            .find(|mark| mark.line == line && mark.kind == prompt::PromptKind::Prompt)
        else {
            return;
        };
        let target = mark.line;
        let offset = view.scrollback_len.saturating_sub(target);
        if let Ok(applied) = self.host.set_scrollback(surface_id, offset) {
            self.scroll.insert(surface_id.to_string(), applied);
            self.request_redraw();
        }
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
            ui::build_sidebar_filtered(
                &state,
                Rect::new(0, 0, nav.area.x, nav.height),
                self.workspace_row_height(),
                &self.theme,
                self.sidebar_scroll,
                self.sidebar_filter_arg(),
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
            ui::RowKind::Agent {
                workspace_id,
                surface_id,
                ..
            } => {
                self.select_workspace(workspace_id);
                self.activate(surface_id);
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
            ui::build_sidebar_filtered(
                &state,
                area,
                self.workspace_row_height(),
                &self.theme,
                0,
                self.sidebar_filter_arg(),
            )
            .total
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
            ui::build_sidebar_filtered(
                &state,
                Rect::new(0, 0, nav.area.x, nav.height),
                self.workspace_row_height(),
                &self.theme,
                self.sidebar_scroll,
                self.sidebar_filter_arg(),
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
                        ui::build_panes(
                            &state,
                            nav.area,
                            nav.cell.height,
                            nav.tab_height,
                            None,
                            self.pane_options(),
                        )
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
            ui::build_panes(
                &state,
                nav.area,
                cell.height,
                nav.tab_height,
                None,
                self.pane_options(),
            )
        };
        let Some(pane) = ui::pane_at(&panes, point.0, point.1) else {
            return;
        };
        if pane.tab_bar.contains(point.0, point.1) {
            let count = lock(&self.state)
                .panes
                .get(&pane.pane_id)
                .map(|pane| pane.surfaces.len())
                .unwrap_or(0);
            let step = if lines > 0 { -1 } else { 1 };
            let offset = self.tab_scroll.entry(pane.pane_id.clone()).or_insert(0);
            *offset = ((*offset as i32 + step).max(0) as usize).min(count.saturating_sub(1));
            self.request_redraw();
            return;
        }
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
                // Copy mode's cursor is an absolute line, so a wheel scroll
                // moves the view under it; the painted rows have to be
                // recomputed or the tint stays on the old text.
                if self
                    .copy_mode
                    .as_ref()
                    .is_some_and(|mode| mode.surface_id == surface_id)
                {
                    self.copy_mode_paint_selection(&surface_id);
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
                self.frames.remove(surface_id);
                self.prompt_surfaces.remove(surface_id);
                // A copy-mode session on this surface would otherwise keep
                // capturing keys for a pane that no longer exists.
                if self
                    .copy_mode
                    .as_ref()
                    .is_some_and(|mode| mode.surface_id == surface_id)
                {
                    self.copy_mode = None;
                }
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
        let direction = cycle_direction(delta);
        let params = serde_json::json!({
            "surface_id": surface_id,
            "direction": direction,
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
        self.copy_surface_selection(&surface_id);
    }

    /// Copy `surface_id`'s selection and remember it. The context menu copies
    /// the pane it was opened on, which is not necessarily the focused one.
    fn copy_surface_selection(&mut self, surface_id: &str) {
        // Read the selection once: two reads could straddle output arriving on
        // the PTY thread and record different text from what was copied.
        let Some(text) = self.selection_text(surface_id) else {
            return;
        };
        if self.copy_text(&text) {
            self.paste_history.record(&text);
        }
    }

    /// Copy without recording. Copy-on-select runs on every drag, so letting it
    /// fill the history would push the user's real copies out of the ring.
    fn copy_surface_selection_quiet(&mut self, surface_id: &str) {
        let Some(text) = self.selection_text(surface_id) else {
            return;
        };
        self.copy_text(&text);
    }

    /// Put `text` on the clipboard. Returns whether it landed, so only a copy
    /// that actually happened is remembered.
    fn copy_text(&mut self, text: &str) -> bool {
        match self.clipboard.as_mut() {
            Some(clipboard) => match clipboard.set_text(text) {
                Ok(()) => true,
                Err(error) => {
                    eprintln!("berth-gui: clipboard: {error}");
                    false
                }
            },
            None => false,
        }
    }

    /// The selected text of `surface_id`, trimmed, or `None` when there is no
    /// selection or it is only whitespace.
    fn selection_text(&self, surface_id: &str) -> Option<String> {
        let selection = self.selection.get(surface_id).copied()?;
        let (start, end) = selection.span();
        let text = self
            .host
            .contents_between(surface_id, start.row, start.col, end.row, end.col)
            .ok()?;
        let text = text
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n");
        (!text.is_empty()).then_some(text)
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
        self.copy_surface_selection_quiet(&surface_id);
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
        self.paste_history.record(&text);
        self.send_paste(&text);
    }

    /// Send pasted text to the active surface, wrapped when the program asked
    /// for bracketed paste.
    fn send_paste(&mut self, text: &str) {
        for surface_id in &self.input_targets() {
            let bracketed = self
                .host
                .screen_view(surface_id)
                .map(|view| view.bracketed_paste)
                .unwrap_or(false);
            let payload = if bracketed {
                format!("\x1b[200~{text}\x1b[201~")
            } else {
                text.to_string()
            };
            if let Err(error) = self.host.send_text(surface_id, &payload) {
                eprintln!("berth-gui: paste: {error}");
            }
            self.reset_scroll(surface_id);
        }
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
            Command::FocusLeft => self.focus_direction('l'),
            Command::FocusRight => self.focus_direction('r'),
            Command::FocusUp => self.focus_direction('u'),
            Command::FocusDown => self.focus_direction('d'),
            Command::NewWorkspace => {
                if let Err(error) = self.dispatch("workspace.create", serde_json::json!({})) {
                    eprintln!("berth-gui: workspace.create: {error}");
                }
            }
            Command::NewWindow => self.new_window(),
            Command::NextWindow => self.next_window(),
            Command::TogglePaneZoom => self.toggle_pane_zoom(),
            Command::ToggleBroadcast => self.toggle_broadcast(),
            Command::DuplicateTab => self.surface_action("surface.duplicate"),
            Command::RestartTab => self.surface_action("surface.restart"),
            Command::SidebarFilter => {
                self.sidebar_filter_active = true;
                self.sidebar_filter.clear();
                self.request_redraw();
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
            Command::JumpPrompt(direction) => self.jump_prompt(direction),
            Command::CopyMode => self.toggle_copy_mode(),
            Command::PasteHistory => self.open_paste_history(),
            Command::ZoomIn => self.zoom_font(1.0),
            Command::ZoomOut => self.zoom_font(-1.0),
            Command::ZoomReset => self.reset_zoom(),
        }
        self.request_redraw();
    }

    // ---- window and pane actions ---------------------------------------

    fn new_window(&mut self) {
        if let Err(error) = self.dispatch("window.create", serde_json::json!({})) {
            eprintln!("berth-gui: window.create: {error}");
        }
        self.request_redraw();
    }

    fn next_window(&mut self) {
        let (windows, current) = {
            let state = lock(&self.state);
            (state.windows.clone(), state.selected_window.clone())
        };
        if windows.len() < 2 {
            return;
        }
        let index = windows
            .iter()
            .position(|w| Some(&w.id) == current.as_ref())
            .unwrap_or(0);
        let next = windows[(index + 1) % windows.len()].id.clone();
        let _ = self.dispatch("window.select", serde_json::json!({"window_id": next}));
    }

    fn toggle_pane_zoom(&mut self) {
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        let pane_id = lock(&self.state)
            .surface(&surface_id)
            .map(|surface| surface.pane_id.clone());
        let Some(pane_id) = pane_id else {
            return;
        };
        if let Err(error) = self.dispatch("pane.zoom", serde_json::json!({"pane_id": pane_id})) {
            eprintln!("berth-gui: pane.zoom: {error}");
        }
        self.sized.clear();
        self.request_redraw();
    }

    fn toggle_broadcast(&mut self) {
        let Some(window_id) = self.current_core_window() else {
            return;
        };
        let current = self
            .current_core_window()
            .and_then(|id| lock(&self.state).window(&id).map(|w| w.broadcast))
            .unwrap_or(false);
        if let Err(error) = self.dispatch(
            "window.update",
            serde_json::json!({"window_id": window_id, "broadcast": !current}),
        ) {
            eprintln!("berth-gui: window.update: {error}");
        }
        self.request_redraw();
    }

    fn surface_action(&mut self, method: &str) {
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        if let Err(error) = self.dispatch(method, serde_json::json!({"surface_id": surface_id})) {
            eprintln!("berth-gui: {method}: {error}");
        }
    }

    fn context_action(&mut self, index: usize, surface_id: Option<&str>, pane_id: Option<&str>) {
        match index {
            0 => {
                let Some(surface_id) = surface_id else {
                    return;
                };
                self.copy_surface_selection(surface_id);
            }
            1 => {
                // Paste belongs to the pane the menu was opened on, so focus
                // moves there first rather than to whichever pane was active.
                let Some(surface_id) = surface_id else {
                    return;
                };
                self.activate(surface_id);
                self.paste();
            }
            2 => {
                let params = pane_id
                    .map(|pane_id| serde_json::json!({"pane_id": pane_id}))
                    .unwrap_or_else(|| serde_json::json!({}));
                let _ = self.dispatch("surface.create", params);
            }
            3..=5 => {
                let Some(surface_id) = surface_id else {
                    return;
                };
                let method = match index {
                    3 => "surface.duplicate",
                    4 => "surface.restart",
                    _ => "surface.close",
                };
                let _ = self.dispatch(method, serde_json::json!({"surface_id": surface_id}));
            }
            6 | 7 => {
                let Some(surface_id) = surface_id else {
                    return;
                };
                let direction = if index == 6 { "right" } else { "down" };
                let _ = self.dispatch(
                    "surface.split",
                    serde_json::json!({"surface_id": surface_id, "direction": direction}),
                );
            }
            8 => self.toggle_pane_zoom(),
            9 => self.toggle_broadcast(),
            10 => {
                let Some(surface_id) = surface_id else {
                    return;
                };
                let (windows, current) = {
                    let state = lock(&self.state);
                    (state.windows.clone(), state.selected_window.clone())
                };
                if windows.len() < 2 {
                    return;
                }
                let index = windows
                    .iter()
                    .position(|window| Some(&window.id) == current.as_ref())
                    .unwrap_or(0);
                let target = windows[(index + 1) % windows.len()].id.clone();
                let _ = self.dispatch(
                    "surface.move",
                    serde_json::json!({"surface_id": surface_id, "window_id": target}),
                );
            }
            _ => {}
        }
        self.request_redraw();
    }

    /// Surfaces that receive typed input. Broadcast returns the active tab of
    /// every pane in the visible workspace.
    fn input_targets(&self) -> Vec<String> {
        let state = lock(&self.state);
        let broadcast = self
            .current_core_window()
            .and_then(|id| state.window(&id).map(|window| window.broadcast))
            .unwrap_or(false);
        if !broadcast {
            return ui::active_surface(&state).into_iter().collect();
        }
        let Some(workspace_id) = ui::active_workspace(&state) else {
            return Vec::new();
        };
        let Some(workspace) = state.workspace(&workspace_id) else {
            return Vec::new();
        };
        workspace
            .root
            .leaves()
            .iter()
            .filter_map(|pane_id| {
                state
                    .panes
                    .get(*pane_id)
                    .and_then(|pane| pane.active_surface())
                    .map(str::to_string)
            })
            .collect()
    }

    /// Arrow-key spatial focus over the visible pane rectangles.
    fn focus_direction(&mut self, direction: char) {
        let Some(current) = self.active_surface() else {
            return;
        };
        let nav = self.nav();
        let panes = {
            let state = lock(&self.state);
            ui::build_panes(
                &state,
                nav.area,
                nav.cell.height,
                nav.tab_height,
                None,
                self.pane_options(),
            )
            .0
        };
        let Some(source) = panes
            .iter()
            .find(|pane| pane.surface.as_deref() == Some(current.as_str()))
        else {
            return;
        };
        let center = |rect: Rect| {
            (
                rect.x as f64 + rect.w as f64 / 2.0,
                rect.y as f64 + rect.h as f64 / 2.0,
            )
        };
        let (sx, sy) = center(source.rect);
        let mut best: Option<(f64, &PaneView)> = None;
        for pane in &panes {
            if pane.surface.as_deref() == Some(current.as_str()) {
                continue;
            }
            let (px, py) = center(pane.rect);
            let (dx, dy) = (px - sx, py - sy);
            let (forward, lateral) = match direction {
                'l' if dx < -1.0 => (-dx, dy.abs()),
                'r' if dx > 1.0 => (dx, dy.abs()),
                'u' if dy < -1.0 => (-dy, dx.abs()),
                'd' if dy > 1.0 => (dy, dx.abs()),
                _ => continue,
            };
            let score = forward + lateral * 0.5;
            if best.is_none_or(|(best_score, _)| score < best_score) {
                best = Some((score, pane));
            }
        }
        if let Some((_, pane)) = best
            && let Some(surface) = &pane.surface
        {
            self.activate(&surface.clone());
        }
    }

    /// Open native windows for new core windows and drop closed ones.
    fn sync_windows(&mut self, event_loop: &ActiveEventLoop) {
        let windows = lock(&self.state).windows.clone();
        let live: std::collections::HashSet<String> =
            windows.iter().map(|window| window.id.clone()).collect();
        let stale: Vec<WindowId> = self
            .core_of
            .iter()
            .filter(|(_, core)| !live.contains(*core))
            .map(|(native, _)| *native)
            .collect();
        for native in stale {
            self.core_of.remove(&native);
            if self.current_window_id() == Some(native) {
                self.window = None;
                self.context = None;
                self.surface = None;
                self.a11y = None;
                self.overlay = None;
                self.drag = Drag::None;
                if let Some(next) = self.stash.keys().next().copied() {
                    self.load_window(next);
                }
            } else {
                self.stash.remove(&native);
            }
        }
        let known: std::collections::HashSet<String> = self.core_of.values().cloned().collect();
        for core in &windows {
            if known.contains(&core.id) {
                continue;
            }
            let previous = self.current_window_id();
            if previous.is_some() {
                self.stash_active();
            }
            match self.open_window(event_loop, core) {
                Ok(()) => {
                    let opened = self.current_window_id();
                    self.stash_active();
                    if let Some(id) = previous.or(opened) {
                        self.load_window(id);
                    }
                }
                Err(error) => {
                    eprintln!("berth-gui: window: {error}");
                    if let Some(id) = previous {
                        self.load_window(id);
                    }
                }
            }
        }
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
            Overlay::Search { hits, .. } => hits.len(),
            Overlay::ClipboardRead { .. } => 2,
            Overlay::PasteHistory { .. } => self.paste_history.len(),
            Overlay::TabMenu { pane_id, .. } => lock(&self.state)
                .panes
                .get(pane_id)
                .map(|pane| pane.surfaces.len())
                .unwrap_or(0),
            Overlay::Context { .. } => Self::context_labels().len(),
        }
    }

    fn context_labels() -> [&'static str; 11] {
        [
            "Copy",
            "Paste",
            "New Tab",
            "Duplicate Tab",
            "Restart Tab",
            "Close Tab",
            "Split Right",
            "Split Down",
            "Zoom Pane",
            "Broadcast Input",
            "Move Tab to Next Window",
        ]
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
        let mut clipboard_deny: Option<(String, String)> = None;
        let mut jump_to: Option<search::Hit> = None;
        let mut marks: Option<HashMap<String, Vec<search::Span>>> = None;
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
            Overlay::TabMenu { pane_id, selected } => {
                let len = lock(&self.state)
                    .panes
                    .get(pane_id)
                    .map(|pane| pane.surfaces.len())
                    .unwrap_or(0);
                match &event.logical_key {
                    Key::Named(NamedKey::Escape) => close = true,
                    Key::Named(NamedKey::Enter) => activate = true,
                    Key::Named(NamedKey::ArrowDown) => {
                        next_selected = Some(panel::move_selection(*selected, 1, len));
                    }
                    Key::Named(NamedKey::ArrowUp) => {
                        next_selected = Some(panel::move_selection(*selected, -1, len));
                    }
                    _ => {}
                }
            }
            Overlay::Context { selected, .. } => {
                let len = Self::context_labels().len();
                match &event.logical_key {
                    Key::Named(NamedKey::Escape) => close = true,
                    Key::Named(NamedKey::Enter) => activate = true,
                    Key::Named(NamedKey::ArrowDown) => {
                        next_selected = Some(panel::move_selection(*selected, 1, len));
                    }
                    Key::Named(NamedKey::ArrowUp) => {
                        next_selected = Some(panel::move_selection(*selected, -1, len));
                    }
                    _ => {}
                }
            }
            Overlay::Search {
                panes,
                query,
                options,
                compiled,
                hits,
                selected,
            } => {
                let mut changed = false;
                let mut jump = false;
                match &event.logical_key {
                    Key::Named(NamedKey::Escape) => {
                        self.search_marks = SearchMarks::default();
                        close = true;
                    }
                    Key::Named(NamedKey::Enter) => {
                        if !hits.is_empty() {
                            let delta = if self.modifiers.shift_key() { -1 } else { 1 };
                            let next =
                                (*selected as i32 + delta).rem_euclid(hits.len() as i32) as usize;
                            *selected = next;
                            jump = true;
                        }
                    }
                    Key::Named(NamedKey::ArrowDown) => {
                        if !hits.is_empty() {
                            *selected = (*selected + 1) % hits.len();
                            jump = true;
                        }
                    }
                    Key::Named(NamedKey::ArrowUp) => {
                        if !hits.is_empty() {
                            *selected = (*selected + hits.len() - 1) % hits.len();
                            jump = true;
                        }
                    }
                    Key::Named(NamedKey::Backspace) => {
                        query.pop();
                        changed = true;
                    }
                    // Cmd/Ctrl+R and Cmd/Ctrl+W toggle the two modes, so the
                    // panel needs no second row of controls.
                    Key::Character(text)
                        if (self.modifiers.super_key() || self.modifiers.control_key())
                            && !self.modifiers.shift_key() =>
                    {
                        match text.to_lowercase().as_str() {
                            "r" => {
                                options.regex = !options.regex;
                                changed = true;
                            }
                            "w" => {
                                options.whole_word = !options.whole_word;
                                changed = true;
                            }
                            _ => {}
                        }
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
                    let (fresh, fresh_hits) = Self::search_run(panes, query, *options);
                    *compiled = fresh;
                    *hits = fresh_hits;
                    *selected = 0;
                    jump = true;
                }
                if jump && let Some(hit) = hits.get(*selected) {
                    let hit = hit.clone();
                    // Group the hits per pane now, so dismissing the panel
                    // leaves them tinted without a second pass over the buffers.
                    marks = Some(Self::marks_from(hits));
                    jump_to = Some(hit);
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
            Overlay::ClipboardRead {
                surface_id,
                selection,
                ..
            } => match &event.logical_key {
                Key::Named(NamedKey::Escape) => {
                    clipboard_deny = Some((surface_id.clone(), selection.clone()));
                    close = true;
                }
                Key::Named(NamedKey::Enter) => activate = true,
                Key::Named(NamedKey::ArrowUp) => next_selected = Some(0),
                Key::Named(NamedKey::ArrowDown) => next_selected = Some(1),
                _ => {}
            },
            Overlay::PasteHistory { .. } => {
                let len = self.paste_history.len();
                match &event.logical_key {
                    Key::Named(NamedKey::Escape) => close = true,
                    Key::Named(NamedKey::Enter) => activate = true,
                    Key::Named(NamedKey::ArrowDown) => {
                        next_selected = Some(panel::move_selection(current, 1, len));
                    }
                    Key::Named(NamedKey::ArrowUp) => {
                        next_selected = Some(panel::move_selection(current, -1, len));
                    }
                    _ => {}
                }
            }
        }
        if let Some((target, value)) = rename_commit {
            self.commit_rename(&target, &value);
        }
        if let Some((surface_id, selection)) = clipboard_deny {
            self.send_clipboard_response(&surface_id, &selection, None);
        }
        if let Some(hit) = jump_to {
            self.jump_to_match(&hit);
        }
        if let Some(by_surface) = marks {
            self.search_marks.by_surface = by_surface;
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

    /// Zoom chords step the point size and keep the choice, like the settings
    /// row does.
    /// Zoom is a session offset from the configured size, so the reset chord
    /// and a relaunch both return to `fontSize` without writing the file.
    fn font_points(&self) -> f32 {
        self.config.font_size + self.font_zoom
    }

    fn zoom_font(&mut self, delta: f32) {
        let total =
            (self.font_points() + delta).clamp(config::MIN_FONT_SIZE, config::MAX_FONT_SIZE);
        let zoom = total - self.config.font_size;
        if (zoom - self.font_zoom).abs() < f32::EPSILON {
            return;
        }
        self.font_zoom = zoom;
        self.invalidate_fonts();
        self.request_redraw();
    }

    fn reset_zoom(&mut self) {
        if self.font_zoom == 0.0 {
            return;
        }
        self.font_zoom = 0.0;
        self.invalidate_fonts();
        self.request_redraw();
    }

    /// Drop the cached renderers so the next frame rebuilds them at the new
    /// metrics, and let each pane resize against the new cell.
    fn invalidate_fonts(&mut self) {
        self.font = None;
        self.chrome_font = None;
        // Drop pre-warmed renderers too, so a font or scale change is not
        // overwritten by the job spawned at startup.
        self.font_job = None;
        self.chrome_job = None;
        self.sized.clear();
    }

    fn apply_setting(&mut self, change: settings::Change) {
        if change == settings::Change::None {
            return;
        }
        if change == settings::Change::Font {
            self.font_zoom = 0.0;
            self.invalidate_fonts();
        }
        if change == settings::Change::Theme {
            self.theme = self.config.theme();
        }
        if change == settings::Change::Opacity
            && let Some(window) = &self.window
        {
            crate::window::set_opacity(window, self.config.opacity);
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

    /// Open the search panel over every pane of the visible workspace, so a
    /// hit in a neighbouring pane is one list entry away. The buffers are
    /// captured once; output that arrives while the panel is open is not
    /// indexed, because a list that shifts under the selection is worse than
    /// one that lags.
    fn open_search(&mut self) {
        let panes = self.search_buffers();
        if panes.is_empty() {
            return;
        }
        self.search_marks = SearchMarks::default();
        self.overlay = Some(Overlay::Search {
            panes,
            query: String::new(),
            options: search::Options::default(),
            compiled: search::Query::new("", search::Options::default()),
            hits: Vec::new(),
            selected: 0,
        });
        self.request_redraw();
    }

    /// One buffer per pane in the visible workspace, in pane order, each with
    /// the title shown beside its hits.
    fn search_buffers(&self) -> Vec<search::PaneBuffer> {
        let surfaces = {
            let state = lock(&self.state);
            let Some(workspace_id) = ui::active_workspace(&state) else {
                return Vec::new();
            };
            let Some(workspace) = state.workspace(&workspace_id) else {
                return Vec::new();
            };
            workspace
                .root
                .leaves()
                .iter()
                .filter_map(|pane_id| {
                    let pane = state.panes.get(*pane_id)?;
                    let surface_id = pane.active_surface()?.to_string();
                    let surface = state.surface(&surface_id)?;
                    Some((surface_id, ui::tab_title(surface, pane.active)))
                })
                .collect::<Vec<_>>()
        };
        surfaces
            .into_iter()
            .filter_map(|(surface_id, title)| {
                let lines = self.host.scrollback_lines(&surface_id).ok()?;
                Some(search::PaneBuffer {
                    surface_id,
                    title,
                    lines,
                })
            })
            .collect()
    }

    /// Group hits by surface so the painter can ask for one pane's rows.
    fn marks_from(hits: &[search::Hit]) -> HashMap<String, Vec<search::Span>> {
        let mut by_surface: HashMap<String, Vec<search::Span>> = HashMap::new();
        for hit in hits {
            by_surface
                .entry(hit.surface_id.clone())
                .or_default()
                .push(hit.span());
        }
        by_surface
    }

    /// Compile `text` and re-run the search over every buffer.
    fn search_run(
        panes: &[search::PaneBuffer],
        query: &str,
        options: search::Options,
    ) -> (search::Query, Vec<search::Hit>) {
        let compiled = search::Query::new(query, options);
        let hits = search::search(panes, &compiled);
        (compiled, hits)
    }

    /// Walk to the previous (`-1`) or next (`1`) prompt the shell marked. The
    /// current view's top line names the position, so the first press moves
    /// relative to what is on screen rather than to the newest prompt.
    fn jump_prompt(&mut self, direction: i32) {
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        let Ok(view) = self.host.screen_view(&surface_id) else {
            return;
        };
        let Ok(lines) = self.host.prompt_lines(&surface_id) else {
            return;
        };
        if lines.is_empty() {
            return;
        }
        let top = view.scrollback_len.saturating_sub(view.scrollback);
        let target = if direction < 0 {
            lines
                .iter()
                .rev()
                .find(|line| **line < top)
                .or_else(|| lines.first())
                .copied()
        } else {
            // The last prompt at or below the top, then the one after it. At
            // the live screen the newest prompt is the target.
            lines
                .iter()
                .find(|line| **line > top)
                .or_else(|| lines.last())
                .copied()
        };
        let Some(target) = target else {
            return;
        };
        let offset = view.scrollback_len.saturating_sub(target);
        if let Ok(applied) = self.host.set_scrollback(&surface_id, offset) {
            self.scroll.insert(surface_id, applied);
        }
        self.request_redraw();
    }

    /// The hits to paint in one pane, and the one the panel was on if it was
    /// this pane. The slice is copied because the painter holds it while the
    /// window is mutably borrowed.
    fn search_marks_for(&self, surface_id: &str) -> (Vec<search::Span>, Option<search::Span>) {
        let hits = self
            .search_marks
            .by_surface
            .get(surface_id)
            .cloned()
            .unwrap_or_default();
        let current = self
            .search_marks
            .current
            .as_ref()
            .filter(|hit| hit.surface_id == surface_id)
            .map(|hit| (hit.line, hit.col, hit.len));
        (hits, current)
    }

    /// Open the paste-history picker, newest entry first. The clipboard itself
    /// is not recorded: the history is what the user copied or pasted in this
    /// session, and seeding it with whatever an external app copied would make
    /// the top row untrustworthy.
    fn open_paste_history(&mut self) {
        if self.active_surface().is_none() {
            return;
        }
        // The chord overlaps the fixed paste binding on macOS, so an empty
        // history must still paste rather than swallow the key.
        if self.paste_history.is_empty() {
            self.paste();
            return;
        }
        self.overlay = Some(Overlay::PasteHistory { selected: 0 });
        self.request_redraw();
    }

    /// Paste one history entry. A multi-line entry still asks for the paste
    /// confirmation a multi-line clipboard paste asks for, through the same path.
    fn paste_history_entry(&mut self, selected: usize) {
        let Some(entry) = self.paste_history.get(selected) else {
            return;
        };
        let text = entry.text.clone();
        if text.lines().count() > 1 {
            self.overlay = Some(Overlay::Confirm { text });
            self.request_redraw();
            return;
        }
        self.send_paste(&text);
    }

    /// Open copy mode on the focused pane, or close it if it is already open.
    /// The cursor starts on the newest line, which is where the shell left it,
    /// and the buffer is captured so a yank can span more than one screen.
    fn toggle_copy_mode(&mut self) {
        if let Some(mode) = self.copy_mode.take() {
            self.selection.remove(&mode.surface_id);
            self.last_selection = None;
            self.request_redraw();
            return;
        }
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        let lines = match self.host.scrollback_lines(&surface_id) {
            Ok(lines) if !lines.is_empty() => lines,
            _ => return,
        };
        let mut session = copymode::CopyMode::new(surface_id, lines);
        session.cursor = (session.last_line(), 0);
        self.copy_mode = Some(session);
        self.request_redraw();
    }

    /// A click in copy mode: put the cursor on the clicked cell, and extend a
    /// visual selection when one is open.
    fn copy_mode_click(&mut self, surface_id: &str, target: Cell) {
        let Ok(view) = self.host.screen_view(surface_id) else {
            return;
        };
        let top = view.scrollback_len.saturating_sub(view.scrollback);
        let line = top.saturating_add(usize::from(target.row));
        let Some(mode) = self.copy_mode.as_mut() else {
            return;
        };
        mode.cursor = (line, target.col);
        mode.clamp_column();
        self.copy_mode_paint_selection(surface_id);
        self.request_redraw();
    }

    /// One key inside copy mode. Every key is consumed: a character the mode
    /// does not use is swallowed rather than typed into the shell.
    fn copy_mode_key(&mut self, event: &KeyEvent) {
        let Some(mode) = self.copy_mode.as_ref() else {
            return;
        };
        let surface_id = mode.surface_id.clone();
        let key = match &event.logical_key {
            Key::Character(text) => text.to_string(),
            Key::Named(name) => match name {
                NamedKey::Escape => "escape",
                NamedKey::Enter => "enter",
                NamedKey::ArrowLeft => "left",
                NamedKey::ArrowRight => "right",
                NamedKey::ArrowUp => "up",
                NamedKey::ArrowDown => "down",
                NamedKey::Home => "home",
                NamedKey::End => "end",
                NamedKey::PageUp => "pageup",
                NamedKey::PageDown => "pagedown",
                NamedKey::Space => "space",
                _ => "ignore",
            }
            .to_string(),
            _ => "ignore".to_string(),
        };
        // Space exits, so the mode can be left without a mouse; so does the
        // chord that opened it, which arrives here as `Ctrl+Shift+Space`.
        if key == "space" {
            self.copy_mode_leave(&surface_id);
            return;
        }
        let action = copymode::action_for(
            &key,
            self.modifiers.shift_key(),
            self.modifiers.control_key(),
        );
        self.copy_mode_apply(action, &surface_id);
    }

    /// Close copy mode, dropping the selection it painted.
    fn copy_mode_leave(&mut self, surface_id: &str) {
        self.copy_mode = None;
        self.selection.remove(surface_id);
        self.last_selection = None;
        self.request_redraw();
    }

    fn copy_mode_apply(&mut self, action: copymode::CopyAction, surface_id: &str) {
        use copymode::CopyAction;
        let Some(mode) = self.copy_mode.as_mut() else {
            return;
        };
        match action {
            CopyAction::Digit(digit) => {
                mode.push_digit(digit);
                self.request_redraw();
                return;
            }
            CopyAction::Exit => {
                self.copy_mode_leave(surface_id);
                return;
            }
            CopyAction::Visual => mode.toggle_visual(),
            CopyAction::Yank => {
                if mode.is_visual() {
                    self.copy_mode_yank(surface_id);
                } else {
                    self.copy_mode_leave(surface_id);
                }
                return;
            }
            _ => {}
        }
        let count = mode.take_count();
        mode.count = None;
        let rows = {
            let Ok(view) = self.host.screen_view(surface_id) else {
                return;
            };
            usize::from(view.rows)
        };
        let Some(mode) = self.copy_mode.as_mut() else {
            return;
        };
        let last = mode.last_line();
        match action {
            CopyAction::MoveLeft => {
                mode.cursor.1 = mode.cursor.1.saturating_sub(count as u16);
            }
            CopyAction::MoveRight => {
                let end = mode.cursor_line_end();
                // A repetition may run into the next line, as vim's `l` does
                // across a line end when the caller asked for a count.
                let mut target = usize::from(mode.cursor.1).saturating_add(count);
                let mut line = mode.cursor.0;
                while target > usize::from(end) && line < last {
                    target = target.saturating_sub(usize::from(end) + 1);
                    line += 1;
                }
                mode.cursor = (line, target.min(usize::from(end)) as u16);
            }
            CopyAction::MoveUp => mode.move_lines(-(count as i64), last),
            CopyAction::MoveDown => mode.move_lines(count as i64, last),
            CopyAction::LineStart => mode.cursor.1 = 0,
            CopyAction::LineEnd => mode.cursor.1 = mode.cursor_line_end(),
            CopyAction::WordForward | CopyAction::WordBack | CopyAction::WordEnd => {
                mode.move_word(action, count)
            }
            CopyAction::Top => {
                mode.cursor = (0, 0);
            }
            CopyAction::Bottom => {
                mode.cursor = (last, 0);
            }
            CopyAction::PageUp(half) | CopyAction::PageDown(half) => {
                let step = if half { (rows / 2).max(1) } else { rows.max(1) };
                let delta = step.saturating_mul(count) as i64;
                let delta = if matches!(action, CopyAction::PageUp(_)) {
                    -delta
                } else {
                    delta
                };
                mode.move_lines(delta, last);
            }
            _ => {}
        }
        self.copy_mode_follow(surface_id);
        self.copy_mode_paint_selection(surface_id);
        self.request_redraw();
    }

    /// Scroll the view so the copy-mode cursor is on screen.
    fn copy_mode_follow(&mut self, surface_id: &str) {
        let Some(mode) = self.copy_mode.as_ref() else {
            return;
        };
        let line = mode.cursor.0;
        let Ok(view) = self.host.screen_view(surface_id) else {
            return;
        };
        let top = view.scrollback_len.saturating_sub(view.scrollback);
        let rows = usize::from(view.rows);
        if let Some(top) = copymode::scroll_to_show(line, top, rows, view.scrollback_len) {
            let offset = view.scrollback_len.saturating_sub(top);
            if let Ok(applied) = self.host.set_scrollback(surface_id, offset) {
                self.scroll.insert(surface_id.to_string(), applied);
            }
        }
    }

    /// Publish the visual selection to the painter. Copy mode owns the
    /// selection while it is open, so the mouse path cannot fight it.
    fn copy_mode_paint_selection(&mut self, surface_id: &str) {
        let Some(mode) = self.copy_mode.as_ref() else {
            return;
        };
        let Some((start, end)) = mode.selection() else {
            self.selection.remove(surface_id);
            return;
        };
        let Ok(view) = self.host.screen_view(surface_id) else {
            return;
        };
        let top = view.scrollback_len.saturating_sub(view.scrollback);
        // Both ends are absolute lines; the painter wants visible rows.
        let row = |line: usize| line.saturating_sub(top).min(usize::from(u16::MAX)) as u16;
        let selection = Selection {
            anchor: Cell {
                row: row(start.0),
                col: start.1,
            },
            head: Cell {
                row: row(end.0),
                col: end.1,
            },
            dragging: false,
        };
        self.selection.insert(surface_id.to_string(), selection);
        self.last_selection = Some(surface_id.to_string());
    }

    /// Copy a visual selection from the captured buffer, which spans more than
    /// one screen. The text is trimmed the way the mouse copy trims it.
    fn copy_mode_yank(&mut self, surface_id: &str) {
        let Some(mode) = self.copy_mode.as_ref() else {
            return;
        };
        let Some((start, end)) = mode.selection() else {
            self.copy_mode_leave(surface_id);
            return;
        };
        let text = copymode::yank_text(&mode.lines, start, end);
        if !text.is_empty() && self.copy_text(&text) {
            self.paste_history.record(&text);
        }
        self.copy_mode_leave(surface_id);
    }

    fn jump_to_match(&mut self, hit: &search::Hit) {
        let focused = self.active_surface();
        if focused.as_deref() != Some(hit.surface_id.as_str()) {
            self.activate(&hit.surface_id);
        }
        let surface_id = hit.surface_id.as_str();
        let Ok(view) = self.host.screen_view(surface_id) else {
            return;
        };
        let target = view.scrollback_len.saturating_sub(hit.line);
        if let Ok(applied) = self.host.set_scrollback(surface_id, target) {
            self.scroll.insert(hit.surface_id.clone(), applied);
        }
        self.search_marks.current = Some(SearchHighlight {
            surface_id: hit.surface_id.clone(),
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
        let nav = self.nav();
        let point = (
            self.pointer.0.max(0.0) as usize,
            self.pointer.1.max(0.0) as usize,
        );
        let (panes, _) = {
            let state = lock(&self.state);
            ui::build_panes(
                &state,
                nav.area,
                nav.cell.height,
                nav.tab_height,
                None,
                self.pane_options(),
            )
        };
        // Dropping over another pane moves the tab there.
        if let Some(target) = ui::pane_at(&panes, point.0, point.1) {
            if target.pane_id != pane_id {
                let _ = self.dispatch(
                    "surface.move",
                    serde_json::json!({"surface_id": surface_id, "pane_id": target.pane_id}),
                );
            } else if start_x.abs_diff(end_x) >= 6 {
                let index = ui::tab_index_at(&target.tabs, end_x);
                let _ = self.dispatch(
                    "surface.move",
                    serde_json::json!({"surface_id": surface_id, "index": index}),
                );
            }
            return;
        }
        if start_x.abs_diff(end_x) < 6 {
            return;
        }
        // Outside every pane: another window, a torn-off window, or the source
        // strip when the pointer landed on chrome.
        let Some((global_x, global_y)) = self.global_pointer() else {
            return;
        };
        if let Some(core_window) = self.core_window_at(global_x, global_y) {
            let _ = self.dispatch(
                "surface.move",
                serde_json::json!({"surface_id": surface_id, "window_id": core_window}),
            );
            return;
        }
        if !self.inside_current_window(global_x, global_y) {
            if let Ok(created) = self.dispatch("window.create", serde_json::json!({"name": "tab"}))
                && let Some(id) = created["window"]["id"].as_str()
            {
                let _ = self.dispatch(
                    "surface.move",
                    serde_json::json!({"surface_id": surface_id, "window_id": id}),
                );
            }
            return;
        }
        if let Some(pane) = panes.iter().find(|pane| pane.pane_id == pane_id) {
            let index = ui::tab_index_at(&pane.tabs, end_x);
            let _ = self.dispatch(
                "surface.move",
                serde_json::json!({"surface_id": surface_id, "index": index}),
            );
        }
    }

    fn global_pointer(&self) -> Option<(f64, f64)> {
        let window = self.window.as_ref()?;
        let origin = window.outer_position().ok()?;
        Some((
            origin.x as f64 + self.pointer.0,
            origin.y as f64 + self.pointer.1,
        ))
    }

    fn window_bounds(&self, window: &Window) -> Option<(f64, f64, f64, f64)> {
        let origin = window.outer_position().ok()?;
        let size = window.inner_size();
        Some((
            origin.x as f64,
            origin.y as f64,
            size.width as f64,
            size.height as f64,
        ))
    }

    /// The core window whose native window is under a global point.
    fn core_window_at(&self, x: f64, y: f64) -> Option<String> {
        for (native, stash) in &self.stash {
            let Some((left, top, width, height)) = self.window_bounds(&stash.window) else {
                continue;
            };
            if x >= left && y >= top && x < left + width && y < top + height {
                return self.core_of.get(native).cloned();
            }
        }
        None
    }

    fn inside_current_window(&self, x: f64, y: f64) -> bool {
        let Some(window) = self.window.as_ref() else {
            return false;
        };
        let Some((left, top, width, height)) = self.window_bounds(window) else {
            return false;
        };
        x >= left && y >= top && x < left + width && y < top + height
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
                panes,
                query,
                options,
                compiled,
                hits,
                selected,
            } => {
                // The title carries the count and the mode chips, so the panel
                // says what it is matching without a second row of controls.
                let mut title = if hits.is_empty() {
                    "Search".to_string()
                } else {
                    format!("Search {}/{}", selected + 1, hits.len())
                };
                let mut chips = Vec::new();
                if options.regex {
                    chips.push("regex");
                }
                if options.whole_word {
                    chips.push("whole word");
                }
                // A query that compiled but found nothing reads differently
                // from one that could not compile at all.
                if let Some(error) = &compiled.error {
                    chips.push(error.as_str());
                } else if !compiled.is_active() && !query.is_empty() {
                    chips.push("empty pattern");
                }
                if !chips.is_empty() {
                    title.push_str("  ·  ");
                    title.push_str(&chips.join(" · "));
                }
                Some(PanelContent {
                    title,
                    query: Some(query.clone()),
                    rows: hits
                        .iter()
                        .map(|hit| {
                            let label = panes
                                .iter()
                                .find(|pane| pane.surface_id == hit.surface_id)
                                .and_then(|pane| pane.lines.get(hit.line))
                                .map(|line| line.trim().to_string())
                                .unwrap_or_default();
                            let title = panes
                                .iter()
                                .find(|pane| pane.surface_id == hit.surface_id)
                                .map(|pane| pane.title.as_str())
                                .unwrap_or_default();
                            panel::PanelRow {
                                label,
                                detail: format!("{title} · line {}", hit.line + 1),
                                color: None,
                            }
                        })
                        .collect(),
                    selected: *selected,
                    hint: "enter next · shift+enter prev · ^r regex · ^w word · esc close"
                        .to_string(),
                })
            }
            Overlay::PasteHistory { selected } => Some(PanelContent {
                title: format!(
                    "Paste History {}/{}",
                    selected + 1,
                    self.paste_history.len()
                ),
                query: None,
                rows: self
                    .paste_history
                    .entries()
                    .iter()
                    .map(|entry| panel::PanelRow {
                        label: entry.preview().to_string(),
                        detail: match entry.line_count() {
                            1 => "1 line".to_string(),
                            lines => format!("{lines} lines"),
                        },
                        color: None,
                    })
                    .collect(),
                selected: *selected,
                hint: "↑↓ move · enter paste · esc close".to_string(),
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
            Overlay::ClipboardRead {
                surface_id,
                selected,
                ..
            } => {
                let label = lock(&self.state)
                    .surface(surface_id)
                    .map(|surface| surface.title.clone())
                    .filter(|title| !title.is_empty())
                    .unwrap_or_else(|| surface_id.clone());
                Some(PanelContent {
                    title: "Clipboard read".to_string(),
                    query: None,
                    rows: vec![
                        panel::PanelRow {
                            label: "Allow".to_string(),
                            detail: format!("send the clipboard to {label}"),
                            color: None,
                        },
                        panel::PanelRow {
                            label: "Deny".to_string(),
                            detail: "send an empty reply".to_string(),
                            color: None,
                        },
                    ],
                    selected: *selected,
                    hint: "↑↓ choose · enter confirm · esc deny".to_string(),
                })
            }
            Overlay::TabMenu { pane_id, selected } => {
                let rows = {
                    let state = lock(&self.state);
                    state
                        .panes
                        .get(pane_id)
                        .map(|pane| {
                            pane.surfaces
                                .iter()
                                .enumerate()
                                .map(|(index, surface_id)| {
                                    let label = state
                                        .surface(surface_id)
                                        .map(|surface| ui::tab_title(surface, index))
                                        .unwrap_or_else(|| surface_id.clone());
                                    panel::PanelRow {
                                        label,
                                        detail: if index == pane.active {
                                            "active".to_string()
                                        } else {
                                            String::new()
                                        },
                                        color: None,
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                };
                Some(PanelContent {
                    title: "Tabs".to_string(),
                    query: None,
                    rows,
                    selected: *selected,
                    hint: "↑↓ move · enter switch · esc close".to_string(),
                })
            }
            Overlay::Context { selected, .. } => Some(PanelContent {
                title: "Actions".to_string(),
                query: None,
                rows: Self::context_labels()
                    .iter()
                    .map(|label| panel::PanelRow {
                        label: (*label).to_string(),
                        detail: String::new(),
                        color: None,
                    })
                    .collect(),
                selected: *selected,
                hint: "↑↓ move · enter run · esc close".to_string(),
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
            Some(Overlay::PasteHistory { selected }) => self.paste_history_entry(selected),
            Some(Overlay::Rename { .. }) | Some(Overlay::Search { .. }) => {}
            Some(Overlay::Confirm { text }) => {
                self.paste_history.record(&text);
                self.send_paste(&text);
            }
            Some(Overlay::ClipboardRead {
                surface_id,
                selection,
                selected,
            }) => {
                let text = if selected == 0 {
                    self.clipboard
                        .as_mut()
                        .and_then(|clipboard| clipboard.get_text().ok())
                } else {
                    None
                };
                self.send_clipboard_response(&surface_id, &selection, text.as_deref());
            }
            Some(Overlay::TabMenu { pane_id, selected }) => {
                let surface_id = lock(&self.state)
                    .panes
                    .get(&pane_id)
                    .and_then(|pane| pane.surfaces.get(selected).cloned());
                if let Some(surface_id) = surface_id {
                    self.activate(&surface_id);
                }
            }
            Some(Overlay::Context {
                surface_id,
                pane_id,
                selected,
            }) => self.context_action(selected, surface_id.as_deref(), pane_id.as_deref()),
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

    /// Answer an OSC 52 read. `None` is a denial and sends an empty payload.
    fn send_clipboard_response(&mut self, surface_id: &str, selection: &str, text: Option<&str>) {
        let bytes = clipboard::response(selection, text);
        if let Err(error) = self.host.send_bytes(surface_id, &bytes) {
            log::line(&format!("clipboard reply {surface_id}: {error}"));
        }
    }

    /// Open the confirmation for a queued OSC 52 read when no other panel is up.
    fn open_pending_clipboard(&mut self) {
        if self.overlay.is_some() {
            return;
        }
        let Some((surface_id, selection)) = self.pending_clipboard.pop_front() else {
            return;
        };
        self.overlay = Some(Overlay::ClipboardRead {
            surface_id,
            selection,
            // Start on Deny so an Enter already in flight cannot approve it.
            selected: 1,
        });
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
        self.persist_window_state();
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
                for surface_id in &self.input_targets() {
                    if let Err(error) = self.host.send_text(surface_id, &text) {
                        eprintln!("berth-gui: ime: {error}");
                    }
                    self.reset_scroll(surface_id);
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
            ui::build_panes(
                &state,
                area,
                cell.height,
                self.tab_height(),
                None,
                self.pane_options(),
            )
            .0
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
        if self.sidebar_filter_active {
            match &event.logical_key {
                Key::Named(NamedKey::Escape) => {
                    self.sidebar_filter_active = false;
                    self.sidebar_filter.clear();
                }
                Key::Named(NamedKey::Enter) => self.sidebar_filter_active = false,
                Key::Named(NamedKey::Backspace) => {
                    self.sidebar_filter.pop();
                }
                Key::Character(text)
                    if !self.modifiers.control_key()
                        && !self.modifiers.super_key()
                        && !self.modifiers.alt_key() =>
                {
                    self.sidebar_filter.push_str(text);
                }
                _ => {}
            }
            self.request_redraw();
            return;
        }
        if self.overlay.is_some() {
            self.overlay_key(&event);
            return;
        }
        // Copy mode is a modal capture in front of the chord table: its keys are
        // vi characters that resolve to no command, so they must be taken here
        // or they would reach the shell. The chord that opens it still closes
        // it, and Escape closes it from inside.
        if self.copy_mode.is_some() {
            self.copy_mode_key(&event);
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
        let targets = self.input_targets();
        if targets.is_empty() {
            return;
        }
        for surface_id in &targets {
            self.reset_scroll(surface_id);
        }
        // Cmd/Super belongs to the platform shell; the window only claims the
        // line motions macOS terminals provide.
        if self.modifiers.super_key() {
            if !self.modifiers.control_key()
                && !self.modifiers.alt_key()
                && !self.modifiers.shift_key()
                && let Some(name) = super_arrow_key(&event.logical_key)
            {
                for surface_id in &targets {
                    let _ = self.host.send_key(surface_id, name, &[]);
                }
            }
            return;
        }

        let action = if event.repeat {
            KeyAction::Repeat
        } else {
            KeyAction::Press
        };
        for surface_id in &targets {
            self.send_key_event(surface_id, &event, action);
        }
    }

    /// A release reaches the PTY only when the program asked for event types.
    fn key_release(&mut self, event: KeyEvent) {
        if self.sidebar_filter_active || self.overlay.is_some() || self.modifiers.super_key() {
            return;
        }
        for surface_id in &self.input_targets() {
            self.send_key_event(surface_id, &event, KeyAction::Release);
        }
    }

    /// Encode one key event for the surface's keyboard protocol.
    fn send_key_event(&mut self, surface_id: &str, event: &KeyEvent, action: KeyAction) {
        let key = key_name(&event.logical_key).unwrap_or_default();
        if key.is_empty() && event.text.is_none() {
            return;
        }
        let mods = self.modifier_names();
        let base_layout = match event.physical_key {
            PhysicalKey::Code(code) => base_layout_char(code),
            _ => None,
        };
        let input = KeyInput {
            key,
            modifiers: &mods,
            action,
            text: event.text.as_deref(),
            has_text_report: true,
            base_layout,
        };
        if let Err(error) = self.host.send_key_event(surface_id, &input) {
            log::line(&format!("key input {surface_id}: {error}"));
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

/// The key name the encoder takes: a layout character, or a named key.
fn key_name(key: &Key) -> Option<&str> {
    match key {
        Key::Named(named) => named_key(*named),
        Key::Character(characters) => Some(characters.as_str()),
        _ => None,
    }
}

/// The base PC-101 layout character for a physical key, reported as the kitty
/// protocol's alternate key. Layouts that are not PC-101 report nothing.
fn base_layout_char(code: KeyCode) -> Option<char> {
    Some(match code {
        KeyCode::KeyA => 'a',
        KeyCode::KeyB => 'b',
        KeyCode::KeyC => 'c',
        KeyCode::KeyD => 'd',
        KeyCode::KeyE => 'e',
        KeyCode::KeyF => 'f',
        KeyCode::KeyG => 'g',
        KeyCode::KeyH => 'h',
        KeyCode::KeyI => 'i',
        KeyCode::KeyJ => 'j',
        KeyCode::KeyK => 'k',
        KeyCode::KeyL => 'l',
        KeyCode::KeyM => 'm',
        KeyCode::KeyN => 'n',
        KeyCode::KeyO => 'o',
        KeyCode::KeyP => 'p',
        KeyCode::KeyQ => 'q',
        KeyCode::KeyR => 'r',
        KeyCode::KeyS => 's',
        KeyCode::KeyT => 't',
        KeyCode::KeyU => 'u',
        KeyCode::KeyV => 'v',
        KeyCode::KeyW => 'w',
        KeyCode::KeyX => 'x',
        KeyCode::KeyY => 'y',
        KeyCode::KeyZ => 'z',
        KeyCode::Digit0 => '0',
        KeyCode::Digit1 => '1',
        KeyCode::Digit2 => '2',
        KeyCode::Digit3 => '3',
        KeyCode::Digit4 => '4',
        KeyCode::Digit5 => '5',
        KeyCode::Digit6 => '6',
        KeyCode::Digit7 => '7',
        KeyCode::Digit8 => '8',
        KeyCode::Digit9 => '9',
        KeyCode::Minus => '-',
        KeyCode::Equal => '=',
        KeyCode::BracketLeft => '[',
        KeyCode::BracketRight => ']',
        KeyCode::Backslash => '\\',
        KeyCode::Semicolon => ';',
        KeyCode::Quote => '\'',
        KeyCode::Backquote => '`',
        KeyCode::Comma => ',',
        KeyCode::Period => '.',
        KeyCode::Slash => '/',
        KeyCode::Space => ' ',
        _ => return None,
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The protocol names the tab step by direction rather than a signed delta.
fn cycle_direction(delta: i32) -> &'static str {
    if delta < 0 { "prev" } else { "next" }
}

#[cfg(test)]
mod blink_timer_tests {
    use super::blinks_needing_timer;

    #[test]
    fn given_blinking_text_in_a_background_pane_then_the_timer_stays_armed() {
        assert!(blinks_needing_timer(true, false, true));
    }

    #[test]
    fn given_nothing_blinking_then_the_timer_sleeps() {
        assert!(!blinks_needing_timer(true, false, false));
    }

    #[test]
    fn given_an_unfocused_window_then_no_blink_timer_runs() {
        assert!(!blinks_needing_timer(false, true, true));
    }
}

#[cfg(test)]
mod key_name_tests {
    use super::{base_layout_char, key_name};
    use winit::keyboard::{Key, KeyCode, NamedKey};

    #[test]
    fn given_a_physical_key_then_its_base_layout_character_is_reported() {
        assert_eq!(base_layout_char(KeyCode::KeyA), Some('a'));
        assert_eq!(base_layout_char(KeyCode::Digit1), Some('1'));
        assert_eq!(base_layout_char(KeyCode::BracketLeft), Some('['));
        assert_eq!(base_layout_char(KeyCode::F1), None);
    }

    #[test]
    fn given_a_key_then_the_encoder_name_is_returned() {
        assert_eq!(key_name(&Key::Named(NamedKey::Enter)), Some("enter"));
        assert_eq!(key_name(&Key::Character("é".into())), Some("é"));
        assert_eq!(key_name(&Key::Named(NamedKey::Shift)), None);
    }
}

#[cfg(test)]
mod cycle_tests {
    use super::cycle_direction;

    #[test]
    fn given_a_step_when_cycling_tabs_then_it_becomes_a_protocol_direction() {
        assert_eq!(cycle_direction(1), "next");
        assert_eq!(cycle_direction(-1), "prev");
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    fn buffer(surface_id: &str, title: &str, lines: &[&str]) -> search::PaneBuffer {
        search::PaneBuffer {
            surface_id: surface_id.to_string(),
            title: title.to_string(),
            lines: lines.iter().map(|line| line.to_string()).collect(),
        }
    }

    #[test]
    fn given_hits_in_two_panes_then_the_marks_are_grouped_by_surface() {
        let hits = vec![
            search::Hit {
                surface_id: "srf_a".into(),
                line: 2,
                col: 0,
                len: 3,
            },
            search::Hit {
                surface_id: "srf_b".into(),
                line: 7,
                col: 4,
                len: 3,
            },
            search::Hit {
                surface_id: "srf_a".into(),
                line: 9,
                col: 1,
                len: 3,
            },
        ];
        let marks = Gui::marks_from(&hits);
        assert_eq!(marks["srf_a"], vec![(2, 0, 3), (9, 1, 3)]);
        assert_eq!(marks["srf_b"], vec![(7, 4, 3)]);
    }

    #[test]
    fn given_a_query_over_two_panes_then_each_pane_buffer_is_searched() {
        let panes = vec![
            buffer("srf_a", "build", &["ok", "error"]),
            buffer("srf_b", "tests", &["error here", "fine"]),
        ];
        let (compiled, hits) = Gui::search_run(&panes, "error", search::Options::default());
        assert!(compiled.is_active());
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].surface_id, "srf_a");
        assert_eq!(hits[1].surface_id, "srf_b");
    }

    #[test]
    fn given_the_regex_toggle_then_the_query_is_read_as_a_pattern() {
        let panes = vec![buffer("srf_a", "build", &["err1 err22"])];
        let options = search::Options {
            regex: true,
            whole_word: false,
        };
        let (_, hits) = Gui::search_run(&panes, r"err\d+", options);
        assert_eq!(hits.len(), 2);
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
