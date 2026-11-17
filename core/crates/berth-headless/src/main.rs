//! `berth-headless`: run Berth's core + socket server without a GUI.
//!
//! Useful for the Linux/Windows shell spike, CI, and remote boxes:
//!
//! ```text
//! berth-headless --mode local
//! berth new --name demo
//! berth send <surface> "echo hi"   # then: berth key <surface> enter
//! berth read <surface>
//! ```

use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::process;
use std::sync::{Arc, Mutex};
use std::thread;

use berth_core::AppState;
use berth_core::paths;
use berth_headless::{PtyHost, restore_session, save_session, session_path};
use berth_server::auth::load_or_create_token;
use berth_server::{Server, ServerConfig, SocketMode};

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Options {
    socket: PathBuf,
    mode: SocketMode,
    token: Option<String>,
    restore: bool,
    workspace: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("berth-headless: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let options = match parse_args()? {
        Some(options) => options,
        None => return Ok(()),
    };

    let state = Arc::new(Mutex::new(AppState::default()));
    let host = Arc::new(PtyHost::new());

    let token = match options.mode {
        SocketMode::Local => Some(match &options.token {
            Some(token) => token.clone(),
            None => load_or_create_token(&paths::token_path())?,
        }),
        SocketMode::Trusted | SocketMode::Off => options.token.clone(),
    };
    host.set_transport(token.clone(), Some(paths::display(&options.socket)));

    if options.restore {
        match restore_session(&state, &*host) {
            Ok(0) => {}
            Ok(count) => eprintln!("berth-headless: restored {count} surface(s)"),
            Err(error) => eprintln!("berth-headless: restore skipped: {error}"),
        }
    }
    if options.workspace && state.lock().unwrap().workspaces.is_empty() {
        state
            .lock()
            .unwrap()
            .create_workspace(Some("1"), None, None, &*host)?;
    }

    let mut config = ServerConfig::new(options.socket.clone(), options.mode);
    if let Some(token) = &token {
        config = config.token(token.clone());
    }

    let save_state = Arc::clone(&state);
    let server = Server::new(config, Arc::clone(&state), host)
        .on_session_save(Arc::new(move || {
            if let Err(error) = save_session(&save_state) {
                eprintln!("berth-headless: save failed: {error}");
            }
        }))
        .start()?;

    println!("berth-headless {VERSION}");
    println!("socket: {}", server.path().display());
    println!("auth: {}", options.mode.as_str());
    println!("session: {}", session_path().display());
    println!("ready");

    // The server owns its threads; the process just stays alive. SIGINT or
    // SIGTERM ends it (no graceful hook yet; `session.save` is explicit).
    loop {
        thread::park();
    }
}

fn parse_args() -> Result<Option<Options>, Box<dyn Error>> {
    let mut options = Options {
        socket: paths::socket_path(),
        mode: SocketMode::Local,
        token: None,
        restore: true,
        workspace: true,
    };

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => {
                options.socket = PathBuf::from(
                    args.next()
                        .ok_or("--socket needs a path (or a \\\\.\\pipe\\name)")?,
                );
            }
            "--mode" => {
                let value = args.next().ok_or("--mode needs off|local|trusted")?;
                options.mode = match value.as_str() {
                    "off" => SocketMode::Off,
                    "local" => SocketMode::Local,
                    "trusted" => SocketMode::Trusted,
                    other => return Err(format!("unknown mode {other}").into()),
                };
            }
            "--token" => options.token = Some(args.next().ok_or("--token needs a value")?),
            "--no-restore" => options.restore = false,
            "--no-workspace" => options.workspace = false,
            "-h" | "--help" => {
                print_help();
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("berth-headless {VERSION}");
                return Ok(None);
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    Ok(Some(options))
}

fn print_help() {
    println!(
        "berth-headless {VERSION}
Headless Berth shell: PTY surfaces, VT screen, and the NDJSON socket server.

USAGE:
    berth-headless [OPTIONS]

OPTIONS:
    --socket <path>      Unix socket or Windows named pipe
                         (default: {})
    --mode <mode>        off | local | trusted  (default: local)
    --token <token>      Token for local mode (default: load or create)
    --no-restore         Ignore the saved session
    --no-workspace       Do not create an empty workspace on boot
    -h, --help           Show this help
    -V, --version        Show the version",
        paths::display(&paths::socket_path())
    );
}
