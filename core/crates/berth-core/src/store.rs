//! Session persistence: a versioned snapshot plus an append-only journal.
//!
//! Snapshot is authoritative; the journal exists so a crash loses at most the
//! last structural change, not the whole layout.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids;
use crate::model::{AgentSurface, Group, Orientation, Pane, PaneNode, Resume, Surface, Workspace};
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

/// Reject a snapshot that cannot be rebuilt before mutating live state. A
/// corrupt journal line should fall back to an older snapshot, not leave the
/// app half-restored or panic on a missing pane.
pub fn validate(snapshot: &Snapshot) -> Result<(), CoreError> {
    check_version(snapshot)?;
    let invalid = |message: String| CoreError::new(ErrorCode::BadRequest, message);

    let mut workspace_ids = std::collections::HashSet::new();
    let mut pane_ids = std::collections::HashSet::new();
    let mut surface_ids = std::collections::HashSet::new();
    let mut agent_ids = std::collections::HashSet::new();

    for wsnap in &snapshot.workspaces {
        let workspace = &wsnap.workspace;
        if !workspace_ids.insert(workspace.id.as_str()) {
            return Err(invalid(format!("duplicate workspace id {}", workspace.id)));
        }

        let leaves = workspace.root.leaves();
        if leaves.is_empty() {
            return Err(invalid(format!(
                "workspace {} has an empty pane tree",
                workspace.id
            )));
        }
        let mut tree_panes = std::collections::HashSet::new();
        for leaf in leaves {
            if !tree_panes.insert(leaf) {
                return Err(invalid(format!(
                    "duplicate pane id {} in workspace {}",
                    leaf, workspace.id
                )));
            }
            if !wsnap.panes.iter().any(|p| p.id == leaf) {
                return Err(invalid(format!(
                    "pane {} is in the tree but has no record",
                    leaf
                )));
            }
        }

        for pane in &wsnap.panes {
            if !pane_ids.insert(pane.id.as_str()) {
                return Err(invalid(format!("duplicate pane id {}", pane.id)));
            }
            if pane.workspace_id != workspace.id {
                return Err(invalid(format!(
                    "pane {} belongs to workspace {}, not {}",
                    pane.id, pane.workspace_id, workspace.id
                )));
            }
            if !tree_panes.contains(pane.id.as_str()) {
                return Err(invalid(format!(
                    "pane {} is not in the workspace tree",
                    pane.id
                )));
            }
            let mut tabs = std::collections::HashSet::new();
            for surface_id in &pane.surfaces {
                if !tabs.insert(surface_id) {
                    return Err(invalid(format!(
                        "duplicate surface {} in pane {}",
                        surface_id, pane.id
                    )));
                }
                let surface = wsnap
                    .surfaces
                    .iter()
                    .find(|s| &s.id == surface_id)
                    .ok_or_else(|| {
                        invalid(format!(
                            "pane {} references missing surface {}",
                            pane.id, surface_id
                        ))
                    })?;
                if surface.pane_id != pane.id || surface.workspace_id != workspace.id {
                    return Err(invalid(format!(
                        "surface {} does not belong to pane {}",
                        surface.id, pane.id
                    )));
                }
            }
        }

        for surface in &wsnap.surfaces {
            if !surface_ids.insert(surface.id.as_str()) {
                return Err(invalid(format!("duplicate surface id {}", surface.id)));
            }
            if surface.workspace_id != workspace.id {
                return Err(invalid(format!(
                    "surface {} belongs to workspace {}, not {}",
                    surface.id, surface.workspace_id, workspace.id
                )));
            }
            let pane = wsnap
                .panes
                .iter()
                .find(|p| p.id == surface.pane_id)
                .ok_or_else(|| {
                    invalid(format!(
                        "surface {} references missing pane {}",
                        surface.id, surface.pane_id
                    ))
                })?;
            if !pane.surfaces.contains(&surface.id) {
                return Err(invalid(format!(
                    "pane {} does not list surface {}",
                    pane.id, surface.id
                )));
            }
        }

        for agent in &wsnap.agents {
            if !agent_ids.insert(agent.id.as_str()) {
                return Err(invalid(format!("duplicate agent id {}", agent.id)));
            }
            if agent.workspace_id != workspace.id {
                return Err(invalid(format!(
                    "agent {} belongs to workspace {}, not {}",
                    agent.id, agent.workspace_id, workspace.id
                )));
            }
            if !surface_ids.contains(agent.surface_id.as_str()) {
                return Err(invalid(format!(
                    "agent {} references missing surface {}",
                    agent.id, agent.surface_id
                )));
            }
        }
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
    validate(&snapshot)?;
    let mut restored = 0;

    for wsnap in snapshot.workspaces {
        let crate::store::WorkspaceSnapshot {
            workspace: ws,
            panes,
            surfaces,
            agents,
        } = wsnap;
        let ws_id = ws.id.clone();
        let saved_active = ws.active_surface.clone();

        // Ensure every pane exists even if a surface failed to spawn. The
        // active tab index is part of the saved layout, so keep it.
        for pane in panes {
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

        let resume_by_surface: std::collections::HashMap<String, Resume> = agents
            .iter()
            .filter_map(|a| a.resume.clone().map(|r| (a.surface_id.clone(), r)))
            .collect();
        let agent_by_surface: std::collections::HashMap<String, String> = agents
            .iter()
            .map(|a| (a.surface_id.clone(), a.id.clone()))
            .collect();
        let mut failed_resumes = std::collections::HashSet::new();

        for mut surface in surfaces {
            let resume = resume_by_surface.get(&surface.id);
            let (argv, cwd) = match resume {
                Some(r) => {
                    let provider = agent_by_surface
                        .get(&surface.id)
                        .and_then(|id| agents.iter().find(|a| &a.id == id))
                        .and_then(|a| crate::providers::by_id(&a.provider));
                    let argv = match provider {
                        Some(p) if r.session_ref.is_some() && !p.resume.is_empty() => {
                            crate::providers::resume_argv(p, r.session_ref.as_deref(), &[])
                        }
                        _ => r.argv.clone(),
                    };
                    let cwd = if r.cwd.is_empty() {
                        surface.cwd.clone()
                    } else {
                        r.cwd.clone()
                    };
                    (argv, cwd)
                }
                None => (Vec::new(), surface.cwd.clone()),
            };
            let mut env = Vec::new();
            if let Some(resume) = resume {
                for (key, value) in &resume.env {
                    // Identity variables belong to the new generation, not the
                    // saved process.
                    if !key.starts_with("BERTH_") {
                        env.push((key.clone(), value.clone()));
                    }
                }
            }
            env.push(("BERTH_SURFACE_ID".into(), surface.id.clone()));
            env.push(("BERTH_WORKSPACE_ID".into(), ws_id.clone()));
            env.push((
                "BERTH_PROTOCOL".into(),
                crate::protocol::PROTOCOL_VERSION.to_string(),
            ));
            if let Some(agent_id) = agent_by_surface.get(&surface.id) {
                env.push(("BERTH_AGENT_ID".into(), agent_id.clone()));
            }
            let spawn_req = SpawnRequest {
                cwd: Some(cwd).filter(|c| !c.is_empty()),
                argv,
                env,
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
                    if resume.is_some() {
                        failed_resumes.insert(surface.id.clone());
                    }
                }
            }
            state.surfaces.insert(surface.id.clone(), surface);
        }

        for mut agent in agents {
            if failed_resumes.contains(&agent.surface_id) {
                agent.state = crate::agent::AgentState::Errored;
                agent.detail = "resume failed".to_string();
                agent.since_ms = time::now_ms();
            } else {
                agent.last_seen_ms = time::now_ms();
            }
            agent.attention = if failed_resumes.contains(&agent.surface_id) {
                crate::agent::Attention::Error
            } else {
                crate::agent::derive_attention(agent.state)
            };
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
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct RecordingOps {
        spawns: Arc<Mutex<Vec<SpawnRequest>>>,
        fail: bool,
    }

    impl SurfaceOps for RecordingOps {
        fn spawn(&self, req: SpawnRequest) -> Result<crate::Spawned, CoreError> {
            if self.fail {
                return Err(CoreError::internal("spawn failed"));
            }
            self.spawns.lock().unwrap().push(req.clone());
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
        fn pid_info(&self, a: &str) -> Result<crate::PidInfo, CoreError> {
            NullOps.pid_info(a)
        }
        fn notify_os(&self, n: &crate::Notification) -> Result<(), CoreError> {
            NullOps.notify_os(n)
        }
    }

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
            false,
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
    fn restore_resumes_agents_with_saved_argv_and_env() {
        let mut st = AppState::default();
        st.create_workspace(Some("agents"), Some("/tmp"), None, &NullOps)
            .unwrap();
        let sid = st.surfaces.keys().next().unwrap().clone();
        let agent = st
            .register_agent(
                &sid,
                Some("claude-code"),
                Some("worker"),
                Some(Resume {
                    argv: vec!["claude".into(), "--resume".into(), "abc".into()],
                    cwd: "/work/project".into(),
                    session_ref: None,
                    env: vec![("PATH".into(), "/opt/homebrew/bin:/usr/bin".into())],
                }),
                json!({}),
                &NullOps,
            )
            .unwrap();
        let snap = st.snapshot();

        let ops = RecordingOps::default();
        restore(&mut AppState::default(), snap, &ops).unwrap();
        let spawns = ops.spawns.lock().unwrap();
        assert_eq!(spawns.len(), 1);
        let spawn = &spawns[0];
        assert_eq!(spawn.argv, vec!["claude", "--resume", "abc"]);
        assert_eq!(spawn.cwd.as_deref(), Some("/work/project"));
        let env: std::collections::HashMap<_, _> = spawn.env.iter().cloned().collect();
        assert_eq!(env.get("BERTH_AGENT_ID"), Some(&agent.id));
        assert!(env.contains_key("BERTH_SURFACE_ID"));
        assert_eq!(
            env.get("PATH").map(String::as_str),
            Some("/opt/homebrew/bin:/usr/bin")
        );
    }

    #[test]
    fn failed_agent_resume_is_reported_instead_of_silently_shelling() {
        let mut st = AppState::default();
        st.create_workspace(Some("agents"), Some("/tmp"), None, &NullOps)
            .unwrap();
        let sid = st.surfaces.keys().next().unwrap().clone();
        st.register_agent(
            &sid,
            Some("codex"),
            Some("worker"),
            Some(Resume {
                argv: vec!["codex".into(), "resume".into(), "abc".into()],
                cwd: "/work/project".into(),
                session_ref: None,
                env: Vec::new(),
            }),
            json!({}),
            &NullOps,
        )
        .unwrap();
        let snap = st.snapshot();

        let ops = RecordingOps {
            fail: true,
            ..Default::default()
        };
        let mut restored = AppState::default();
        restore(&mut restored, snap, &ops).unwrap();
        let agent = restored.agents.values().next().unwrap();
        assert_eq!(agent.state, crate::agent::AgentState::Errored);
        assert_eq!(agent.detail, "resume failed");
    }

    #[test]
    fn wrong_version_is_rejected() {
        let mut st = AppState::default();
        let mut snap = st.snapshot();
        snap.version = 99;
        let err = restore(&mut st, snap, &NullOps).unwrap_err();
        assert_eq!(err.code, ErrorCode::Unsupported);
    }

    #[test]
    fn corrupt_snapshots_are_rejected_before_mutating_state() {
        let base = seeded().snapshot();
        validate(&base).unwrap();

        let mut missing_pane = base.clone();
        missing_pane.workspaces[0].panes.pop();
        let mut state = AppState::default();
        let err = restore(&mut state, missing_pane, &NullOps).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadRequest);
        assert!(state.workspaces.is_empty(), "validation must not mutate");

        let mut duplicate_surface = base.clone();
        let surface = duplicate_surface.workspaces[0].surfaces[0].clone();
        duplicate_surface.workspaces[0].surfaces.push(surface);
        assert_eq!(
            restore(&mut AppState::default(), duplicate_surface, &NullOps)
                .unwrap_err()
                .code,
            ErrorCode::BadRequest
        );

        let mut dangling_agent = base.clone();
        dangling_agent
            .workspaces
            .iter_mut()
            .find(|w| !w.agents.is_empty())
            .unwrap()
            .agents[0]
            .surface_id = "srf_missing".into();
        assert_eq!(
            restore(&mut AppState::default(), dangling_agent, &NullOps)
                .unwrap_err()
                .code,
            ErrorCode::BadRequest
        );
    }
}
