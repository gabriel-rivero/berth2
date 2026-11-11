//! `AppState`: the aggregate the router mutates. No I/O, no threads, no clock
//! except the `now_ms` passed in by callers (which makes tests deterministic).

use std::collections::BTreeMap;

use serde_json::Value;

use crate::agent::{self, AgentState, Attention};
use crate::ids;
use crate::model::{
    AgentSurface, Group, NotifLevel, Notification, Orientation, Pane, PaneNode, Resume, Surface,
    SurfaceKind, Workspace,
};
use crate::protocol::{Event, names};
use crate::time;
use crate::{CoreError, ErrorCode, SpawnRequest, SurfaceOps};

#[derive(Debug, Default)]
pub struct AppState {
    pub workspaces: Vec<Workspace>,
    pub groups: Vec<Group>,
    pub panes: BTreeMap<String, Pane>,
    pub surfaces: BTreeMap<String, Surface>,
    pub agents: BTreeMap<String, AgentSurface>,
    pub notifications: Vec<Notification>,
    pub selected_workspace: Option<String>,
    pub selected_surface: Option<String>,
    /// Minted lazily; identifies this app run in `system.hello`.
    pub instance: String,
    pending: Vec<Event>,
    seq: u64,
}

/// Registration payload shared by the explicit and inferred-id paths.
pub struct AgentRegistration<'a> {
    pub surface_id: &'a str,
    pub id: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub name: Option<&'a str>,
    pub resume: Option<Resume>,
    pub meta: Value,
}

impl AppState {
    pub fn instance(&mut self) -> String {
        if self.instance.is_empty() {
            self.instance = ids::mint("inst");
        }
        self.instance.clone()
    }

    // ---- events -------------------------------------------------------

    pub fn emit(&mut self, name: &str, data: Value) {
        self.pending.push(Event {
            event: name.to_string(),
            seq: 0,
            ts: String::new(),
            data,
        });
    }

    /// Drain queued events, stamping `seq` and `ts`. Cheap enough to call once
    /// per runloop turn.
    pub fn drain_events(&mut self) -> Vec<Event> {
        let now = time::rfc3339(time::now_ms());
        let mut out = std::mem::take(&mut self.pending);
        for e in out.iter_mut() {
            self.seq += 1;
            e.seq = self.seq;
            e.ts = now.clone();
        }
        out
    }

    pub fn last_seq(&self) -> u64 {
        self.seq
    }

    // ---- lookups ------------------------------------------------------

    pub fn workspace(&self, id: &str) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.id == id)
    }

    pub fn workspace_mut(&mut self, id: &str) -> Option<&mut Workspace> {
        self.workspaces.iter_mut().find(|w| w.id == id)
    }

    pub fn surface(&self, id: &str) -> Option<&Surface> {
        self.surfaces.get(id)
    }

    pub fn surface_mut(&mut self, id: &str) -> Option<&mut Surface> {
        self.surfaces.get_mut(id)
    }

    pub fn agent(&self, id: &str) -> Option<&AgentSurface> {
        self.agents.get(id)
    }

    pub fn agent_for_surface(&self, surface_id: &str) -> Option<&AgentSurface> {
        self.agents.values().find(|a| a.surface_id == surface_id)
    }

    pub fn workspace_of_surface(&self, surface_id: &str) -> Option<&str> {
        self.surfaces
            .get(surface_id)
            .map(|s| s.workspace_id.as_str())
    }

    /// The surface a workspace should show: the remembered one when it is
    /// still alive, otherwise the active tab of the first pane.
    pub fn active_surface_of_workspace(&self, workspace_id: &str) -> Option<String> {
        let ws = self.workspace(workspace_id)?;
        if let Some(id) = &ws.active_surface {
            if self.surfaces.contains_key(id) {
                return Some(id.clone());
            }
        }
        ws.root
            .leaves()
            .iter()
            .find_map(|pid| self.panes.get(*pid).and_then(Pane::active_surface))
            .map(str::to_string)
    }

    fn require_surface(&self, id: &str) -> Result<&Surface, CoreError> {
        self.surface(id)
            .ok_or_else(|| CoreError::not_found(format!("no surface {id}")))
    }

    // ---- construction -------------------------------------------------

    pub fn create_workspace(
        &mut self,
        name: Option<&str>,
        cwd: Option<&str>,
        group_id: Option<String>,
        ops: &dyn SurfaceOps,
    ) -> Result<Workspace, CoreError> {
        let now = time::now_ms();
        let ws_id = ids::mint(ids::WORKSPACE);
        let pane_id = ids::mint(ids::PANE);
        let ws_name = name
            .map(str::to_string)
            .unwrap_or_else(|| format!("workspace {}", self.workspaces.len() + 1));

        let mut ws = Workspace {
            id: ws_id.clone(),
            name: ws_name,
            group_id,
            icon: None,
            color: None,
            pinned: false,
            unread: 0,
            created_ms: now,
            root: PaneNode::leaf(pane_id.clone()),
            active_surface: None,
        };

        let surface = self.spawn_into(&ws, &pane_id, cwd, vec![], vec![], ops)?;
        ws.active_surface = Some(surface.id.clone());

        self.workspaces.push(ws);
        self.selected_workspace.get_or_insert(ws_id.clone());
        let snapshot = self.workspace(&ws_id).cloned();
        self.emit(names::WORKSPACE_CREATED, serde_json::json!(snapshot));

        self.workspace(&ws_id)
            .cloned()
            .ok_or_else(|| CoreError::internal("workspace vanished"))
    }

    /// Spawn a terminal surface into an existing pane and register metadata.
    fn spawn_into(
        &mut self,
        ws: &Workspace,
        pane_id: &str,
        cwd: Option<&str>,
        argv: Vec<String>,
        env: Vec<(String, String)>,
        ops: &dyn SurfaceOps,
    ) -> Result<Surface, CoreError> {
        let surface_id = ids::mint(ids::SURFACE);
        let mut req = SpawnRequest {
            cwd: cwd.map(str::to_string),
            argv,
            env,
            cols: 80,
            rows: 24,
        };
        req.env
            .push(("BERTH_SURFACE_ID".into(), surface_id.clone()));
        req.env.push(("BERTH_WORKSPACE_ID".into(), ws.id.clone()));
        req.env.push((
            "BERTH_PROTOCOL".into(),
            crate::protocol::PROTOCOL_VERSION.to_string(),
        ));

        let spawned = ops
            .spawn(req)
            .map_err(|e| CoreError::new(e.code, format!("spawn failed: {}", e.message)))?;

        let surface = Surface {
            id: surface_id.clone(),
            pane_id: pane_id.to_string(),
            workspace_id: ws.id.clone(),
            kind: SurfaceKind::Terminal,
            title: String::new(),
            cwd: cwd.unwrap_or_default().to_string(),
            pid: if spawned.pid == 0 {
                None
            } else {
                Some(spawned.pid)
            },
            tty: if spawned.tty.is_empty() {
                None
            } else {
                Some(spawned.tty)
            },
            cols: spawned.cols,
            rows: spawned.rows,
            progress: None,
            bell_count: 0,
            agent_id: None,
            created_ms: time::now_ms(),
        };

        let pane = self
            .panes
            .entry(pane_id.to_string())
            .or_insert_with(|| Pane {
                id: pane_id.to_string(),
                workspace_id: ws.id.clone(),
                surfaces: Vec::new(),
                active: 0,
            });
        pane.surfaces.push(surface_id.clone());
        pane.active = pane.surfaces.len() - 1;
        self.surfaces.insert(surface_id, surface.clone());
        if self.selected_surface.is_none() {
            self.selected_surface = Some(surface.id.clone());
        }
        self.emit(names::SURFACE_CREATED, serde_json::json!(surface));
        Ok(surface)
    }

    pub fn add_surface(
        &mut self,
        pane_id: &str,
        req: SpawnRequest,
        ops: &dyn SurfaceOps,
        focus: bool,
    ) -> Result<Surface, CoreError> {
        let pane = self
            .panes
            .get(pane_id)
            .cloned()
            .ok_or_else(|| CoreError::not_found(format!("no pane {pane_id}")))?;
        let ws = self
            .workspace(&pane.workspace_id)
            .cloned()
            .ok_or_else(|| CoreError::not_found("workspace for pane"))?;

        let surface = self.spawn_into(&ws, pane_id, req.cwd.as_deref(), req.argv, req.env, ops)?;

        if focus {
            ops.focus(&surface.id)?;
            self.selected_workspace = Some(ws.id.clone());
            self.selected_surface = Some(surface.id.clone());
            self.emit(
                names::PANE_FOCUS_CHANGED,
                serde_json::json!({"pane_id": pane_id, "surface_id": surface.id}),
            );
        }
        if let Some(w) = self.workspace_mut(&ws.id) {
            w.active_surface = Some(surface.id.clone());
        }
        Ok(surface)
    }

    /// Make an existing tab the visible surface for its pane and workspace.
    pub fn activate_surface(
        &mut self,
        surface_id: &str,
        ops: &dyn SurfaceOps,
    ) -> Result<Surface, CoreError> {
        let surface = self.require_surface(surface_id)?.clone();
        if let Some(pane) = self.panes.get_mut(&surface.pane_id) {
            if let Some(idx) = pane.surfaces.iter().position(|s| s == surface_id) {
                pane.active = idx;
            }
        }
        ops.focus(surface_id)?;
        self.selected_workspace = Some(surface.workspace_id.clone());
        self.selected_surface = Some(surface.id.clone());

        let mut changed = None;
        if let Some(w) = self.workspace_mut(&surface.workspace_id) {
            if w.active_surface.as_deref() != Some(surface_id) || w.unread != 0 {
                w.active_surface = Some(surface.id.clone());
                w.unread = 0;
                changed = Some(w.clone());
            }
        }
        self.emit(
            names::PANE_FOCUS_CHANGED,
            serde_json::json!({"pane_id": surface.pane_id, "surface_id": surface.id}),
        );
        if let Some(snapshot) = changed {
            self.emit(names::WORKSPACE_CHANGED, serde_json::json!(snapshot));
        }
        Ok(surface)
    }

    /// Cycle tabs inside the surface's pane. Wraps in both directions.
    pub fn cycle_surface(
        &mut self,
        surface_id: &str,
        delta: i32,
        ops: &dyn SurfaceOps,
    ) -> Result<Surface, CoreError> {
        let surface = self.require_surface(surface_id)?.clone();
        let pane = self
            .panes
            .get(&surface.pane_id)
            .ok_or_else(|| CoreError::not_found("pane for surface"))?;
        if pane.surfaces.len() < 2 {
            return self.activate_surface(surface_id, ops);
        }
        let current = pane
            .surfaces
            .iter()
            .position(|s| s == surface_id)
            .unwrap_or(pane.active) as i32;
        let len = pane.surfaces.len() as i32;
        let next = (current + delta).rem_euclid(len) as usize;
        let target = pane.surfaces[next].clone();
        self.activate_surface(&target, ops)
    }

    pub fn split_surface(
        &mut self,
        surface_id: &str,
        orientation: Orientation,
        before: bool,
        req: SpawnRequest,
        focus: bool,
        ops: &dyn SurfaceOps,
    ) -> Result<(Surface, Pane), CoreError> {
        let surface = self.require_surface(surface_id)?.clone();
        let pane = self
            .panes
            .get(&surface.pane_id)
            .cloned()
            .ok_or_else(|| CoreError::not_found("pane for surface"))?;
        let ws = self
            .workspace(&pane.workspace_id)
            .cloned()
            .ok_or_else(|| CoreError::not_found("workspace for surface"))?;

        let new_pane_id = ids::mint(ids::PANE);
        let mut root = ws.root.clone();
        if !root.split_side(&pane.id, &new_pane_id, orientation, before) {
            return Err(CoreError::new(
                ErrorCode::InvalidState,
                format!("pane {} not in workspace tree", pane.id),
            ));
        }
        if let Some(w) = self.workspace_mut(&ws.id) {
            w.root = root;
        }
        self.panes.insert(
            new_pane_id.clone(),
            Pane {
                id: new_pane_id.clone(),
                workspace_id: ws.id.clone(),
                surfaces: Vec::new(),
                active: 0,
            },
        );

        let new_surface = self.spawn_into(
            &ws,
            &new_pane_id,
            req.cwd.as_deref(),
            req.argv,
            req.env,
            ops,
        )?;

        let new_pane = self
            .panes
            .get(&new_pane_id)
            .cloned()
            .ok_or_else(|| CoreError::internal("pane vanished"))?;

        if focus {
            ops.focus(&new_surface.id)?;
            self.selected_workspace = Some(ws.id.clone());
            self.selected_surface = Some(new_surface.id.clone());
            self.emit(
                names::PANE_FOCUS_CHANGED,
                serde_json::json!({"pane_id": new_pane_id, "surface_id": new_surface.id}),
            );
        }
        if let Some(w) = self.workspace_mut(&ws.id) {
            w.active_surface = Some(new_surface.id.clone());
        }
        let snapshot = self.workspace(&ws.id).cloned();
        self.emit(names::WORKSPACE_CHANGED, serde_json::json!(snapshot));
        Ok((new_surface, new_pane))
    }

    pub fn close_surface(
        &mut self,
        surface_id: &str,
        ops: &dyn SurfaceOps,
    ) -> Result<(), CoreError> {
        let surface = self.require_surface(surface_id)?.clone();
        ops.close(surface_id)?;

        if let Some(agent) = self.agent_for_surface(surface_id).cloned() {
            self.agents.remove(&agent.id);
            self.emit(
                names::AGENT_DELETED,
                serde_json::json!({"agent_id": agent.id}),
            );
        }
        self.surfaces.remove(surface_id);
        self.emit(
            names::SURFACE_CLOSED,
            serde_json::json!({"surface_id": surface_id}),
        );

        let mut pane_empty = false;
        if let Some(pane) = self.panes.get_mut(&surface.pane_id) {
            pane.surfaces.retain(|s| s != surface_id);
            if pane.active >= pane.surfaces.len() {
                pane.active = pane.surfaces.len().saturating_sub(1);
            }
            pane_empty = pane.surfaces.is_empty();
        }

        let ws_id = surface.workspace_id.clone();
        if pane_empty {
            self.panes.remove(&surface.pane_id);
            let mut ws_gone = false;
            if let Some(ws) = self.workspace_mut(&ws_id) {
                let root_was_leaf = matches!(
                    &ws.root,
                    PaneNode::Leaf { pane_id } if pane_id == &surface.pane_id
                );
                ws.root.remove(&surface.pane_id);
                ws_gone = root_was_leaf || ws.root.is_empty();
            }
            if ws_gone {
                self.workspaces.retain(|w| w.id != ws_id);
                if self.selected_workspace.as_deref() == Some(ws_id.as_str()) {
                    self.selected_workspace = self.workspaces.first().map(|w| w.id.clone());
                }
                self.emit(
                    names::WORKSPACE_CLOSED,
                    serde_json::json!({"workspace_id": ws_id}),
                );
            }
        }

        // A surviving workspace must point at a live tab after the close.
        let active = self.active_surface_of_workspace(&ws_id);
        if let Some(ws) = self.workspace_mut(&ws_id) {
            if ws.active_surface != active {
                ws.active_surface = active.clone();
                let snapshot = ws.clone();
                self.emit(names::WORKSPACE_CHANGED, serde_json::json!(snapshot));
            }
        }
        if self.selected_workspace.as_deref() == Some(ws_id.as_str()) {
            if let Some(s) = active {
                self.selected_surface = Some(s);
            }
        }

        if self.selected_surface.as_deref() == Some(surface_id) {
            let next = self
                .selected_workspace
                .as_deref()
                .and_then(|w| self.active_surface_of_workspace(w))
                .or_else(|| self.surfaces.keys().next().cloned());
            self.selected_surface = next;
        }
        Ok(())
    }

    pub fn select_workspace(&mut self, workspace_id: &str) -> Result<(), CoreError> {
        if self.workspace(workspace_id).is_none() {
            return Err(CoreError::not_found(format!("no workspace {workspace_id}")));
        }
        self.selected_workspace = Some(workspace_id.to_string());
        let surface = self.active_surface_of_workspace(workspace_id);
        let mut changed = None;
        if let Some(w) = self.workspace_mut(workspace_id) {
            w.active_surface = surface.clone();
            if w.unread != 0 {
                w.unread = 0;
                changed = Some(w.clone());
            }
        }
        if let Some(s) = surface {
            self.selected_surface = Some(s);
        }
        self.emit(
            names::WORKSPACE_SELECTED,
            serde_json::json!({"workspace_id": workspace_id}),
        );
        if let Some(snapshot) = changed {
            self.emit(names::WORKSPACE_CHANGED, serde_json::json!(snapshot));
        }
        Ok(())
    }

    // ---- groups -------------------------------------------------------

    pub fn create_group(&mut self, name: &str, color: Option<String>) -> Group {
        let group = Group {
            id: ids::mint(ids::GROUP),
            name: name.to_string(),
            color,
            collapsed: false,
        };
        self.groups.push(group.clone());
        self.emit(names::GROUP_CREATED, serde_json::json!(group));
        group
    }

    pub fn update_group(
        &mut self,
        id: &str,
        name: Option<String>,
        color: Option<Option<String>>,
        collapsed: Option<bool>,
    ) -> Result<Group, CoreError> {
        let group = self
            .groups
            .iter_mut()
            .find(|g| g.id == id)
            .ok_or_else(|| CoreError::not_found(format!("no group {id}")))?;
        if let Some(n) = name {
            group.name = n;
        }
        if let Some(c) = color {
            group.color = c;
        }
        if let Some(c) = collapsed {
            group.collapsed = c;
        }
        let snapshot = group.clone();
        self.emit(names::GROUP_UPDATED, serde_json::json!(snapshot));
        Ok(snapshot)
    }

    pub fn delete_group(&mut self, id: &str) -> Result<(), CoreError> {
        if !self.groups.iter().any(|g| g.id == id) {
            return Err(CoreError::not_found(format!("no group {id}")));
        }
        self.groups.retain(|g| g.id != id);
        for ws in self.workspaces.iter_mut() {
            if ws.group_id.as_deref() == Some(id) {
                ws.group_id = None;
            }
        }
        self.emit(names::GROUP_DELETED, serde_json::json!({"group_id": id}));
        Ok(())
    }

    // ---- agents -------------------------------------------------------

    pub fn register_agent(
        &mut self,
        surface_id: &str,
        provider: Option<&str>,
        name: Option<&str>,
        resume: Option<Resume>,
        meta: Value,
        ops: &dyn SurfaceOps,
    ) -> Result<AgentSurface, CoreError> {
        self.register_agent_with_id(
            AgentRegistration {
                surface_id,
                id: None,
                provider,
                name,
                resume,
                meta,
            },
            ops,
        )
    }

    /// Register with an optional caller-chosen id. The CLI wrapper mints the
    /// id before spawning so `BERTH_AGENT_ID` can be present in the child's
    /// environment.
    pub fn register_agent_with_id(
        &mut self,
        reg: AgentRegistration<'_>,
        ops: &dyn SurfaceOps,
    ) -> Result<AgentSurface, CoreError> {
        let AgentRegistration {
            surface_id,
            id: explicit_id,
            provider,
            name,
            resume,
            meta,
        } = reg;
        let surface = self.require_surface(surface_id)?.clone();

        let mut agent = match self.agent_for_surface(surface_id).cloned() {
            Some(mut existing) => {
                if let Some(p) = provider {
                    existing.provider = p.to_string();
                }
                if let Some(n) = name {
                    existing.name = n.to_string();
                }
                if resume.is_some() {
                    existing.resume = resume;
                }
                existing.last_seen_ms = time::now_ms();
                existing
            }
            None => {
                let detected = ops
                    .pid_info(surface_id)
                    .ok()
                    .and_then(|info| crate::providers::detect(info.comm.as_deref(), &info.argv));
                let provider = provider
                    .map(str::to_string)
                    .or_else(|| detected.map(|p| p.id.to_string()))
                    .unwrap_or_else(|| "custom".to_string());
                let id = explicit_id
                    .map(str::to_string)
                    .unwrap_or_else(|| ids::mint(ids::AGENT));
                let now = time::now_ms();
                AgentSurface {
                    id: id.clone(),
                    surface_id: surface_id.to_string(),
                    workspace_id: surface.workspace_id.clone(),
                    name: name
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("{} {}", provider, &id[id.len() - 4..])),
                    provider,
                    state: AgentState::Idle,
                    detail: String::new(),
                    progress: None,
                    attention: Attention::None,
                    since_ms: now,
                    last_seen_ms: now,
                    tokens_in: 0,
                    tokens_out: 0,
                    cost_usd: 0.0,
                    resume,
                    meta,
                }
            }
        };

        agent.attention = agent::derive_attention(agent.state);
        self.agents.insert(agent.id.clone(), agent.clone());
        if let Some(s) = self.surface_mut(surface_id) {
            s.agent_id = Some(agent.id.clone());
        }
        self.emit(names::AGENT_CREATED, serde_json::json!(agent));
        Ok(agent)
    }

    /// Passive detection: create an `unknown` agent when the foreground
    /// process matches a provider and nothing is registered for the surface.
    /// Returns `None` when there is nothing new to report.
    pub fn detect_agent(
        &mut self,
        surface_id: &str,
        ops: &dyn SurfaceOps,
    ) -> Result<Option<AgentSurface>, CoreError> {
        let surface = self.require_surface(surface_id)?.clone();
        if self.agent_for_surface(surface_id).is_some() {
            return Ok(None);
        }
        let Some(provider) = ops
            .pid_info(surface_id)
            .ok()
            .and_then(|info| crate::providers::detect(info.comm.as_deref(), &info.argv))
        else {
            return Ok(None);
        };

        let id = ids::mint(ids::AGENT);
        let now = time::now_ms();
        let agent = AgentSurface {
            name: format!("{} {}", provider.display, &id[id.len() - 4..]),
            id: id.clone(),
            surface_id: surface_id.to_string(),
            workspace_id: surface.workspace_id,
            provider: provider.id.to_string(),
            state: AgentState::Unknown,
            detail: String::new(),
            progress: None,
            attention: Attention::None,
            since_ms: now,
            last_seen_ms: now,
            tokens_in: 0,
            tokens_out: 0,
            cost_usd: 0.0,
            resume: None,
            meta: Value::Null,
        };
        self.agents.insert(id.clone(), agent.clone());
        if let Some(s) = self.surface_mut(surface_id) {
            s.agent_id = Some(id);
        }
        self.emit(names::AGENT_CREATED, serde_json::json!(agent));
        Ok(Some(agent))
    }

    pub fn update_agent(
        &mut self,
        agent_id: &str,
        new_state: Option<AgentState>,
        detail: Option<String>,
        progress: Option<Option<f32>>,
        meta: Option<Value>,
    ) -> Result<AgentSurface, CoreError> {
        let now = time::now_ms();
        let mut agent = self
            .agents
            .get(agent_id)
            .cloned()
            .ok_or_else(|| CoreError::not_found(format!("no agent {agent_id}")))?;

        let from = agent.state;
        if let Some(to) = new_state {
            if !agent::transition_allowed(from, to) {
                return Err(CoreError::new(
                    ErrorCode::InvalidState,
                    format!(
                        "illegal agent transition {} -> {}",
                        from.as_str(),
                        to.as_str()
                    ),
                ));
            }
            if to != from {
                agent.state = to;
                agent.since_ms = now;
                agent.attention = agent::derive_attention(to);
            }
        }
        if let Some(d) = detail {
            agent.detail = d.chars().take(120).collect();
        }
        if let Some(p) = progress {
            agent.progress = p.map(|v| v.clamp(0.0, 1.0));
            if let Some(v) = agent.progress {
                self.emit(
                    names::AGENT_PROGRESS,
                    serde_json::json!({"agent_id": agent.id, "progress": v, "detail": agent.detail}),
                );
            }
        }
        if let Some(m) = meta {
            agent.meta = m;
        }
        agent.last_seen_ms = now;

        self.agents.insert(agent.id.clone(), agent.clone());
        if new_state.map(|s| s != from).unwrap_or(false) {
            self.emit(
                names::AGENT_STATE_CHANGED,
                serde_json::json!({
                    "agent_id": agent.id,
                    "from": from,
                    "to": agent.state,
                    "reason": "update",
                }),
            );
            self.maybe_notify_attention(&agent);
        }
        Ok(agent)
    }

    fn maybe_notify_attention(&mut self, agent: &AgentSurface) {
        if !agent.state.needs_attention() {
            return;
        }
        let level = match agent.state {
            AgentState::WaitingInput => NotifLevel::NeedsInput,
            _ => NotifLevel::Error,
        };
        let now = time::now_ms();
        let dup = self.notifications.iter().any(|n| {
            n.agent_id.as_deref() == Some(agent.id.as_str())
                && !n.acked
                && n.level == level
                && now.saturating_sub(n.created_ms) < 60_000
        });
        if dup {
            return;
        }
        let title = match level {
            NotifLevel::NeedsInput => format!("{} needs input", agent.name),
            NotifLevel::Error => format!("{} is blocked", agent.name),
            NotifLevel::Info => agent.name.clone(),
        };
        let n = Notification {
            id: ids::mint(ids::NOTIFICATION),
            level,
            title,
            body: agent.detail.clone(),
            workspace_id: Some(agent.workspace_id.clone()),
            surface_id: Some(agent.surface_id.clone()),
            agent_id: Some(agent.id.clone()),
            created_ms: now,
            acked: false,
        };
        self.notifications.push(n.clone());
        self.emit(names::NOTIFICATION_CREATED, serde_json::json!(n));
    }

    /// Escalate agents that stopped reporting. Returns ids that changed.
    pub fn sweep_stale(&mut self, now_ms: u64, ttl_ms: u64) -> Vec<String> {
        let stale: Vec<String> = self
            .agents
            .values()
            .filter(|a| a.state.is_active() && now_ms.saturating_sub(a.last_seen_ms) > ttl_ms)
            .map(|a| a.id.clone())
            .collect();

        for id in &stale {
            let (from, snapshot) = {
                let Some(agent) = self.agents.get_mut(id) else {
                    continue;
                };
                let from = agent.state;
                agent.state = AgentState::Blocked;
                agent.attention = Attention::Error;
                agent.since_ms = now_ms;
                agent.detail = format!("no update for {}s", ttl_ms / 1000);
                (from, agent.clone())
            };
            self.emit(
                names::AGENT_STATE_CHANGED,
                serde_json::json!({
                    "agent_id": snapshot.id,
                    "from": from,
                    "to": AgentState::Blocked,
                    "reason": "stale",
                }),
            );
            self.maybe_notify_attention(&snapshot);
        }
        stale
    }

    pub fn add_notification(
        &mut self,
        level: NotifLevel,
        title: &str,
        body: Option<&str>,
        workspace_id: Option<String>,
        surface_id: Option<String>,
        ops: &dyn SurfaceOps,
    ) -> Notification {
        let agent_id = surface_id
            .as_deref()
            .and_then(|s| self.agent_for_surface(s))
            .map(|a| a.id.clone());
        let n = Notification {
            id: ids::mint(ids::NOTIFICATION),
            level,
            title: title.to_string(),
            body: body.unwrap_or_default().to_string(),
            workspace_id,
            surface_id,
            agent_id,
            created_ms: time::now_ms(),
            acked: false,
        };
        self.notifications.push(n.clone());
        self.emit(names::NOTIFICATION_CREATED, serde_json::json!(n));
        // OS delivery is best effort; the in-app ladder does not depend on it.
        let _ = ops.notify_os(&n);
        n
    }

    pub fn ack_notification(&mut self, id: &str) -> Result<(), CoreError> {
        let n = self
            .notifications
            .iter_mut()
            .find(|n| n.id == id)
            .ok_or_else(|| CoreError::not_found(format!("no notification {id}")))?;
        n.acked = true;
        self.emit(
            names::NOTIFICATION_ACKED,
            serde_json::json!({"notification_id": id}),
        );
        Ok(())
    }
}
