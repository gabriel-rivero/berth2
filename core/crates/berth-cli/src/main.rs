//! `berth` — a thin client for the Berth socket protocol.
//!
//! Deliberately dumb: it formats requests and prints responses. Everything it
//! can do, another client can do; that is the point of the protocol.

use std::io::{BufRead, BufReader, Read, Write};
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

fn provider_for(argv: &[String]) -> String {
    let program = argv
        .first()
        .map(|a| a.rsplit('/').next().unwrap_or(a).to_ascii_lowercase())
        .unwrap_or_default();
    if program.starts_with("claude") {
        "claude-code".into()
    } else if program.starts_with("codex") {
        "codex".into()
    } else if program.starts_with("opencode") {
        "opencode".into()
    } else {
        "custom".into()
    }
}

/// Ghostty launches commands through a login shell with a minimal environment,
/// so `claude` on the user's PATH is invisible unless we resolve it here and
/// pass that PATH along. Absolute paths also keep restore independent of PATH.
fn resolve_program(program: &str) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;

    fn executable(path: &std::path::Path) -> bool {
        path.metadata()
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    if program.contains('/') {
        let path = std::path::Path::new(program);
        return executable(path).then(|| program.to_string());
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(program))
        .find(|path| executable(path))
        .map(|path| path.display().to_string())
}

fn agent_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("agt_{nanos:x}{:x}", std::process::id())
}

fn agent_for_surface(c: &mut Client, surface_id: &str) -> Result<String, String> {
    let listed = c.call("agent.list", json!({}))?;
    if let Some(id) = listed["agents"]
        .as_array()
        .and_then(|agents| {
            agents
                .iter()
                .find(|a| a["surface_id"].as_str() == Some(surface_id))
        })
        .and_then(|a| a["id"].as_str())
    {
        return Ok(id.to_string());
    }
    let detected = c.call("agent.detect", json!({"surface_id": surface_id}))?;
    detected["agent"]["id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("no agent detected on surface {surface_id}"))
}

/// The agent id comes from the wrapper's environment. Passive agents can be
/// resolved from the surface id instead.
fn hook_agent(c: &mut Client) -> Result<String, String> {
    if let Ok(id) = std::env::var("BERTH_AGENT_ID") {
        if !id.is_empty() {
            return Ok(id);
        }
    }
    let surface = std::env::var("BERTH_SURFACE_ID")
        .map_err(|_| "neither BERTH_AGENT_ID nor BERTH_SURFACE_ID is set".to_string())?;
    agent_for_surface(c, &surface)
}

fn claude_hook_state<'a>(event: &str, detail: &'a str) -> Option<(&'static str, &'a str)> {
    match event {
        "SessionStart" => Some(("idle", "session started")),
        "UserPromptSubmit" => Some(("thinking", "working")),
        "PreToolUse" => Some(("tool", detail)),
        "PostToolUse" => Some(("thinking", "thinking")),
        "Notification" => {
            if detail.to_ascii_lowercase().contains("permission") {
                Some(("blocked", detail))
            } else {
                Some(("waiting_input", detail))
            }
        }
        "Stop" => Some(("idle", "turn complete")),
        "PreCompact" => Some(("thinking", "compacting context")),
        "SessionEnd" => Some(("done", "session ended")),
        _ => None,
    }
}

fn run_claude_hook(c: &mut Client) -> Result<(), String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| e.to_string())?;
    let payload: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    let event = payload["hook_event_name"].as_str().unwrap_or_default();
    let tool = payload["tool_name"].as_str().unwrap_or_default();
    let message = payload["message"].as_str().unwrap_or_default();
    let detail = if tool.is_empty() { message } else { tool };
    let Some((state, fallback)) = claude_hook_state(event, detail) else {
        return Ok(());
    };
    let agent_id = hook_agent(c)?;
    let detail = if detail.is_empty() { fallback } else { detail };
    let mut params = json!({"agent_id": agent_id.clone(), "state": state, "detail": detail});
    if event == "SessionStart" {
        if let Some(session) = payload["session_id"].as_str() {
            params["session_ref"] = json!(session);
        }
    }
    // A hook can arrive out of order (for example PreToolUse before the prompt
    // event). Pass through thinking once instead of losing the update.
    if c.call("agent.update", params.clone()).is_err() {
        let _ = c.call(
            "agent.update",
            json!({"agent_id": agent_id, "state":"thinking"}),
        );
        c.call("agent.update", params)?;
    }
    Ok(())
}

fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".to_string())
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn write_atomic(path: &std::path::Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, contents).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

const CLAUDE_HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Notification",
    "Stop",
    "PreCompact",
    "SessionEnd",
];

fn claude_hook_entry(event: &str, command: &str) -> Value {
    let hook = json!({"type": "command", "command": command});
    if matches!(event, "PreToolUse" | "PostToolUse") {
        json!({"matcher": "*", "hooks": [hook]})
    } else {
        json!({"hooks": [hook]})
    }
}

fn claude_hook_settings(berth: &str) -> Value {
    let command = format!("{} hook claude >/dev/null 2>&1 || true", shell_quote(berth));
    let mut hooks = serde_json::Map::new();
    for event in CLAUDE_HOOK_EVENTS {
        hooks.insert(
            (*event).to_string(),
            json!([claude_hook_entry(event, &command)]),
        );
    }
    json!({"hooks": hooks})
}

fn install_claude_hooks(berth: &str) -> Result<PathBuf, String> {
    let path = home_dir()?.join(".claude/settings.json");
    let mut root: Value = if path.exists() {
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&text)
            .map_err(|e| format!("{} is not valid JSON: {e}", path.display()))?
    } else {
        json!({})
    };
    if !root.is_object() {
        return Err(format!("{} must contain a JSON object", path.display()));
    }
    if !root["hooks"].is_object() {
        root["hooks"] = json!({});
    }
    let hooks = root["hooks"].as_object_mut().unwrap();
    let command = format!("{} hook claude >/dev/null 2>&1 || true", shell_quote(berth));
    for event in CLAUDE_HOOK_EVENTS {
        let groups = hooks
            .entry((*event).to_string())
            .or_insert_with(|| json!([]));
        if !groups.is_array() {
            return Err(format!("hooks.{event} must be an array"));
        }
        let groups = groups.as_array_mut().unwrap();
        // Reinstall replaces our previous entry and leaves other hooks alone.
        groups.retain(|group| !group.to_string().contains(" hook claude"));
        groups.push(claude_hook_entry(event, &command));
    }
    let text = format!(
        "{}\n",
        serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?
    );
    write_atomic(&path, &text)?;
    Ok(path)
}

fn opencode_plugin(berth: &str) -> String {
    format!(
        r#"// berth — report OpenCode session activity to the running Berth instance.
// Installed by `berth hooks install opencode`; safe to delete.
import {{ spawn }} from "node:child_process";
import {{ Plugin }} from "@opencode/plugin";

const BERTH = {berth:?};

function update(state: string, detail: string, sessionRef?: string): Promise<void> {{
  const agent = process.env.BERTH_AGENT_ID;
  if (!agent) return Promise.resolve();
  const args = [BERTH, "agent-set", agent, "--state", state, "--detail", detail];
  if (sessionRef) args.push("--session-ref", sessionRef);
  return new Promise((resolve) => {{
    const child = spawn(BERTH, args, {{ stdio: "ignore", env: process.env }});
    child.on("error", () => resolve());
    child.on("close", () => resolve());
  }});
}}

// OpenCode V2 plugins are definitions with an id and a setup/effect function,
// not a bare event handler. setup subscribes to the public event stream and
// returns the cleanup that unsubscribes on reload.
export default Plugin.define({{
  id: "berth",
  async setup(ctx) {{
    const controller = new AbortController();
    void (async () => {{
      for await (const event of ctx.event.subscribe({{ signal: controller.signal }})) {{
        const p = (event as any).properties ?? {{}};
        const session = p.sessionID ?? p.info?.id;
        switch (event.type) {{
          case "session.created":
            await update("idle", "session started", session);
            break;
          case "session.idle":
            await update("idle", "waiting for input");
            break;
          case "session.error":
            await update("errored", p.error?.message ?? "session error");
            break;
          case "permission.updated":
          case "permission.asked":
            await update("waiting_input", "needs approval");
            break;
          case "permission.replied":
            await update("thinking", "permission answered");
            break;
          case "session.compacted":
            await update("thinking", "compacting context");
            break;
        }}
      }}
    }})().catch(() => {{}});
    return () => controller.abort();
  }},
}});
"#,
        berth = berth
    )
}

fn install_opencode_plugin(berth: &str) -> Result<PathBuf, String> {
    let path = home_dir()?.join(".config/opencode/plugins/berth.ts");
    if path.exists() {
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if !existing.contains("Installed by `berth hooks install opencode`") {
            let backup = path.with_extension("ts.bak");
            std::fs::write(&backup, existing).map_err(|e| format!("{}: {e}", backup.display()))?;
        }
    }
    write_atomic(&path, &opencode_plugin(berth))?;
    Ok(path)
}

fn hooks_install(target: &str) -> Result<(), String> {
    let berth = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .display()
        .to_string();
    let claude = target == "claude" || target == "all";
    let opencode = target == "opencode" || target == "all";
    if !claude && !opencode {
        return Err("hooks install: target must be claude, opencode, or all".into());
    }
    if claude {
        println!("claude: {}", install_claude_hooks(&berth)?.display());
    }
    if opencode {
        println!("opencode: {}", install_opencode_plugin(&berth)?.display());
    }
    Ok(())
}

fn hooks_print(target: &str) -> Result<(), String> {
    let berth = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .display()
        .to_string();
    match target {
        "claude" | "all" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&claude_hook_settings(&berth)).unwrap()
            );
        }
        _ => {}
    }
    if target == "opencode" || target == "all" {
        if target == "all" {
            println!();
        }
        println!("{}", opencode_plugin(&berth));
    }
    Ok(())
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
  berth agent-set AGENT_ID [--state S] [--detail D] [--progress 0.4] [--session-ref ID]
  berth agent-set --surface SURFACE_ID [--state S] [--detail D]
  berth agent-ack AGENT_ID       # clear attention + acknowledge notifications
  berth hooks install [claude|opencode|all]
  berth hooks print [claude|opencode|all]
  berth hook claude             read a Claude Code hook payload on stdin
  berth run [--name N] [--provider P] [--cwd DIR] -- <argv…>
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

    if matches!(cmd.as_str(), "help" | "-h" | "--help") {
        print!("{USAGE}");
        return Ok(());
    }
    if cmd == "hooks" {
        let sub = rest.first().map(String::as_str).unwrap_or("print");
        let target = rest.get(1).map(String::as_str).unwrap_or("all");
        return match sub {
            "install" => hooks_install(target),
            "print" => hooks_print(target),
            other => Err(format!("hooks: unknown subcommand {other}")),
        };
    }

    if cmd == "schema" {
        // Generated from the core's Rust types; no running app required.
        println!(
            "{}",
            serde_json::to_string_pretty(&berth_core::schema::document()).unwrap()
        );
        return Ok(());
    }

    let mut c = Client::connect()?;

    match cmd.as_str() {
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
            let id = v["workspace"]["id"]
                .as_str()
                .ok_or("workspace.create returned no workspace id")?;
            println!("{id}");
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
            let new_id = v["surface_id"]
                .as_str()
                .ok_or("surface.split returned no surface id")?;
            println!("{new_id}");
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
            if let Some(id) = arg_after(rest, "--id") {
                params["agent_id"] = json!(id);
            }
            let v = c.call("agent.register", params)?;
            println!("{}", v["agent"]["id"].as_str().unwrap_or("?"));
        }

        "agent-set" => {
            let pos = positional(rest);
            let mut aid = pos.first().cloned();
            if aid.is_none() {
                if let Some(surface) = arg_after(rest, "--surface") {
                    aid = Some(agent_for_surface(&mut c, &surface)?);
                }
            }
            if aid.is_none() {
                aid = std::env::var("BERTH_AGENT_ID")
                    .ok()
                    .filter(|s| !s.is_empty());
            }
            let aid = aid.ok_or("agent-set: missing AGENT_ID or --surface")?;
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
            if let Some(r) = arg_after(rest, "--session-ref") {
                params["session_ref"] = json!(r);
            }
            if rest.iter().any(|a| a == "--clear-progress") {
                params["clear_progress"] = json!(true);
            }
            c.call("agent.update", params)?;
        }

        "agent-ack" => {
            let pos = positional(rest);
            let aid = pos.first().ok_or("agent-ack: missing AGENT_ID")?;
            c.call("agent.ack", json!({"agent_id": aid}))?;
        }

        "hook" => {
            let provider = rest.first().map(String::as_str).unwrap_or_default();
            match provider {
                "claude" => run_claude_hook(&mut c)?,
                other => return Err(format!("hook: unknown provider {other}")),
            }
        }

        "run" => {
            let sep = rest
                .iter()
                .position(|a| a == "--")
                .ok_or("run: expected `-- <argv…>`")?;
            let flags = &rest[..sep];
            let argv = &rest[sep + 1..];
            if argv.is_empty() {
                return Err("run: missing command after --".into());
            }
            let program = resolve_program(&argv[0])
                .ok_or_else(|| format!("run: {} not found on PATH", argv[0]))?;
            let mut argv = argv.to_vec();
            argv[0] = program;
            let cwd = arg_after(flags, "--cwd").or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| p.display().to_string())
            });
            let provider = arg_after(flags, "--provider").unwrap_or_else(|| provider_for(&argv));
            let name = arg_after(flags, "--name")
                .unwrap_or_else(|| argv[0].rsplit('/').next().unwrap_or("agent").to_string());

            let listed = c.call("workspace.list", json!({}))?;
            if listed["workspaces"].as_array().is_none_or(|w| w.is_empty()) {
                c.call("workspace.create", json!({"name": "agents"}))?;
            }

            let id = agent_id();
            let mut env = vec![vec!["BERTH_AGENT_ID".to_string(), id.clone()]];
            if let Ok(path) = std::env::var("PATH") {
                env.push(vec!["PATH".to_string(), path]);
            }
            let created = c.call(
                "surface.create",
                json!({
                    "argv": argv,
                    "cwd": cwd.clone(),
                    "env": env,
                    "focus": true,
                }),
            )?;
            let surface_id = created["surface_id"]
                .as_str()
                .ok_or("surface.create returned no surface id")?;
            c.call(
                "agent.register",
                json!({
                    "surface_id": surface_id,
                    "agent_id": id,
                    "provider": provider,
                    "name": name,
                    "resume": {"argv": argv, "cwd": cwd, "env": env, "session_ref": null},
                }),
            )?;
            println!("{surface_id}");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_hooks_map_to_agent_states() {
        assert_eq!(
            claude_hook_state("SessionStart", ""),
            Some(("idle", "session started"))
        );
        assert_eq!(
            claude_hook_state("UserPromptSubmit", ""),
            Some(("thinking", "working"))
        );
        assert_eq!(
            claude_hook_state("PreToolUse", "Bash"),
            Some(("tool", "Bash"))
        );
        assert_eq!(
            claude_hook_state("Notification", "Claude needs your permission to use Bash"),
            Some(("blocked", "Claude needs your permission to use Bash"))
        );
        assert_eq!(
            claude_hook_state("Notification", "Claude is waiting for your input"),
            Some(("waiting_input", "Claude is waiting for your input"))
        );
        assert_eq!(claude_hook_state("UnknownEvent", ""), None);
    }

    #[test]
    fn claude_settings_cover_every_hook_event() {
        let settings = claude_hook_settings("/opt/berth");
        for event in CLAUDE_HOOK_EVENTS {
            assert!(
                settings["hooks"][event].is_array(),
                "missing hook for {event}"
            );
        }
        let command = settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(command.contains("hook claude"));
        assert!(!command.contains('\''));
    }

    #[test]
    fn opencode_plugin_uses_the_installed_berth_path() {
        let plugin = opencode_plugin("/opt/berth");
        assert!(plugin.contains("const BERTH = \"/opt/berth\""));
        assert!(plugin.contains("BERTH_AGENT_ID"));
        assert!(plugin.contains("permission.updated"));
    }

    #[test]
    fn opencode_plugin_is_a_v2_definition() {
        // OpenCode V2 rejects a bare handler export with
        // "Expected object at [\"default\"]". Guard the exact schema it asks
        // for: a Plugin.define object with an id and a setup/effect function.
        let plugin = opencode_plugin("/opt/berth");
        assert!(plugin.contains("import { Plugin } from \"@opencode/plugin\""));
        assert!(plugin.contains("export default Plugin.define({"));
        assert!(plugin.contains("id: \"berth\""));
        assert!(plugin.contains("async setup(ctx)"));
        assert!(plugin.contains("ctx.event.subscribe"));
        assert!(plugin.contains("return () => controller.abort()"));
        assert!(!plugin.contains("export default BerthPlugin"));
        assert!(!plugin.contains("event: async ({ event })"));
    }
}
