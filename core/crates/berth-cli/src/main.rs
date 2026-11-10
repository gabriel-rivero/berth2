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

struct Client {
    stream: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
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
        })
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({"id": id, "method": method, "params": params}).to_string();
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

    fn stream_events(&mut self, types: Option<Vec<String>>) -> Result<(), String> {
        let mut params = json!({});
        if let Some(t) = types {
            params["types"] = json!(t);
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
  berth ls                      workspaces, surfaces and agents
  berth agents [--state S]
  berth new [--name N] [--cwd DIR]
  berth split SURFACE_ID [--direction right|down]
  berth send SURFACE_ID TEXT    type into a surface
  berth notify TITLE [--level info|needs_input|error] [--body B]
  berth agent-register SURFACE_ID [--provider P] [--name N]
  berth agent-set AGENT_ID [--state S] [--detail D] [--progress 0.4]
  berth events [--types a,b]
  berth snapshot                session JSON on stdout

env:
  BERTH_SOCKET_PATH   socket location
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
            print_ls(&workspaces, &surfaces, &agents);
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
            c.stream_events(types)?;
        }

        "snapshot" => {
            let v = c.call("session.dump", json!({}))?;
            println!("{}", serde_json::to_string_pretty(&v["session"]).unwrap());
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
