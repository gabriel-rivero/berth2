//! End-to-end transport tests against a real local socket. These run on
//! Linux, macOS and Windows in CI; no GUI and no Ghostty required.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use berth_core::{AppState, NullOps};
use berth_server::{Server, ServerConfig, SocketMode};
use berth_transport::LocalStream;
use serde_json::{Value, json};

fn endpoint(tag: &str) -> PathBuf {
    #[cfg(unix)]
    {
        let path = std::env::temp_dir().join(format!(
            "berth-server-test-{}-{tag}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        path
    }
    #[cfg(windows)]
    {
        PathBuf::from(format!(
            r"\\.\pipe\berth-server-test-{}-{tag}",
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
    serde_json::from_str(line.trim()).unwrap_or_else(|e| panic!("{e}: {line:?}"))
}

/// Send a request and collect events until its response arrives.
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
fn handshake_auth_mutations_and_session_save() {
    let path = endpoint("handshake");
    let state = Arc::new(Mutex::new(AppState::default()));
    let saves = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&saves);
    let mut server = Server::new(
        ServerConfig::new(path.clone(), SocketMode::Local).token("s3cret"),
        state,
        Arc::new(NullOps),
    )
    .on_session_save(Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }))
    .start()
    .expect("server starts");

    let stream = connect(server.path());
    let mut reader = BufReader::new(&stream);

    // 1. The greeting is an event, not a response, and reports the real mode.
    let hello = read_line(&mut reader);
    assert_eq!(hello["event"], "hello");
    assert_eq!(hello["data"]["protocol"], 1);
    assert_eq!(hello["data"]["auth"], "local");

    // 2. Read-only methods need no token.
    let (listed, events) = request(
        &stream,
        &mut reader,
        json!({"id": 1, "method": "workspace.list", "params": {}}),
    );
    assert_eq!(listed["ok"], true);
    assert!(events.is_empty());

    // 3. Mutations do.
    let (denied, _) = request(
        &stream,
        &mut reader,
        json!({"id": 2, "method": "workspace.create", "params": {"name": "nope"}}),
    );
    assert_eq!(denied["ok"], false);
    assert_eq!(denied["error"]["code"], "permission_denied");

    // 4. With the token the workspace exists and its event reaches us.
    let (created, _) = request(
        &stream,
        &mut reader,
        json!({
            "id": 3,
            "method": "workspace.create",
            "params": {"name": "portable"},
            "token": "s3cret",
        }),
    );
    assert_eq!(created["ok"], true, "{created}");
    assert_eq!(created["result"]["workspace"]["name"], "portable");

    // 5. Subscriptions replay and then receive live events.
    let (subscribed, replayed) = request(
        &stream,
        &mut reader,
        json!({
            "id": 4,
            "method": "events.subscribe",
            "params": {"types": ["workspace.created"], "since": 0},
        }),
    );
    assert_eq!(subscribed["ok"], true);
    assert!(
        replayed
            .iter()
            .any(|event| event["event"] == "workspace.created"),
        "replay should include the earlier creation: {replayed:?}"
    );

    let (second, live) = request(
        &stream,
        &mut reader,
        json!({
            "id": 5,
            "method": "workspace.create",
            "params": {"name": "second"},
            "token": "s3cret",
        }),
    );
    assert_eq!(second["ok"], true, "{second}");
    assert!(
        live.iter()
            .any(|event| event["event"] == "workspace.created"),
        "live event missing: {live:?}"
    );

    // 6. session.save is shell policy, forwarded exactly once.
    let (saved, _) = request(
        &stream,
        &mut reader,
        json!({"id": 6, "method": "session.save", "params": {}, "token": "s3cret"}),
    );
    assert_eq!(saved["ok"], true);
    assert_eq!(saves.load(Ordering::SeqCst), 1);
    assert!(
        !saved["result"]["path"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );

    server.stop();
}

#[test]
fn an_oversized_request_gets_payload_too_large_and_a_close() {
    let path = endpoint("oversized");
    let mut config = ServerConfig::new(path, SocketMode::Trusted);
    config.max_line_bytes = 64;
    let mut server = Server::new(
        config,
        Arc::new(Mutex::new(AppState::default())),
        Arc::new(NullOps),
    )
    .start()
    .expect("server starts");

    let stream = connect(server.path());
    let mut reader = BufReader::new(&stream);
    let _hello = read_line(&mut reader);

    let payload = format!(
        "{{\"id\":1,\"method\":\"system.ping\",\"params\":\"{}\"}}\n",
        "x".repeat(200)
    );
    (&stream).write_all(payload.as_bytes()).unwrap();
    let error = read_line(&mut reader);
    assert_eq!(error["ok"], false);
    assert_eq!(error["error"]["code"], "payload_too_large");

    // The server closes the desynchronised connection.
    let mut line = String::new();
    assert_eq!(reader.read_line(&mut line).unwrap(), 0);

    server.stop();
}

#[test]
fn trusted_mode_accepts_mutations_without_a_token() {
    let path = endpoint("trusted");
    let mut server = Server::new(
        ServerConfig::new(path, SocketMode::Trusted),
        Arc::new(Mutex::new(AppState::default())),
        Arc::new(NullOps),
    )
    .start()
    .expect("server starts");

    let stream = connect(server.path());
    let mut reader = BufReader::new(&stream);
    let _hello = read_line(&mut reader);

    let (created, _) = request(
        &stream,
        &mut reader,
        json!({"id": 1, "method": "workspace.create", "params": {"name": "trusted"}}),
    );
    assert_eq!(created["ok"], true, "{created}");

    server.stop();
}
