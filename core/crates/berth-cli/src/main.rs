//! `berth` — a thin client for the Berth socket protocol.
//!
//! Deliberately dumb: it formats requests and prints responses. Everything it
//! can do, another client can do; that is the point of the protocol.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

fn socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("BERTH_SOCKET_PATH") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Library/Application Support/Berth/berth.sock")
}

fn token_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Library/Application Support/Berth/token")
}

fn token() -> Option<String> {
    if let Ok(value) = std::env::var("BERTH_TOKEN") {
        if !value.is_empty() {
            return Some(value);
        }
    }
    std::fs::read_to_string(token_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

struct Client {
    stream: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
    token: Option<String>,
}

impl Client {
    fn connect() -> Result<Self, String> {
        let path = socket_path();
        let stream = UnixStream::connect(&path).map_err(|e| {
            format!(
                "cannot connect to {} ({e}). Is Berth running? Set BERTH_SOCKET_PATH to override.",
                path.display()
            )
        })?;
        let writer = stream.try_clone().map_err(|e| e.to_string())?;
        Ok(Self {
            stream: BufReader::new(stream),
            writer,
            next_id: 1,
            token: token(),
        })
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let mut request = json!({"id": id, "method": method, "params": params});
        if let Some(token) = &self.token {
            request["token"] = json!(token);
        }
        let line = request.to_string();
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .map_err(|e| e.to_string())?;

        loop {
            let mut buf = String::new();
            let n = self.stream.read_line(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("connection closed by server".into());
            }
            let v: Value = serde_json::from_str(buf.trim()).map_err(|e| e.to_string())?;
            if v.get("id").and_then(Value::as_i64) == Some(id as i64) {
                if v.get("ok").and_then(Value::as_bool) == Some(true) {
                    return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                }
                let err = v.get("error").cloned().unwrap_or(Value::Null);
                return Err(format!(
                    "{}: {}",
                    err.get("code").and_then(Value::as_str).unwrap_or("error"),
                    err.get("message").and_then(Value::as_str).unwrap_or("")
                ));
            }
            // Not our response: it's an event. Print it when streaming.
            if std::env::var_os("BERTH_VERBOSE").is_some() {
                eprintln!("{buf}");
            }
        }
    }

    fn stream_events(
        &mut self,
        types: Option<Vec<String>>,
        since: Option<u64>,
    ) -> Result<(), String> {
        let mut params = json!({});
        if let Some(t) = types {
            params["types"] = json!(t);
        }
        if let Some(s) = since {
            params["since"] = json!(s);
        }
        let _ = self.call("events.subscribe", params)?;
        loop {
            let mut buf = String::new();
            let n = self.stream.read_line(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                return Ok(());
            }
            print!("{buf}");
            let _ = std::io::stdout().flush();
        }
    }
}

fn arg_after(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

fn positional(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
            continue;
        }
        if a.starts_with("--") {
            skip = !a.contains('=');
            continue;
        }
        out.push(a.clone());
    }
    out
}

const USAGE: &str = "\
berth — control a running Berth instance

usage:
  berth ping
  berth ls [--json]             workspaces, surfaces and agents
  berth agents [--state S]
  berth new [--name N] [--cwd DIR]
  berth close-workspace [WORKSPACE_ID]
  berth split SURFACE_ID [--direction right|left|up|down]
  berth send SURFACE_ID TEXT    type into a surface
  berth send-key SURFACE_ID KEY send a named key (enter, esc, ctrl-c, …)
  berth read SURFACE_ID [--scrollback] [--max-bytes N]
  berth notify TITLE [--level info|needs_input|error] [--body B]
  berth agent-register SURFACE_ID [--provider P] [--name N]
  berth agent-set AGENT_ID [--state S] [--detail D] [--progress 0.4]
  berth events [--types a,b] [--since SEQ]
  berth snapshot                session JSON on stdout
  berth active                  active surface id of the selected workspace
  berth schema                  protocol methods and events as JSON

env:
  BERTH_SOCKET_PATH   socket location
  BERTH_TOKEN         socket token (defaults to the token file)
  BERTH_VERBOSE       print unclaimed events to stderr
";

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().cloned().unwrap_or_else(|| "ls".into());
    let rest = if args.len() > 1 { &args[1..] } else { &[][..] };
    let mut c = Client::connect()?;

    match cmd.as_str() {
        "help" | "-h" | "--help" => print!("{USAGE}"),
        "ping" => println!("{}", c.call("system.ping", json!({}))?),

        "ls" => {
            let workspaces = c.call("workspace.list", json!({}))?;
            let surfaces = c.call("surface.list", json!({}))?;
            let agents = c.call("agent.list", json!({}))?;
            if rest.iter().any(|a| a == "--json") {
                let combined = json!({
                    "workspaces": workspaces["workspaces"],
                    "groups": workspaces["groups"],
                    "selected_workspace": workspaces["selected_workspace"],
                    "surfaces": surfaces["surfaces"],
                    "agents": agents["agents"],
                });
                println!("{}", serde_json::to_string_pretty(&combined).unwrap());
            } else {
                print_ls(&workspaces, &surfaces, &agents);
            }
        }

        "agents" => {
            let mut params = json!({});
            if let Some(s) = arg_after(rest, "--state") {
                params["state"] = json!(s);
            }
            let v = c.call("agent.list", params)?;
            for a in v["agents"].as_array().cloned().unwrap_or_default() {
                println!(
                    "{}\t{}\t{}\t{}",
                    a["name"].as_str().unwrap_or("?"),
                    a["state"].as_str().unwrap_or("?"),
                    a["provider"].as_str().unwrap_or("?"),
                    a["detail"].as_str().unwrap_or("")
                );
            }
        }

        "new" => {
            let mut params = json!({});
            if let Some(n) = arg_after(rest, "--name") {
                params["name"] = json!(n);
            }
            if let Some(d) = arg_after(rest, "--cwd") {
                params["cwd"] = json!(d);
            }
            let v = c.call("workspace.create", params)?;
            println!("{}", v["workspace"]["id"].as_str().unwrap_or("?"));
        }

        "close-workspace" => {
            let pos = positional(rest);
            let wid = if let Some(id) = pos.first() {
                id.clone()
            } else {
                let v = c.call("workspace.list", json!({}))?;
                v["selected_workspace"]
                    .as_str()
                    .ok_or("close-workspace: no selected workspace")?
                    .to_string()
            };
            c.call("workspace.close", json!({"workspace_id": wid}))?;
        }

        "split" => {
            let pos = positional(rest);
            let sid = pos.first().ok_or("split: missing SURFACE_ID")?;
            let dir = arg_after(rest, "--direction").unwrap_or_else(|| "right".into());
            let v = c.call(
                "surface.split",
                json!({"surface_id": sid, "direction": dir}),
            )?;
            println!("{}", v["surface_id"].as_str().unwrap_or("?"));
        }

        "send" => {
            let pos = positional(rest);
            let sid = pos.first().ok_or("send: missing SURFACE_ID")?;
            let text = pos.get(1..).map(|t| t.join(" ")).unwrap_or_default();
            c.call(
                "surface.send_text",
                json!({"surface_id": sid, "text": text}),
            )?;
        }

        "send-key" => {
            let pos = positional(rest);
            let sid = pos.first().ok_or("send-key: missing SURFACE_ID")?;
            let key = pos.get(1).ok_or("send-key: missing KEY")?;
            c.call("surface.send_key", json!({"surface_id": sid, "key": key}))?;
        }

        "read" => {
            let pos = positional(rest);
            let sid = pos.first().ok_or("read: missing SURFACE_ID")?;
            let mut params = json!({"surface_id": sid, "scope": "screen"});
            if rest.iter().any(|a| a == "--scrollback") {
                params["scope"] = json!("scrollback");
            }
            if let Some(value) = arg_after(rest, "--max-bytes") {
                let n: u64 = value.parse().map_err(|_| "max-bytes must be a number")?;
                params["max_bytes"] = json!(n);
            }
            let v = c.call("surface.read_text", params)?;
            print!("{}", v["text"].as_str().unwrap_or(""));
        }

        "notify" => {
            let pos = positional(rest);
            let title = pos.join(" ");
            if title.is_empty() {
                return Err("notify: missing TITLE".into());
            }
            let mut params = json!({"title": title});
            if let Some(l) = arg_after(rest, "--level") {
                params["level"] = json!(l);
            }
            if let Some(b) = arg_after(rest, "--body") {
                params["body"] = json!(b);
            }
            c.call("notification.create", params)?;
        }

        "agent-register" => {
            let pos = positional(rest);
            let sid = pos.first().ok_or("agent-register: missing SURFACE_ID")?;
            let mut params = json!({"surface_id": sid});
            if let Some(p) = arg_after(rest, "--provider") {
                params["provider"] = json!(p);
            }
            if let Some(n) = arg_after(rest, "--name") {
                params["name"] = json!(n);
            }
            let v = c.call("agent.register", params)?;
            println!("{}", v["agent"]["id"].as_str().unwrap_or("?"));
        }

        "agent-set" => {
            let pos = positional(rest);
            let aid = pos.first().ok_or("agent-set: missing AGENT_ID")?;
            let mut params = json!({"agent_id": aid});
            if let Some(s) = arg_after(rest, "--state") {
                params["state"] = json!(s);
            }
            if let Some(d) = arg_after(rest, "--detail") {
                params["detail"] = json!(d);
            }
            if let Some(p) = arg_after(rest, "--progress") {
                let f: f64 = p.parse().map_err(|_| "progress must be 0..1")?;
                params["progress"] = json!(f);
            }
            c.call("agent.update", params)?;
        }

        "events" => {
            let types = arg_after(rest, "--types").map(|t| {
                t.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect()
            });
            let since = arg_after(rest, "--since")
                .map(|s| s.parse::<u64>())
                .transpose()
                .map_err(|_| "since must be a sequence number")?;
            c.stream_events(types, since)?;
        }

        "snapshot" => {
            let v = c.call("session.dump", json!({}))?;
            println!("{}", serde_json::to_string_pretty(&v["session"]).unwrap());
        }

        "active" => {
            let v = c.call("workspace.list", json!({}))?;
            println!("{}", v["selected_surface"].as_str().unwrap_or(""));
        }

        "schema" => {
            let v = c.call("system.capabilities", json!({}))?;
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
        }

        other => {
            eprintln!("unknown command: {other}\n\n{USAGE}");
            return Err(String::new());
        }
    }
    Ok(())
}

fn print_ls(workspaces: &Value, surfaces: &Value, agents: &Value) {
    let empty = vec![];
    let ws = workspaces["workspaces"].as_array().unwrap_or(&empty);
    let sfs = surfaces["surfaces"].as_array().unwrap_or(&empty);
    let ags = agents["agents"].as_array().unwrap_or(&empty);

    for w in ws {
        let wid = w["id"].as_str().unwrap_or("?");
        let count = sfs
            .iter()
            .filter(|s| s["workspace_id"].as_str() == Some(wid))
            .count();
        let unread = w["unread"].as_u64().unwrap_or(0);
        let marker = if unread > 0 {
            format!(" ●{unread}")
        } else {
            String::new()
        };
        println!("{wid}  {}{marker}", w["name"].as_str().unwrap_or("?"),);
        println!("  {count} surface(s)");
        for a in ags
            .iter()
            .filter(|a| a["workspace_id"].as_str() == Some(wid))
        {
            println!(
                "  └ {}  [{}]  {}",
                a["name"].as_str().unwrap_or("?"),
                a["state"].as_str().unwrap_or("?"),
                a["detail"].as_str().unwrap_or("")
            );
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.is_empty() => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("berth: {e}");
            ExitCode::FAILURE
        }
    }
}
