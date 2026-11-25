//! `AppState`: the aggregate the router mutates. No I/O, no threads, no clock
//! except the `now_ms` passed in by callers (which makes tests deterministic).

use std::collections::BTreeMap;

use serde_json::Value;

use crate::agent::{self, AgentState, Attention};
use crate::ids;
use crate::model::{
    AgentSurface, Group, NotifLevel, Notification, Orientation, Pane, PaneNode, Resume, Surface,
    SurfaceKind, Window, WindowKind, Workspace,
};
use crate::protocol::{Event, names};
use crate::time;
use crate::{CoreError, ErrorCode, SpawnRequest, SurfaceOps};

#[derive(Debug, Default)]
pub struct AppState {
    pub windows: Vec<Window>,
    pub workspaces: Vec<Workspace>,
    pub groups: Vec<Group>,
    pub panes: BTreeMap<String, Pane>,
    pub surfaces: BTreeMap<String, Surface>,
    pub agents: BTreeMap<String, AgentSurface>,
    pub notifications: Vec<Notification>,
    pub selected_window: Option<String>,
    pub selected_workspace: Option<String>,
    pub selected_surface: Option<String>,
    /// Providers whose OS notifications are silenced. In-app attention stays.
    pub muted_providers: std::collections::HashSet<String>,
    /// Launch profiles and custom commands, pushed in by the shell from
    /// `config.json`. The core never reads a config file.
    pub launches: crate::launch::Launches,
    /// Repositories the user saved with `layout.save`, by name.
    pub layouts: std::collections::BTreeMap<String, crate::launch::SavedLayout>,
    /// Minted lazily; identifies this app run in `system.hello`.
    pub instance: String,
    /// First activity timestamp, used by `system.ping` uptime.
    pub started_ms: u64,
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
        if self.started_ms == 0 {
            self.started_ms = time::now_ms();
        }
        if self.instance.is_empty() {
            self.instance = ids::mint("inst");
        }
        self.instance.clone()
    }

    /// Milliseconds since this state was first touched. Zero before boot.
    pub fn uptime_ms(&self) -> u64 {
        if self.started_ms == 0 {
            0
        } else {
            time::now_ms().saturating_sub(self.started_ms)
        }
    }

    /// Drop all live entities, closing their PTYs first. Used by
    /// `session.restore` so restoring over a running app cannot leave stale
    /// surfaces pointing at the old snapshot.
    pub fn set_muted_providers(&mut self, providers: Vec<String>) {
        self.muted_providers = providers.into_iter().collect();
    }

    pub fn reset(&mut self, ops: &dyn SurfaceOps) {
        let surfaces: Vec<String> = self.surfaces.keys().cloned().collect();
        for id in surfaces {
            let _ = ops.close(&id);
        }
        self.workspaces.clear();
        self.windows.clear();
        self.groups.clear();
        self.panes.clear();
        self.surfaces.clear();
        self.agents.clear();
        self.notifications.clear();
        self.selected_window = None;
        self.selected_workspace = None;
        self.selected_surface = None;
        self.pending.clear();
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

    // ---- windows ------------------------------------------------------

    pub fn window(&self, id: &str) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    pub fn window_mut(&mut self, id: &str) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    pub fn active_window(&self) -> Option<&Window> {
        self.selected_window
            .as_deref()
            .and_then(|id| self.window(id))
            .or_else(|| self.windows.first())
    }

    pub fn window_for_workspace(&self, workspace_id: &str) -> Option<&Window> {
        let ws = self.workspace(workspace_id)?;
        ws.window_id
            .as_deref()
            .and_then(|id| self.window(id))
            .or_else(|| self.windows.first())
    }

    pub fn create_window(&mut self, name: Option<&str>) -> Window {
        self.create_window_of(WindowKind::Main, name)
    }

    /// Create a window of a given kind.
    pub fn create_window_of(&mut self, kind: WindowKind, name: Option<&str>) -> Window {
        let window = Window {
            id: ids::mint(ids::WINDOW),
            name: name
                .map(str::to_string)
                .unwrap_or_else(|| format!("window {}", self.windows.len() + 1)),
            created_ms: time::now_ms(),
            bounds: None,
            monitor: None,
            sidebar_width: None,
            active_workspace: None,
            active_surface: None,
            broadcast: false,
            kind,
        };
        self.windows.push(window.clone());
        if self.selected_window.is_none() {
            self.selected_window = Some(window.id.clone());
        }
        self.emit(names::WINDOW_CREATED, serde_json::json!(window));
        window
    }

    /// The selected window, creating `main` when a state has none.
    pub fn ensure_window(&mut self) -> String {
        if let Some(id) = &self.selected_window
            && self.window(id).is_some()
        {
            return id.clone();
        }
        if let Some(id) = self.windows.first().map(|w| w.id.clone()) {
            self.selected_window = Some(id.clone());
            return id;
        }
        self.create_window(Some("main")).id
    }

    /// The quick terminal window, created on first use with its own workspace
    /// so it never disturbs the layout the user is working in.
    pub fn ensure_quick_window(&mut self, ops: &dyn SurfaceOps) -> String {
        if let Some(window) = self.window_kind(WindowKind::Quick) {
            return window.id.clone();
        }
        let window = self.create_window_of(WindowKind::Quick, Some("quick"));
        // The workspace must be spawned through the caller's ops: a real PTY
        // lives behind it, and `NullOps` would leave the drop-down with a pane
        // that can never be read or typed into.
        // `create_workspace` with no window binds to the *selected* window,
        // which is the one the user is working in; the drop-down must own its
        // workspace outright.
        let workspace = self.create_workspace_in(Some(&window.id), Some("quick"), None, None, ops);
        match workspace {
            Ok(workspace) => {
                let _ = self.assign_workspace_to_window(&workspace.id, &window.id);
                // Pin the window to its own workspace explicitly:
                // `assign_workspace_to_window` leaves an existing choice alone,
                // and a new window can inherit one from the previous selection.
                if let Some(w) = self.window_mut(&window.id) {
                    w.active_workspace = Some(workspace.id.clone());
                    w.active_surface = workspace.active_surface.clone();
                }
            }
            Err(_) => {
                // No shell: drop the window rather than leave a dead drop-down
                // that a later press would keep returning.
                self.windows.retain(|w| w.id != window.id);
            }
        }
        window.id
    }

    /// Create a workspace whose first pane runs `argv` with `env`, which is how
    /// a launch profile opens a workspace rather than a tab.
    pub fn create_workspace_with(
        &mut self,
        name: &str,
        label: Option<&str>,
        cwd: Option<&str>,
        argv: Vec<String>,
        env: Vec<(String, String)>,
        ops: &dyn SurfaceOps,
    ) -> Result<Workspace, CoreError> {
        let window_id = self.ensure_window();
        let ws_id = ids::mint(ids::WORKSPACE);
        let pane_id = ids::mint(ids::PANE);
        let mut ws = Workspace {
            id: ws_id.clone(),
            name: if name.is_empty() {
                format!("workspace {}", self.workspaces.len() + 1)
            } else {
                name.to_string()
            },
            group_id: None,
            icon: None,
            color: None,
            pinned: false,
            unread: 0,
            created_ms: time::now_ms(),
            root: PaneNode::leaf(pane_id.clone()),
            active_surface: None,
            window_id: Some(window_id),
            zoomed_pane: None,
        };
        let surface = self.spawn_into(&ws, &pane_id, cwd, argv, env, ops)?;
        ws.active_surface = Some(surface.id.clone());
        if let Some(label) = label.map(str::trim).filter(|label| !label.is_empty())
            && let Some(surface) = self.surface_mut(&surface.id)
        {
            surface.user_title = Some(label.to_string());
        }
        self.workspaces.push(ws.clone());
        self.emit(names::WORKSPACE_CREATED, serde_json::json!(ws));
        // `select_workspace` points the window, the selection and the focus at
        // the new workspace; setting the fields directly left the window's
        // `active_workspace` stale and emitted no selection event.
        self.select_workspace(&ws.id)?;
        Ok(ws)
    }

    /// Capture a workspace's tree, with each pane's cwd and argv, as a layout.
    pub fn save_layout(
        &self,
        workspace_id: &str,
        name: &str,
    ) -> Result<crate::launch::SavedLayout, CoreError> {
        let workspace = self
            .workspace(workspace_id)
            .ok_or_else(|| CoreError::not_found(format!("no workspace {workspace_id}")))?;
        let mut cwd: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
        let mut argv: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for pane_id in workspace.root.leaves() {
            let Some(pane) = self.panes.get(pane_id) else {
                continue;
            };
            let Some(surface) = pane.active_surface().and_then(|id| self.surface(id)) else {
                continue;
            };
            cwd.insert(pane_id.to_string(), surface.cwd.clone());
            if !surface.argv.is_empty() {
                argv.insert(pane_id.to_string(), surface.argv.clone());
            }
        }
        Ok(crate::launch::SavedLayout {
            name: name.to_string(),
            root: workspace.root.clone(),
            cwd,
            argv,
            saved_ms: time::now_ms(),
        })
    }

    /// Open a saved layout as a new workspace: the same tree shape, each pane
    /// in its saved cwd, and a pane's saved command re-run where it had one.
    pub fn open_layout(
        &mut self,
        layout: &crate::launch::SavedLayout,
        ops: &dyn SurfaceOps,
    ) -> Result<Workspace, CoreError> {
        let saved_leaves: Vec<String> =
            layout.root.leaves().iter().map(|s| s.to_string()).collect();
        if saved_leaves.is_empty() {
            return Err(CoreError::new(
                ErrorCode::BadRequest,
                "a layout needs at least one pane",
            ));
        }
        let window_id = self.ensure_window();
        let ws_id = ids::mint(ids::WORKSPACE);
        // Pane ids in the saved tree may still be live in the workspace the
        // layout was captured from, so every leaf gets a fresh id.
        let leaves: Vec<String> = saved_leaves.iter().map(|_| ids::mint(ids::PANE)).collect();
        let mut ws = Workspace {
            id: ws_id.clone(),
            name: layout.name.clone(),
            group_id: None,
            icon: None,
            color: None,
            pinned: false,
            unread: 0,
            created_ms: time::now_ms(),
            root: rebuild_tree(&layout.root, &leaves),
            active_surface: None,
            window_id: Some(window_id),
            zoomed_pane: None,
        };

        // Spawn every pane first, tracking what was created. A spawn can fail
        // (a program that no longer exists, a cwd that is gone), and leaving
        // half a layout behind would strand live PTYs outside any tree.
        let mut spawned: Vec<(String, String)> = Vec::new();
        for (index, leaf) in leaves.iter().enumerate() {
            let saved = &saved_leaves[index];
            let cwd = layout.cwd.get(saved).map(String::as_str);
            let argv = layout.argv.get(saved).cloned().unwrap_or_default();
            let result = self.spawn_into(&ws, leaf, cwd, argv, Vec::new(), ops);
            match result {
                Ok(surface) => {
                    spawned.push((leaf.clone(), surface.id.clone()));
                    if ws.active_surface.is_none() {
                        ws.active_surface = Some(surface.id.clone());
                    }
                }
                Err(error) => {
                    // Roll back: close the surfaces this layout opened and drop
                    // the panes it registered.
                    for (_, surface_id) in &spawned {
                        let _ = self.close_surface(surface_id, ops);
                    }
                    return Err(error);
                }
            }
        }
        for (leaf, surface_id) in &spawned {
            self.panes.insert(
                leaf.clone(),
                Pane {
                    id: leaf.clone(),
                    workspace_id: ws_id.clone(),
                    surfaces: vec![surface_id.clone()],
                    active: 0,
                },
            );
        }
        self.workspaces.push(ws.clone());
        self.emit(names::WORKSPACE_CREATED, serde_json::json!(ws));
        self.select_workspace(&ws.id)?;
        Ok(ws)
    }

    /// The first window of a kind.
    pub fn window_kind(&self, kind: WindowKind) -> Option<&Window> {
        self.windows.iter().find(|window| window.kind == kind)
    }

    pub fn select_window(&mut self, window_id: &str) -> Result<(), CoreError> {
        if self.window(window_id).is_none() {
            return Err(CoreError::not_found(format!("no window {window_id}")));
        }
        self.selected_window = Some(window_id.to_string());
        self.sync_selection_from_window(window_id);
        self.emit(
            names::WINDOW_SELECTED,
            serde_json::json!({"window_id": window_id}),
        );
        Ok(())
    }

    /// Point the global selection at the given window's visible workspace and
    /// surface. Existing callers (CLI, `workspace.list`) still read the globals.
    pub fn sync_selection_from_window(&mut self, window_id: &str) {
        let Some(win) = self.window(window_id).cloned() else {
            return;
        };
        let workspace = win
            .active_workspace
            .clone()
            .filter(|id| self.workspace(id).is_some())
            .or_else(|| {
                self.workspaces
                    .iter()
                    .find(|w| w.window_id.as_deref() == Some(window_id))
                    .map(|w| w.id.clone())
            });
        let surface = workspace
            .as_deref()
            .and_then(|id| self.active_surface_of_workspace(id));
        if let Some(w) = self.window_mut(window_id) {
            w.active_workspace = workspace.clone();
            w.active_surface = surface.clone();
        }
        self.selected_window = Some(window_id.to_string());
        self.selected_workspace = workspace;
        self.selected_surface = surface;
    }

    pub fn assign_workspace_to_window(
        &mut self,
        workspace_id: &str,
        window_id: &str,
    ) -> Result<(), CoreError> {
        if self.window(window_id).is_none() {
            return Err(CoreError::not_found(format!("no window {window_id}")));
        }
        let ws = self
            .workspace_mut(workspace_id)
            .ok_or_else(|| CoreError::not_found(format!("no workspace {workspace_id}")))?;
        ws.window_id = Some(window_id.to_string());
        let active = self
            .workspace(workspace_id)
            .and_then(|w| w.active_surface.clone());
        if let Some(win) = self.window_mut(window_id) {
            if win.active_workspace.is_none() {
                win.active_workspace = Some(workspace_id.to_string());
            }
            if win.active_workspace.as_deref() == Some(workspace_id) {
                win.active_surface = active;
            }
        }
        Ok(())
    }

    pub fn close_window(&mut self, window_id: &str, ops: &dyn SurfaceOps) -> Result<(), CoreError> {
        if self.window(window_id).is_none() {
            return Err(CoreError::not_found(format!("no window {window_id}")));
        }
        if self.windows.len() <= 1 {
            return Err(CoreError::new(
                ErrorCode::InvalidState,
                "cannot close the last window",
            ));
        }
        let workspaces: Vec<String> = self
            .workspaces
            .iter()
            .filter(|w| w.window_id.as_deref() == Some(window_id))
            .map(|w| w.id.clone())
            .collect();
        for id in workspaces {
            let _ = self.close_workspace(&id, ops);
        }
        self.windows.retain(|w| w.id != window_id);
        if self.selected_window.as_deref() == Some(window_id) {
            let next = self.windows.first().map(|w| w.id.clone());
            match next {
                Some(id) => {
                    self.selected_window = Some(id.clone());
                    self.sync_selection_from_window(&id);
                }
                None => {
                    self.selected_window = None;
                    self.selected_workspace = None;
                    self.selected_surface = None;
                }
            }
        }
        self.emit(
            names::WINDOW_CLOSED,
            serde_json::json!({"window_id": window_id}),
        );
        Ok(())
    }

    /// Close a workspace and every surface under it.
    pub fn close_workspace(
        &mut self,
        workspace_id: &str,
        ops: &dyn SurfaceOps,
    ) -> Result<(), CoreError> {
        let ws = self
            .workspace(workspace_id)
            .cloned()
            .ok_or_else(|| CoreError::not_found(format!("no workspace {workspace_id}")))?;
        let mut surfaces: Vec<String> = Vec::new();
        for pane_id in ws.root.leaves() {
            if let Some(pane) = self.panes.get(pane_id) {
                surfaces.extend(pane.surfaces.iter().cloned());
            }
        }
        for surface in surfaces {
            let _ = self.close_surface(&surface, ops);
        }
        if self.workspace(workspace_id).is_some() {
            self.workspaces.retain(|w| w.id != workspace_id);
            if self.selected_workspace.as_deref() == Some(workspace_id) {
                self.selected_workspace = self.workspaces.first().map(|w| w.id.clone());
            }
            self.emit(
                names::WORKSPACE_CLOSED,
                serde_json::json!({"workspace_id": workspace_id}),
            );
        }
        // A window whose last workspace closed keeps existing but points at
        // nothing; the shell decides whether that window should close.
        if let Some(win) = self.window_mut(ws.window_id.as_deref().unwrap_or_default())
            && win.active_workspace.as_deref() == Some(workspace_id)
        {
            win.active_workspace = None;
            win.active_surface = None;
        }
        Ok(())
    }

    /// Maximise or restore a pane. Returns the workspace's new zoomed pane.
    pub fn set_zoomed_pane(
        &mut self,
        pane_id: &str,
        zoomed: bool,
    ) -> Result<Option<String>, CoreError> {
        let pane = self
            .panes
            .get(pane_id)
            .ok_or_else(|| CoreError::not_found(format!("no pane {pane_id}")))?;
        let workspace_id = pane.workspace_id.clone();
        let ws = self
            .workspace_mut(&workspace_id)
            .ok_or_else(|| CoreError::not_found("workspace for pane"))?;
        if !ws.root.contains(pane_id) {
            return Err(CoreError::new(
                ErrorCode::InvalidState,
                format!("pane {pane_id} is not in the workspace tree"),
            ));
        }
        if zoomed {
            ws.zoomed_pane = Some(pane_id.to_string());
        } else if ws.zoomed_pane.as_deref() == Some(pane_id) {
            ws.zoomed_pane = None;
        }
        let zoomed_pane = ws.zoomed_pane.clone();
        let snapshot = ws.clone();
        self.emit(
            names::PANE_ZOOM_CHANGED,
            serde_json::json!({
                "workspace_id": workspace_id,
                "pane_id": zoomed_pane,
                "workspace": snapshot,
            }),
        );
        Ok(zoomed_pane)
    }

    /// Keep the owning window's visible workspace/surface in step with a
    /// selection change. Used by every focus path.
    fn note_window_selection(&mut self, workspace_id: &str, surface_id: Option<&str>) {
        let window_id = self
            .workspace(workspace_id)
            .and_then(|w| w.window_id.clone());
        if let Some(window_id) = window_id {
            self.selected_window = Some(window_id.clone());
            if let Some(win) = self.window_mut(&window_id) {
                win.active_workspace = Some(workspace_id.to_string());
                win.active_surface = surface_id.map(str::to_string);
            }
        }
    }

    /// The surface a workspace should show: the remembered one when it is
    /// still alive, otherwise the active tab of the first pane.
    pub fn active_surface_of_workspace(&self, workspace_id: &str) -> Option<String> {
        let ws = self.workspace(workspace_id)?;
        if let Some(id) = &ws.active_surface
            && self.surfaces.contains_key(id)
        {
            return Some(id.clone());
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
        self.create_workspace_in(None, name, cwd, group_id, ops)
    }

    /// Create a workspace inside `window_id`, or the selected window when
    /// `None`. `workspace.create {window_id}` uses the explicit form.
    pub fn create_workspace_in(
        &mut self,
        window_id: Option<&str>,
        name: Option<&str>,
        cwd: Option<&str>,
        group_id: Option<String>,
        ops: &dyn SurfaceOps,
    ) -> Result<Workspace, CoreError> {
        let window_id = match window_id {
            Some(id) => {
                if self.window(id).is_none() {
                    return Err(CoreError::not_found(format!("no window {id}")));
                }
                id.to_string()
            }
            None => self.ensure_window(),
        };
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
            window_id: Some(window_id.clone()),
            zoomed_pane: None,
        };

        let surface = self.spawn_into(&ws, &pane_id, cwd, vec![], vec![], ops)?;
        ws.active_surface = Some(surface.id.clone());

        self.workspaces.push(ws);
        self.selected_workspace.get_or_insert(ws_id.clone());
        if let Some(win) = self.window_mut(&window_id)
            && win.active_workspace.is_none()
        {
            win.active_workspace = Some(ws_id.clone());
            win.active_surface = Some(surface.id.clone());
        }
        let snapshot = self.workspace(&ws_id).cloned();
        self.emit(names::WORKSPACE_CREATED, serde_json::json!(snapshot));

        self.workspace(&ws_id)
            .cloned()
            .ok_or_else(|| CoreError::internal("workspace vanished"))
    }

    /// Create a workspace with one pane but no surface. Used when a window is
    /// about to receive a moved tab, so a tear-off does not leave a spare
    /// default shell behind.
    pub fn create_empty_workspace_in(
        &mut self,
        window_id: &str,
        name: Option<&str>,
    ) -> Result<Workspace, CoreError> {
        if self.window(window_id).is_none() {
            return Err(CoreError::not_found(format!("no window {window_id}")));
        }
        let ws_id = ids::mint(ids::WORKSPACE);
        let pane_id = ids::mint(ids::PANE);
        let ws = Workspace {
            id: ws_id.clone(),
            name: name
                .map(str::to_string)
                .unwrap_or_else(|| format!("workspace {}", self.workspaces.len() + 1)),
            group_id: None,
            icon: None,
            color: None,
            pinned: false,
            unread: 0,
            created_ms: time::now_ms(),
            root: PaneNode::leaf(pane_id.clone()),
            active_surface: None,
            window_id: Some(window_id.to_string()),
            zoomed_pane: None,
        };
        self.workspaces.push(ws.clone());
        self.panes.insert(
            pane_id.clone(),
            Pane {
                id: pane_id,
                workspace_id: ws_id.clone(),
                surfaces: Vec::new(),
                active: 0,
            },
        );
        if let Some(win) = self.window_mut(window_id)
            && win.active_workspace.is_none()
        {
            win.active_workspace = Some(ws_id.clone());
            win.active_surface = None;
        }
        self.emit(names::WORKSPACE_CREATED, serde_json::json!(ws));
        Ok(ws)
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
        let launch_argv = argv.clone();
        let launch_env = env.clone();
        let mut req = SpawnRequest {
            cwd: cwd.map(str::to_string),
            argv,
            env,
            cols: 80,
            rows: 24,
            restore_text: None,
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
            user_title: None,
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
            argv: launch_argv,
            env: launch_env,
            restore_text: None,
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
            self.note_window_selection(&ws.id, Some(&surface.id));
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
        if let Some(pane) = self.panes.get_mut(&surface.pane_id)
            && let Some(idx) = pane.surfaces.iter().position(|s| s == surface_id)
        {
            pane.active = idx;
        }
        ops.focus(surface_id)?;
        self.selected_workspace = Some(surface.workspace_id.clone());
        self.selected_surface = Some(surface.id.clone());
        self.note_window_selection(&surface.workspace_id, Some(&surface.id));

        let mut changed = None;
        if let Some(w) = self.workspace_mut(&surface.workspace_id)
            && (w.active_surface.as_deref() != Some(surface_id) || w.unread != 0)
        {
            w.active_surface = Some(surface.id.clone());
            w.unread = 0;
            changed = Some(w.clone());
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
            self.note_window_selection(&ws.id, Some(&new_surface.id));
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
                if ws.zoomed_pane.as_deref() == Some(surface.pane_id.as_str()) {
                    ws.zoomed_pane = None;
                }
                ws_gone = root_was_leaf || ws.root.is_empty();
            }
            if ws_gone {
                let window_id = self.workspace(&ws_id).and_then(|w| w.window_id.clone());
                self.workspaces.retain(|w| w.id != ws_id);
                if self.selected_workspace.as_deref() == Some(ws_id.as_str()) {
                    self.selected_workspace = self.workspaces.first().map(|w| w.id.clone());
                }
                if let Some(window_id) = window_id
                    && let Some(win) = self.window_mut(&window_id)
                    && win.active_workspace.as_deref() == Some(ws_id.as_str())
                {
                    win.active_workspace = None;
                    win.active_surface = None;
                }
                self.emit(
                    names::WORKSPACE_CLOSED,
                    serde_json::json!({"workspace_id": ws_id}),
                );
            }
        }

        // A surviving workspace must point at a live tab after the close.
        let active = self.active_surface_of_workspace(&ws_id);
        if let Some(ws) = self.workspace_mut(&ws_id)
            && ws.active_surface != active
        {
            ws.active_surface = active.clone();
            let snapshot = ws.clone();
            self.emit(names::WORKSPACE_CHANGED, serde_json::json!(snapshot));
        }
        if self.selected_workspace.as_deref() == Some(ws_id.as_str())
            && let Some(s) = active
        {
            self.selected_surface = Some(s);
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
        self.note_window_selection(workspace_id, self.selected_surface.clone().as_deref());
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
        ops: &dyn SurfaceOps,
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
            self.maybe_notify_attention(&agent, ops);
        }
        Ok(agent)
    }

    /// The child process behind a surface exited. A live agent becomes
    /// `errored` immediately instead of waiting for the 180s staleness sweep;
    /// that is what makes a dead CLI visible within the one-second M3 budget.
    pub fn mark_agent_exited(
        &mut self,
        surface_id: &str,
        ops: &dyn SurfaceOps,
    ) -> Option<AgentSurface> {
        let id = self.agent_for_surface(surface_id)?.id.clone();
        let from = self.agents.get(&id)?.state;
        if from.is_terminal() || !agent::transition_allowed(from, AgentState::Errored) {
            return None;
        }
        let now = time::now_ms();
        {
            let agent = self.agents.get_mut(&id)?;
            agent.state = AgentState::Errored;
            agent.attention = Attention::Error;
            agent.since_ms = now;
            agent.last_seen_ms = now;
            agent.detail = "process exited".to_string();
            agent.progress = None;
        }
        let snapshot = self.agents.get(&id)?.clone();
        self.emit(
            names::AGENT_STATE_CHANGED,
            serde_json::json!({
                "agent_id": snapshot.id,
                "from": from,
                "to": AgentState::Errored,
                "reason": "process_exit",
            }),
        );
        self.maybe_notify_attention(&snapshot, ops);
        Some(snapshot)
    }

    /// Clear one agent's attention after the user visits it. Derived attention
    /// is set to `None`; the next real state transition derives it again.
    /// Notifications for the same agent are acknowledged in the same pass.
    pub fn ack_agent_attention(&mut self, agent_id: &str) -> Result<AgentSurface, CoreError> {
        if !self.agents.contains_key(agent_id) {
            return Err(CoreError::not_found(format!("no agent {agent_id}")));
        }
        let acked: Vec<String> = self
            .notifications
            .iter()
            .filter(|n| n.agent_id.as_deref() == Some(agent_id) && !n.acked)
            .map(|n| n.id.clone())
            .collect();
        for id in &acked {
            if let Some(n) = self.notifications.iter_mut().find(|n| &n.id == id) {
                n.acked = true;
            }
            self.emit(
                names::NOTIFICATION_ACKED,
                serde_json::json!({"notification_id": id}),
            );
        }
        let agent = self.agents.get_mut(agent_id).expect("checked above");
        let changed = agent.attention != Attention::None;
        agent.attention = Attention::None;
        let snapshot = agent.clone();
        if changed {
            self.emit(
                names::AGENT_ATTENTION_CHANGED,
                serde_json::json!({
                    "agent_id": snapshot.id,
                    "attention": "none",
                    "reason": "ack",
                }),
            );
        }
        Ok(snapshot)
    }

    /// Record the provider session id discovered by an agent hook. A passively
    /// detected agent has no launch payload, so its first session id builds one
    /// from the surface; a wrapper's argv and environment stay as saved.
    /// Record provider-reported usage. Zero fields are left unchanged so a hook
    /// can report tokens without inventing a cost.
    pub fn set_agent_usage(
        &mut self,
        agent_id: &str,
        tokens_in: Option<u64>,
        tokens_out: Option<u64>,
        cost_usd: Option<f64>,
    ) -> Result<AgentSurface, CoreError> {
        let mut agent = self
            .agents
            .get(agent_id)
            .cloned()
            .ok_or_else(|| CoreError::not_found(format!("no agent {agent_id}")))?;
        if let Some(value) = tokens_in {
            agent.tokens_in = value;
        }
        if let Some(value) = tokens_out {
            agent.tokens_out = value;
        }
        if let Some(value) = cost_usd {
            agent.cost_usd = value.max(0.0);
        }
        agent.last_seen_ms = time::now_ms();
        self.agents.insert(agent.id.clone(), agent.clone());
        Ok(agent)
    }

    pub fn set_agent_session_ref(
        &mut self,
        agent_id: &str,
        session_ref: &str,
        ops: &dyn SurfaceOps,
    ) -> Result<AgentSurface, CoreError> {
        let mut agent = self
            .agents
            .get(agent_id)
            .cloned()
            .ok_or_else(|| CoreError::not_found(format!("no agent {agent_id}")))?;
        if agent.resume.is_none() {
            let cwd = self
                .surface(&agent.surface_id)
                .map(|surface| surface.cwd.clone())
                .unwrap_or_default();
            agent.resume = Some(Resume {
                argv: Vec::new(),
                cwd,
                session_ref: None,
                env: Vec::new(),
            });
        }
        let resume = agent.resume.as_mut().expect("resume set above");
        if resume.argv.is_empty() {
            // The hook runs inside the provider process, so the foreground argv
            // is the launch command, including flags such as
            // `--allow-dangerously-skip-permissions`.
            resume.argv = ops
                .pid_info(&agent.surface_id)
                .map(|info| info.argv)
                .unwrap_or_default();
        }
        if resume.session_ref.as_deref() != Some(session_ref) {
            resume.session_ref = Some(session_ref.to_string());
        }
        agent.last_seen_ms = time::now_ms();
        self.agents.insert(agent.id.clone(), agent.clone());
        Ok(agent)
    }

    fn maybe_notify_attention(&mut self, agent: &AgentSurface, ops: &dyn SurfaceOps) {
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
        // The in-app ladder is primary, but attention must still reach the OS
        // when the user is in another workspace (M3 acceptance). A muted
        // provider keeps the row and the ring, and loses only the OS banner.
        if !self.muted_providers.contains(&agent.provider) {
            let _ = ops.notify_os(&n);
        }
    }

    /// Escalate agents that stopped reporting. Returns ids that changed.
    pub fn sweep_stale(&mut self, now_ms: u64, ttl_ms: u64, ops: &dyn SurfaceOps) -> Vec<String> {
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
            self.maybe_notify_attention(&snapshot, ops);
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

/// Rebuild a saved tree with fresh leaf ids, in the same order, so the shape
/// (splits and weights) survives without reusing pane ids that now belong to
/// another workspace.
fn rebuild_tree(node: &PaneNode, leaves: &[String]) -> PaneNode {
    fn walk(node: &PaneNode, leaves: &[String], cursor: &mut usize) -> PaneNode {
        match node {
            PaneNode::Leaf { .. } => {
                let id = leaves
                    .get(*cursor)
                    .cloned()
                    .unwrap_or_else(|| ids::mint(ids::PANE));
                *cursor += 1;
                PaneNode::leaf(id)
            }
            PaneNode::Split {
                id: _,
                orientation,
                children,
                weights,
            } => PaneNode::Split {
                id: ids::mint(ids::PANE),
                orientation: *orientation,
                children: children
                    .iter()
                    .map(|child| walk(child, leaves, cursor))
                    .collect(),
                weights: weights.clone(),
            },
        }
    }
    walk(node, leaves, &mut 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NullOps, PidInfo};

    /// Reports a fixed foreground argv, the way a shell sees a running agent.
    struct PidOps(Vec<String>);

    impl SurfaceOps for PidOps {
        fn spawn(&self, req: SpawnRequest) -> Result<crate::Spawned, CoreError> {
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
        fn pid_info(&self, _: &str) -> Result<PidInfo, CoreError> {
            Ok(PidInfo {
                pid: Some(1),
                comm: Some("claude".into()),
                argv: self.0.clone(),
            })
        }
        fn notify_os(&self, n: &crate::Notification) -> Result<(), CoreError> {
            NullOps.notify_os(n)
        }
    }

    #[test]
    fn given_a_passive_agent_when_a_session_ref_arrives_then_the_live_argv_is_kept() {
        let ops = PidOps(vec![
            "/usr/local/bin/claude".into(),
            "--allow-dangerously-skip-permissions".into(),
        ]);
        let mut st = AppState::default();
        st.create_workspace(Some("main"), Some("/work"), None, &ops)
            .unwrap();
        let sid = st.surfaces.keys().next().unwrap().clone();
        let agent = st
            .register_agent(
                &sid,
                Some("claude-code"),
                None,
                None,
                serde_json::json!({}),
                &ops,
            )
            .unwrap();
        let updated = st.set_agent_session_ref(&agent.id, "sess-1", &ops).unwrap();
        let resume = updated.resume.unwrap();
        assert_eq!(
            resume.argv,
            vec![
                "/usr/local/bin/claude",
                "--allow-dangerously-skip-permissions"
            ]
        );
        assert_eq!(resume.cwd, "/work");
        assert_eq!(resume.session_ref.as_deref(), Some("sess-1"));
    }

    #[test]
    fn given_a_wrapper_command_then_a_session_ref_keeps_the_saved_argv() {
        let ops = PidOps(vec!["/usr/local/bin/claude".into()]);
        let mut st = AppState::default();
        st.create_workspace(Some("main"), Some("/work"), None, &ops)
            .unwrap();
        let sid = st.surfaces.keys().next().unwrap().clone();
        let agent = st
            .register_agent(
                &sid,
                Some("claude-code"),
                None,
                Some(Resume {
                    argv: vec!["/opt/claude".into(), "--model".into(), "opus".into()],
                    cwd: "/saved".into(),
                    session_ref: None,
                    env: Vec::new(),
                }),
                serde_json::json!({}),
                &ops,
            )
            .unwrap();
        let updated = st.set_agent_session_ref(&agent.id, "sess-1", &ops).unwrap();
        let resume = updated.resume.unwrap();
        assert_eq!(resume.argv, vec!["/opt/claude", "--model", "opus"]);
        assert_eq!(resume.cwd, "/saved");
    }
}
