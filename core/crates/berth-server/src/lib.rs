//! Portable NDJSON socket server for Berth.
//!
//! The server is transport only: framing, authentication, subscriptions,
//! replay and backpressure live here (docs/03-protocol.md). Domain decisions
//! belong to `berth-core`, and terminals belong to the shell via [`SurfaceOps`].
//!
//! ```no_run
//! use std::sync::{Arc, Mutex};
//! use berth_core::{AppState, NullOps};
//! use berth_server::{Server, ServerConfig, SocketMode};
//! use berth_core::paths;
//!
//! let mut server = Server::new(
//!     ServerConfig::new(paths::socket_path(), SocketMode::Local),
//!     Arc::new(Mutex::new(AppState::default())),
//!     Arc::new(NullOps),
//! ).start().unwrap();
//! // ... shell runs ...
//! server.stop();
//! ```
//!
//! Any shell can host it: the macOS app currently uses its Swift twin, while
//! Linux and Windows shells use this crate so they share one contract.

pub mod auth;
pub mod ring;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use berth_core::protocol::{ErrorCode, Request, Response};
use berth_core::{AppState, CoreError, SurfaceOps, dispatch};
use berth_transport::{Framer, LocalStream};
use interprocess::TryClone;
use interprocess::local_socket::traits::{Listener as _, Stream as _};
use serde_json::{Value, json};

pub use auth::SocketMode;
pub use ring::{EventReplay, EventRing, WireEvent};

/// How long a connection may be unable to read before the server re-checks its
/// shutdown flag. Bounds graceful-stop latency, not request latency.
const READ_POLL: Duration = Duration::from_millis(250);

/// Server settings.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Unix socket path, or Windows named-pipe path (`\\.\pipe\berth`).
    pub path: PathBuf,
    /// `local` requires a token for mutations; `trusted` relies on socket
    /// permissions; `off` starts nothing.
    pub mode: SocketMode,
    /// Token for `local` mode. `None` loads or creates `paths::token_path()`.
    pub token: Option<String>,
    /// Replay history length (events).
    pub replay_capacity: usize,
    /// A subscriber that cannot drain this many bytes is disconnected.
    pub max_pending_bytes: usize,
    /// Longest accepted request line.
    pub max_line_bytes: usize,
}

impl ServerConfig {
    pub fn new(path: impl Into<PathBuf>, mode: SocketMode) -> Self {
        Self {
            path: path.into(),
            mode,
            token: None,
            replay_capacity: 1024,
            max_pending_bytes: 4 << 20,
            max_line_bytes: berth_transport::MAX_LINE_BYTES,
        }
    }

    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }
}

/// Shell policy invoked by `session.save` before the core dispatches it.
pub type SessionHook = Arc<dyn Fn() + Send + Sync>;

/// Builder + owner of the accept thread.
pub struct Server {
    config: ServerConfig,
    state: Arc<Mutex<AppState>>,
    ops: Arc<dyn SurfaceOps>,
    on_session_save: Option<SessionHook>,
}

impl Server {
    pub fn new(
        config: ServerConfig,
        state: Arc<Mutex<AppState>>,
        ops: Arc<dyn SurfaceOps>,
    ) -> Self {
        Self {
            config,
            state,
            ops,
            on_session_save: None,
        }
    }

    /// Called for every `session.save` request. The shell owns paths and the
    /// journal, so persistence is a shell callback, not a core concern.
    pub fn on_session_save(mut self, hook: SessionHook) -> Self {
        self.on_session_save = Some(hook);
        self
    }

    /// Bind and start accepting. Returns immediately; connections run on
    /// their own threads.
    pub fn start(self) -> std::io::Result<ServerHandle> {
        let token = match self.config.mode {
            SocketMode::Local => match self.config.token {
                Some(token) => Some(token),
                None => Some(auth::load_or_create_token(&berth_core::paths::token_path())?),
            },
            SocketMode::Trusted | SocketMode::Off => self.config.token,
        };

        let shared = Arc::new(Shared {
            state: self.state,
            ops: self.ops,
            mode: self.config.mode,
            token,
            on_session_save: self.on_session_save,
            fanout: Mutex::new(Fanout {
                ring: EventRing::new(self.config.replay_capacity),
                clients: std::collections::HashMap::new(),
            }),
            request: Mutex::new(()),
            next_client: AtomicU64::new(0),
            running: AtomicBool::new(true),
            max_pending_bytes: self.config.max_pending_bytes,
            max_line_bytes: self.config.max_line_bytes,
            path: self.config.path.clone(),
        });

        let mut handle = ServerHandle {
            shared: Arc::clone(&shared),
            thread: None,
        };
        if self.config.mode == SocketMode::Off {
            return Ok(handle);
        }

        let listener = berth_transport::bind(&self.config.path)?;
        let thread = thread::Builder::new()
            .name("berth-accept".into())
            .spawn(move || accept_loop(listener, shared))?;
        handle.thread = Some(thread);
        Ok(handle)
    }
}

/// Running server. Drop stops it; call [`stop`](Self::stop) to join cleanly.
pub struct ServerHandle {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl ServerHandle {
    /// Endpoint the server is listening on.
    pub fn path(&self) -> &Path {
        &self.shared.path
    }

    /// Token required by mutating requests in `local` mode.
    pub fn token(&self) -> Option<&str> {
        self.shared.token.as_deref()
    }

    /// Drain core events queued by the shell (detection sweeps, timers) and
    /// fan them out. Requests already do this; shells call it on their own
    /// tick when the core changes state outside a request.
    pub fn flush_events(&self) {
        self.shared.flush_events();
    }

    /// Broadcast an already-serialized NDJSON event batch.
    pub fn broadcast(&self, ndjson: &str) {
        self.shared.broadcast(ndjson);
    }

    /// Stop accepting, wake blocked reads, and join the accept thread.
    pub fn stop(&mut self) {
        self.shared.running.store(false, Ordering::SeqCst);
        // A connection wakes the blocking accept on every platform.
        let _ = berth_transport::connect(&self.shared.path);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.shared.path);
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

impl std::fmt::Debug for ServerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerHandle")
            .field("path", &self.shared.path)
            .finish()
    }
}

struct Fanout {
    ring: EventRing,
    clients: std::collections::HashMap<u64, ClientTx>,
}

struct ClientTx {
    tx: Sender<Command>,
    pending: Arc<AtomicUsize>,
    limit: usize,
}

impl ClientTx {
    fn send(&self, command: Command, bytes: usize) -> bool {
        let pending = self.pending.fetch_add(bytes, Ordering::SeqCst) + bytes;
        if pending > self.limit {
            return false;
        }
        self.tx.send(command).is_ok()
    }
}

enum Command {
    Line(String),
    Subscribe {
        types: Option<Vec<String>>,
        last_seq: u64,
    },
    Unsubscribe,
    Events(Vec<WireEvent>),
    Close,
}

struct Shared {
    state: Arc<Mutex<AppState>>,
    ops: Arc<dyn SurfaceOps>,
    mode: SocketMode,
    token: Option<String>,
    on_session_save: Option<SessionHook>,
    fanout: Mutex<Fanout>,
    /// Serializes request dispatch so event sequence order matches response
    /// order across connections (normally the shell's main thread).
    request: Mutex<()>,
    next_client: AtomicU64,
    running: AtomicBool,
    max_pending_bytes: usize,
    max_line_bytes: usize,
    path: PathBuf,
}

impl Shared {
    fn lock_state(&self) -> MutexGuard<'_, AppState> {
        // A panic inside the core must not brick the server; the state is plain
        // data and remains usable.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_fanout(&self) -> MutexGuard<'_, Fanout> {
        self.fanout
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn request_guard(&self) -> MutexGuard<'_, ()> {
        self.request
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The greeting: protocol/app/instance facts, no event sequence.
    fn hello(&self) -> Option<String> {
        let _guard = self.request_guard();
        let response = {
            let mut state = self.lock_state();
            dispatch(
                &mut state,
                self.ops.as_ref(),
                &Request {
                    id: json!("hello"),
                    method: "system.hello".into(),
                    params: json!({}),
                },
            )
        };
        let mut result = serde_json::to_value(response).ok()?.get("result")?.clone();
        result["auth"] = json!(self.mode.as_str());
        serde_json::to_string(&json!({"event": "hello", "data": result})).ok()
    }

    /// Handle one request line. Returns the response line; events are fanned
    /// out before the response, matching the Swift server.
    fn handle(&self, line: &str) -> Option<String> {
        let parsed: Result<Value, _> = serde_json::from_str(line);
        let (method, id, presented) = match &parsed {
            Ok(value) => (
                value.get("method").and_then(Value::as_str),
                value.get("id").cloned().unwrap_or(Value::Null),
                presented_token(value),
            ),
            Err(_) => (None, Value::Null, None),
        };

        if let Some(method) = method {
            if !auth::authorized(
                method,
                presented.as_deref(),
                self.token.as_deref(),
                self.mode,
            ) {
                return Some(error_line(
                    id,
                    ErrorCode::PermissionDenied,
                    &format!("token required for {method}"),
                ));
            }
            if method == "session.save"
                && let Some(hook) = &self.on_session_save
            {
                hook();
            }
        }

        let _guard = self.request_guard();
        let response = self.dispatch_line(line);
        let response = if method == Some("system.hello") {
            inject_auth_mode(&response, self.mode.as_str())
        } else {
            response
        };
        self.flush_events_locked();
        Some(response)
    }

    fn dispatch_line(&self, line: &str) -> String {
        let response = match serde_json::from_str::<Request>(line) {
            Ok(request) => {
                let mut state = self.lock_state();
                dispatch(&mut state, self.ops.as_ref(), &request)
            }
            Err(error) => Response::err(
                Value::Null,
                CoreError::new(ErrorCode::BadRequest, format!("invalid request: {error}")),
            ),
        };
        serde_json::to_string(&response).unwrap_or_else(|_| {
            r#"{"id":null,"ok":false,"error":{"code":"internal","message":"encode"}}"#.into()
        })
    }

    fn flush_events(&self) {
        let _guard = self.request_guard();
        self.flush_events_locked();
    }

    fn flush_events_locked(&self) {
        let ndjson = {
            let mut state = self.lock_state();
            state
                .drain_events()
                .iter()
                .filter_map(|event| serde_json::to_string(event).ok())
                .collect::<Vec<_>>()
                .join("\n")
        };
        if !ndjson.is_empty() {
            self.broadcast(&ndjson);
        }
    }

    /// Record an event batch for replay, then fan it out. Filtering happens in
    /// each client so a reconnect can replay events it never asked to see live.
    fn broadcast(&self, ndjson: &str) {
        let mut fanout = self.lock_fanout();
        let events = fanout.ring.record(ndjson);
        if events.is_empty() {
            return;
        }
        let bytes: usize = events.iter().map(|event| event.line.len() + 1).sum();
        let mut dead = Vec::new();
        for (id, client) in &fanout.clients {
            if !client.send(Command::Events(events.clone()), bytes) {
                dead.push(*id);
            }
        }
        for id in dead {
            if let Some(client) = fanout.clients.remove(&id) {
                let _ = client.tx.send(Command::Close);
            }
        }
    }

    fn register(&self, id: u64, client: ClientTx) {
        self.lock_fanout().clients.insert(id, client);
    }

    fn unregister(&self, id: u64) {
        self.lock_fanout().clients.remove(&id);
    }

    fn send_to(&self, id: u64, line: String) -> bool {
        let mut fanout = self.lock_fanout();
        let bytes = line.len() + 1;
        let Some(client) = fanout.clients.get(&id) else {
            return false;
        };
        if client.send(Command::Line(line), bytes) {
            true
        } else {
            if let Some(client) = fanout.clients.remove(&id) {
                let _ = client.tx.send(Command::Close);
            }
            false
        }
    }

    /// `events.subscribe`: queue the subscription plus any replay under the
    /// fanout lock so no live event can slip in between.
    fn subscribe(&self, id: u64, types: Option<Vec<String>>, since: Option<u64>) -> bool {
        let fanout = self.lock_fanout();
        let (replay, head) = match since {
            Some(since) => (Some(fanout.ring.replay(since, types.as_deref())), 0),
            None => (None, fanout.ring.head()),
        };
        let last_seq = replay.as_ref().map_or(head, |replay| replay.up_to);
        let Some(client) = fanout.clients.get(&id) else {
            return false;
        };
        let mut ok = client.send(Command::Subscribe { types, last_seq }, 0);
        if let Some(replay) = replay {
            if replay.dropped > 0 {
                let gap = gap_line(replay.dropped);
                let bytes = gap.len() + 1;
                ok &= client.send(Command::Line(gap), bytes);
            }
            for line in replay.lines {
                let bytes = line.len() + 1;
                ok &= client.send(Command::Line(line), bytes);
            }
        }
        ok
    }

    fn unsubscribe(&self, id: u64) -> bool {
        let fanout = self.lock_fanout();
        fanout
            .clients
            .get(&id)
            .is_some_and(|client| client.send(Command::Unsubscribe, 0))
    }
}

fn accept_loop(listener: berth_transport::Listener, shared: Arc<Shared>) {
    loop {
        if !shared.running.load(Ordering::SeqCst) {
            break;
        }
        let stream = match listener.accept() {
            Ok(stream) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        if !shared.running.load(Ordering::SeqCst) {
            break;
        }
        let shared = Arc::clone(&shared);
        let _ = thread::Builder::new()
            .name("berth-client".into())
            .spawn(move || client_loop(shared, stream));
    }
}

fn client_loop(shared: Arc<Shared>, stream: LocalStream) {
    let id = shared.next_client.fetch_add(1, Ordering::SeqCst) + 1;
    let Ok(writer_stream) = stream.try_clone() else {
        return;
    };
    let pending = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();

    let writer_pending = Arc::clone(&pending);
    let writer_closed = Arc::clone(&closed);
    let writer = thread::Builder::new()
        .name("berth-writer".into())
        .spawn(move || writer_loop(writer_stream, rx, writer_pending, writer_closed))
        .ok();

    shared.register(
        id,
        ClientTx {
            tx: tx.clone(),
            pending: Arc::clone(&pending),
            limit: shared.max_pending_bytes,
        },
    );
    if let Some(hello) = shared.hello()
        && !shared.send_to(id, hello)
    {
        shared.unregister(id);
        return;
    }

    let _ = stream.set_recv_timeout(Some(READ_POLL));
    let mut framer = Framer::new(shared.max_line_bytes);
    let mut buffer = [0u8; 16 * 1024];
    let mut alive = true;

    while alive && shared.running.load(Ordering::SeqCst) && !closed.load(Ordering::SeqCst) {
        match (&stream).read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                let framed = framer.append(&buffer[..read]);
                if framed.oversized {
                    shared.send_to(
                        id,
                        error_line(
                            Value::Null,
                            ErrorCode::PayloadTooLarge,
                            "request exceeds the maximum line size",
                        ),
                    );
                    break;
                }
                for line in framed.lines {
                    if !handle_client_line(&shared, id, &line) {
                        alive = false;
                        break;
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => break,
        }
    }

    shared.unregister(id);
    drop(tx);
    closed.store(true, Ordering::SeqCst);
    if let Some(writer) = writer {
        let _ = writer.join();
    }
}

/// Transport-level methods are handled here as well as in the core: the
/// subscription must be armed before live events can be queued.
fn handle_client_line(shared: &Shared, id: u64, line: &str) -> bool {
    let parsed = serde_json::from_str::<Value>(line).ok();
    let method = parsed
        .as_ref()
        .and_then(|value| value.get("method"))
        .and_then(Value::as_str);

    match method {
        Some("events.subscribe") => {
            let params = parsed
                .as_ref()
                .and_then(|value| value.get("params"))
                .cloned()
                .unwrap_or(Value::Null);
            let types = params.get("types").and_then(Value::as_array).map(|types| {
                types
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            });
            let since = params.get("since").and_then(Value::as_u64);
            if !shared.subscribe(id, types, since) {
                return false;
            }
        }
        Some("events.unsubscribe") => {
            let armed = shared.unsubscribe(id);
            if !armed {
                return false;
            }
        }
        _ => {}
    }

    match shared.handle(line) {
        Some(response) => shared.send_to(id, response),
        None => true,
    }
}

fn writer_loop(
    stream: LocalStream,
    rx: mpsc::Receiver<Command>,
    pending: Arc<AtomicUsize>,
    closed: Arc<AtomicBool>,
) {
    let mut subscribed = false;
    let mut filters: Option<Vec<String>> = None;
    let mut last_seq = 0u64;

    'commands: while let Ok(command) = rx.recv() {
        let bytes = match command {
            Command::Line(line) => {
                if write_line(&stream, &line).is_err() {
                    break 'commands;
                }
                line.len() + 1
            }
            Command::Subscribe {
                types,
                last_seq: resume,
            } => {
                subscribed = true;
                filters = types;
                last_seq = last_seq.max(resume);
                0
            }
            Command::Unsubscribe => {
                subscribed = false;
                filters = None;
                0
            }
            Command::Events(events) => {
                let mut bytes = 0usize;
                for event in events {
                    bytes += event.line.len() + 1;
                    if !subscribed || event.seq <= last_seq {
                        continue;
                    }
                    last_seq = event.seq;
                    if filters
                        .as_ref()
                        .is_none_or(|filters| filters.iter().any(|t| t == &event.event_type))
                        && write_line(&stream, &event.line).is_err()
                    {
                        break 'commands;
                    }
                }
                bytes
            }
            Command::Close => break,
        };
        pending.fetch_sub(bytes, Ordering::SeqCst);
    }
    closed.store(true, Ordering::SeqCst);
}

fn write_line(stream: &LocalStream, line: &str) -> std::io::Result<()> {
    (&*stream).write_all(line.as_bytes())?;
    (&*stream).write_all(b"\n")
}

fn presented_token(value: &Value) -> Option<String> {
    value
        .get("token")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("params")
                .and_then(|params| params.get("token"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
}

fn error_line(id: Value, code: ErrorCode, message: &str) -> String {
    serde_json::to_string(&json!({
        "id": id,
        "ok": false,
        "error": {"code": code.as_str(), "message": message},
    }))
    .unwrap_or_else(|_| {
        r#"{"id":null,"ok":false,"error":{"code":"internal","message":"encode"}}"#.into()
    })
}

fn gap_line(dropped: u64) -> String {
    format!(r#"{{"event":"gap","data":{{"dropped":{dropped}}}}}"#)
}

/// Replace the core's placeholder `auth` field with the transport's real mode.
fn inject_auth_mode(response: &str, mode: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(response) else {
        return response.to_string();
    };
    if let Some(result) = value.get_mut("result") {
        result["auth"] = json!(mode);
    }
    serde_json::to_string(&value).unwrap_or_else(|_| response.to_string())
}
