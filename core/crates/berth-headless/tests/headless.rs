//! End-to-end tests for the headless shell: a real PTY, a real socket.
//!
//! These are the shell spike's acceptance check on Linux/macOS/Windows CI:
//! create a workspace, type into its terminal, and read the rendered screen
//! back through the same protocol the macOS app uses.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use berth_core::AppState;
use berth_headless::PtyHost;
use berth_server::{Server, ServerConfig, SocketMode};
use berth_transport::LocalStream;
use serde_json::{Value, json};

fn endpoint(tag: &str) -> PathBuf {
    #[cfg(unix)]
    {
        let path = std::env::temp_dir().join(format!(
            "berth-headless-test-{}-{tag}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        path
    }
    #[cfg(windows)]
    {
        PathBuf::from(format!(
            r"\\.\pipe\berth-headless-test-{}-{tag}",
            std::process::id()
        ))
    }
}

fn connect(path: &std::path::Path) -> LocalStream {
    let mut last = None;
    for _ in 0..100 {
        match berth_transport::connect(path) {
            Ok(stream) => return stream,
            Err(error) => {
                last = Some(error);
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    panic!("connect failed: {last:?}");
}

fn read_line(reader: &mut BufReader<&LocalStream>) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).expect("read line");
    serde_json::from_str(line.trim()).unwrap_or_else(|error| panic!("{error}: {line:?}"))
}

fn request(
    stream: &LocalStream,
    reader: &mut BufReader<&LocalStream>,
    body: Value,
) -> (Value, Vec<Value>) {
    let id = body["id"].clone();
    (&*stream)
        .write_all(format!("{body}\n").as_bytes())
        .expect("write request");
    let mut events = Vec::new();
    loop {
        let value = read_line(reader);
        if value.get("id") == Some(&id) {
            return (value, events);
        }
        events.push(value);
    }
}

#[test]
fn spawn_type_and_read_back_through_the_protocol() {
    let path = endpoint("screen");
    let cwd = std::env::temp_dir().join(format!("berth-headless-cwd-{}", std::process::id()));
    std::fs::create_dir_all(&cwd).expect("cwd");

    let host = Arc::new(PtyHost::new());
    let mut server = Server::new(
        ServerConfig::new(path, SocketMode::Trusted),
        Arc::new(Mutex::new(AppState::default())),
        host.clone(),
    )
    .start()
    .expect("server starts");

    let stream = connect(server.path());
    let mut reader = BufReader::new(&stream);
    let hello = read_line(&mut reader);
    assert_eq!(hello["event"], "hello");

    // Creating a workspace also spawns its first terminal.
    let (created, _) = request(
        &stream,
        &mut reader,
        json!({
            "id": 1,
            "method": "workspace.create",
            "params": {"name": "spike", "cwd": cwd},
        }),
    );
    assert_eq!(created["ok"], true, "{created}");
    let surface_id = created["result"]["workspace"]["active_surface"]
        .as_str()
        .expect("active surface")
        .to_string();
    assert_eq!(host.len(), 1, "the PTY should exist before the response");

    // Send text, then Enter as a synthetic key (the two protocol paths).
    let (sent, _) = request(
        &stream,
        &mut reader,
        json!({
            "id": 2,
            "method": "surface.send_text",
            "params": {"surface_id": surface_id, "text": "echo berth-spike-ok"},
        }),
    );
    assert_eq!(sent["ok"], true, "{sent}");

    let (keyed, _) = request(
        &stream,
        &mut reader,
        json!({
            "id": 3,
            "method": "surface.send_key",
            "params": {"surface_id": surface_id, "key": "enter"},
        }),
    );
    assert_eq!(keyed["ok"], true, "{keyed}");

    // Wait for the shell to run it and the VT parser to render the result.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut screen = String::new();
    while Instant::now() < deadline {
        let (read, _) = request(
            &stream,
            &mut reader,
            json!({
                "id": 4,
                "method": "surface.read_text",
                "params": {"surface_id": surface_id, "scope": "screen"},
            }),
        );
        assert_eq!(read["ok"], true, "{read}");
        screen = read["result"]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if screen.contains("berth-spike-ok") {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        screen.contains("berth-spike-ok"),
        "screen never showed the command output:\n{screen}"
    );

    let view = host.screen_view(&surface_id).expect("screen view");
    assert_eq!((view.cols, view.rows), (80, 24));
    assert_eq!(
        view.cells.len(),
        usize::from(view.cols) * usize::from(view.rows)
    );
    assert!(view.cursor.0 < view.rows && view.cursor.1 < view.cols);

    // Resize reaches both the kernel and the parser.
    let (resized, _) = request(
        &stream,
        &mut reader,
        json!({
            "id": 5,
            "method": "surface.resize",
            "params": {"surface_id": surface_id, "cols": 100, "rows": 30},
        }),
    );
    assert_eq!(resized["ok"], true, "{resized}");
    let (listed, _) = request(
        &stream,
        &mut reader,
        json!({"id": 6, "method": "surface.list", "params": {}}),
    );
    assert_eq!(listed["result"]["surfaces"][0]["cols"], 100);
    assert_eq!(listed["result"]["surfaces"][0]["rows"], 30);

    // Closing the workspace closes the PTY, not just the core record.
    let workspace_id = created["result"]["workspace"]["id"]
        .as_str()
        .expect("workspace id");
    let (closed, _) = request(
        &stream,
        &mut reader,
        json!({
            "id": 7,
            "method": "workspace.close",
            "params": {"workspace_id": workspace_id},
        }),
    );
    assert_eq!(closed["ok"], true, "{closed}");
    assert!(host.is_empty(), "the PTY should be closed");

    server.stop();
    let _ = std::fs::remove_dir_all(&cwd);
}
