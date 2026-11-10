//! UniFFI bridge: the only place Rust and Swift meet.
//!
//! Design: the core stays the single source of truth for the wire protocol, so
//! the bridge is deliberately JSON-shaped — `dispatch()` takes a protocol
//! request and returns a protocol response. Swift never learns the domain
//! types, which means no second schema to keep in sync and no bindings churn
//! when the protocol grows.
//!
//! The one typed surface is [`HostOps`], the callback interface Swift
//! implements to reach Ghostty/AppKit. That direction *must* be typed: it is
//! the portability seam (docs/02-architecture.md, ADR A3).

use std::sync::{Arc, Mutex, MutexGuard};

use berth_core::protocol::{ErrorCode, Request, Response};
use berth_core::{
    AppState, CoreError, Notification, PidInfo, ReadScope, Readback, SpawnRequest, Spawned,
    SurfaceOps, dispatch as core_dispatch, store,
};

uniffi::setup_scaffolding!();

/// Error type crossing the FFI boundary. Flat so Swift gets a thrown error
/// with a readable message rather than a tagged enum it must switch on.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
#[uniffi(flat_error)]
pub enum FfiError {
    Internal { message: String },
}

impl std::fmt::Display for FfiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FfiError::Internal { message } => write!(f, "{message}"),
        }
    }
}

impl From<CoreError> for FfiError {
    fn from(e: CoreError) -> Self {
        FfiError::Internal {
            message: e.to_string(),
        }
    }
}

/// What the shell must be able to do on the core's behalf.
#[derive(Debug, Clone, uniffi::Record)]
pub struct FfiSpawnRequest {
    pub cwd: Option<String>,
    /// Ready to hand to Ghostty: a POSIX-shell-quoted command line, or `None`
    /// for the user's default shell.
    pub command: Option<String>,
    /// `KEY=VALUE` pairs, already merged by the core.
    pub env: Vec<String>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct FfiSpawned {
    pub pid: u32,
    /// Empty when the shell cannot determine it yet.
    pub tty: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct FfiPidInfo {
    pub pid: Option<u32>,
    pub comm: Option<String>,
    pub argv: Vec<String>,
}

/// Implemented in Swift by `GhosttyHost`. Called on the thread that dispatched
/// the request (usually a socket thread), so the Swift side hops to the main
/// thread itself.
#[uniffi::export(with_foreign)]
pub trait HostOps: Send + Sync {
    fn spawn(&self, req: FfiSpawnRequest) -> Result<FfiSpawned, FfiError>;
    fn send_text(&self, surface_id: String, text: String) -> Result<(), FfiError>;
    fn send_key(&self, surface_id: String, key: String, mods: Vec<String>) -> Result<(), FfiError>;
    /// `scrollback = true` asks for history as well as the visible screen.
    fn read_text(
        &self,
        surface_id: String,
        scrollback: bool,
        max_bytes: u32,
    ) -> Result<String, FfiError>;
    fn close(&self, surface_id: String) -> Result<(), FfiError>;
    fn focus(&self, surface_id: String) -> Result<(), FfiError>;
    fn resize(&self, surface_id: String, cols: u16, rows: u16) -> Result<(), FfiError>;
    fn pid_info(&self, surface_id: String) -> Result<Option<FfiPidInfo>, FfiError>;
    /// Best-effort OS notification (UNUserNotificationCenter / NSUserNotification).
    fn notify(&self, title: String, body: String, workspace_id: Option<String>);
}

struct HostAdapter(Arc<dyn HostOps>);

impl SurfaceOps for HostAdapter {
    fn spawn(&self, req: SpawnRequest) -> Result<Spawned, CoreError> {
        let ffi = self
            .0
            .spawn(FfiSpawnRequest {
                cwd: req.cwd,
                command: (!req.argv.is_empty()).then(|| berth_core::shell::join_argv(&req.argv)),
                env: req
                    .env
                    .into_iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect(),
                cols: req.cols,
                rows: req.rows,
            })
            .map_err(to_core)?;
        Ok(Spawned {
            pid: ffi.pid,
            tty: ffi.tty,
            cols: ffi.cols,
            rows: ffi.rows,
        })
    }

    fn send_text(&self, surface_id: &str, text: &str) -> Result<(), CoreError> {
        self.0
            .send_text(surface_id.to_string(), text.to_string())
            .map_err(to_core)
    }

    fn send_key(&self, surface_id: &str, key: &str, mods: &[String]) -> Result<(), CoreError> {
        self.0
            .send_key(surface_id.to_string(), key.to_string(), mods.to_vec())
            .map_err(to_core)
    }

    fn read_text(
        &self,
        surface_id: &str,
        scope: ReadScope,
        max_bytes: usize,
    ) -> Result<Readback, CoreError> {
        let text = self
            .0
            .read_text(
                surface_id.to_string(),
                matches!(scope, ReadScope::Scrollback),
                max_bytes.min(u32::MAX as usize) as u32,
            )
            .map_err(to_core)?;
        let truncated = text.len() >= max_bytes;
        Ok(Readback { text, truncated })
    }

    fn close(&self, surface_id: &str) -> Result<(), CoreError> {
        self.0.close(surface_id.to_string()).map_err(to_core)
    }

    fn focus(&self, surface_id: &str) -> Result<(), CoreError> {
        self.0.focus(surface_id.to_string()).map_err(to_core)
    }

    fn resize(&self, surface_id: &str, cols: u16, rows: u16) -> Result<(), CoreError> {
        self.0
            .resize(surface_id.to_string(), cols, rows)
            .map_err(to_core)
    }

    fn pid_info(&self, surface_id: &str) -> Result<PidInfo, CoreError> {
        let info = self.0.pid_info(surface_id.to_string()).map_err(to_core)?;
        Ok(info.map_or_else(PidInfo::default, |p| PidInfo {
            pid: p.pid,
            comm: p.comm,
            argv: p.argv,
        }))
    }

    fn notify_os(&self, notification: &Notification) -> Result<(), CoreError> {
        self.0.notify(
            notification.title.clone(),
            notification.body.clone(),
            notification.workspace_id.clone(),
        );
        Ok(())
    }
}

fn to_core(e: FfiError) -> CoreError {
    CoreError::internal(e.to_string())
}

/// The core, owned by the shell. One instance per app process.
#[derive(uniffi::Object)]
pub struct Engine {
    state: Mutex<AppState>,
    host: Arc<dyn HostOps>,
}

fn lock(state: &Mutex<AppState>) -> MutexGuard<'_, AppState> {
    // A panic inside the core must not brick the shell permanently; the state
    // is plain data and remains consistent enough to keep serving.
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[uniffi::export]
impl Engine {
    #[uniffi::constructor]
    pub fn new(host: Arc<dyn HostOps>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(AppState::default()),
            host,
        })
    }

    /// Handle one protocol request. Never panics; a bad payload becomes a
    /// `bad_request` response so the client always gets an answer.
    pub fn dispatch(&self, request: String) -> String {
        let req: Request = match serde_json::from_str(&request) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response::err(
                    serde_json::Value::Null,
                    CoreError::new(ErrorCode::BadRequest, format!("invalid request: {e}")),
                );
                return serde_json::to_string(&resp).unwrap_or_else(|_| {
                    r#"{"id":null,"ok":false,"error":{"code":"internal","message":"encode"}}"#
                        .to_string()
                });
            }
        };
        let ops = HostAdapter(Arc::clone(&self.host));
        let mut state = lock(&self.state);
        let resp = core_dispatch(&mut state, &ops, &req);
        serde_json::to_string(&resp).unwrap_or_else(|_| {
            r#"{"id":null,"ok":false,"error":{"code":"internal","message":"encode"}}"#.to_string()
        })
    }

    /// Newline-delimited events queued since the last drain. Empty when quiet.
    pub fn drain_events(&self) -> String {
        let mut state = lock(&self.state);
        let events = state.drain_events();
        events
            .iter()
            .filter_map(|e| serde_json::to_string(e).ok())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Session snapshot at this instant.
    pub fn snapshot(&self) -> String {
        let state = lock(&self.state);
        serde_json::to_string(&state.snapshot()).unwrap_or_else(|_| "{}".to_string())
    }

    /// Respawn terminals from a snapshot. Returns how many were restored.
    pub fn restore(&self, snapshot: String) -> Result<u32, FfiError> {
        let snap: store::Snapshot =
            serde_json::from_str(&snapshot).map_err(|e| FfiError::Internal {
                message: format!("invalid snapshot: {e}"),
            })?;
        let ops = HostAdapter(Arc::clone(&self.host));
        let mut state = lock(&self.state);
        store::restore(&mut state, snap, &ops)
            .map(|n| n as u32)
            .map_err(FfiError::from)
    }

    /// Mark agents whose last update is older than `ttl_ms` as blocked.
    /// Returns the ids that changed.
    pub fn sweep_stale(&self, ttl_ms: u64) -> Vec<String> {
        let now = berth_core::time::now_ms();
        let mut state = lock(&self.state);
        state.sweep_stale(now, ttl_ms)
    }

    pub fn protocol_version(&self) -> u32 {
        berth_core::protocol::PROTOCOL_VERSION
    }

    /// Opaque instance id, minted on first use.
    pub fn instance_id(&self) -> String {
        let mut state = lock(&self.state);
        state.instance().to_string()
    }
}
