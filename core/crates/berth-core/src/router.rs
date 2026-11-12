//! The request router: a pure function over `AppState` + a [`SurfaceOps`]
//! implementation. No socket, no threads, no AppKit — which is why it is
//! fuzzable and testable on any platform.

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::agent::AgentState;
use crate::model::{NotifLevel, Orientation, Pane, Resume, Surface};
use crate::protocol::{
    APP_NAME, APP_VERSION, ErrorCode, PROTOCOL_VERSION, Request, Response, names,
};
use crate::providers;
use crate::state::{AgentRegistration, AppState};
use crate::{CoreError, ReadScope, SpawnRequest, SurfaceOps};

/// Handle one request. Emits events into `state.pending`; the caller is
/// responsible for draining and broadcasting them.
pub fn dispatch(state: &mut AppState, ops: &dyn SurfaceOps, req: &Request) -> Response {
    match handle(state, ops, &req.method, &req.params) {
        Ok(v) => Response::ok(req.id.clone(), v),
        Err(e) => Response::err(req.id.clone(), e),
    }
}

fn params<T: DeserializeOwned>(v: &Value) -> Result<T, CoreError> {
    let v = if v.is_null() {
        Value::Object(Default::default())
    } else {
        v.clone()
    };
    serde_json::from_value(v).map_err(|e| CoreError::new(ErrorCode::BadRequest, e.to_string()))
}

fn handle(
    state: &mut AppState,
    ops: &dyn SurfaceOps,
    method: &str,
    raw: &Value,
) -> Result<Value, CoreError> {
    match method {
        // ---- system ----------------------------------------------------
        "system.hello" => {
            let instance = state.instance();
            Ok(json!({
                "protocol": PROTOCOL_VERSION,
                "app": APP_NAME,
                "version": APP_VERSION,
                "instance": instance,
                "auth": "local",
            }))
        }
        "system.ping" => {
            let _ = state.instance();
            Ok(json!({"pong": true, "uptime_ms": state.uptime_ms()}))
        }
        "system.capabilities" => Ok(json!({
            "protocol": PROTOCOL_VERSION,
            "methods": METHODS,
            "events": names::ALL,
            "features": ["agents", "notifications", "events", "session"],
        })),

        // ---- workspace -------------------------------------------------
        "workspace.list" => Ok(json!({
            "workspaces": state.workspaces,
            "groups": state.groups,
            "selected_workspace": state.selected_workspace,
            "selected_surface": state.selected_surface,
        })),
        "workspace.create" => {
            let p: WorkspaceCreate = params(raw)?;
            let ws =
                state.create_workspace(p.name.as_deref(), p.cwd.as_deref(), p.group_id, ops)?;
            if p.focus.unwrap_or(true) {
                state.select_workspace(&ws.id)?;
                if let Some(s) = state.active_surface_of_workspace(&ws.id) {
                    let _ = ops.focus(&s);
                }
            }
            Ok(json!({"workspace": ws}))
        }
        "workspace.get" => {
            let p: IdParam = params(raw)?;
            let ws = state
                .workspace(&p.workspace_id)
                .cloned()
                .ok_or_else(|| CoreError::not_found(format!("no workspace {}", p.workspace_id)))?;
            Ok(json!({"workspace": ws}))
        }
        "workspace.layout" => {
            let p: IdParam = params(raw)?;
            let snap = state
                .snapshot()
                .workspaces
                .into_iter()
                .find(|w| w.workspace.id == p.workspace_id)
                .ok_or_else(|| CoreError::not_found(format!("no workspace {}", p.workspace_id)))?;
            // `Pane::active` is an index into `Pane::surfaces`. That is an
            // implementation detail, not something a UI should have to mirror
            // (the first Swift port read it as an id and rendered gray panes),
            // so each pane also carries the resolved id.
            let panes: Vec<Value> = snap
                .panes
                .iter()
                .map(|pane| {
                    let mut value = serde_json::to_value(pane).unwrap_or(Value::Null);
                    if let (Some(id), Some(object)) = (pane.active_surface(), value.as_object_mut())
                    {
                        object.insert("active_surface".into(), json!(id));
                    }
                    value
                })
                .collect();
            Ok(json!({
                "workspace": snap.workspace,
                "panes": panes,
                "surfaces": snap.surfaces,
                "agents": snap.agents,
            }))
        }
        "workspace.select" => {
            let p: IdParam = params(raw)?;
            state.select_workspace(&p.workspace_id)?;
            if let Some(s) = state
                .workspace(&p.workspace_id)
                .and_then(|w| w.active_surface.clone())
            {
                let _ = ops.focus(&s);
            }
            Ok(json!({}))
        }
        "workspace.update" => {
            let p: WorkspaceUpdate = params(raw)?;
            let ws = state
                .workspace_mut(&p.workspace_id)
                .ok_or_else(|| CoreError::not_found(format!("no workspace {}", p.workspace_id)))?;
            if let Some(n) = p.name {
                ws.name = n;
            }
            if p.icon.is_some() {
                ws.icon = p.icon;
            }
            if p.color.is_some() {
                ws.color = p.color;
            }
            if p.clear_group_id.unwrap_or(false) {
                ws.group_id = None;
            } else if p.group_id.is_some() {
                ws.group_id = p.group_id;
            }
            if let Some(pinned) = p.pinned {
                ws.pinned = pinned;
            }
            let snapshot = ws.clone();
            state.emit(names::WORKSPACE_CHANGED, json!(snapshot));
            Ok(json!({"workspace": snapshot}))
        }
        "workspace.close" => {
            let p: IdParam = params(raw)?;
            close_workspace(state, ops, &p.workspace_id)?;
            Ok(json!({}))
        }
        "workspace.move" => {
            let p: WorkspaceMove = params(raw)?;
            move_workspace(state, &p.workspace_id, p.index)?;
            state.emit(
                names::WORKSPACE_MOVED,
                json!({"workspace_id": p.workspace_id, "index": p.index}),
            );
            Ok(json!({}))
        }

        // ---- group -----------------------------------------------------
        "group.list" => Ok(json!({"groups": state.groups})),
        "group.create" => {
            let p: GroupCreate = params(raw)?;
            let group = state.create_group(&p.name, p.color);
            Ok(json!({"group": group}))
        }
        "group.update" => {
            let p: GroupUpdate = params(raw)?;
            let color = if p.clear_color.unwrap_or(false) {
                Some(None)
            } else {
                p.color.map(Some)
            };
            let group = state.update_group(&p.group_id, p.name, color, p.collapsed)?;
            Ok(json!({"group": group}))
        }
        "group.delete" => {
            let p: GroupIdParam = params(raw)?;
            state.delete_group(&p.group_id)?;
            Ok(json!({}))
        }

        // ---- pane ------------------------------------------------------
        "pane.list" => {
            let p: OptionalWorkspace = params(raw)?;
            let panes: Vec<&Pane> = state
                .panes
                .values()
                .filter(|pane| {
                    p.workspace_id
                        .as_deref()
                        .map(|w| pane.workspace_id == w)
                        .unwrap_or(true)
                })
                .collect();
            Ok(json!({"panes": panes}))
        }
        "pane.focus" => {
            let p: PaneIdParam = params(raw)?;
            let pane = state
                .panes
                .get(&p.pane_id)
                .cloned()
                .ok_or_else(|| CoreError::not_found(format!("no pane {}", p.pane_id)))?;
            if let Some(s) = pane.active_surface() {
                let s = s.to_string();
                state.activate_surface(&s, ops)?;
            }
            Ok(json!({}))
        }
        "pane.resize" => {
            let p: PaneResize = params(raw)?;
            let pane = state
                .panes
                .get(&p.pane_id)
                .cloned()
                .ok_or_else(|| CoreError::not_found(format!("no pane {}", p.pane_id)))?;
            if !state_set_weight(state, &pane, p.weight) {
                return Err(CoreError::not_found("pane not in tree"));
            }
            let snapshot = state.workspace(&pane.workspace_id).cloned();
            state.emit(names::WORKSPACE_CHANGED, json!(snapshot));
            Ok(json!({}))
        }
        "split.resize" => {
            let p: SplitResize = params(raw)?;
            let mut changed = None;
            for ws in &mut state.workspaces {
                if ws.root.set_child_weight(&p.split_id, p.index, p.weight) {
                    changed = Some(ws.clone());
                    break;
                }
            }
            let Some(ws) = changed else {
                return Err(CoreError::not_found(format!("no split {}", p.split_id)));
            };
            state.emit(names::WORKSPACE_CHANGED, json!(ws));
            Ok(json!({}))
        }
        "pane.close" => {
            let p: PaneIdParam = params(raw)?;
            let pane = state
                .panes
                .get(&p.pane_id)
                .cloned()
                .ok_or_else(|| CoreError::not_found(format!("no pane {}", p.pane_id)))?;
            for surface_id in pane.surfaces {
                state.close_surface(&surface_id, ops)?;
            }
            Ok(json!({}))
        }

        // ---- surface ---------------------------------------------------
        "surface.list" => {
            let p: SurfaceList = params(raw)?;
            let surfaces: Vec<&Surface> = state
                .surfaces
                .values()
                .filter(|s| {
                    p.workspace_id
                        .as_deref()
                        .is_none_or(|w| s.workspace_id == w)
                })
                .filter(|s| p.pane_id.as_deref().is_none_or(|pid| s.pane_id == pid))
                .collect();
            Ok(json!({"surfaces": surfaces}))
        }
        "surface.create" => {
            let p: SurfaceCreate = params(raw)?;
            let pane_id = match p.pane_id {
                Some(pid) => pid,
                None => default_pane(state).ok_or_else(|| {
                    CoreError::new(ErrorCode::InvalidState, "no workspace is open")
                })?,
            };
            let surface = state.add_surface(
                &pane_id,
                SpawnRequest {
                    cwd: p.cwd,
                    argv: p.argv.unwrap_or_default(),
                    env: p.env.unwrap_or_default(),
                    cols: 80,
                    rows: 24,
                },
                ops,
                p.focus.unwrap_or(true),
            )?;
            Ok(json!({"surface_id": surface.id}))
        }
        "surface.split" => {
            let p: SurfaceSplit = params(raw)?;
            let surface = state
                .surface(&p.surface_id)
                .cloned()
                .ok_or_else(|| CoreError::not_found(format!("no surface {}", p.surface_id)))?;
            let (orientation, before) = match p.direction.as_str() {
                "right" | "horizontal" => (Orientation::Horizontal, false),
                "left" => (Orientation::Horizontal, true),
                "down" | "vertical" => (Orientation::Vertical, false),
                "up" => (Orientation::Vertical, true),
                other => {
                    return Err(CoreError::new(
                        ErrorCode::BadRequest,
                        format!("unknown direction {other}"),
                    ));
                }
            };
            let cwd = p.cwd.or(Some(surface.cwd.clone()));
            let (s, pane) = state.split_surface(
                &p.surface_id,
                orientation,
                before,
                SpawnRequest {
                    cwd,
                    argv: p.argv.unwrap_or_default(),
                    env: vec![],
                    cols: 80,
                    rows: 24,
                },
                p.focus.unwrap_or(true),
                ops,
            )?;
            Ok(json!({"surface_id": s.id, "pane_id": pane.id}))
        }
        "surface.focus" => {
            let p: SurfaceIdParam = params(raw)?;
            let s = state.activate_surface(&p.surface_id, ops)?;
            Ok(json!({"surface_id": s.id}))
        }
        "surface.activate" => {
            let p: SurfaceIdParam = params(raw)?;
            let s = state.activate_surface(&p.surface_id, ops)?;
            Ok(json!({"surface_id": s.id}))
        }
        "surface.cycle" => {
            let p: SurfaceCycle = params(raw)?;
            let delta = match p.direction.as_str() {
                "next" => 1,
                "prev" | "previous" => -1,
                other => {
                    return Err(CoreError::new(
                        ErrorCode::BadRequest,
                        format!("unknown direction {other}"),
                    ));
                }
            };
            let s = state.cycle_surface(&p.surface_id, delta, ops)?;
            Ok(json!({"surface_id": s.id}))
        }
        "surface.close" => {
            let p: SurfaceIdParam = params(raw)?;
            state.close_surface(&p.surface_id, ops)?;
            Ok(json!({}))
        }
        "surface.update" => {
            let p: SurfaceUpdate = params(raw)?;
            let mut title = None;
            let mut pwd = None;
            let mut progress = None;
            let mut bells = None;
            let mut bell_workspace = None;
            {
                let s = state
                    .surface_mut(&p.surface_id)
                    .ok_or_else(|| CoreError::not_found(format!("no surface {}", p.surface_id)))?;
                if let Some(t) = p.title {
                    s.title = t.clone();
                    title = Some(t);
                }
                if let Some(c) = p.cwd {
                    s.cwd = c.clone();
                    pwd = Some(c);
                }
                if p.clear_progress.unwrap_or(false) {
                    s.progress = None;
                    progress = Some(None);
                } else if let Some(v) = p.progress {
                    let v = v.clamp(0.0, 1.0) as f32;
                    s.progress = Some(v);
                    progress = Some(Some(v));
                }
                if p.bell.unwrap_or(false) {
                    s.bell_count = s.bell_count.saturating_add(1);
                    bells = Some(s.bell_count);
                    bell_workspace = Some(s.workspace_id.clone());
                }
            }
            let id = p.surface_id;
            if let Some(ws_id) = bell_workspace {
                if state.selected_workspace.as_deref() != Some(ws_id.as_str()) {
                    if let Some(ws) = state.workspace_mut(&ws_id) {
                        ws.unread = ws.unread.saturating_add(1);
                        let snapshot = ws.clone();
                        state.emit(names::WORKSPACE_CHANGED, json!(snapshot));
                    }
                }
            }
            if let Some(t) = title {
                state.emit(
                    names::SURFACE_TITLE_CHANGED,
                    json!({"surface_id": id, "title": t}),
                );
            }
            if let Some(c) = pwd {
                state.emit(
                    names::SURFACE_PWD_CHANGED,
                    json!({"surface_id": id, "cwd": c}),
                );
            }
            if let Some(v) = progress {
                state.emit(
                    names::SURFACE_PROGRESS,
                    json!({"surface_id": id, "progress": v}),
                );
            }
            if let Some(n) = bells {
                state.emit(names::SURFACE_BELL, json!({"surface_id": id, "count": n}));
            }
            if p.exited.unwrap_or(false) {
                state.mark_agent_exited(&id, ops);
                state.emit(names::SURFACE_EXITED, json!({"surface_id": id}));
            }
            Ok(json!({"surface": state.surface(&id)}))
        }
        "surface.send_text" => {
            let p: SendText = params(raw)?;
            state
                .surface(&p.surface_id)
                .ok_or_else(|| CoreError::not_found(format!("no surface {}", p.surface_id)))?;
            ops.send_text(&p.surface_id, &p.text)?;
            Ok(json!({}))
        }
        "surface.send_key" => {
            let p: SendKey = params(raw)?;
            state
                .surface(&p.surface_id)
                .ok_or_else(|| CoreError::not_found(format!("no surface {}", p.surface_id)))?;
            ops.send_key(&p.surface_id, &p.key, &p.mods.unwrap_or_default())?;
            Ok(json!({}))
        }
        "surface.read_text" => {
            let p: ReadText = params(raw)?;
            state
                .surface(&p.surface_id)
                .ok_or_else(|| CoreError::not_found(format!("no surface {}", p.surface_id)))?;
            let scope = match p.scope.as_deref() {
                None | Some("screen") => ReadScope::Screen,
                Some("scrollback") => ReadScope::Scrollback,
                Some(other) => {
                    return Err(CoreError::new(
                        ErrorCode::BadRequest,
                        format!("unknown scope {other}"),
                    ));
                }
            };
            let rb = ops.read_text(&p.surface_id, scope, p.max_bytes.unwrap_or(1 << 20))?;
            Ok(json!({"text": rb.text, "truncated": rb.truncated}))
        }
        "surface.resize" => {
            let p: SurfaceResize = params(raw)?;
            ops.resize(&p.surface_id, p.cols, p.rows)?;
            if let Some(s) = state.surface_mut(&p.surface_id) {
                s.cols = p.cols;
                s.rows = p.rows;
            }
            Ok(json!({}))
        }

        // ---- agent -----------------------------------------------------
        "agent.register" => {
            let p: AgentRegister = params(raw)?;
            let agent = state.register_agent_with_id(
                AgentRegistration {
                    surface_id: &p.surface_id,
                    id: p.agent_id.as_deref(),
                    provider: p.provider.as_deref(),
                    name: p.name.as_deref(),
                    resume: p.resume,
                    meta: p.meta.unwrap_or(Value::Null),
                },
                ops,
            )?;
            Ok(json!({"agent": agent}))
        }
        "agent.detect" => {
            let p: SurfaceIdParam = params(raw)?;
            let agent = state.detect_agent(&p.surface_id, ops)?;
            Ok(json!({"agent": agent}))
        }
        "agent.update" => {
            let p: AgentUpdate = params(raw)?;
            let progress = if p.clear_progress {
                Some(None)
            } else {
                p.progress.map(Some)
            };
            let mut agent =
                state.update_agent(&p.agent_id, p.state, p.detail, progress, p.meta, ops)?;
            if let Some(session_ref) = p.session_ref {
                agent = state.set_agent_session_ref(&p.agent_id, &session_ref)?;
            }
            Ok(json!({"agent": agent}))
        }
        "agent.ack" => {
            let p: AgentIdParam = params(raw)?;
            let agent = state.ack_agent_attention(&p.agent_id)?;
            Ok(json!({"agent": agent}))
        }
        "agent.get" => {
            let p: AgentIdParam = params(raw)?;
            let agent = state
                .agent(&p.agent_id)
                .cloned()
                .ok_or_else(|| CoreError::not_found(format!("no agent {}", p.agent_id)))?;
            Ok(json!({"agent": agent}))
        }
        "agent.list" => {
            let p: AgentList = params(raw)?;
            let agents: Vec<_> = state
                .agents
                .values()
                .filter(|a| {
                    p.workspace_id
                        .as_deref()
                        .is_none_or(|w| a.workspace_id == w)
                })
                .filter(|a| p.state.map(|s| a.state == s).unwrap_or(true))
                .collect();
            Ok(json!({"agents": agents}))
        }
        "agent.clear" => {
            let p: AgentIdParam = params(raw)?;
            let agent = state
                .agents
                .remove(&p.agent_id)
                .ok_or_else(|| CoreError::not_found(format!("no agent {}", p.agent_id)))?;
            if let Some(s) = state.surface_mut(&agent.surface_id) {
                s.agent_id = None;
            }
            state.emit(names::AGENT_DELETED, json!({"agent_id": agent.id}));
            Ok(json!({}))
        }
        "agent.resume" => {
            let p: AgentIdParam = params(raw)?;
            let agent = state
                .agent(&p.agent_id)
                .cloned()
                .ok_or_else(|| CoreError::not_found(format!("no agent {}", p.agent_id)))?;
            let (argv, cwd) = match &agent.resume {
                Some(r) => {
                    let provider = providers::by_id(&agent.provider);
                    let argv = match provider {
                        Some(p) if r.session_ref.is_some() && !p.resume.is_empty() => {
                            providers::resume_argv(p, r.session_ref.as_deref(), &[])
                        }
                        _ => r.argv.clone(),
                    };
                    (argv, r.cwd.clone())
                }
                None => {
                    return Err(CoreError::new(
                        ErrorCode::Unsupported,
                        format!("agent {} has no resume payload", agent.id),
                    ));
                }
            };
            let surface = state.add_surface(
                &{
                    let s = state
                        .surface(&agent.surface_id)
                        .ok_or_else(|| CoreError::not_found("surface for agent"))?;
                    s.pane_id.clone()
                },
                SpawnRequest {
                    cwd: Some(cwd),
                    argv,
                    env: vec![],
                    cols: 80,
                    rows: 24,
                },
                ops,
                true,
            )?;
            let new_agent = state.register_agent(
                &surface.id,
                Some(&agent.provider),
                Some(&agent.name),
                agent.resume.clone(),
                agent.meta.clone(),
                ops,
            )?;
            Ok(json!({"surface_id": surface.id, "agent_id": new_agent.id}))
        }

        // ---- notification ----------------------------------------------
        "notification.create" => {
            let p: NotifCreate = params(raw)?;
            let level = match p.level.as_deref() {
                Some("needs_input") => NotifLevel::NeedsInput,
                Some("error") => NotifLevel::Error,
                None | Some("info") => NotifLevel::Info,
                Some(other) => {
                    return Err(CoreError::new(
                        ErrorCode::BadRequest,
                        format!("unknown level {other}"),
                    ));
                }
            };
            let n = state.add_notification(
                level,
                &p.title,
                p.body.as_deref(),
                p.workspace_id,
                p.surface_id,
                ops,
            );
            Ok(json!({"notification": n}))
        }
        "notification.list" => {
            let p: NotifList = params(raw)?;
            let list: Vec<_> = state
                .notifications
                .iter()
                .filter(|n| !p.unread_only || !n.acked)
                .collect();
            Ok(json!({"notifications": list}))
        }
        "notification.ack" => {
            let p: NotifAck = params(raw)?;
            state.ack_notification(&p.notification_id)?;
            Ok(json!({}))
        }
        "notification.clear" => {
            state.notifications.clear();
            Ok(json!({}))
        }

        // ---- events ----------------------------------------------------
        "events.subscribe" => {
            let p: Subscribe = params(raw)?;
            let types = p
                .types
                .unwrap_or_else(|| names::ALL.iter().map(|s| s.to_string()).collect());
            Ok(json!({"subscribed": types}))
        }
        "events.unsubscribe" => Ok(json!({"subscribed": []})),

        // ---- session ---------------------------------------------------
        "session.dump" => Ok(json!({"session": state.snapshot()})),
        "session.save" => Ok(json!({"path": crate::store::snapshot_path()})),
        "session.restore" => {
            let p: SessionRestore = params(raw)?;
            let restored = crate::store::restore(state, p.session, ops)?;
            Ok(json!({"restored": restored}))
        }

        other => Err(CoreError::new(
            ErrorCode::MethodNotFound,
            format!("unknown method {other}"),
        )),
    }
}

// ---- helpers ----------------------------------------------------------

fn state_set_weight(state: &mut AppState, pane: &Pane, weight: f32) -> bool {
    state
        .workspace_mut(&pane.workspace_id)
        .map(|w| w.root.set_weight(&pane.id, weight))
        .unwrap_or(false)
}

fn default_pane(state: &AppState) -> Option<String> {
    let ws = match &state.selected_workspace {
        Some(id) => state.workspace(id)?,
        None => state.workspaces.first()?,
    };
    ws.active_surface
        .as_deref()
        .and_then(|s| state.surface(s))
        .map(|s| s.pane_id.clone())
        .or_else(|| ws.root.leaves().first().map(|s| s.to_string()))
}

fn close_workspace(state: &mut AppState, ops: &dyn SurfaceOps, id: &str) -> Result<(), CoreError> {
    let ws = state
        .workspace(id)
        .cloned()
        .ok_or_else(|| CoreError::not_found(format!("no workspace {id}")))?;
    let pane_ids: Vec<String> = ws.root.leaves().iter().map(|s| s.to_string()).collect();
    let mut surfaces: Vec<String> = Vec::new();
    for pid in pane_ids {
        if let Some(p) = state.panes.get(&pid) {
            surfaces.extend(p.surfaces.iter().cloned());
        }
    }
    for s in surfaces {
        let _ = state.close_surface(&s, ops);
    }
    if state.workspace(id).is_some() {
        state.workspaces.retain(|w| w.id != id);
        if state.selected_workspace.as_deref() == Some(id) {
            state.selected_workspace = state.workspaces.first().map(|w| w.id.clone());
        }
        state.emit(names::WORKSPACE_CLOSED, json!({"workspace_id": id}));
    }
    Ok(())
}

fn move_workspace(state: &mut AppState, id: &str, index: usize) -> Result<(), CoreError> {
    let pos = state
        .workspaces
        .iter()
        .position(|w| w.id == id)
        .ok_or_else(|| CoreError::not_found(format!("no workspace {id}")))?;
    let ws = state.workspaces.remove(pos);
    let idx = index.min(state.workspaces.len());
    state.workspaces.insert(idx, ws);
    Ok(())
}

// ---- params -----------------------------------------------------------

#[derive(Debug, Deserialize)]
struct IdParam {
    workspace_id: String,
}
#[derive(Debug, Deserialize)]
struct OptionalWorkspace {
    #[serde(default)]
    workspace_id: Option<String>,
}
#[derive(Debug, Deserialize)]
struct PaneIdParam {
    pane_id: String,
}
#[derive(Debug, Deserialize)]
struct PaneResize {
    pane_id: String,
    weight: f32,
}
#[derive(Debug, Deserialize)]
struct SplitResize {
    split_id: String,
    index: usize,
    weight: f32,
}
#[derive(Debug, Deserialize)]
struct WorkspaceCreate {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    focus: Option<bool>,
}
#[derive(Debug, Deserialize)]
struct GroupCreate {
    name: String,
    #[serde(default)]
    color: Option<String>,
}
#[derive(Debug, Deserialize)]
struct GroupUpdate {
    group_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    clear_color: Option<bool>,
    #[serde(default)]
    collapsed: Option<bool>,
}
#[derive(Debug, Deserialize)]
struct GroupIdParam {
    group_id: String,
}
#[derive(Debug, Deserialize)]
struct WorkspaceUpdate {
    workspace_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    clear_group_id: Option<bool>,
    #[serde(default)]
    pinned: Option<bool>,
}
#[derive(Debug, Deserialize)]
struct WorkspaceMove {
    workspace_id: String,
    index: usize,
}
#[derive(Debug, Deserialize)]
struct SurfaceList {
    #[serde(default)]
    workspace_id: Option<String>,
    #[serde(default)]
    pane_id: Option<String>,
}
#[derive(Debug, Deserialize)]
struct SurfaceCreate {
    #[serde(default)]
    pane_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    argv: Option<Vec<String>>,
    #[serde(default)]
    env: Option<Vec<(String, String)>>,
    #[serde(default)]
    focus: Option<bool>,
}
#[derive(Debug, Deserialize)]
struct SurfaceSplit {
    surface_id: String,
    direction: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    argv: Option<Vec<String>>,
    #[serde(default)]
    focus: Option<bool>,
}
#[derive(Debug, Deserialize)]
struct SurfaceIdParam {
    surface_id: String,
}
#[derive(Debug, Deserialize)]
struct SurfaceCycle {
    surface_id: String,
    direction: String,
}
#[derive(Debug, Deserialize)]
struct SurfaceUpdate {
    surface_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    progress: Option<f64>,
    #[serde(default)]
    clear_progress: Option<bool>,
    #[serde(default)]
    bell: Option<bool>,
    #[serde(default)]
    exited: Option<bool>,
}
#[derive(Debug, Deserialize)]
struct SendText {
    surface_id: String,
    text: String,
}
#[derive(Debug, Deserialize)]
struct SendKey {
    surface_id: String,
    key: String,
    #[serde(default)]
    mods: Option<Vec<String>>,
}
#[derive(Debug, Deserialize)]
struct ReadText {
    surface_id: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    max_bytes: Option<usize>,
}
#[derive(Debug, Deserialize)]
struct SurfaceResize {
    surface_id: String,
    cols: u16,
    rows: u16,
}
#[derive(Debug, Deserialize)]
struct AgentRegister {
    surface_id: String,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    resume: Option<Resume>,
    #[serde(default)]
    meta: Option<Value>,
}
#[derive(Debug, Deserialize)]
struct AgentUpdate {
    agent_id: String,
    #[serde(default)]
    state: Option<AgentState>,
    #[serde(default)]
    detail: Option<String>,
    #[serde(default)]
    progress: Option<f32>,
    #[serde(default)]
    clear_progress: bool,
    #[serde(default)]
    meta: Option<Value>,
    #[serde(default)]
    session_ref: Option<String>,
}
#[derive(Debug, Deserialize)]
struct AgentIdParam {
    agent_id: String,
}
#[derive(Debug, Deserialize)]
struct AgentList {
    #[serde(default)]
    workspace_id: Option<String>,
    #[serde(default)]
    state: Option<AgentState>,
}
#[derive(Debug, Deserialize)]
struct NotifCreate {
    title: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    workspace_id: Option<String>,
    #[serde(default)]
    surface_id: Option<String>,
}
#[derive(Debug, Deserialize)]
struct NotifList {
    #[serde(default)]
    unread_only: bool,
}
#[derive(Debug, Deserialize)]
struct NotifAck {
    notification_id: String,
}
#[derive(Debug, Deserialize)]
struct Subscribe {
    #[serde(default)]
    types: Option<Vec<String>>,
}
#[derive(Debug, Deserialize)]
struct SessionRestore {
    session: crate::store::Snapshot,
}

/// Methods advertised by `system.capabilities`.
pub const METHODS: &[&str] = &[
    "system.hello",
    "system.ping",
    "system.capabilities",
    "workspace.list",
    "workspace.create",
    "workspace.get",
    "workspace.layout",
    "workspace.update",
    "workspace.select",
    "workspace.close",
    "workspace.move",
    "group.list",
    "group.create",
    "group.update",
    "group.delete",
    "pane.list",
    "pane.focus",
    "pane.resize",
    "split.resize",
    "pane.close",
    "surface.list",
    "surface.create",
    "surface.split",
    "surface.focus",
    "surface.activate",
    "surface.cycle",
    "surface.close",
    "surface.update",
    "surface.send_text",
    "surface.send_key",
    "surface.read_text",
    "surface.resize",
    "agent.register",
    "agent.detect",
    "agent.update",
    "agent.get",
    "agent.list",
    "agent.clear",
    "agent.ack",
    "agent.resume",
    "notification.create",
    "notification.list",
    "notification.ack",
    "notification.clear",
    "events.subscribe",
    "events.unsubscribe",
    "session.dump",
    "session.save",
    "session.restore",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NullOps;

    fn req(method: &str, params: Value) -> Request {
        Request {
            id: json!(1),
            method: method.into(),
            params,
        }
    }

    fn call(state: &mut AppState, method: &str, params: Value) -> Value {
        let r = dispatch(state, &NullOps, &req(method, params));
        assert!(r.ok, "{} failed: {:?}", method, r.error);
        r.result.unwrap()
    }

    #[test]
    fn surface_update_reports_osc_state_and_events() {
        let mut st = AppState::default();
        call(&mut st, "workspace.create", json!({"name":"main"}));
        let sid = call(&mut st, "surface.list", json!({}))["surfaces"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        st.drain_events();

        let v = call(
            &mut st,
            "surface.update",
            json!({
                "surface_id": sid,
                "title": "vim README.md",
                "cwd": "/tmp/project",
                "progress": 0.42,
                "bell": true,
            }),
        );
        assert_eq!(v["surface"]["title"], "vim README.md");
        assert_eq!(v["surface"]["cwd"], "/tmp/project");
        assert_eq!(v["surface"]["bell_count"], 1);

        let events: Vec<String> = st.drain_events().into_iter().map(|e| e.event).collect();
        assert!(events.contains(&names::SURFACE_TITLE_CHANGED.to_string()));
        assert!(events.contains(&names::SURFACE_PWD_CHANGED.to_string()));
        assert!(events.contains(&names::SURFACE_PROGRESS.to_string()));
        assert!(events.contains(&names::SURFACE_BELL.to_string()));

        // Clearing is explicit so 0% is not confused with "no progress".
        call(
            &mut st,
            "surface.update",
            json!({"surface_id": sid, "clear_progress": true}),
        );
        assert!(call(&mut st, "surface.list", json!({}))["surfaces"][0]["progress"].is_null());
    }

    #[test]
    fn workspace_layout_describes_tree_and_surfaces() {
        let mut st = AppState::default();
        let ws = call(&mut st, "workspace.create", json!({"name":"main"}));
        let wsid = ws["workspace"]["id"].as_str().unwrap().to_string();
        let sid = call(&mut st, "surface.list", json!({}))["surfaces"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        call(
            &mut st,
            "surface.split",
            json!({"surface_id": sid, "direction": "right"}),
        );

        let layout = call(&mut st, "workspace.layout", json!({"workspace_id": wsid}));
        assert_eq!(layout["workspace"]["root"]["type"], "split");
        assert_eq!(layout["panes"].as_array().unwrap().len(), 2);
        for pane in layout["panes"].as_array().unwrap() {
            let active = pane["active_surface"].as_str().expect("active_surface id");
            let surfaces = pane["surfaces"].as_array().unwrap();
            assert!(surfaces.iter().any(|s| s.as_str() == Some(active)));
        }
        assert_eq!(layout["surfaces"].as_array().unwrap().len(), 2);
        assert!(layout["workspace"]["active_surface"].is_string());
    }

    #[test]
    fn tabs_activate_cycle_and_keep_workspace_pointer_in_sync() {
        let mut st = AppState::default();
        let ws = call(&mut st, "workspace.create", json!({"name":"main"}));
        let wsid = ws["workspace"]["id"].as_str().unwrap().to_string();
        let layout = call(&mut st, "workspace.layout", json!({"workspace_id": wsid}));
        let pane_id = layout["panes"][0]["id"].as_str().unwrap().to_string();
        let first = layout["panes"][0]["active_surface"]
            .as_str()
            .unwrap()
            .to_string();
        let second = call(
            &mut st,
            "surface.create",
            json!({"pane_id": pane_id, "focus": true}),
        )["surface_id"]
            .as_str()
            .unwrap()
            .to_string();
        st.drain_events();

        // Cycling from the newest tab wraps back to the first.
        let cycled = call(
            &mut st,
            "surface.cycle",
            json!({"surface_id": second, "direction": "next"}),
        );
        assert_eq!(cycled["surface_id"], first);
        let list = call(&mut st, "workspace.list", json!({}));
        assert_eq!(list["selected_surface"], first);
        assert_eq!(list["workspaces"][0]["active_surface"], first);
        let events: Vec<String> = st.drain_events().into_iter().map(|e| e.event).collect();
        assert!(events.contains(&names::PANE_FOCUS_CHANGED.to_string()));
        assert!(events.contains(&names::WORKSPACE_CHANGED.to_string()));

        // Closing the visible tab promotes the surviving one.
        call(&mut st, "surface.close", json!({"surface_id": first}));
        let list = call(&mut st, "workspace.list", json!({}));
        assert_eq!(list["workspaces"][0]["active_surface"], second);
        assert_eq!(list["selected_surface"], second);
        let layout = call(&mut st, "workspace.layout", json!({"workspace_id": wsid}));
        assert_eq!(layout["panes"].as_array().unwrap().len(), 1);
        assert_eq!(layout["panes"][0]["surfaces"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn bells_in_background_workspaces_increment_unread() {
        let mut st = AppState::default();
        let first = call(&mut st, "workspace.create", json!({"name":"one"}));
        let first_ws = first["workspace"]["id"].as_str().unwrap().to_string();
        let first_sid = call(&mut st, "surface.list", json!({}))["surfaces"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let second = call(&mut st, "workspace.create", json!({"name":"two"}));
        assert_eq!(
            second["workspace"]["id"],
            call(&mut st, "workspace.list", json!({}))["selected_workspace"]
        );

        call(
            &mut st,
            "surface.update",
            json!({"surface_id": first_sid, "bell": true}),
        );
        let list = call(&mut st, "workspace.list", json!({}));
        let one = list["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["id"] == first_ws)
            .unwrap();
        assert_eq!(one["unread"], 1);

        // Selecting the workspace clears the dot.
        call(
            &mut st,
            "workspace.select",
            json!({"workspace_id": first_ws}),
        );
        let list = call(&mut st, "workspace.list", json!({}));
        assert_eq!(list["workspaces"][0]["unread"], 0);
    }

    #[test]
    fn groups_can_be_created_and_deleted() {
        let mut st = AppState::default();
        let ws = call(&mut st, "workspace.create", json!({"name":"main"}));
        let wsid = ws["workspace"]["id"].as_str().unwrap().to_string();
        let group = call(
            &mut st,
            "group.create",
            json!({"name":"work", "color":"blue"}),
        );
        let gid = group["group"]["id"].as_str().unwrap().to_string();
        call(
            &mut st,
            "workspace.update",
            json!({"workspace_id": wsid, "group_id": gid}),
        );
        call(
            &mut st,
            "group.update",
            json!({"group_id": gid, "name":"client", "collapsed":true}),
        );
        let list = call(&mut st, "workspace.list", json!({}));
        assert_eq!(list["groups"][0]["name"], "client");
        assert_eq!(list["groups"][0]["collapsed"], true);
        assert_eq!(list["workspaces"][0]["group_id"], gid);

        call(&mut st, "group.delete", json!({"group_id": gid}));
        let list = call(&mut st, "workspace.list", json!({}));
        assert!(list["groups"].as_array().unwrap().is_empty());
        assert!(list["workspaces"][0]["group_id"].is_null());
    }

    #[test]
    fn workspace_create_returns_default_pane_and_surface() {
        let mut st = AppState::default();
        let v = call(
            &mut st,
            "workspace.create",
            json!({"name":"eng","cwd":"/tmp"}),
        );
        let ws = &v["workspace"];
        assert_eq!(ws["name"], "eng");
        assert_eq!(ws["root"]["type"], "leaf");
        let surfaces = call(&mut st, "surface.list", json!({}));
        assert_eq!(surfaces["surfaces"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn split_creates_nested_tree_and_events() {
        let mut st = AppState::default();
        call(&mut st, "workspace.create", json!({}));
        let sid = call(&mut st, "surface.list", json!({}))["surfaces"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let split = call(
            &mut st,
            "surface.split",
            json!({"surface_id": sid, "direction":"right"}),
        );
        assert!(split["surface_id"].is_string());
        let events = st.drain_events();
        assert!(events.iter().any(|e| e.event == names::SURFACE_CREATED));
        assert!(events.windows(2).all(|w| w[1].seq == w[0].seq + 1));
    }

    #[test]
    fn closing_last_surface_closes_workspace() {
        let mut st = AppState::default();
        call(&mut st, "workspace.create", json!({}));
        let sid = call(&mut st, "surface.list", json!({}))["surfaces"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        call(&mut st, "surface.close", json!({"surface_id": sid}));
        assert!(st.workspaces.is_empty());
    }

    #[test]
    fn agent_lifecycle_and_illegal_transition() {
        let mut st = AppState::default();
        call(&mut st, "workspace.create", json!({}));
        let sid = call(&mut st, "surface.list", json!({}))["surfaces"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let agent = call(
            &mut st,
            "agent.register",
            json!({"surface_id": sid, "provider":"claude-code", "name":"fix-auth"}),
        )["agent"]
            .clone();
        let aid = agent["id"].as_str().unwrap().to_string();

        call(
            &mut st,
            "agent.update",
            json!({"agent_id": aid, "state":"thinking"}),
        );
        call(
            &mut st,
            "agent.update",
            json!({"agent_id": aid, "state":"tool", "detail":"pnpm test"}),
        );
        let v = call(
            &mut st,
            "agent.update",
            json!({"agent_id": aid, "state":"waiting_input"}),
        );
        assert_eq!(v["agent"]["attention"], "needs_input");

        // blocked -> tool is not a legal transition
        call(
            &mut st,
            "agent.update",
            json!({"agent_id": aid, "state":"blocked"}),
        );
        let r = dispatch(
            &mut st,
            &NullOps,
            &req("agent.update", json!({"agent_id": aid, "state":"tool"})),
        );
        assert!(!r.ok);
        assert_eq!(r.error.unwrap().code, ErrorCode::InvalidState);
    }

    #[test]
    fn agent_update_records_a_session_ref_for_resume() {
        let mut st = AppState::default();
        call(&mut st, "workspace.create", json!({}));
        let sid = call(&mut st, "surface.list", json!({}))["surfaces"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let aid = call(
            &mut st,
            "agent.register",
            json!({
                "surface_id": sid,
                "provider": "claude-code",
                "resume": {"argv":["/bin/claude"], "cwd":"/tmp", "session_ref": null}
            }),
        )["agent"]["id"]
            .as_str()
            .unwrap()
            .to_string();

        let updated = call(
            &mut st,
            "agent.update",
            json!({"agent_id": aid, "state":"thinking", "session_ref":"sess-123"}),
        );
        assert_eq!(updated["agent"]["resume"]["session_ref"], "sess-123");
        assert_eq!(updated["agent"]["resume"]["argv"][0], "/bin/claude");
    }

    #[test]
    fn attention_creates_exactly_one_notification() {
        let mut st = AppState::default();
        call(&mut st, "workspace.create", json!({}));
        let sid = call(&mut st, "surface.list", json!({}))["surfaces"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let aid = call(&mut st, "agent.register", json!({"surface_id": sid}))["agent"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        call(
            &mut st,
            "agent.update",
            json!({"agent_id": aid, "state":"waiting_input"}),
        );
        call(
            &mut st,
            "agent.update",
            json!({"agent_id": aid, "state":"thinking"}),
        );
        call(
            &mut st,
            "agent.update",
            json!({"agent_id": aid, "state":"waiting_input"}),
        );
        let v = call(&mut st, "notification.list", json!({"unread_only": true}));
        assert_eq!(v["notifications"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn process_exit_marks_agent_errored_immediately() {
        let mut st = AppState::default();
        call(&mut st, "workspace.create", json!({}));
        let sid = call(&mut st, "surface.list", json!({}))["surfaces"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let aid = call(
            &mut st,
            "agent.register",
            json!({"surface_id": sid, "provider": "claude-code"}),
        )["agent"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        call(
            &mut st,
            "agent.update",
            json!({"agent_id": aid, "state":"thinking"}),
        );

        call(
            &mut st,
            "surface.update",
            json!({"surface_id": sid, "exited": true}),
        );

        let agent = call(&mut st, "agent.get", json!({"agent_id": aid}))["agent"].clone();
        assert_eq!(agent["state"], "errored");
        assert_eq!(agent["attention"], "error");
        assert_eq!(agent["detail"], "process exited");
        let unread = call(&mut st, "notification.list", json!({"unread_only": true}));
        assert_eq!(unread["notifications"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn acknowledging_attention_clears_it_and_its_notifications() {
        let mut st = AppState::default();
        call(&mut st, "workspace.create", json!({}));
        let sid = call(&mut st, "surface.list", json!({}))["surfaces"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let aid = call(&mut st, "agent.register", json!({"surface_id": sid}))["agent"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        call(
            &mut st,
            "agent.update",
            json!({"agent_id": aid, "state":"waiting_input"}),
        );
        let before = call(&mut st, "agent.get", json!({"agent_id": aid}));
        assert_eq!(before["agent"]["attention"], "needs_input");

        let after = call(&mut st, "agent.ack", json!({"agent_id": aid}));
        assert_eq!(after["agent"]["attention"], "none");
        let unread = call(&mut st, "notification.list", json!({"unread_only": true}));
        assert!(unread["notifications"].as_array().unwrap().is_empty());
    }

    #[test]
    fn stale_agent_escalates_to_blocked() {
        let mut st = AppState::default();
        call(&mut st, "workspace.create", json!({}));
        let sid = call(&mut st, "surface.list", json!({}))["surfaces"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let aid = call(&mut st, "agent.register", json!({"surface_id": sid}))["agent"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        call(
            &mut st,
            "agent.update",
            json!({"agent_id": aid, "state":"thinking"}),
        );
        let now = crate::time::now_ms() + 200_000;
        let changed = st.sweep_stale(now, 180_000, &NullOps);
        assert_eq!(changed.len(), 1);
        assert_eq!(st.agent(&aid).unwrap().state, AgentState::Blocked);
    }

    #[test]
    fn unknown_method_is_reported_not_panicked() {
        let mut st = AppState::default();
        let r = dispatch(&mut st, &NullOps, &req("nope.nope", json!({})));
        assert!(!r.ok);
        assert_eq!(r.error.unwrap().code, ErrorCode::MethodNotFound);
    }

    #[test]
    fn bad_params_are_bad_request() {
        let mut st = AppState::default();
        let r = dispatch(
            &mut st,
            &NullOps,
            &req("workspace.get", json!({"workspace_id": 5})),
        );
        assert!(!r.ok);
        assert_eq!(r.error.unwrap().code, ErrorCode::BadRequest);
    }

    #[test]
    fn surface_ids_are_opaque_and_prefixed() {
        let mut st = AppState::default();
        call(&mut st, "workspace.create", json!({}));
        let v = call(&mut st, "surface.list", json!({}));
        let id = v["surfaces"][0]["id"].as_str().unwrap();
        assert!(id.starts_with("srf_"));
        assert!(crate::ids::looks_valid(id));
    }

    #[test]
    fn directional_splits_insert_on_the_requested_side() {
        let mut st = AppState::default();
        let ws = call(&mut st, "workspace.create", json!({"name":"main"}));
        let wsid = ws["workspace"]["id"].as_str().unwrap().to_string();
        let sid = call(&mut st, "surface.list", json!({}))["surfaces"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();

        call(
            &mut st,
            "surface.split",
            json!({"surface_id": sid, "direction": "left", "focus": false}),
        );
        let layout = call(&mut st, "workspace.layout", json!({"workspace_id": wsid}));
        let children = layout["workspace"]["root"]["children"].as_array().unwrap();
        let first_pane = children[0]["pane_id"].as_str().unwrap();
        let surface_pane = call(&mut st, "surface.list", json!({}))["surfaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"].as_str() == Some(sid.as_str()))
            .and_then(|s| s["pane_id"].as_str())
            .unwrap()
            .to_string();
        assert_ne!(first_pane, surface_pane, "left split must lead the tree");
    }

    #[test]
    fn pane_close_removes_every_tab_in_that_pane() {
        let mut st = AppState::default();
        call(&mut st, "workspace.create", json!({}));
        let sid = call(&mut st, "surface.list", json!({}))["surfaces"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        call(
            &mut st,
            "surface.split",
            json!({"surface_id": sid, "direction": "right", "focus": false}),
        );
        let panes = call(&mut st, "pane.list", json!({}));
        let pane_id = panes["panes"][0]["id"].as_str().unwrap().to_string();
        call(&mut st, "pane.close", json!({"pane_id": pane_id}));
        let after = call(&mut st, "pane.list", json!({}));
        assert_eq!(after["panes"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn session_restore_accepts_a_dumped_snapshot() {
        let mut source = AppState::default();
        call(&mut source, "workspace.create", json!({"name":"saved"}));
        let snapshot = call(&mut source, "session.dump", json!({}))["session"].clone();

        let mut target = AppState::default();
        let restored = call(&mut target, "session.restore", json!({"session": snapshot}));
        assert_eq!(restored["restored"], 1);
        assert_eq!(
            call(&mut target, "workspace.list", json!({}))["workspaces"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    struct ClaudeOps;

    impl SurfaceOps for ClaudeOps {
        fn spawn(&self, req: crate::SpawnRequest) -> Result<crate::Spawned, CoreError> {
            NullOps.spawn(req)
        }
        fn send_text(&self, a: &str, b: &str) -> Result<(), CoreError> {
            NullOps.send_text(a, b)
        }
        fn send_key(&self, a: &str, b: &str, c: &[String]) -> Result<(), CoreError> {
            NullOps.send_key(a, b, c)
        }
        fn read_text(
            &self,
            a: &str,
            b: crate::ReadScope,
            c: usize,
        ) -> Result<crate::Readback, CoreError> {
            NullOps.read_text(a, b, c)
        }
        fn close(&self, a: &str) -> Result<(), CoreError> {
            NullOps.close(a)
        }
        fn focus(&self, a: &str) -> Result<(), CoreError> {
            NullOps.focus(a)
        }
        fn resize(&self, a: &str, b: u16, c: u16) -> Result<(), CoreError> {
            NullOps.resize(a, b, c)
        }
        fn pid_info(&self, _: &str) -> Result<crate::PidInfo, CoreError> {
            Ok(crate::PidInfo {
                pid: Some(4242),
                comm: Some("claude".into()),
                argv: vec!["claude".into()],
            })
        }
        fn notify_os(&self, n: &crate::Notification) -> Result<(), CoreError> {
            NullOps.notify_os(n)
        }
    }

    #[test]
    fn agent_register_accepts_an_explicit_id() {
        let mut st = AppState::default();
        call(&mut st, "workspace.create", json!({}));
        let sid = call(&mut st, "surface.list", json!({}))["surfaces"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let v = call(
            &mut st,
            "agent.register",
            json!({"surface_id": sid, "agent_id": "agt_wrapper", "provider": "custom", "name": "wrapped"}),
        );
        assert_eq!(v["agent"]["id"], "agt_wrapper");
        assert_eq!(v["agent"]["state"], "idle");
    }

    #[test]
    fn agent_detect_is_passive_and_idempotent() {
        let mut st = AppState::default();
        call(&mut st, "workspace.create", json!({}));
        let sid = call(&mut st, "surface.list", json!({}))["surfaces"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let ops = ClaudeOps;
        let first = dispatch(
            &mut st,
            &ops,
            &req("agent.detect", json!({"surface_id": sid})),
        );
        assert!(first.ok);
        let agent = first.result.unwrap()["agent"].clone();
        assert_eq!(agent["provider"], "claude-code");
        assert_eq!(agent["state"], "unknown");

        let again = dispatch(
            &mut st,
            &ops,
            &req("agent.detect", json!({"surface_id": sid})),
        );
        assert!(again.ok);
        assert!(again.result.unwrap()["agent"].is_null());
        assert_eq!(st.agents.len(), 1);
    }
}
