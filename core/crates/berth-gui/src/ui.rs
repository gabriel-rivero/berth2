//! View models built from the core under one lock.
//!
//! Painting never reads `AppState`: everything it needs is copied into these
//! values first, so a frame cannot hold the state lock while it draws. The
//! ordering rules (pinned, then attention, then core order) mirror the macOS
//! sidebar; the pane rectangles come from [`crate::layout`].

use std::collections::{HashMap, HashSet};

use berth_core::model::AgentSurface;
use berth_core::{AppState, Attention, Surface};

use crate::layout::{self, Divider, Rect};
use crate::theme::Theme;

pub const PANEL_PAD: usize = 2;
/// The sidebar header height in logical points. It is scaled by the window
/// before layout, like the row heights: the chrome font is sized in physical
/// pixels, so a fixed physical header would be shorter than one line of its own
/// label at a 2× scale and the first row would paint over it.
pub const SIDEBAR_HEADER: f64 = 24.0;
pub const TAB_HEIGHT: f64 = 28.0;
pub const GROUP_ROW_HEIGHT: f64 = 26.0;
pub const WORKSPACE_ROW_HEIGHT: f64 = 62.0;
pub const WORKSPACE_ROW_COMPACT: f64 = 52.0;

/// Header height in physical pixels for a window at `scale`.
pub fn sidebar_header(scale: f64) -> usize {
    (SIDEBAR_HEADER * scale).round() as usize
}

/// The sidebar's vertical measurements, both in physical pixels.
///
/// They travel together because both are logical sizes multiplied by the
/// window scale; passing them separately let the header stay unscaled while the
/// rows grew, which painted the first row over the header label.
#[derive(Clone, Copy, Debug)]
pub struct SidebarLayout {
    pub header: usize,
    pub row_height: f64,
}

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
        let mut start = col;
        while start > 0 && is_word_char(chars[start - 1]) {
            start -= 1;
        }
        let mut end = col;
        while end + 1 < chars.len() && is_word_char(chars[end + 1]) {
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

/// The characters a double-click selects as one word. Whole-word search uses
/// the same class, so `value` is one word while `value.` is a word and a dot.
pub fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '.'
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
    /// The prompt gutter left of `content`, where OSC 133 marks are drawn.
    pub gutter: Rect,
    pub focused: bool,
    pub tabs: Vec<TabView>,
    pub tab_bar: Rect,
    /// A newly arrived attention event flashes this pane's ring.
    pub attention_flash: bool,
    /// The overflow button when the tab strip cannot show every tab.
    pub overflow: Option<Rect>,
}

/// The width of the prompt gutter, or zero when the pane is too narrow to
/// spend it. It is part of the layout rather than an overlay, so the grid, the
/// pointer mapping and inline images all agree on where the text starts.
fn gutter_width(cell_height: usize, content_width: usize) -> usize {
    let width = (cell_height / 2).max(4);
    if content_width > width + 8 { width } else { 0 }
}

/// Per-frame inputs that come from the shell rather than the core.
#[derive(Default, Clone, Copy)]
pub struct PaneOptions<'a> {
    /// Maximise this pane over the workspace.
    pub zoom: Option<&'a str>,
    /// First visible tab per pane while its strip is scrolled.
    pub tab_scroll: Option<&'a HashMap<String, usize>>,
    /// Surfaces whose program emits OSC 133 marks. Only those panes spend
    /// columns on the prompt gutter, so a shell without integration keeps the
    /// full grid.
    pub prompt_marks: Option<&'a HashSet<String>>,
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
    pub color: Option<String>,
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
        color: Option<String>,
    },
    Workspace(WorkspaceView),
    /// One agent under its workspace, so a workspace with several agents gets
    /// a row each instead of one summary line.
    Agent {
        workspace_id: String,
        surface_id: String,
        agent: AgentView,
    },
}

pub struct SidebarRow {
    pub kind: RowKind,
    pub rect: Rect,
}

/// Insertion index for a tab drag at `x`: the tab whose right edge passes the
/// pointer, else the last slot.
pub fn tab_index_at(tabs: &[TabView], x: usize) -> usize {
    tabs.iter()
        .position(|tab| x < tab.rect.x + tab.rect.w)
        .unwrap_or(tabs.len().saturating_sub(1))
}

/// The rendered sidebar plus how many rows exist in total, for scroll limits.
pub struct Sidebar {
    pub rows: Vec<SidebarRow>,
    pub total: usize,
}

/// The header button that creates a workspace, matching the macOS "+".
///
/// Both header buttons are sized from the scaled header height so they grow
/// with the icon inside them; a fixed box left the glyph looking pinched on a
/// Retina display.
pub fn new_workspace_button(sidebar: Rect, header: usize) -> Rect {
    let size = button_size(header);
    Rect::new(
        sidebar.right().saturating_sub(size + 2),
        0,
        size,
        header.max(size),
    )
}

/// The header button that creates a group, left of the workspace "+".
pub fn new_group_button(sidebar: Rect, header: usize) -> Rect {
    let size = button_size(header);
    Rect::new(
        sidebar.right().saturating_sub(size * 2 + 6),
        0,
        size,
        header.max(size),
    )
}

/// One header button's side: a square that fits the header, floored so the
/// icon keeps a usable hit target at a 1× scale.
fn button_size(header: usize) -> usize {
    (header.saturating_sub(4)).max(20)
}

/// The square a header button's icon is drawn inside, centred in the button.
pub fn button_icon(button: Rect) -> Rect {
    let side = (button.w.min(button.h) * 2 / 5).max(7);
    Rect::new(
        button.x + button.w.saturating_sub(side) / 2,
        button.y + button.h.saturating_sub(side) / 2,
        side,
        side,
    )
}

/// Parse a stored row colour (`#rrggbb` or a few names) to RGB.
pub fn parse_color(value: Option<&str>) -> Option<u32> {
    let text = value?.trim();
    if let Some(hex) = text.strip_prefix('#')
        && hex.len() == 6
    {
        return u32::from_str_radix(hex, 16).ok();
    }
    Some(match text.to_ascii_lowercase().as_str() {
        "orange" => 0x00e8_b06a,
        "green" => 0x008b_d5a6,
        "blue" => 0x007f_b4ff,
        "red" => 0x00e0_6c75,
        "purple" => 0x00c7_92ea,
        _ => return None,
    })
}

/// The workspace whose tree is visible: the selection, else the first one.
pub fn active_workspace(state: &AppState) -> Option<String> {
    state
        .selected_workspace
        .clone()
        .filter(|id| state.workspace(id).is_some())
        .or_else(|| state.workspaces.first().map(|w| w.id.clone()))
}

/// The workspace a given window is showing. Reads the window's own record, so
/// a palette or sidebar built for a background window cannot report the
/// focused window's selection. `None` falls back to the global selection.
pub fn active_workspace_in(state: &AppState, window_id: Option<&str>) -> Option<String> {
    let Some(window_id) = window_id else {
        return active_workspace(state);
    };
    let workspace = state
        .window(window_id)
        .and_then(|window| window.active_workspace.clone())
        .filter(|id| {
            state
                .workspace(id)
                .is_some_and(|workspace| in_window(workspace, Some(window_id)))
        });
    workspace.or_else(|| {
        state
            .workspaces
            .iter()
            .find(|workspace| in_window(workspace, Some(window_id)))
            .map(|workspace| workspace.id.clone())
    })
}

/// The surface the visible workspace wants shown, else the last selection.
/// Takes the state by reference so a caller already holding the lock can read
/// it without locking again.
pub fn active_surface(state: &AppState) -> Option<String> {
    let workspace_id = active_workspace(state)?;
    state
        .active_surface_of_workspace(&workspace_id)
        .or_else(|| state.selected_surface.clone())
}

/// Short, stable tab label. OSC titles win; the cwd's last component and a tab
/// number are the fallback.
pub fn tab_title(surface: &Surface, index: usize) -> String {
    if let Some(title) = &surface.user_title {
        return title.clone();
    }
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
    let mut detail = agent.detail.clone();
    if agent.tokens_in + agent.tokens_out > 0 {
        let tokens = agent.tokens_in + agent.tokens_out;
        if !detail.is_empty() {
            detail.push_str(" · ");
        }
        detail.push_str(&format!("{tokens} tok"));
    }
    if agent.cost_usd > 0.0 {
        if !detail.is_empty() {
            detail.push_str(" · ");
        }
        detail.push_str(&format!("${:.2}", agent.cost_usd));
    }
    AgentView {
        glyph: Theme::provider_glyph(&agent.provider),
        name: agent.name.clone(),
        detail,
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
/// Kept for the tests and any caller that does not filter.
#[cfg_attr(not(test), allow(dead_code))]
pub fn build_sidebar(
    state: &AppState,
    area: Rect,
    row_height: f64,
    theme: &Theme,
    scroll: usize,
) -> Sidebar {
    build_sidebar_filtered(
        state,
        area,
        SidebarLayout {
            header: sidebar_header(2.0),
            row_height,
        },
        theme,
        scroll,
        None,
    )
}

fn workspace_matches_filter(
    state: &AppState,
    workspace: &berth_core::model::Workspace,
    filter: &str,
) -> bool {
    let needle = filter.to_lowercase();
    if workspace.name.to_lowercase().contains(&needle) {
        return true;
    }
    if let Some(surface) = workspace
        .active_surface
        .as_deref()
        .and_then(|surface_id| state.surface(surface_id))
        && surface.cwd.to_lowercase().contains(&needle)
    {
        return true;
    }
    state
        .agents
        .values()
        .filter(|agent| agent.workspace_id == workspace.id)
        .any(|agent| {
            agent.name.to_lowercase().contains(&needle)
                || agent.provider.to_lowercase().contains(&needle)
        })
}

/// Sidebar rows with a case-insensitive filter over names, cwds and agents.
pub fn build_sidebar_filtered(
    state: &AppState,
    area: Rect,
    layout: SidebarLayout,
    theme: &Theme,
    scroll: usize,
    filter: Option<&str>,
) -> Sidebar {
    build_sidebar_scoped(state, None, area, layout, theme, scroll, filter)
}

/// The sidebar for one window: only the workspaces assigned to it.
///
/// Workspaces carry a `window_id`, and each window renders an independent
/// sidebar, so a window that listed every workspace would offer rows whose
/// selection moves another window's content. `None` keeps the whole-state
/// behaviour for callers that are not a window (tests, CLI snapshots).
pub fn build_sidebar_for_window(
    state: &AppState,
    window_id: &str,
    area: Rect,
    layout: SidebarLayout,
    theme: &Theme,
    scroll: usize,
    filter: Option<&str>,
) -> Sidebar {
    build_sidebar_scoped(state, Some(window_id), area, layout, theme, scroll, filter)
}

/// Whether a workspace belongs to the given window. `None` as the window means
/// every workspace; a workspace with no `window_id` belongs to every window
/// too, because it would otherwise be unreachable: no sidebar would list it
/// and no window could select it.
pub fn in_window(workspace: &berth_core::model::Workspace, window_id: Option<&str>) -> bool {
    match (window_id, workspace.window_id.as_deref()) {
        (None, _) | (_, None) => true,
        (Some(id), Some(owned)) => id == owned,
    }
}

fn build_sidebar_scoped(
    state: &AppState,
    window_id: Option<&str>,
    area: Rect,
    layout: SidebarLayout,
    theme: &Theme,
    scroll: usize,
    filter: Option<&str>,
) -> Sidebar {
    let mut rows = Vec::new();
    let mut y = 0usize;
    let row_height = layout.row_height;
    let group_height = (GROUP_ROW_HEIGHT * row_height / WORKSPACE_ROW_HEIGHT) as usize;
    let workspace_height = row_height as usize;

    let push_group = |rows: &mut Vec<SidebarRow>,
                      y: &mut usize,
                      id: &str,
                      name: &str,
                      collapsed: bool,
                      color: Option<String>| {
        let rect = Rect::new(area.x, *y, area.w, group_height);
        rows.push(SidebarRow {
            kind: RowKind::Group {
                id: id.to_string(),
                name: name.to_string(),
                collapsed,
                color,
            },
            rect,
        });
        *y = y.saturating_add(group_height);
    };

    let workspace_row = |id: &str, y: usize, height: usize| -> Option<SidebarRow> {
        let workspace = state.workspace(id)?;
        if let Some(filter) = filter
            && !filter.is_empty()
            && !workspace_matches_filter(state, workspace, filter)
        {
            return None;
        }
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
        let mut label = match &workspace.icon {
            Some(icon) if !icon.is_empty() => format!("{icon} {base}"),
            _ => base,
        };
        if workspace.pinned {
            label = format!("★ {label}");
        }
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
        let agents = state
            .agents
            .values()
            .filter(|agent| agent.workspace_id == id)
            .count();
        if agents > 1 {
            parts.push(format!("{agents} agents"));
        }
        Some(SidebarRow {
            kind: RowKind::Workspace(WorkspaceView {
                id: workspace.id.clone(),
                label,
                color: workspace.color.clone(),
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

    let push_agents = |rows: &mut Vec<SidebarRow>, y: &mut usize, workspace_id: &str| {
        let primary = workspace_agent(state, workspace_id).map(|agent| agent.id.clone());
        for agent in state.agents.values() {
            if agent.workspace_id != workspace_id || Some(&agent.id) == primary.as_ref() {
                continue;
            }
            let rect = Rect::new(area.x + 14, *y, area.w.saturating_sub(14), group_height);
            rows.push(SidebarRow {
                kind: RowKind::Agent {
                    workspace_id: workspace_id.to_string(),
                    surface_id: agent.surface_id.clone(),
                    agent: agent_view(agent, theme),
                },
                rect,
            });
            *y = y.saturating_add(group_height);
        }
    };

    // A group row is only useful when this window has a workspace in it.
    for group in &state.groups {
        let ids: Vec<String> = state
            .workspaces
            .iter()
            .filter(|workspace| workspace.group_id.as_deref() == Some(group.id.as_str()))
            .filter(|workspace| in_window(workspace, window_id))
            .map(|workspace| workspace.id.clone())
            .collect();
        if ids.is_empty() {
            continue;
        }
        push_group(
            &mut rows,
            &mut y,
            &group.id,
            &group.name,
            group.collapsed,
            group.color.clone(),
        );
        if group.collapsed {
            continue;
        }
        for id in ordered_workspaces(state, &ids) {
            if let Some(row) = workspace_row(&id, y, workspace_height) {
                y = y.saturating_add(workspace_height);
                rows.push(row);
                push_agents(&mut rows, &mut y, &id);
            }
        }
    }

    let grouped: Vec<&str> = state.groups.iter().map(|group| group.id.as_str()).collect();
    let loose: Vec<String> = state
        .workspaces
        .iter()
        .filter(|workspace| in_window(workspace, window_id))
        .filter(|workspace| match workspace.group_id.as_deref() {
            Some(group_id) => !grouped.contains(&group_id),
            None => true,
        })
        .map(|workspace| workspace.id.clone())
        .collect();
    for id in ordered_workspaces(state, &loose) {
        if let Some(row) = workspace_row(&id, y, workspace_height) {
            y = y.saturating_add(workspace_height);
            rows.push(row);
            push_agents(&mut rows, &mut y, &id);
        }
    }

    // Rows are laid out in virtual space, then scrolled and clipped to the
    // list area below the header.
    let total = rows.len();
    let offset = rows.get(scroll).map(|row| row.rect.y).unwrap_or(y);
    let top = area.y.saturating_add(layout.header);
    let mut visible = Vec::with_capacity(total.saturating_sub(scroll));
    for mut row in rows.into_iter().skip(scroll) {
        row.rect.y = top + row.rect.y.saturating_sub(offset);
        if row.rect.y >= area.bottom() {
            break;
        }
        if let RowKind::Workspace(workspace) = &mut row.kind {
            workspace.rect = row.rect;
        }
        visible.push(row);
    }
    Sidebar {
        rows: visible,
        total,
    }
}

/// Width reserved for the tab-overflow button.
pub const OVERFLOW_WIDTH: usize = 24;
/// No tab shrinks below this; beyond it the strip scrolls and overflows.
pub const MIN_TAB_WIDTH: usize = 72;

/// The visible workspace's panes, their tab strips and the draggable dividers.
pub fn build_panes(
    state: &AppState,
    area: Rect,
    cell_height: usize,
    tab_height: usize,
    flash: Option<&str>,
    options: PaneOptions<'_>,
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
    let zoom = options
        .zoom
        .or(workspace.zoomed_pane.as_deref())
        .filter(|pane| workspace.root.contains(pane));
    let (leaves, dividers) = match zoom {
        Some(pane_id) => (
            vec![layout::Leaf {
                pane_id: pane_id.to_string(),
                rect: area,
            }],
            Vec::new(),
        ),
        None => layout::layout(&workspace.root, area),
    };
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
            let mut overflow = None;
            let mut marks = false;
            if let Some(pane) = state.panes.get(&pane_id) {
                surface = pane.active_surface().map(str::to_owned);
                marks = surface
                    .as_deref()
                    .is_some_and(|id| options.prompt_marks.is_some_and(|set| set.contains(id)));
                let count = pane.surfaces.len().max(1);
                let needs_overflow = count * MIN_TAB_WIDTH + OVERFLOW_WIDTH > bar.w;
                let strip_width = if needs_overflow {
                    bar.w.saturating_sub(OVERFLOW_WIDTH)
                } else {
                    bar.w
                };
                let skip = options
                    .tab_scroll
                    .and_then(|map| map.get(&pane_id).copied())
                    .unwrap_or(0)
                    .min(count.saturating_sub(1));
                let visible = count - skip;
                let share = strip_width / visible.max(1);
                let mut x = bar.x;
                for (index, surface_id) in pane.surfaces.iter().enumerate().skip(skip) {
                    let Some(tab_surface) = state.surface(surface_id) else {
                        continue;
                    };
                    let remaining = strip_width.saturating_sub(x.saturating_sub(bar.x));
                    if remaining < 16 {
                        break;
                    }
                    let title = tab_title(tab_surface, index);
                    let text_width = title.chars().count() * cell_height / 2;
                    // Tabs share the strip equally and truncate their label, so
                    // a long OSC title cannot hide the tabs after it.
                    let width = (text_width + 34)
                        .min(share.max(MIN_TAB_WIDTH.min(strip_width)))
                        .min(remaining);
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
                if needs_overflow {
                    overflow = Some(Rect::new(
                        bar.right().saturating_sub(OVERFLOW_WIDTH),
                        bar.y,
                        OVERFLOW_WIDTH.min(bar.w),
                        bar.h,
                    ));
                }
            }
            let focused = surface.is_some() && surface.as_deref() == active.as_deref();
            let attention_flash = surface.as_deref() == flash;
            let gutter_width = if marks {
                gutter_width(cell_height, content.w)
            } else {
                0
            };
            let gutter = Rect::new(content.x, content.y, gutter_width, content.h);
            content.x = content.x.saturating_add(gutter_width);
            content.w = content.w.saturating_sub(gutter_width);
            PaneView {
                pane_id,
                surface,
                rect: leaf.rect,
                content,
                gutter,
                focused,
                tabs,
                tab_bar: bar,
                attention_flash,
                overflow,
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

    /// A 2× display, the scale the clipping was reported on.
    fn test_layout() -> SidebarLayout {
        SidebarLayout {
            header: sidebar_header(2.0),
            row_height: WORKSPACE_ROW_HEIGHT * 2.0,
        }
    }

    /// The chrome font is sized in physical pixels (35px tall at 2×), so the
    /// header has to be at least that tall or the first row paints over the
    /// label. This is the bug the scaled header fixes.
    #[test]
    fn given_a_retina_scale_then_the_header_is_taller_than_the_chrome_text() {
        let chrome_line = 35;
        let layout = test_layout();
        assert!(
            layout.header >= chrome_line,
            "header {} cannot hold a {chrome_line}px label",
            layout.header
        );
        // And the scale has to matter: a fixed header would be identical here.
        assert_eq!(sidebar_header(1.0) * 2, sidebar_header(2.0));
    }

    #[test]
    fn given_a_header_when_rows_are_built_then_the_first_row_starts_below_it() {
        let mut state = AppState::default();
        state
            .create_workspace(Some("one"), None, None, &berth_core::NullOps)
            .unwrap();
        let layout = test_layout();
        let sidebar = build_sidebar(
            &state,
            Rect::new(0, 0, 200, 600),
            layout.row_height,
            &Theme::default(),
            0,
        );
        assert_eq!(sidebar.rows[0].rect.y, layout.header);
    }

    /// The header buttons have to grow with the header, or a 2× display leaves
    /// the icon the same size in a box twice as tall.
    #[test]
    fn given_a_retina_scale_then_the_header_buttons_grow_and_do_not_overlap() {
        let area = Rect::new(0, 0, 200, 600);
        let one = sidebar_header(1.0);
        let two = sidebar_header(2.0);
        let small = new_workspace_button(area, one);
        let large = new_workspace_button(area, two);
        assert!(large.w > small.w);
        assert!(large.h > small.h);

        let group = new_group_button(area, two);
        assert!(
            group.right() <= large.x,
            "the group button {} overlaps the workspace button {}",
            group.right(),
            large.x
        );
    }

    #[test]
    fn given_a_tab_strip_then_the_drop_index_follows_the_pointer() {
        let tabs = vec![
            TabView {
                id: "a".into(),
                title: "a".into(),
                rect: Rect::new(0, 0, 100, 20),
                active: true,
                close: Rect::new(80, 0, 20, 20),
            },
            TabView {
                id: "b".into(),
                title: "b".into(),
                rect: Rect::new(100, 0, 100, 20),
                active: false,
                close: Rect::new(180, 0, 20, 20),
            },
        ];
        assert_eq!(tab_index_at(&tabs, 10), 0);
        assert_eq!(tab_index_at(&tabs, 90), 0);
        assert_eq!(tab_index_at(&tabs, 110), 1);
        assert_eq!(tab_index_at(&tabs, 250), 1);
        assert_eq!(tab_index_at(&[], 10), 0);
    }

    #[test]
    fn given_a_zoomed_pane_when_building_then_only_that_pane_is_shown() {
        let mut state = AppState::default();
        let workspace = state
            .create_workspace(Some("one"), None, None, &berth_core::NullOps)
            .unwrap();
        let surface_id = workspace.active_surface.clone().unwrap();
        let pane_id = state.surface(&surface_id).unwrap().pane_id.clone();
        state
            .split_surface(
                &surface_id,
                Orientation::Horizontal,
                false,
                berth_core::SpawnRequest::default(),
                false,
                &berth_core::NullOps,
            )
            .unwrap();
        state.set_zoomed_pane(&pane_id, true).unwrap();

        let (panes, dividers) = build_panes(
            &state,
            Rect::new(0, 0, 400, 300),
            16,
            28,
            None,
            PaneOptions::default(),
        );
        assert_eq!(panes.len(), 1);
        assert!(dividers.is_empty());
        assert_eq!(panes[0].pane_id, pane_id);
    }

    #[test]
    fn given_a_stored_colour_then_hex_and_names_parse() {
        assert_eq!(parse_color(Some("#e8b06a")), Some(0x00e8_b06a));
        assert_eq!(parse_color(Some("green")), Some(0x008b_d5a6));
        assert_eq!(parse_color(Some("nope")), None);
        assert_eq!(parse_color(None), None);
    }

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

    /// Two windows, each with its own workspace. A window's sidebar must list
    /// only the workspaces that belong to it: a row the window does not own is
    /// a row whose click would move the other window's content.
    #[test]
    fn given_two_windows_when_building_a_sidebar_then_it_lists_only_its_own() {
        let mut state = AppState::default();
        let first = state.create_window(Some("first"));
        let second = state.create_window(Some("second"));
        let in_first = state
            .create_workspace_in(
                Some(&first.id),
                Some("mine"),
                None,
                None,
                &berth_core::NullOps,
            )
            .unwrap();
        let in_second = state
            .create_workspace_in(
                Some(&second.id),
                Some("theirs"),
                None,
                None,
                &berth_core::NullOps,
            )
            .unwrap();

        let sidebar = build_sidebar_for_window(
            &state,
            &first.id,
            Rect::new(0, 0, 200, 600),
            test_layout(),
            &Theme::default(),
            0,
            None,
        );
        let ids: Vec<String> = sidebar
            .rows
            .iter()
            .filter_map(|row| match &row.kind {
                RowKind::Workspace(workspace) => Some(workspace.id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(ids, vec![in_first.id.clone()]);
        assert!(!ids.contains(&in_second.id));

        // The whole-state builder is the behaviour this replaced: it lists
        // both, which is exactly the cross-window row the scoped one drops.
        let unscoped = build_sidebar_filtered(
            &state,
            Rect::new(0, 0, 200, 600),
            test_layout(),
            &Theme::default(),
            0,
            None,
        );
        assert_eq!(unscoped.rows.len(), 2);
    }

    #[test]
    fn given_a_window_with_no_selection_when_active_then_it_falls_back_to_its_own() {
        let mut state = AppState::default();
        let first = state.create_window(Some("first"));
        let second = state.create_window(Some("second"));
        let in_second = state
            .create_workspace_in(
                Some(&second.id),
                Some("theirs"),
                None,
                None,
                &berth_core::NullOps,
            )
            .unwrap();
        // The focused window points at the second window's workspace; the
        // first window has nothing of its own selected yet.
        state.selected_workspace = Some(in_second.id.clone());

        assert_eq!(
            active_workspace_in(&state, Some(&first.id)),
            None,
            "a window with no workspaces has no active workspace, even when another window has one"
        );
        assert_eq!(
            active_workspace_in(&state, Some(&second.id)),
            Some(in_second.id.clone())
        );
    }

    /// A workspace with no `window_id` must stay reachable: scoping it out of
    /// every window would hide it with no way back.
    #[test]
    fn given_a_workspace_with_no_window_when_scoping_then_it_stays_visible() {
        let mut state = AppState::default();
        let window = state.create_window(Some("only"));
        let orphan = state
            .create_workspace_in(None, Some("orphan"), None, None, &berth_core::NullOps)
            .unwrap();
        if let Some(ws) = state.workspaces.iter_mut().find(|w| w.id == orphan.id) {
            ws.window_id = None;
        }

        let sidebar = build_sidebar_for_window(
            &state,
            &window.id,
            Rect::new(0, 0, 200, 600),
            test_layout(),
            &Theme::default(),
            0,
            None,
        );
        assert_eq!(sidebar.rows.len(), 1, "the orphan workspace stays listed");
    }

    /// Painting and hit-testing must agree: a row that is drawn is clickable,
    /// and a row that is not drawn cannot be clicked. Because both go through
    /// the same scoped builder, the list a click searches is the list on
    /// screen.
    #[test]
    fn given_two_windows_when_hit_testing_then_only_drawn_rows_are_found() {
        let mut state = AppState::default();
        let first = state.create_window(Some("first"));
        let second = state.create_window(Some("second"));
        let mine = state
            .create_workspace_in(
                Some(&first.id),
                Some("mine"),
                None,
                None,
                &berth_core::NullOps,
            )
            .unwrap();
        state
            .create_workspace_in(
                Some(&second.id),
                Some("theirs"),
                None,
                None,
                &berth_core::NullOps,
            )
            .unwrap();

        let sidebar = build_sidebar_for_window(
            &state,
            &first.id,
            Rect::new(0, 0, 200, 600),
            test_layout(),
            &Theme::default(),
            0,
            None,
        );
        // Every visible row is a row this window owns, at the painted rect.
        for row in &sidebar.rows {
            if let RowKind::Workspace(workspace) = &row.kind {
                assert_eq!(workspace.id, mine.id);
                assert!(
                    sidebar
                        .rows
                        .iter()
                        .any(|other| other.rect.contains(row.rect.x, row.rect.y)),
                    "a painted row must be hit-testable at its own origin"
                );
            }
        }
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
            user_title: None,
            cwd: cwd.into(),
            pid: None,
            tty: None,
            cols: 80,
            rows: 24,
            progress: None,
            bell_count: 0,
            agent_id: None,
            argv: Vec::new(),
            env: Vec::new(),
            restore_text: None,
            created_ms: 0,
        }
    }

    #[test]
    fn given_no_workspace_when_building_panes_then_none_are_returned() {
        let state = AppState::default();
        let (panes, dividers) = build_panes(
            &state,
            Rect::new(0, 0, 100, 100),
            16,
            28,
            None,
            PaneOptions::default(),
        );
        assert!(panes.is_empty());
        assert!(dividers.is_empty());
    }

    #[test]
    fn given_no_workspace_when_asking_for_the_active_surface_then_none_is_returned() {
        assert_eq!(active_surface(&AppState::default()), None);
    }

    #[test]
    fn given_a_workspace_with_a_live_surface_when_asking_then_that_surface_is_returned() {
        let mut state = AppState::default();
        let workspace = state
            .create_workspace(Some("main"), None, None, &berth_core::NullOps)
            .unwrap();
        assert_eq!(active_surface(&state), workspace.active_surface);
    }

    #[test]
    fn given_a_workspace_with_no_live_surface_when_asking_then_the_last_selection_wins() {
        let mut state = AppState::default();
        let workspace = state
            .create_workspace(Some("main"), None, None, &berth_core::NullOps)
            .unwrap();
        let surface = workspace.active_surface.clone().expect("a surface");
        if let Some(ws) = state.workspace_mut(&workspace.id) {
            ws.active_surface = None;
        }
        for pane in state.panes.values_mut() {
            pane.surfaces.clear();
            pane.active = 0;
        }
        state.selected_surface = Some(surface.clone());
        assert_eq!(active_surface(&state), Some(surface));
    }

    #[test]
    fn given_a_vanished_selection_when_asking_for_the_active_surface_then_it_falls_back() {
        let mut state = AppState::default();
        state.selected_surface = Some("srf_gone".to_string());
        assert_eq!(active_surface(&state), None);
        let workspace = state
            .create_workspace(Some("main"), None, None, &berth_core::NullOps)
            .unwrap();
        assert_eq!(active_surface(&state), workspace.active_surface);
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
        let (panes, dividers) = build_panes(
            &state,
            Rect::new(0, 0, 200, 100),
            16,
            28,
            None,
            PaneOptions::default(),
        );
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
    fn given_a_workspace_when_the_sidebar_is_built_then_rows_start_below_the_header() {
        let mut state = AppState::default();
        state
            .create_workspace(Some("one"), None, None, &berth_core::NullOps)
            .unwrap();
        let sidebar = build_sidebar(
            &state,
            Rect::new(0, 0, 200, 400),
            62.0,
            &Theme::default(),
            0,
        );
        assert_eq!(sidebar.total, 1);
        assert_eq!(sidebar.rows[0].rect.y, test_layout().header);
    }

    #[test]
    fn given_a_scrolled_sidebar_when_built_then_the_first_row_moves_up_by_rows() {
        let mut state = AppState::default();
        for index in 0..5 {
            state
                .create_workspace(Some(&format!("w{index}")), None, None, &berth_core::NullOps)
                .unwrap();
        }
        let sidebar = build_sidebar(
            &state,
            Rect::new(0, 0, 200, 400),
            62.0,
            &Theme::default(),
            2,
        );
        assert_eq!(sidebar.total, 5);
        assert_eq!(sidebar.rows.len(), 3);
        assert_eq!(sidebar.rows[0].rect.y, test_layout().header);
        assert_eq!(sidebar.rows[1].rect.y, test_layout().header + 62);
    }

    #[test]
    fn given_two_workspaces_when_one_needs_attention_then_it_is_listed_first() {
        let mut state = AppState::default();
        let _quiet = workspace_with_agent(&mut state, "quiet", berth_core::AgentState::Thinking);
        let loud = workspace_with_agent(&mut state, "loud", berth_core::AgentState::WaitingInput);
        let rows = build_sidebar(
            &state,
            Rect::new(0, 0, 200, 400),
            62.0,
            &Theme::default(),
            0,
        )
        .rows;
        let first = rows.iter().find_map(|row| match &row.kind {
            RowKind::Workspace(workspace) => Some(workspace.id.clone()),
            RowKind::Group { .. } | RowKind::Agent { .. } => None,
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
        let rows = build_sidebar(
            &state,
            Rect::new(0, 0, 200, 400),
            62.0,
            &Theme::default(),
            0,
        )
        .rows;
        let ids: Vec<String> = rows
            .iter()
            .filter_map(|row| match &row.kind {
                RowKind::Workspace(workspace) => Some(workspace.id.clone()),
                RowKind::Group { .. } | RowKind::Agent { .. } => None,
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
        let rows = build_sidebar(
            &state,
            Rect::new(0, 0, 200, 400),
            62.0,
            &Theme::default(),
            0,
        )
        .rows;
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
        let rows = build_sidebar(
            &state,
            Rect::new(0, 0, 200, 400),
            62.0,
            &Theme::default(),
            0,
        )
        .rows;
        assert_eq!(rows.len(), 2);
        assert!(matches!(&rows[1].kind, RowKind::Workspace(w) if w.id == workspace.id));
    }

    #[test]
    fn given_no_workspace_when_the_sidebar_is_built_then_it_is_empty() {
        let state = AppState::default();
        let rows = build_sidebar(
            &state,
            Rect::new(0, 0, 200, 400),
            62.0,
            &Theme::default(),
            0,
        )
        .rows;
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
