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

/// A live PTY-backed terminal and its parsed screen.
struct Surface {
    /// Kept for resize and foreground-process queries.
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    screen: Arc<Mutex<vt100::Parser>>,
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

        let screen = Arc::new(Mutex::new(vt100::Parser::new(
            std::num::NonZeroU16::new(rows).expect("rows > 0"),
            std::num::NonZeroU16::new(cols).expect("cols > 0"),
            self.scrollback,
        )));
        let alive = Arc::new(AtomicBool::new(true));

        spawn_reader(reader, Arc::clone(&screen), Arc::clone(&alive));
        spawn_waiter(child, Arc::clone(&alive));

        let surface = Arc::new(Surface {
            master: Mutex::new(master),
            writer: Mutex::new(writer),
            screen,
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
    screen: Arc<Mutex<vt100::Parser>>,
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

#[cfg(not(target_os = "linux"))]
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

#[cfg(not(target_os = "linux"))]
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
