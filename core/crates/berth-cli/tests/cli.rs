//! The `berth` binary against a real `berth-server`: transport, auth and
//! framing on every desktop OS, with no GUI.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use berth_core::{AppState, NullOps};
use berth_server::{Server, ServerConfig, SocketMode};

fn endpoint(tag: &str) -> PathBuf {
    #[cfg(unix)]
    {
        let path =
            std::env::temp_dir().join(format!("berth-cli-test-{}-{tag}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }
    #[cfg(windows)]
    {
        PathBuf::from(format!(
            r"\\.\pipe\berth-cli-test-{}-{tag}",
            std::process::id()
        ))
    }
}

fn berth(path: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_berth"))
        .args(args)
        .env("BERTH_SOCKET_PATH", path)
        .env_remove("BERTH_TOKEN")
        .output()
        .expect("run berth")
}

#[test]
fn cli_round_trips_workspaces_over_the_portable_transport() {
    let path = endpoint("roundtrip");
    let mut server = Server::new(
        ServerConfig::new(path.clone(), SocketMode::Trusted),
        Arc::new(Mutex::new(AppState::default())),
        Arc::new(NullOps),
    )
    .start()
    .expect("server starts");

    let ping = berth(server.path(), &["ping"]);
    assert!(ping.status.success(), "{ping:?}");
    assert!(String::from_utf8_lossy(&ping.stdout).contains("pong"));

    let created = berth(server.path(), &["new", "--name", "portable"]);
    assert!(
        created.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&created.stderr)
    );
    let workspace_id = String::from_utf8_lossy(&created.stdout).trim().to_string();
    assert!(workspace_id.starts_with("ws_"), "{workspace_id}");

    let listed = berth(server.path(), &["ls", "--json"]);
    assert!(listed.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&listed.stdout).expect("ls --json is JSON");
    assert_eq!(json["workspaces"][0]["name"], "portable");
    assert_eq!(json["selected_workspace"], workspace_id);

    let schema = berth(server.path(), &["schema"]);
    assert!(schema.status.success());
    let schema: serde_json::Value = serde_json::from_slice(&schema.stdout).expect("schema JSON");
    assert!(schema["methods"].as_array().is_some_and(|m| !m.is_empty()));

    server.stop();
}
