//! View models built from the core under one lock.
//!
//! Painting never reads `AppState`: everything it needs is copied into these
//! values first, so a frame cannot hold the state lock while it draws. The
//! ordering rules (pinned, then attention, then core order) mirror the macOS
//! sidebar; the pane rectangles come from [`crate::layout`].

use berth_core::model::AgentSurface;
use berth_core::{AppState, Attention, Surface};

use crate::layout::{self, Divider, Rect};
use crate::theme::Theme;

pub const PANEL_PAD: usize = 2;
pub const SIDEBAR_HEADER: usize = 30;
pub const TAB_HEIGHT: f64 = 28.0;
pub const GROUP_ROW_HEIGHT: f64 = 26.0;
pub const WORKSPACE_ROW_HEIGHT: f64 = 62.0;
pub const WORKSPACE_ROW_COMPACT: f64 = 52.0;

/// One terminal cell in view coordinates. Rows count from the top of the
/// visible screen; the window shifts them when the view scrolls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cell {
    pub row: u16,
    pub col: u16,
}

/// An inclusive selection between two cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Cell,
    pub head: Cell,
    /// Set while the button is held, so a drag keeps extending.
    pub dragging: bool,
}

impl Selection {
    pub fn new(cell: Cell) -> Self {
        Self {
            anchor: cell,
            head: cell,
            dragging: true,
        }
    }

    /// Start and end in reading order.
    pub fn span(&self) -> (Cell, Cell) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    pub fn contains(&self, cell: Cell) -> bool {
        let (start, end) = self.span();
        cell >= start && cell <= end
    }

    /// Keep the selection over the same text after the view scrolls by
    /// `delta` lines (positive scrolls up, revealing older rows). Both ends
    /// move together; the shift stops when either would leave the view.
    pub fn shift(&mut self, delta: i32, max_row: u16) {
        let (start, end) = self.span();
        let lower = -i32::from(start.row);
        let upper = i32::from(max_row) - i32::from(end.row);
        let delta = delta.clamp(lower, upper.max(lower));
        self.anchor.row = shift_row(self.anchor.row, delta, max_row);
        self.head.row = shift_row(self.head.row, delta, max_row);
    }

    /// Extend the head to `cell`, as a drag or a shift-click does.
    pub fn extend(&mut self, cell: Cell) {
        self.head = cell;
    }

    /// Select the word around `cell` on the same row.
    pub fn word_at(cell: Cell, text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let col = cell.col as usize;
        if chars.is_empty() || col >= chars.len() {
            return Self {
                anchor: cell,
                head: cell,
                dragging: false,
            };
        }
        let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '-' || c == '.';
        let mut start = col;
        while start > 0 && is_word(chars[start - 1]) {
            start -= 1;
        }
        let mut end = col;
        while end + 1 < chars.len() && is_word(chars[end + 1]) {
            end += 1;
        }
        Self {
            anchor: Cell {
                row: cell.row,
                col: start as u16,
            },
            head: Cell {
                row: cell.row,
                col: end as u16,
            },
            dragging: false,
        }
    }
}

fn shift_row(row: u16, delta: i32, max_row: u16) -> u16 {
    let shifted = i32::from(row) + delta;
    shifted.clamp(0, i32::from(max_row)) as u16
}

pub struct TabView {
    pub id: String,
    pub title: String,
    pub rect: Rect,
    pub active: bool,
    pub close: Rect,
}

pub struct PaneView {
    pub pane_id: String,
    pub surface: Option<String>,
    pub rect: Rect,
    pub content: Rect,
    pub focused: bool,
    pub tabs: Vec<TabView>,
    pub tab_bar: Rect,
    /// A newly arrived attention event flashes this pane's ring.
    pub attention_flash: bool,
}

pub struct AgentView {
    pub glyph: &'static str,
    pub name: String,
    pub detail: String,
    pub state_label: &'static str,
    pub color: u32,
    pub progress: Option<f32>,
}

pub struct WorkspaceView {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub unread: u32,
    pub attention: Attention,
    pub agent: Option<AgentView>,
    pub rect: Rect,
    pub active: bool,
}

pub enum RowKind {
    Group {
        id: String,
        name: String,
        collapsed: bool,
    },
    Workspace(WorkspaceView),
}

pub struct SidebarRow {
    pub kind: RowKind,
    pub rect: Rect,
}

/// The header button that creates a workspace, matching the macOS "+".
pub fn new_workspace_button(sidebar: Rect) -> Rect {
    Rect::new(
        sidebar.right().saturating_sub(30),
        1,
        28,
        SIDEBAR_HEADER.saturating_sub(2),
    )
}

/// The workspace whose tree is visible: the selection, else the first one.
pub fn active_workspace(state: &AppState) -> Option<String> {
    state
        .selected_workspace
        .clone()
        .filter(|id| state.workspace(id).is_some())
        .or_else(|| state.workspaces.first().map(|w| w.id.clone()))
}

/// Short, stable tab label. OSC titles win; the cwd's last component and a tab
/// number are the fallback.
pub fn tab_title(surface: &Surface, index: usize) -> String {
    if !surface.title.is_empty() {
        return surface.title.clone();
    }
    let base = std::path::Path::new(&surface.cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("shell");
    if index == 0 {
        base.to_string()
    } else {
        format!("{base} {}", index + 1)
    }
}

/// Workspaces in sidebar order: pinned first, then the attention rank, then
/// the order the core keeps.
pub fn ordered_workspaces(state: &AppState, ids: &[String]) -> Vec<String> {
    let mut rows: Vec<(usize, &str, bool, u8)> = ids
        .iter()
        .enumerate()
        .filter_map(|(index, id)| {
            let workspace = state.workspace(id)?;
            let rank = workspace_agent(state, id)
                .map(|agent| agent.attention.rank())
                .unwrap_or(0);
            Some((index, workspace.id.as_str(), workspace.pinned, rank))
        })
        .collect();
    rows.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then_with(|| b.3.cmp(&a.3))
            .then_with(|| a.0.cmp(&b.0))
    });
    rows.into_iter()
        .map(|(_, id, _, _)| id.to_string())
        .collect()
}

/// The agent a sidebar row reports: the active surface's, else the first.
pub fn workspace_agent<'a>(state: &'a AppState, workspace_id: &str) -> Option<&'a AgentSurface> {
    let active = state.active_surface_of_workspace(workspace_id);
    let mut fallback = None;
    for agent in state.agents.values() {
        if agent.workspace_id != workspace_id {
            continue;
        }
        if active.as_deref() == Some(agent.surface_id.as_str()) {
            return Some(agent);
        }
        fallback.get_or_insert(agent);
    }
    fallback
}

fn agent_view(agent: &AgentSurface, theme: &Theme) -> AgentView {
    AgentView {
        glyph: Theme::provider_glyph(&agent.provider),
        name: agent.name.clone(),
        detail: agent.detail.clone(),
        state_label: Theme::agent_state_label(agent.state.as_str()),
        color: theme.agent_state_color(agent.state.as_str()),
        progress: agent.progress,
    }
}

/// Abbreviate the home directory the way the macOS sidebar does.
pub fn abbreviate(path: &str) -> String {
    let Some(home) = berth_core::paths::home_dir() else {
        return path.to_string();
    };
    let home = home.to_string_lossy();
    if path == home {
        return "~".to_string();
    }
    match path.strip_prefix(&format!("{home}/")) {
        Some(rest) => format!("~/{rest}"),
        None => path.to_string(),
    }
}

/// Build the sidebar rows. Groups keep core order; each group's workspaces are
/// ordered by pinned/attention and hidden when the group is collapsed.
pub fn build_sidebar(
    state: &AppState,
    area: Rect,
    row_height: f64,
    theme: &Theme,
) -> Vec<SidebarRow> {
    let mut rows = Vec::new();
    let mut y = area.y.saturating_add(SIDEBAR_HEADER);
    let group_height = (GROUP_ROW_HEIGHT * row_height / WORKSPACE_ROW_HEIGHT) as usize;
    let workspace_height = row_height as usize;

    let push_group =
        |rows: &mut Vec<SidebarRow>, y: &mut usize, id: &str, name: &str, collapsed: bool| {
            let rect = Rect::new(area.x, *y, area.w, group_height);
            rows.push(SidebarRow {
                kind: RowKind::Group {
                    id: id.to_string(),
                    name: name.to_string(),
                    collapsed,
                },
                rect,
            });
            *y = y.saturating_add(group_height);
        };

    let workspace_row = |id: &str, y: usize, height: usize| -> Option<SidebarRow> {
        let workspace = state.workspace(id)?;
        let agent = workspace_agent(state, id);
        let live = workspace
            .active_surface
            .as_deref()
            .and_then(|surface_id| state.surface(surface_id))
            .map(|surface| surface.title.trim().to_string())
            .unwrap_or_default();
        let base = if live.is_empty() {
            workspace.name.clone()
        } else {
            live
        };
        let label = if workspace.pinned {
            format!("★ {base}")
        } else {
            base
        };
        let mut parts = Vec::new();
        if !workspace.name.is_empty() {
            parts.push(workspace.name.clone());
        }
        let cwd = workspace
            .active_surface
            .as_deref()
            .and_then(|surface_id| state.surface(surface_id))
            .map(|surface| surface.cwd.clone())
            .unwrap_or_default();
        if !cwd.is_empty() {
            parts.push(abbreviate(&cwd));
        }
        Some(SidebarRow {
            kind: RowKind::Workspace(WorkspaceView {
                id: workspace.id.clone(),
                label,
                detail: parts.join("  "),
                unread: workspace.unread,
                attention: agent.map(|agent| agent.attention).unwrap_or_default(),
                agent: agent.map(|agent| agent_view(agent, theme)),
                rect: Rect::new(area.x, y, area.w, height),
                active: state.selected_workspace.as_deref() == Some(id),
            }),
            rect: Rect::new(area.x, y, area.w, height),
        })
    };

    for group in &state.groups {
        push_group(&mut rows, &mut y, &group.id, &group.name, group.collapsed);
        if group.collapsed {
            continue;
        }
        let ids: Vec<String> = state
            .workspaces
            .iter()
            .filter(|workspace| workspace.group_id.as_deref() == Some(group.id.as_str()))
            .map(|workspace| workspace.id.clone())
            .collect();
        for id in ordered_workspaces(state, &ids) {
            if y + workspace_height > area.bottom() {
                break;
            }
            if let Some(row) = workspace_row(&id, y, workspace_height) {
                y = y.saturating_add(workspace_height);
                rows.push(row);
            }
        }
    }

    let grouped: Vec<&str> = state.groups.iter().map(|group| group.id.as_str()).collect();
    let loose: Vec<String> = state
        .workspaces
        .iter()
        .filter(|workspace| match workspace.group_id.as_deref() {
            Some(group_id) => !grouped.contains(&group_id),
            None => true,
        })
        .map(|workspace| workspace.id.clone())
        .collect();
    for id in ordered_workspaces(state, &loose) {
        if y + workspace_height > area.bottom() {
            break;
        }
        if let Some(row) = workspace_row(&id, y, workspace_height) {
            y = y.saturating_add(workspace_height);
            rows.push(row);
        }
    }
    rows
}

/// The visible workspace's panes, their tab strips and the draggable dividers.
pub fn build_panes(
    state: &AppState,
    area: Rect,
    cell_height: usize,
    tab_height: usize,
    flash: Option<&str>,
) -> (Vec<PaneView>, Vec<Divider>) {
    let Some(workspace_id) = active_workspace(state) else {
        return (Vec::new(), Vec::new());
    };
    let Some(workspace) = state.workspace(&workspace_id) else {
        return (Vec::new(), Vec::new());
    };
    let active = state
        .selected_surface
        .clone()
        .filter(|id| {
            state
                .surface(id)
                .is_some_and(|surface| surface.workspace_id == workspace.id)
        })
        .or_else(|| workspace.active_surface.clone());
    let (leaves, dividers) = layout::layout(&workspace.root, area);
    let panes = leaves
        .into_iter()
        .map(|leaf| {
            let pane_id = leaf.pane_id;
            let mut content = leaf.rect.inset(PANEL_PAD);
            let bar = Rect::new(content.x, content.y, content.w, tab_height.min(content.h));
            content.y = content.y.saturating_add(bar.h);
            content.h = content.h.saturating_sub(bar.h);
            let mut tabs = Vec::new();
            let mut surface = None;
            if let Some(pane) = state.panes.get(&pane_id) {
                surface = pane.active_surface().map(str::to_owned);
                let mut x = bar.x;
                let count = pane.surfaces.len().max(1);
                let share = bar.w / count;
                for (index, surface_id) in pane.surfaces.iter().enumerate() {
                    let Some(tab_surface) = state.surface(surface_id) else {
                        continue;
                    };
                    let remaining = bar.right().saturating_sub(x);
                    if remaining < 16 {
                        break;
                    }
                    let title = tab_title(tab_surface, index);
                    let text_width = title.chars().count() * cell_height / 2;
                    // Tabs share the strip equally and truncate their label, so
                    // a long OSC title cannot hide the tabs after it.
                    let width = (text_width + 34).min(share.max(48)).min(remaining);
                    let close = Rect::new(
                        x + width.saturating_sub(cell_height + 6),
                        bar.y + (bar.h.saturating_sub(cell_height) / 2),
                        cell_height,
                        cell_height,
                    );
                    tabs.push(TabView {
                        id: surface_id.clone(),
                        title,
                        rect: Rect::new(x, bar.y, width, bar.h),
                        active: index == pane.active,
                        close,
                    });
                    x = x.saturating_add(width);
                }
            }
            let focused = surface.is_some() && surface.as_deref() == active.as_deref();
            let attention_flash = surface.as_deref() == flash;
            PaneView {
                pane_id,
                surface,
                rect: leaf.rect,
                content,
                focused,
                tabs,
                tab_bar: bar,
                attention_flash,
            }
        })
        .collect();
    (panes, dividers)
}

/// The pane that contains a point, for click routing.
pub fn pane_at(panes: &[PaneView], x: usize, y: usize) -> Option<&PaneView> {
    panes.iter().find(|pane| pane.rect.contains(x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use berth_core::model::{Orientation, PaneNode};

    #[test]
    fn given_cells_in_two_orders_when_spanned_then_the_span_is_the_same() {
        let forward = Selection {
            anchor: Cell { row: 1, col: 2 },
            head: Cell { row: 3, col: 4 },
            dragging: false,
        };
        let backward = Selection {
            anchor: Cell { row: 3, col: 4 },
            head: Cell { row: 1, col: 2 },
            dragging: false,
        };
        assert_eq!(forward.span(), backward.span());
        assert_eq!(
            forward.span(),
            (Cell { row: 1, col: 2 }, Cell { row: 3, col: 4 })
        );
    }

    #[test]
    fn given_a_selection_when_a_cell_is_tested_then_membership_is_inclusive() {
        let selection = Selection {
            anchor: Cell { row: 0, col: 1 },
            head: Cell { row: 0, col: 3 },
            dragging: false,
        };
        assert!(selection.contains(Cell { row: 0, col: 1 }));
        assert!(selection.contains(Cell { row: 0, col: 3 }));
        assert!(!selection.contains(Cell { row: 0, col: 4 }));
        assert!(!selection.contains(Cell { row: 1, col: 2 }));
    }

    #[test]
    fn given_a_scroll_when_the_selection_shifts_then_it_tracks_the_text() {
        let mut selection = Selection {
            anchor: Cell { row: 2, col: 0 },
            head: Cell { row: 4, col: 5 },
            dragging: false,
        };
        selection.shift(3, 20);
        assert_eq!(selection.anchor.row, 5);
        assert_eq!(selection.head.row, 7);
        selection.shift(-50, 20);
        assert_eq!(selection.anchor.row, 0);
        assert_eq!(selection.head.row, 2);
    }

    #[test]
    fn given_a_column_in_a_command_when_the_word_is_selected_then_it_includes_the_flag() {
        let selection = Selection::word_at(Cell { row: 0, col: 3 }, "cargo test --workspace");
        assert_eq!(selection.anchor.col, 0);
        assert_eq!(selection.head.col, 4);
    }

    #[test]
    fn given_a_punctuated_path_when_a_word_is_selected_then_the_path_is_one_word() {
        let selection = Selection::word_at(Cell { row: 0, col: 8 }, "open src/lib.rs now");
        assert_eq!(selection.anchor.col, 5);
        assert_eq!(selection.head.col, 14);
    }

    #[test]
    fn given_no_title_when_labelled_then_the_tab_uses_the_cwd_basename() {
        let mut surface = surface("", "/home/me/project");
        assert_eq!(tab_title(&surface, 0), "project");
        assert_eq!(tab_title(&surface, 1), "project 2");
        surface.title = "tests".into();
        assert_eq!(tab_title(&surface, 0), "tests");
        surface.title.clear();
        surface.cwd.clear();
        assert_eq!(tab_title(&surface, 0), "shell");
    }

    fn surface(title: &str, cwd: &str) -> Surface {
        Surface {
            id: "srf_test".into(),
            pane_id: "pne_test".into(),
            workspace_id: "wsp_test".into(),
            kind: berth_core::model::SurfaceKind::Terminal,
            title: title.into(),
            cwd: cwd.into(),
            pid: None,
            tty: None,
            cols: 80,
            rows: 24,
            progress: None,
            bell_count: 0,
            agent_id: None,
            created_ms: 0,
        }
    }

    #[test]
    fn given_no_workspace_when_building_panes_then_none_are_returned() {
        let state = AppState::default();
        let (panes, dividers) = build_panes(&state, Rect::new(0, 0, 100, 100), 16, 28, None);
        assert!(panes.is_empty());
        assert!(dividers.is_empty());
    }

    #[test]
    fn given_a_split_workspace_when_building_panes_then_each_leaf_has_a_tab_strip() {
        let mut state = AppState::default();
        let workspace = state
            .create_workspace(Some("main"), None, None, &berth_core::NullOps)
            .unwrap();
        let leaf = workspace.root.leaves()[0].to_string();
        let mut root = workspace.root.clone();
        assert!(matches!(root, PaneNode::Leaf { .. }));
        assert!(root.split(&leaf, "pne_second", Orientation::Horizontal));
        if let Some(ws) = state.workspace_mut(&workspace.id) {
            ws.root = root;
        }
        let (panes, dividers) = build_panes(&state, Rect::new(0, 0, 200, 100), 16, 28, None);
        assert_eq!(panes.len(), 2);
        assert_eq!(dividers.len(), 1);
        for pane in &panes {
            assert_eq!(pane.tab_bar.h, 28);
            assert_eq!(pane.content.y, pane.tab_bar.bottom());
        }
    }

    fn workspace_with_agent(
        state: &mut AppState,
        name: &str,
        agent_state: berth_core::AgentState,
    ) -> String {
        let workspace = state
            .create_workspace(Some(name), None, None, &berth_core::NullOps)
            .unwrap();
        let surface = workspace.active_surface.clone().unwrap();
        let agent = state
            .register_agent(
                &surface,
                Some("claude-code"),
                Some("Claude"),
                None,
                serde_json::Value::Null,
                &berth_core::NullOps,
            )
            .unwrap();
        state
            .update_agent(
                &agent.id,
                Some(agent_state),
                Some("detail".to_string()),
                None,
                None,
                &berth_core::NullOps,
            )
            .unwrap();
        workspace.id
    }

    #[test]
    fn given_two_workspaces_when_one_needs_attention_then_it_is_listed_first() {
        let mut state = AppState::default();
        let _quiet = workspace_with_agent(&mut state, "quiet", berth_core::AgentState::Thinking);
        let loud = workspace_with_agent(&mut state, "loud", berth_core::AgentState::WaitingInput);
        let rows = build_sidebar(&state, Rect::new(0, 0, 200, 400), 62.0, &Theme::default());
        let first = rows.iter().find_map(|row| match &row.kind {
            RowKind::Workspace(workspace) => Some(workspace.id.clone()),
            RowKind::Group { .. } => None,
        });
        assert_eq!(first.as_deref(), Some(loud.as_str()));
    }

    #[test]
    fn given_a_pinned_workspace_when_listed_then_it_leads_the_loose_rows() {
        let mut state = AppState::default();
        let plain = state
            .create_workspace(Some("plain"), None, None, &berth_core::NullOps)
            .unwrap();
        let pinned = state
            .create_workspace(Some("pinned"), None, None, &berth_core::NullOps)
            .unwrap();
        if let Some(workspace) = state.workspace_mut(&pinned.id) {
            workspace.pinned = true;
        }
        let rows = build_sidebar(&state, Rect::new(0, 0, 200, 400), 62.0, &Theme::default());
        let ids: Vec<String> = rows
            .iter()
            .filter_map(|row| match &row.kind {
                RowKind::Workspace(workspace) => Some(workspace.id.clone()),
                RowKind::Group { .. } => None,
            })
            .collect();
        assert_eq!(ids.first().map(String::as_str), Some(pinned.id.as_str()));
        assert_eq!(ids.last().map(String::as_str), Some(plain.id.as_str()));
    }

    #[test]
    fn given_a_collapsed_group_when_listed_then_its_workspaces_are_hidden() {
        let mut state = AppState::default();
        let group = state.create_group("clients", None);
        let workspace = state
            .create_workspace(
                Some("inside"),
                None,
                Some(group.id.clone()),
                &berth_core::NullOps,
            )
            .unwrap();
        state
            .update_group(&group.id, None, None, Some(true))
            .unwrap();
        let rows = build_sidebar(&state, Rect::new(0, 0, 200, 400), 62.0, &Theme::default());
        assert_eq!(rows.len(), 1);
        assert!(matches!(
            rows[0].kind,
            RowKind::Group {
                collapsed: true,
                ..
            }
        ));
        state
            .update_group(&group.id, None, None, Some(false))
            .unwrap();
        let rows = build_sidebar(&state, Rect::new(0, 0, 200, 400), 62.0, &Theme::default());
        assert_eq!(rows.len(), 2);
        assert!(matches!(&rows[1].kind, RowKind::Workspace(w) if w.id == workspace.id));
    }

    #[test]
    fn given_no_workspace_when_the_sidebar_is_built_then_it_is_empty() {
        let state = AppState::default();
        let rows = build_sidebar(&state, Rect::new(0, 0, 200, 400), 62.0, &Theme::default());
        assert!(rows.is_empty());
    }

    #[test]
    fn given_a_path_under_home_when_abbreviated_then_the_home_prefix_is_shortened() {
        let Some(home) = berth_core::paths::home_dir() else {
            return;
        };
        let home = home.to_string_lossy().to_string();
        assert_eq!(abbreviate(&format!("{home}/projects")), "~/projects");
        assert_eq!(abbreviate("/tmp/other"), "/tmp/other");
    }
}
