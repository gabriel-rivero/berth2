//! Minimal Linux/Windows window for Berth.
//!
//! One window, the core's split tree, no GPU: `softbuffer` presents 8×16
//! bitmap-font grids and `winit` routes keys and clicks. It embeds the same
//! `berth-server` and `PtyHost` as the headless shell, so the CLI and hooks keep
//! working.
//!
//! Each leaf of the selected workspace gets its own terminal view, sized to its
//! rectangle. Clicking a pane focuses its active surface. Still missing: tabs,
//! sidebar, selection, scroll gestures and clipboard. `--smoke-ms` renders one
//! frame under Xvfb and exits, which is how CI checks the window path.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use berth_core::model::Orientation;
use berth_core::{AppState, PaneNode, SpawnRequest, Surface, SurfaceOps, paths};
use berth_headless::{CellView, PtyHost, ScreenView, restore_session, save_session};
use berth_server::{Server, ServerConfig, ServerHandle, SocketMode, auth};
use font8x8::{BASIC_FONTS, UnicodeFonts};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

const FRAME_INTERVAL: Duration = Duration::from_millis(33);
const BG: u32 = 0x0010_1014;
const FG: u32 = 0x00e6_e6ee;
const BORDER: u32 = 0x0024_2430;
const FOCUS_BORDER: u32 = 0x005f_9fda;
const SIDEBAR_BG: u32 = 0x0017_171e;
const SIDEBAR_ACTIVE: u32 = 0x002a_2a38;
const SIDEBAR_TEXT: u32 = 0x00b8_b8c8;
const SIDEBAR_DIM: u32 = 0x0070_7080;
const TAB_BG: u32 = 0x001a_1a22;
const TAB_ACTIVE: u32 = 0x0030_3040;
const TAB_TEXT: u32 = 0x00c8_c8d8;
const TAB_DIM: u32 = 0x0078_7888;
const PANEL_PAD: usize = 2;
const DIVIDER: usize = 2;
const SIDEBAR_WIDTH: f64 = 168.0;

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
        sized: HashMap::new(),
        pointer: (0.0, 0.0),
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
    sized: HashMap<String, (u16, u16)>,
    pointer: (f64, f64),
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
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = (position.x, position.y);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => self.click(),
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

    /// Lay out the selected workspace's split tree and resolve each leaf to its
    /// visible surface and tabs. The tree is read under the state lock and
    /// copied out.
    fn views(&self, area: Rect) -> Vec<PaneView> {
        let (cell_w, cell_h) = self.cell_size();
        let state = lock(&self.state);
        let Some(workspace_id) = state.selected_workspace.clone().or_else(|| {
            state
                .workspaces
                .first()
                .map(|workspace| workspace.id.clone())
        }) else {
            return Vec::new();
        };
        let Some(workspace) = state.workspace(&workspace_id) else {
            return Vec::new();
        };
        let active = state
            .selected_surface
            .clone()
            .filter(|id| {
                state
                    .surface(id)
                    .is_some_and(|surface| surface.workspace_id == workspace.id)
            })
            .or_else(|| workspace.active_surface.clone());
        let mut leaves = Vec::new();
        layout_panes(&workspace.root, area, &mut leaves);
        leaves
            .into_iter()
            .map(|(pane_id, rect)| {
                let mut content = rect.inset(PANEL_PAD);
                let mut tabs = Vec::new();
                let mut tab_bar = None;
                let surface = state.panes.get(&pane_id).and_then(|pane| {
                    if pane.surfaces.len() > 1 {
                        let tab_height = cell_h + 4;
                        let bar = Rect {
                            x: content.x,
                            y: content.y,
                            w: content.w,
                            h: tab_height,
                        };
                        tab_bar = Some(bar);
                        let mut x = bar.x;
                        for (index, surface_id) in pane.surfaces.iter().enumerate() {
                            let Some(surface) = state.surface(surface_id) else {
                                continue;
                            };
                            let remaining = bar.x.saturating_add(bar.w).saturating_sub(x);
                            if remaining < 12 {
                                break;
                            }
                            let title = tab_title(surface, index);
                            let width = (title.chars().count() * cell_w + 16).min(remaining);
                            tabs.push(TabView {
                                id: surface_id.clone(),
                                title,
                                rect: Rect {
                                    x,
                                    y: bar.y,
                                    w: width,
                                    h: bar.h,
                                },
                                active: index == pane.active,
                            });
                            x = x.saturating_add(width);
                        }
                        content.y = content.y.saturating_add(tab_height);
                        content.h = content.h.saturating_sub(tab_height);
                    }
                    pane.active_surface().map(str::to_owned)
                });
                let focused = surface.is_some() && surface.as_deref() == active.as_deref();
                PaneView {
                    surface,
                    rect,
                    content,
                    focused,
                    tabs,
                    tab_bar,
                }
            })
            .collect()
    }

    fn sidebar_width(&self) -> usize {
        let scale = self
            .window
            .as_ref()
            .map(|window| window.scale_factor())
            .unwrap_or(1.0)
            .max(1.0);
        (SIDEBAR_WIDTH * scale) as usize
    }

    fn content_area(&self, width: usize, height: usize) -> Rect {
        let sidebar = self.sidebar_width().min(width.saturating_sub(80));
        Rect {
            x: sidebar,
            y: 0,
            w: width.saturating_sub(sidebar),
            h: height,
        }
    }

    /// The flat workspace list. Groups are visible in the core but not yet
    /// rendered as headers in this window.
    fn workspace_views(&self, width: usize, height: usize) -> Vec<WorkspaceView> {
        let state = lock(&self.state);
        let row_height = self.cell_size().1 + 8;
        let selected = state.selected_workspace.as_deref();
        let mut y = 28usize;
        let mut rows = Vec::new();
        for workspace in &state.workspaces {
            if y.saturating_add(row_height) > height {
                break;
            }
            rows.push(WorkspaceView {
                id: workspace.id.clone(),
                name: workspace.name.clone(),
                unread: workspace.unread,
                active: selected == Some(workspace.id.as_str()),
                rect: Rect {
                    x: 0,
                    y,
                    w: width,
                    h: row_height,
                },
            });
            y = y.saturating_add(row_height);
        }
        rows
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

    /// Give every visible surface the grid that fits its pane content.
    fn sync_sizes(&mut self, views: &[PaneView]) {
        let (cell_w, cell_h) = self.cell_size();
        for view in views {
            let Some(surface_id) = &view.surface else {
                continue;
            };
            let cols = (view.content.w / cell_w).max(1).min(u16::MAX as usize) as u16;
            let rows = (view.content.h / cell_h).max(1).min(u16::MAX as usize) as u16;
            if self.sized.get(surface_id) == Some(&(cols, rows)) {
                continue;
            }
            if self.host.resize(surface_id, cols, rows).is_ok() {
                let mut state = lock(&self.state);
                if let Some(surface) = state.surface_mut(surface_id) {
                    surface.cols = cols;
                    surface.rows = rows;
                }
                self.sized.insert(surface_id.clone(), (cols, rows));
            }
        }
    }

    fn sync_title(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        let state = lock(&self.state);
        let Some(workspace_id) = state.selected_workspace.clone().or_else(|| {
            state
                .workspaces
                .first()
                .map(|workspace| workspace.id.clone())
        }) else {
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

    /// Publisher of core events for actions the GUI takes directly (currently
    /// focus). Server-dispatched requests already drain their own events.
    fn flush_events(&self) {
        let events = {
            let mut state = lock(&self.state);
            state.drain_events()
        };
        if events.is_empty() {
            return;
        }
        let Some(server) = &self.server else {
            return;
        };
        for event in events {
            if let Ok(line) = serde_json::to_string(&event) {
                server.broadcast(&line);
            }
        }
    }

    fn click(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        let PhysicalSize { width, height } = window.inner_size();
        let (width, height) = (width as usize, height as usize);
        let point = (
            self.pointer.0.max(0.0) as usize,
            self.pointer.1.max(0.0) as usize,
        );
        let area = self.content_area(width, height);
        if point.0 < area.x {
            let rows = self.workspace_views(area.x, height);
            let Some(row) = rows.iter().find(|row| row.rect.contains(point.0, point.1)) else {
                return;
            };
            let mut state = lock(&self.state);
            if let Err(error) = state.select_workspace(&row.id) {
                eprintln!("berth-gui: select workspace {}: {error}", row.id);
                return;
            }
            let active = state.active_surface_of_workspace(&row.id);
            drop(state);
            if let Some(surface_id) = active {
                let _ = self.host.focus(&surface_id);
            }
            self.flush_events();
            self.request_redraw();
            return;
        }

        let views = self.views(area);
        for view in &views {
            if let Some(tab) = view
                .tabs
                .iter()
                .find(|tab| tab.rect.contains(point.0, point.1))
            {
                self.activate(&tab.id);
                return;
            }
        }
        let Some(view) = views
            .iter()
            .find(|view| view.rect.contains(point.0, point.1))
        else {
            return;
        };
        if let Some(surface_id) = &view.surface {
            self.activate(surface_id);
        }
    }

    fn activate(&mut self, surface_id: &str) {
        let mut state = lock(&self.state);
        if let Err(error) = state.activate_surface(surface_id, self.host.as_ref()) {
            eprintln!("berth-gui: focus {surface_id}: {error}");
            return;
        }
        drop(state);
        self.flush_events();
        self.request_redraw();
    }

    fn new_tab(&mut self) {
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        let mut state = lock(&self.state);
        let Some(surface) = state.surface(&surface_id).cloned() else {
            return;
        };
        let request = SpawnRequest {
            cwd: (!surface.cwd.is_empty()).then(|| surface.cwd.clone()),
            argv: Vec::new(),
            env: Vec::new(),
            cols: surface.cols.max(1),
            rows: surface.rows.max(1),
        };
        let result = state.add_surface(&surface.pane_id, request, self.host.as_ref(), true);
        drop(state);
        match result {
            Ok(surface) => {
                self.flush_events();
                println!("berth-gui: new tab {}", surface.id);
                self.request_redraw();
            }
            Err(error) => eprintln!("berth-gui: new tab: {error}"),
        }
    }

    fn close_tab(&mut self) {
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        let mut state = lock(&self.state);
        let Some(surface) = state.surface(&surface_id).cloned() else {
            return;
        };
        let last = state
            .panes
            .get(&surface.pane_id)
            .is_none_or(|pane| pane.surfaces.len() <= 1);
        if last {
            return;
        }
        let result = state.close_surface(&surface_id, self.host.as_ref());
        drop(state);
        match result {
            Ok(()) => {
                self.flush_events();
                self.request_redraw();
            }
            Err(error) => eprintln!("berth-gui: close tab: {error}"),
        }
    }

    fn cycle_tab(&mut self, delta: i32) {
        let Some(surface_id) = self.active_surface() else {
            return;
        };
        let mut state = lock(&self.state);
        let result = state.cycle_surface(&surface_id, delta, self.host.as_ref());
        drop(state);
        match result {
            Ok(surface) => {
                self.flush_events();
                let _ = self.host.focus(&surface.id);
                self.request_redraw();
            }
            Err(error) => eprintln!("berth-gui: cycle tab: {error}"),
        }
    }

    fn key(&mut self, event: winit::event::KeyEvent) {
        if self.shortcut(&event) {
            return;
        }
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

    /// Window shortcuts. They are deliberately Ctrl+Shift so the shell keeps
    /// plain Ctrl+Tab/Ctrl+W.
    fn shortcut(&mut self, event: &KeyEvent) -> bool {
        if event.repeat
            || !self.modifiers.control_key()
            || self.modifiers.alt_key()
            || self.modifiers.super_key()
        {
            return false;
        }
        if !self.modifiers.shift_key() {
            match event.logical_key {
                Key::Named(NamedKey::PageUp) => {
                    self.cycle_tab(-1);
                    return true;
                }
                Key::Named(NamedKey::PageDown) => {
                    self.cycle_tab(1);
                    return true;
                }
                _ => return false,
            }
        }
        if let Key::Character(character) = &event.logical_key {
            if character.eq_ignore_ascii_case("t") {
                self.new_tab();
                return true;
            }
            if character.eq_ignore_ascii_case("w") {
                self.close_tab();
                return true;
            }
        }
        false
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
        let Some(window) = self.window.clone() else {
            return;
        };
        let PhysicalSize { width, height } = window.inner_size();
        let (width, height) = (width as usize, height as usize);
        if width == 0 || height == 0 {
            return;
        }
        let (cell_w, cell_h) = self.cell_size();
        let area = self.content_area(width, height);
        let workspaces = self.workspace_views(area.x, height);
        let views = self.views(area);
        self.sync_sizes(&views);
        self.sync_title();

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
        paint_sidebar(
            &mut buffer,
            width,
            height,
            area.x,
            &workspaces,
            cell_w,
            cell_h,
        );

        if views.is_empty() {
            draw_text(
                &mut buffer,
                width,
                area.x + 8,
                8,
                "berth: no live surface",
                FG,
                cell_w,
                cell_h,
                area.w.saturating_sub(16),
            );
        }

        for view in &views {
            let border = if view.focused { FOCUS_BORDER } else { BORDER };
            stroke_rect(&mut buffer, width, view.rect, border);
            if let Some(bar) = view.tab_bar {
                paint_tabs(&mut buffer, width, bar, &view.tabs, cell_w, cell_h);
            }
            let Some(surface_id) = &view.surface else {
                continue;
            };
            let Ok(screen) = self.host.screen_view(surface_id) else {
                continue;
            };
            paint_screen(
                &mut buffer,
                &screen,
                width,
                view.content.x,
                view.content.y,
                view.content.w,
                view.content.h,
                cell_w,
                cell_h,
                view.focused,
            );
        }

        if buffer.present().is_err() {
            return;
        }
        if !self.ready {
            self.ready = true;
            println!(
                "berth-gui: window ready {width}x{height} ({} pane(s))",
                views.len()
            );
        }
    }
}

fn paint_sidebar(
    buffer: &mut [u32],
    stride: usize,
    height: usize,
    width: usize,
    rows: &[WorkspaceView],
    cell_w: usize,
    cell_h: usize,
) {
    if width == 0 {
        return;
    }
    fill_rect(buffer, stride, 0, 0, width, height, SIDEBAR_BG);
    draw_text(
        buffer,
        stride,
        12,
        8,
        "WORKSPACES",
        SIDEBAR_DIM,
        cell_w,
        cell_h,
        width.saturating_sub(20),
    );
    for row in rows {
        if row.active {
            fill_rect(
                buffer,
                stride,
                row.rect.x,
                row.rect.y,
                row.rect.w,
                row.rect.h,
                SIDEBAR_ACTIVE,
            );
            fill_rect(
                buffer,
                stride,
                row.rect.x,
                row.rect.y,
                3,
                row.rect.h,
                FOCUS_BORDER,
            );
        }
        let color = if row.active { FG } else { SIDEBAR_TEXT };
        draw_text(
            buffer,
            stride,
            12,
            row.rect.y + 4,
            &row.name,
            color,
            cell_w,
            cell_h,
            row.rect.w.saturating_sub(32),
        );
        if row.unread > 0 {
            fill_rect(
                buffer,
                stride,
                row.rect.w.saturating_sub(14),
                row.rect.y + row.rect.h / 2 - 3,
                6,
                6,
                FOCUS_BORDER,
            );
        }
    }
    fill_rect(
        buffer,
        stride,
        width.saturating_sub(1),
        0,
        1,
        height,
        BORDER,
    );
}

fn paint_tabs(
    buffer: &mut [u32],
    stride: usize,
    bar: Rect,
    tabs: &[TabView],
    cell_w: usize,
    cell_h: usize,
) {
    fill_rect(buffer, stride, bar.x, bar.y, bar.w, bar.h, TAB_BG);
    for tab in tabs {
        if tab.active {
            fill_rect(
                buffer, stride, tab.rect.x, tab.rect.y, tab.rect.w, tab.rect.h, TAB_ACTIVE,
            );
            fill_rect(
                buffer,
                stride,
                tab.rect.x,
                tab.rect.y + tab.rect.h.saturating_sub(2),
                tab.rect.w,
                2,
                FOCUS_BORDER,
            );
        }
        draw_text(
            buffer,
            stride,
            tab.rect.x + 8,
            tab.rect.y + 2,
            &tab.title,
            if tab.active { TAB_TEXT } else { TAB_DIM },
            cell_w,
            cell_h,
            tab.rect.w.saturating_sub(16),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    buffer: &mut [u32],
    stride: usize,
    x: usize,
    y: usize,
    text: &str,
    color: u32,
    cell_w: usize,
    cell_h: usize,
    max_width: usize,
) {
    if cell_w == 0 {
        return;
    }
    let max_chars = max_width / cell_w;
    for (index, character) in text.chars().take(max_chars).enumerate() {
        draw_glyph(
            buffer,
            stride,
            x + index * cell_w,
            y,
            character,
            color,
            cell_w,
            cell_h,
            false,
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

impl Rect {
    fn inset(self, padding: usize) -> Self {
        let width = self.w.saturating_sub(padding.saturating_mul(2));
        let height = self.h.saturating_sub(padding.saturating_mul(2));
        Self {
            x: self.x.saturating_add(padding),
            y: self.y.saturating_add(padding),
            w: width,
            h: height,
        }
    }

    fn contains(self, x: usize, y: usize) -> bool {
        self.w > 0
            && self.h > 0
            && x >= self.x
            && y >= self.y
            && x < self.x.saturating_add(self.w)
            && y < self.y.saturating_add(self.h)
    }
}

struct PaneView {
    surface: Option<String>,
    rect: Rect,
    content: Rect,
    focused: bool,
    tabs: Vec<TabView>,
    tab_bar: Option<Rect>,
}

struct TabView {
    id: String,
    title: String,
    rect: Rect,
    active: bool,
}

struct WorkspaceView {
    id: String,
    name: String,
    unread: u32,
    rect: Rect,
    active: bool,
}

/// Short, stable tab label. PTY title reporting is not wired yet, so fall back
/// to the cwd's last component and a tab number.
fn tab_title(surface: &Surface, index: usize) -> String {
    if !surface.title.is_empty() {
        return surface.title.clone();
    }
    let base = std::path::Path::new(&surface.cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("shell");
    if index == 0 {
        base.to_string()
    } else {
        format!("{base} {}", index + 1)
    }
}

/// Walk the persisted split tree and assign pixel rectangles to leaves. Splits
/// leave a divider between children; the last child takes the remainder so
/// rounding never leaves a seam.
fn layout_panes(node: &PaneNode, rect: Rect, out: &mut Vec<(String, Rect)>) {
    match node {
        PaneNode::Leaf { pane_id } => out.push((pane_id.clone(), rect)),
        PaneNode::Split {
            orientation,
            children,
            weights,
            ..
        } => {
            let count = children.len();
            if count == 0 {
                return;
            }
            let gap = DIVIDER.saturating_mul(count.saturating_sub(1));
            let available = match orientation {
                Orientation::Horizontal => rect.w,
                Orientation::Vertical => rect.h,
            }
            .saturating_sub(gap);
            let mut resolved = Vec::with_capacity(count);
            let mut total = 0.0f32;
            for index in 0..count {
                let weight = weights
                    .get(index)
                    .copied()
                    .filter(|weight| weight.is_finite() && *weight > 0.0)
                    .unwrap_or(1.0);
                resolved.push(weight);
                total += weight;
            }
            let mut consumed = 0usize;
            let mut offset = 0usize;
            for (index, child) in children.iter().enumerate() {
                let extent = if index + 1 == count {
                    available.saturating_sub(consumed)
                } else {
                    let share = available as f32 * (resolved[index] / total);
                    (share.round() as usize).min(available.saturating_sub(consumed))
                };
                consumed = consumed.saturating_add(extent);
                let child_rect = match orientation {
                    Orientation::Horizontal => Rect {
                        x: rect.x.saturating_add(offset),
                        y: rect.y,
                        w: extent,
                        h: rect.h,
                    },
                    Orientation::Vertical => Rect {
                        x: rect.x,
                        y: rect.y.saturating_add(offset),
                        w: rect.w,
                        h: extent,
                    },
                };
                layout_panes(child, child_rect, out);
                offset = offset.saturating_add(extent).saturating_add(DIVIDER);
            }
        }
    }
}

fn stroke_rect(buffer: &mut [u32], stride: usize, rect: Rect, color: u32) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    fill_rect(buffer, stride, rect.x, rect.y, rect.w, 1, color);
    fill_rect(
        buffer,
        stride,
        rect.x,
        rect.y + rect.h - 1,
        rect.w,
        1,
        color,
    );
    fill_rect(buffer, stride, rect.x, rect.y, 1, rect.h, color);
    fill_rect(
        buffer,
        stride,
        rect.x + rect.w - 1,
        rect.y,
        1,
        rect.h,
        color,
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_screen(
    buffer: &mut [u32],
    view: &ScreenView,
    stride: usize,
    origin_x: usize,
    origin_y: usize,
    max_width: usize,
    max_height: usize,
    cell_w: usize,
    cell_h: usize,
    show_cursor: bool,
) {
    let cols = view.cols.min((max_width / cell_w) as u16);
    let rows = view.rows.min((max_height / cell_h) as u16);
    for row in 0..rows {
        for col in 0..cols {
            let index = usize::from(row) * usize::from(view.cols) + usize::from(col);
            let Some(cell) = view.cells.get(index) else {
                continue;
            };
            let cursor = show_cursor && view.cursor == (row, col);
            paint_cell(
                buffer,
                stride,
                origin_x + usize::from(col) * cell_w,
                origin_y + usize::from(row) * cell_h,
                cell,
                cursor,
                cell_w,
                cell_h,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_cell(
    buffer: &mut [u32],
    stride: usize,
    x: usize,
    y: usize,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn split(orientation: Orientation, weights: Vec<f32>, children: Vec<PaneNode>) -> PaneNode {
        PaneNode::Split {
            id: "split".into(),
            orientation,
            children,
            weights,
        }
    }

    fn leaf(id: &str) -> PaneNode {
        PaneNode::leaf(id)
    }

    fn layout(node: &PaneNode, width: usize, height: usize) -> Vec<(String, Rect)> {
        let mut out = Vec::new();
        layout_panes(
            node,
            Rect {
                x: 0,
                y: 0,
                w: width,
                h: height,
            },
            &mut out,
        );
        out
    }

    #[test]
    fn a_single_leaf_fills_the_window() {
        let leaves = layout(&leaf("one"), 100, 60);
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].0, "one");
        assert_eq!(
            leaves[0].1,
            Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 60
            }
        );
    }

    #[test]
    fn horizontal_children_share_width_with_a_divider() {
        let node = split(
            Orientation::Horizontal,
            vec![0.7, 0.3],
            vec![leaf("left"), leaf("right")],
        );
        let leaves = layout(&node, 100, 50);
        assert_eq!(
            leaves[0].1,
            Rect {
                x: 0,
                y: 0,
                w: 69,
                h: 50
            }
        );
        assert_eq!(
            leaves[1].1,
            Rect {
                x: 71,
                y: 0,
                w: 29,
                h: 50
            }
        );
        assert_eq!(leaves[0].1.w + leaves[1].1.w, 98);
    }

    #[test]
    fn vertical_children_share_height_with_a_divider() {
        let node = split(
            Orientation::Vertical,
            vec![1.0, 1.0],
            vec![leaf("top"), leaf("bottom")],
        );
        let leaves = layout(&node, 80, 60);
        assert_eq!(
            leaves[0].1,
            Rect {
                x: 0,
                y: 0,
                w: 80,
                h: 29
            }
        );
        assert_eq!(
            leaves[1].1,
            Rect {
                x: 0,
                y: 31,
                w: 80,
                h: 29
            }
        );
    }

    #[test]
    fn nested_splits_keep_their_rectangles_inside_the_parent() {
        let node = split(
            Orientation::Horizontal,
            vec![0.5, 0.5],
            vec![
                leaf("left"),
                split(
                    Orientation::Vertical,
                    vec![0.5, 0.5],
                    vec![leaf("top-right"), leaf("bottom-right")],
                ),
            ],
        );
        let leaves = layout(&node, 100, 60);
        let left = &leaves[0].1;
        let top = &leaves[1].1;
        let bottom = &leaves[2].1;
        assert_eq!(left.x, 0);
        assert_eq!(left.w, 49);
        assert_eq!(top.x, 51);
        assert_eq!(top.w, 49);
        assert_eq!(top.y, 0);
        assert_eq!(bottom.y, 31);
        assert!(top.contains(60, 10));
        assert!(!top.contains(60, 40));
    }

    #[test]
    fn inset_keeps_the_panel_content_inside_its_border() {
        let rect = Rect {
            x: 10,
            y: 20,
            w: 100,
            h: 60,
        };
        assert_eq!(
            rect.inset(PANEL_PAD),
            Rect {
                x: 12,
                y: 22,
                w: 96,
                h: 56
            }
        );
    }

    #[test]
    fn tab_labels_prefer_the_title_then_the_cwd() {
        let mut surface = Surface {
            id: "srf_test".into(),
            pane_id: "pne_test".into(),
            workspace_id: "wsp_test".into(),
            kind: berth_core::model::SurfaceKind::Terminal,
            title: String::new(),
            cwd: "/home/me/project".into(),
            pid: None,
            tty: None,
            cols: 80,
            rows: 24,
            progress: None,
            bell_count: 0,
            agent_id: None,
            created_ms: 0,
        };
        assert_eq!(tab_title(&surface, 0), "project");
        assert_eq!(tab_title(&surface, 1), "project 2");
        surface.title = "tests".into();
        assert_eq!(tab_title(&surface, 0), "tests");
        surface.title.clear();
        surface.cwd.clear();
        assert_eq!(tab_title(&surface, 0), "shell");
    }
}
