//! Berth core: portable domain logic (no I/O, no UI, no PTY).
//!
//! Layers:
//! - [`ids`]        opaque ids minted per entity kind
//! - [`model`]      workspaces, panes, split tree, surfaces, notifications
//! - [`agent`]      agent lifecycle state machine + attention derivation
//! - [`providers`]  data table of known agent CLIs and resume templates
//! - [`protocol`]   wire types: request / response / event envelopes
//! - [`router`]     `dispatch()` — the pure request handler
//! - [`state`]      `AppState` aggregate + staleness sweep + event queue
//! - [`store`]      session snapshot schema and migration hook
//! - [`time`]       dependency-free epoch/UTC formatting
//!
//! Everything the outside world needs is behind [`SurfaceOps`]; a platform
//! shell (AppKit/GhosttyKit today, GTK/libghostty-vt later) implements it.

pub mod agent;
pub mod ids;
pub mod model;
pub mod protocol;
pub mod providers;
pub mod router;
pub mod schema;
pub mod shell;
pub mod state;
pub mod store;
pub mod time;

pub use agent::{AgentState, Attention};
pub use model::{AgentSurface, Notification, Pane, PaneNode, Surface, Workspace};
pub use protocol::{ErrorBody, ErrorCode, Event, Request, Response};
pub use router::dispatch;
pub use state::AppState;

/// Capabilities the shell provides to the core. The *only* impure surface.
///
/// Implemented by the macOS shell (Swift ↔ UniFFI), and later by a Linux shell.
/// All ids are opaque strings minted by [`ids`].
pub trait SurfaceOps {
    /// Create a terminal surface and its child process.
    fn spawn(&self, req: SpawnRequest) -> Result<Spawned, CoreError>;
    /// Write text to the surface as if typed.
    fn send_text(&self, surface_id: &str, text: &str) -> Result<(), CoreError>;
    /// Send a named key with modifiers.
    fn send_key(&self, surface_id: &str, key: &str, mods: &[String]) -> Result<(), CoreError>;
    /// Read rendered screen text (`ghostty_surface_read_text`).
    fn read_text(
        &self,
        surface_id: &str,
        scope: ReadScope,
        max_bytes: usize,
    ) -> Result<Readback, CoreError>;
    /// Close the surface and terminate its child.
    fn close(&self, surface_id: &str) -> Result<(), CoreError>;
    /// Focus the surface (window + pane + tab).
    fn focus(&self, surface_id: &str) -> Result<(), CoreError>;
    /// Resize the underlying terminal grid.
    fn resize(&self, surface_id: &str, cols: u16, rows: u16) -> Result<(), CoreError>;
    /// Process facts for passive agent detection.
    fn pid_info(&self, surface_id: &str) -> Result<PidInfo, CoreError>;
    /// Deliver a user-visible notification through the OS.
    fn notify_os(&self, notification: &Notification) -> Result<(), CoreError>;
}

#[derive(Debug, Clone, Default)]
pub struct SpawnRequest {
    pub cwd: Option<String>,
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Default)]
pub struct Spawned {
    pub pid: u32,
    pub tty: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadScope {
    Screen,
    Scrollback,
}

#[derive(Debug, Clone, Default)]
pub struct Readback {
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PidInfo {
    /// Foreground process id, if the shell can determine it.
    pub pid: Option<u32>,
    /// Process name (`argv[0]` basename), if known.
    pub comm: Option<String>,
    /// Full argv of the foreground process, best effort.
    pub argv: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreError {
    pub code: ErrorCode,
    pub message: String,
}

impl CoreError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for CoreError {}

/// A shell that does nothing — used by tests and by `--headless` core runs.
#[derive(Debug, Default)]
pub struct NullOps;

impl SurfaceOps for NullOps {
    fn spawn(&self, req: SpawnRequest) -> Result<Spawned, CoreError> {
        Ok(Spawned {
            pid: 0,
            tty: String::new(),
            cols: req.cols.max(80),
            rows: req.rows.max(24),
        })
    }
    fn send_text(&self, _: &str, _: &str) -> Result<(), CoreError> {
        Ok(())
    }
    fn send_key(&self, _: &str, _: &str, _: &[String]) -> Result<(), CoreError> {
        Ok(())
    }
    fn read_text(&self, _: &str, _: ReadScope, _: usize) -> Result<Readback, CoreError> {
        Ok(Readback::default())
    }
    fn close(&self, _: &str) -> Result<(), CoreError> {
        Ok(())
    }
    fn focus(&self, _: &str) -> Result<(), CoreError> {
        Ok(())
    }
    fn resize(&self, _: &str, _: u16, _: u16) -> Result<(), CoreError> {
        Ok(())
    }
    fn pid_info(&self, _: &str) -> Result<PidInfo, CoreError> {
        Ok(PidInfo::default())
    }
    fn notify_os(&self, _: &Notification) -> Result<(), CoreError> {
        Ok(())
    }
}
