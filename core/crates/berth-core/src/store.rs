//! Session persistence: a versioned snapshot plus an append-only journal.
//!
//! Snapshot is authoritative; the journal exists so a crash loses at most the
//! last structural change, not the whole layout.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids;
use crate::model::{AgentSurface, Group, Orientation, Pane, PaneNode, Surface, Workspace};
use crate::state::AppState;
use crate::time;
use crate::{CoreError, ErrorCode, SpawnRequest, SurfaceOps};

pub const SESSION_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub workspace: Workspace,
    pub panes: Vec<Pane>,
    pub surfaces: Vec<Surface>,
    #[serde(default)]
    pub agents: Vec<AgentSurface>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub saved_ms: u64,
    #[serde(default)]
    pub selected_workspace: Option<String>,
    #[serde(default)]
    pub selected_surface: Option<String>,
    #[serde(default)]
    pub groups: Vec<Group>,
    pub workspaces: Vec<WorkspaceSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalLine {
    pub ts_ms: u64,
    pub op: String,
    #[serde(default)]
    pub data: Value,
}

pub fn snapshot_path() -> String {
    // The shell supplies the real path; this is the documented default.
    "~/Library/Application Support/Berth/session.json".to_string()
}

impl AppState {
    pub fn snapshot(&self) -> Snapshot {
        let workspaces = self
            .workspaces
            .iter()
            .map(|w| {
                let pane_ids = w.root.leaves();
                let panes: Vec<Pane> = pane_ids
                    .iter()
                    .filter_map(|p| self.panes.get(*p).cloned())
                    .collect();
                let surfaces: Vec<Surface> = self
                    .surfaces
                    .values()
                    .filter(|s| s.workspace_id == w.id)
                    .cloned()
                    .collect();
                let agents: Vec<AgentSurface> = self
                    .agents
                    .values()
                    .filter(|a| a.workspace_id == w.id)
                    .cloned()
                    .collect();
                WorkspaceSnapshot {
                    workspace: w.clone(),
                    panes,
                    surfaces,
                    agents,
                }
            })
            .collect();

        Snapshot {
            version: SESSION_VERSION,
            saved_ms: time::now_ms(),
            selected_workspace: self.selected_workspace.clone(),
            selected_surface: self.selected_surface.clone(),
            groups: self.groups.clone(),
            workspaces,
        }
    }
}

/// Version check before any restore work happens.
pub fn check_version(snapshot: &Snapshot) -> Result<(), CoreError> {
    if snapshot.version != SESSION_VERSION {
        return Err(CoreError::new(
            ErrorCode::Unsupported,
            format!(
                "session version {} is not supported (expected {})",
                snapshot.version, SESSION_VERSION
            ),
        ));
    }
    Ok(())
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            version: SESSION_VERSION,
            saved_ms: 0,
            selected_workspace: None,
            selected_surface: None,
            groups: Vec::new(),
            workspaces: Vec::new(),
        }
    }
}

/// Rebuild a session. Terminals are respawned (scrollback is not restored);
/// agents are re-registered with their resume payload so the shell can offer
/// or perform a resume.
pub fn restore(
    state: &mut AppState,
    snapshot: Snapshot,
    ops: &dyn SurfaceOps,
) -> Result<usize, CoreError> {
    check_version(&snapshot)?;
    let mut restored = 0;

    for wsnap in snapshot.workspaces {
        let ws = wsnap.workspace;
        let ws_id = ws.id.clone();
        let saved_active = ws.active_surface.clone();

        // Ensure every pane exists even if a surface failed to spawn. The
        // active tab index is part of the saved layout, so keep it.
        for pane in &wsnap.panes {
            let mut pane = pane.clone();
            if pane.active >= pane.surfaces.len() {
                pane.active = 0;
            }
            state.panes.insert(pane.id.clone(), pane);
        }
        state.workspaces.push(Workspace {
            active_surface: None,
            root: ws.root.clone(),
            ..ws.clone()
        });

        for mut surface in wsnap.surfaces {
            let spawn_req = SpawnRequest {
                cwd: Some(surface.cwd.clone()).filter(|c| !c.is_empty()),
                argv: Vec::new(),
                env: vec![
                    ("BERTH_SURFACE_ID".into(), surface.id.clone()),
                    ("BERTH_WORKSPACE_ID".into(), ws_id.clone()),
                ],
                cols: surface.cols.max(80),
                rows: surface.rows.max(24),
            };
            match ops.spawn(spawn_req) {
                Ok(spawned) => {
                    surface.pid = if spawned.pid == 0 {
                        None
                    } else {
                        Some(spawned.pid)
                    };
                    surface.tty = if spawned.tty.is_empty() {
                        None
                    } else {
                        Some(spawned.tty)
                    };
                    restored += 1;
                }
                Err(_) => {
                    // Keep the pane in the tree; the surface becomes a dead tab
                    // the user can close, rather than silently losing layout.
                    surface.pid = None;
                    surface.tty = None;
                }
            }
            state.surfaces.insert(surface.id.clone(), surface);
        }

        for mut agent in wsnap.agents {
            agent.attention = crate::agent::derive_attention(agent.state);
            state.agents.insert(agent.id.clone(), agent.clone());
            if let Some(s) = state.surface_mut(&agent.surface_id) {
                s.agent_id = Some(agent.id);
            }
        }

        let fallback = state
            .workspace(&ws_id)
            .and_then(|w| w.root.leaves().first().map(|p| p.to_string()))
            .and_then(|p| {
                state
                    .panes
                    .get(&p)
                    .and_then(|pane| pane.active_surface().map(str::to_string))
            });
        let active = saved_active
            .filter(|id| state.surfaces.contains_key(id))
            .or(fallback);
        if let Some(w) = state.workspace_mut(&ws_id) {
            w.active_surface = active;
        }
    }

    if snapshot.selected_workspace.is_some()
        && snapshot
            .selected_workspace
            .as_deref()
            .and_then(|id| state.workspace(id))
            .is_some()
    {
        state.selected_workspace = snapshot.selected_workspace;
    } else {
        state.selected_workspace = state.workspaces.first().map(|w| w.id.clone());
    }
    state.groups = snapshot.groups;

    let selected_ws = state.selected_workspace.clone();
    state.selected_surface = snapshot
        .selected_surface
        .filter(|id| {
            state
                .surface(id)
                .is_some_and(|s| selected_ws.as_deref().is_none_or(|w| s.workspace_id == w))
        })
        .or_else(|| {
            selected_ws
                .as_deref()
                .and_then(|w| state.active_surface_of_workspace(w))
        });
    Ok(restored)
}

/// An empty workspace, used by `berth new` and first launch.
pub fn ensure_default_workspace(
    state: &mut AppState,
    ops: &dyn SurfaceOps,
) -> Result<(), CoreError> {
    if !state.workspaces.is_empty() {
        return Ok(());
    }
    state.create_workspace(Some("main"), dirs_fallback().as_deref(), None, ops)?;
    Ok(())
}

fn dirs_fallback() -> Option<String> {
    std::env::var("HOME").ok()
}

/// A fresh tree with a single pane, for tests and restore-of-empty.
pub fn single_pane_tree() -> PaneNode {
    PaneNode::Split {
        id: ids::mint(ids::PANE),
        orientation: Orientation::Horizontal,
        children: vec![PaneNode::leaf(ids::mint(ids::PANE))],
        weights: vec![1.0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NullOps;
    use serde_json::json;

    fn seeded() -> AppState {
        let mut st = AppState::default();
        st.create_workspace(Some("one"), Some("/tmp"), None, &NullOps)
            .unwrap();
        st.create_workspace(Some("two"), Some("/tmp"), None, &NullOps)
            .unwrap();
        let sid = st.surfaces.keys().next().unwrap().clone();
        st.split_surface(
            &sid,
            Orientation::Horizontal,
            SpawnRequest {
                cwd: Some("/tmp".into()),
                ..Default::default()
            },
            false,
            &NullOps,
        )
        .unwrap();
        st.register_agent(
            &sid,
            Some("claude-code"),
            Some("a"),
            None,
            json!({}),
            &NullOps,
        )
        .unwrap();
        st
    }

    #[test]
    fn snapshot_roundtrips_through_json() {
        let st = seeded();
        let snap = st.snapshot();
        let text = serde_json::to_string_pretty(&snap).unwrap();
        let back: Snapshot = serde_json::from_str(&text).unwrap();
        assert_eq!(back.version, SESSION_VERSION);
        assert_eq!(back.workspaces.len(), 2);
        assert_eq!(back.workspaces[1].workspace.name, "two");
        assert!(back.workspaces.iter().any(|w| !w.agents.is_empty()));
    }

    #[test]
    fn restore_rebuilds_layout_and_agents() {
        let st = seeded();
        let snap = st.snapshot();
        let surfaces_before = st.surfaces.len();

        let mut fresh = AppState::default();
        let n = restore(&mut fresh, snap, &NullOps).unwrap();
        assert_eq!(n, surfaces_before);
        assert_eq!(fresh.workspaces.len(), 2);
        assert_eq!(fresh.agents.len(), 1);
        assert_eq!(fresh.surfaces.len(), surfaces_before);
        // agents stay bound to their surface
        let agent = fresh.agents.values().next().unwrap();
        assert_eq!(
            fresh
                .surface(&agent.surface_id)
                .unwrap()
                .agent_id
                .as_deref(),
            Some(agent.id.as_str())
        );
    }

    #[test]
    fn restore_keeps_active_tab_and_selection() {
        let mut st = AppState::default();
        let ws = st
            .create_workspace(Some("tabs"), Some("/tmp"), None, &NullOps)
            .unwrap();
        let pane_id = ws.root.leaves()[0].to_string();
        let first = st
            .panes
            .get(&pane_id)
            .and_then(|p| p.active_surface())
            .unwrap()
            .to_string();
        st.add_surface(
            &pane_id,
            SpawnRequest {
                cwd: Some("/tmp".into()),
                ..Default::default()
            },
            &NullOps,
            true,
        )
        .unwrap();
        st.activate_surface(&first, &NullOps).unwrap();
        let snap = st.snapshot();
        assert_eq!(snap.selected_surface.as_deref(), Some(first.as_str()));
        assert_eq!(
            snap.workspaces[0].panes[0].active,
            snap.workspaces[0].panes[0]
                .surfaces
                .iter()
                .position(|s| s == &first)
                .unwrap()
        );

        let mut fresh = AppState::default();
        restore(&mut fresh, snap, &NullOps).unwrap();
        assert_eq!(
            fresh.workspace(&ws.id).unwrap().active_surface.as_deref(),
            Some(first.as_str())
        );
        assert_eq!(fresh.selected_surface.as_deref(), Some(first.as_str()));
        assert_eq!(fresh.panes[&pane_id].active_surface(), Some(first.as_str()));
    }

    #[test]
    fn wrong_version_is_rejected() {
        let mut st = AppState::default();
        let mut snap = st.snapshot();
        snap.version = 99;
        let err = restore(&mut st, snap, &NullOps).unwrap_err();
        assert_eq!(err.code, ErrorCode::Unsupported);
    }
}
