//! End-to-end tests for the headless shell: a real PTY, a real socket.
//!
//! These are the shell spike's acceptance check on Linux/macOS/Windows CI:
//! create a workspace, type into its terminal, and read the rendered screen
//! back through the same protocol the macOS app uses.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use berth_core::{AppState, SurfaceOps};
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
    // A cold ConPTY can take a while to consume its first input, so the command
    // is re-sent a few times before the test gives up.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut screen = String::new();
    let mut resends = 0;
    let mut last_send = Instant::now();
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
        if last_send.elapsed() >= Duration::from_secs(4) && resends < 3 {
            resends += 1;
            last_send = Instant::now();
            let _ = request(
                &stream,
                &mut reader,
                json!({
                    "id": 10 + resends,
                    "method": "surface.send_text",
                    "params": {"surface_id": surface_id, "text": "echo berth-spike-ok"},
                }),
            );
            let _ = request(
                &stream,
                &mut reader,
                json!({
                    "id": 20 + resends,
                    "method": "surface.send_key",
                    "params": {"surface_id": surface_id, "key": "enter"},
                }),
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        screen.contains("berth-spike-ok"),
        "screen never showed the command output (alive={}, info={:?}, resends={resends}):\n{screen:?}",
        host.is_alive(&surface_id),
        host.pid_info(&surface_id).ok(),
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

/// A plain terminal spawn carries no argv; the host's default shell fills the
/// gap. This is the path `shell` in `config.json` takes in the portable window.
#[test]
fn given_a_default_shell_when_a_plain_terminal_spawns_then_it_runs_that_program() {
    use berth_core::{ReadScope, SpawnRequest};

    let host = PtyHost::new();
    #[cfg(windows)]
    let argv: Vec<String> = vec![
        "cmd.exe".into(),
        "/c".into(),
        "echo berth-default-shell".into(),
    ];
    #[cfg(unix)]
    let argv: Vec<String> = vec![
        "/bin/sh".into(),
        "-c".into(),
        "echo berth-default-shell".into(),
    ];
    host.set_default_shell(argv);

    let surface_id = format!("surf-default-shell-{}", std::process::id());
    host.spawn(SpawnRequest {
        cwd: None,
        argv: Vec::new(),
        env: vec![("BERTH_SURFACE_ID".into(), surface_id.clone())],
        cols: 80,
        rows: 24,
    })
    .expect("spawn with the configured shell");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let text = host
            .read_text(&surface_id, ReadScope::Screen, 1 << 16)
            .expect("read screen")
            .text;
        if text.contains("berth-default-shell") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the configured shell never printed its marker: {text:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// DECSET 2026 is a private mode `vt100` drops. The PTY reader must observe it
/// and expose it on the screen view the window renders from.
#[cfg(unix)]
#[test]
fn given_synchronized_output_from_a_pty_then_the_screen_view_reports_it() {
    use berth_core::SpawnRequest;

    let host = PtyHost::new();
    let surface_id = format!("surf-sync-output-{}", std::process::id());
    host.spawn(SpawnRequest {
        cwd: None,
        argv: vec![
            "/bin/sh".into(),
            "-c".into(),
            "printf '\\033[?2026h'; sleep 2".into(),
        ],
        env: vec![("BERTH_SURFACE_ID".into(), surface_id.clone())],
        cols: 80,
        rows: 24,
    })
    .expect("spawn a writer of DECSET 2026");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let view = host.screen_view(&surface_id).expect("screen view");
        if view.is_synchronized_output_enabled {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the screen view never reported synchronized output"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Once a program enables the kitty flags, `surface.send_key` must encode with
/// them on the PTY.
#[cfg(unix)]
#[test]
fn given_kitty_flags_when_a_key_is_sent_then_the_csi_u_form_reaches_the_pty() {
    use berth_core::{ReadScope, SpawnRequest};

    let host = PtyHost::new();
    let surface_id = format!("surf-kitty-key-{}", std::process::id());
    host.spawn(SpawnRequest {
        cwd: None,
        argv: vec![
            "/bin/sh".into(),
            "-c".into(),
            "stty raw -echo 2>/dev/null; printf '\\033[>1u'; \
             dd bs=1 count=7 2>/dev/null | od -An -tx1 | tr -d ' \\n'; sleep 1"
                .into(),
        ],
        env: vec![("BERTH_SURFACE_ID".into(), surface_id.clone())],
        cols: 80,
        rows: 24,
    })
    .expect("spawn a program that enables the kitty flags");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let view = host.screen_view(&surface_id).expect("screen view");
        if view.kitty_keyboard_flags.bits() == 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the program never enabled the kitty flags"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    host.send_key(&surface_id, "c", &["ctrl".to_string()])
        .expect("send ctrl+c");

    loop {
        let text = host
            .read_text(&surface_id, ReadScope::Screen, 1 << 16)
            .expect("read screen")
            .text;
        if text.contains("1b5b39393b3575") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the program never received the CSI u key: {text:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The kitty query must be answered on the PTY, not only recorded in the UI.
#[cfg(unix)]
#[test]
fn given_a_kitty_keyboard_query_then_the_flags_are_answered_on_the_pty() {
    use berth_core::{ReadScope, SpawnRequest};

    let host = PtyHost::new();
    let surface_id = format!("surf-kitty-query-{}", std::process::id());
    host.spawn(SpawnRequest {
        cwd: None,
        argv: vec![
            "/bin/sh".into(),
            "-c".into(),
            "stty raw -echo 2>/dev/null; printf '\\033[>1u\\033[?u'; \
             dd bs=1 count=5 2>/dev/null | od -An -tx1 | tr -d ' \\n'; sleep 1"
                .into(),
        ],
        env: vec![("BERTH_SURFACE_ID".into(), surface_id.clone())],
        cols: 80,
        rows: 24,
    })
    .expect("spawn a program that queries the keyboard protocol");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let text = host
            .read_text(&surface_id, ReadScope::Screen, 1 << 16)
            .expect("read screen")
            .text;
        if text.contains("1b5b3f3175") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the program never received the flag reply: {text:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Extended SGR and OSC 8 must reach the cells the window renders.
#[cfg(unix)]
#[test]
fn given_styled_text_and_a_hyperlink_then_the_screen_view_carries_them() {
    use berth_core::SpawnRequest;
    use berth_headless::style::indexed_rgb;

    let host = PtyHost::new();
    let surface_id = format!("surf-styled-text-{}", std::process::id());
    host.spawn(SpawnRequest {
        cwd: None,
        argv: vec![
            "/bin/sh".into(),
            "-c".into(),
            "printf '\\033[4:3;58;5;196;9mstyled\\033[0m \
             \\033]8;;https://example.com\\007link\\033]8;;\\007'; sleep 1"
                .into(),
        ],
        env: vec![("BERTH_SURFACE_ID".into(), surface_id.clone())],
        cols: 80,
        rows: 24,
    })
    .expect("spawn a writer of styled text");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let view = host.screen_view(&surface_id).expect("screen view");
        if view.cells.len() > 7 && view.cells[0].contents == "s" && view.cells[7].contents == "l" {
            let styled = &view.cells[0];
            assert_eq!(
                styled.underline_style,
                berth_headless::style::UnderlineStyle::Curly
            );
            assert_eq!(styled.underline_color, Some(indexed_rgb(196)));
            assert!(styled.strikethrough);
            assert!(styled.hyperlink.is_none());
            assert_eq!(
                view.cells[7].hyperlink.as_deref(),
                Some("https://example.com")
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the styled text never reached the grid"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// OSC 133 prompt marks must survive the PTY reader and reach the view the
/// window draws the gutter from.
#[cfg(unix)]
#[test]
fn given_osc_133_prompt_marks_then_the_screen_view_reports_them() {
    use berth_core::SpawnRequest;
    use berth_headless::prompt::PromptKind;

    let host = PtyHost::new();
    let surface_id = format!("surf-prompt-marks-{}", std::process::id());
    host.spawn(SpawnRequest {
        cwd: None,
        argv: vec![
            "/bin/sh".into(),
            "-c".into(),
            "printf '\\033]133;A\\007$ false\\r\\n'; \
             printf '\\033]133;C\\007boom\\r\\n'; \
             printf '\\033]133;D;1\\007'; \
             printf '\\033]133;A\\007$ '; sleep 1"
                .into(),
        ],
        env: vec![("BERTH_SURFACE_ID".into(), surface_id.clone())],
        cols: 80,
        rows: 24,
    })
    .expect("spawn a program that marks its prompt");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let view = host.screen_view(&surface_id).expect("screen view");
        if view.prompt_marks.len() >= 2 {
            assert_eq!(view.prompt_marks[0].kind, PromptKind::Prompt);
            assert_eq!(view.prompt_marks[0].status, None);
            let last = view.prompt_marks.last().expect("the second prompt");
            assert_eq!(last.kind, PromptKind::Prompt);
            assert_eq!(last.status, Some(1));
            assert!(last.failed());
            let lines = host.prompt_lines(&surface_id).expect("prompt lines");
            assert_eq!(lines.len(), 2);
            // Scrolling the view must not prune the marks: they are bounded by
            // the scrollback capacity, not by the current offset.
            host.set_scrollback(&surface_id, 1)
                .expect("scroll back one");
            let scrolled = host.prompt_lines(&surface_id).expect("prompt lines");
            assert_eq!(scrolled.len(), 2, "scrolling dropped a prompt mark");
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the prompt marks never reached the screen view"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// DECSCUSR, CSI ?1004 and OSC 52 read requests must survive the PTY reader.
#[cfg(unix)]
#[test]
fn given_cursor_focus_and_clipboard_requests_then_the_screen_view_reports_them() {
    use berth_core::SpawnRequest;
    use berth_headless::cursor::CursorShape;

    let host = PtyHost::new();
    let surface_id = format!("surf-cursor-focus-{}", std::process::id());
    host.spawn(SpawnRequest {
        cwd: None,
        argv: vec![
            "/bin/sh".into(),
            "-c".into(),
            "printf '\\033[4 q\\033[?1004h\\033]52;c;?\\007'; sleep 1".into(),
        ],
        env: vec![("BERTH_SURFACE_ID".into(), surface_id.clone())],
        cols: 80,
        rows: 24,
    })
    .expect("spawn a writer of cursor and focus modes");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let view = host.screen_view(&surface_id).expect("screen view");
        let meta = host
            .take_screen_meta_requests(&surface_id)
            .expect("meta requests");
        if view.cursor_style.shape == CursorShape::Underline
            && !view.cursor_style.blinks
            && view.is_focus_reporting_enabled
            && meta.clipboard_read.as_deref() == Some("c")
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "cursor, focus or clipboard state never arrived"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// SGR 5 must reach the cell, not just the style map.
#[cfg(unix)]
#[test]
fn given_blinking_text_then_the_screen_view_marks_the_cell() {
    use berth_core::SpawnRequest;

    let host = PtyHost::new();
    let surface_id = format!("surf-blink-{}", std::process::id());
    host.spawn(SpawnRequest {
        cwd: None,
        argv: vec![
            "/bin/sh".into(),
            "-c".into(),
            "printf '\\033[5mB\\033[25m'; sleep 1".into(),
        ],
        env: vec![("BERTH_SURFACE_ID".into(), surface_id.clone())],
        cols: 80,
        rows: 24,
    })
    .expect("spawn a writer of SGR 5");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let view = host.screen_view(&surface_id).expect("screen view");
        if view.cells.first().is_some_and(|cell| cell.contents == "B") {
            assert!(view.cells[0].blink, "the cell did not carry SGR 5");
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the blinking text never reached the grid"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A narrow pane must re-wrap history instead of cutting it off.
#[cfg(unix)]
#[test]
fn given_wrapped_output_when_the_surface_narrows_then_no_character_is_lost() {
    use berth_core::{SpawnRequest, SurfaceOps};

    const TOKEN: &str = "BEGINabcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZEND";
    let host = PtyHost::new();
    let surface_id = format!("surf-reflow-{}", std::process::id());
    host.spawn(SpawnRequest {
        cwd: None,
        argv: vec![
            "/bin/sh".into(),
            "-c".into(),
            format!("printf '{TOKEN}\\n'; sleep 5"),
        ],
        env: vec![("BERTH_SURFACE_ID".into(), surface_id.clone())],
        cols: 80,
        rows: 24,
    })
    .expect("spawn a writer of a long line");

    let deadline = Instant::now() + Duration::from_secs(5);
    while !row_text(&host, &surface_id).contains(TOKEN) {
        assert!(
            Instant::now() < deadline,
            "the long line never reached the grid:\n{}",
            row_text(&host, &surface_id)
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    host.resize(&surface_id, 30, 24).expect("narrow the pane");
    // Narrowing pushes the head of the line into history, so read both.
    let text = all_text(&host, &surface_id);
    assert!(
        text.contains(TOKEN),
        "the narrow pane lost characters:\n{text}"
    );
}

/// History and visible rows, whitespace removed so a re-wrapped line matches.
#[cfg(unix)]
fn all_text(host: &PtyHost, surface_id: &str) -> String {
    host.scrollback_lines(surface_id)
        .expect("scrollback")
        .join("")
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

/// Every cell of the visible screen, spaces kept, rows joined.
#[cfg(unix)]
fn row_text(host: &PtyHost, surface_id: &str) -> String {
    let view = host.screen_view(surface_id).expect("screen view");
    view.cells
        .iter()
        .map(|cell| cell.contents.as_str())
        .collect::<String>()
}
