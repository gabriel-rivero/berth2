//! Modal panels: the command palette and the attention list.
//!
//! One geometry and one row model serve both, so keyboard handling, hit testing
//! and painting stay in one place. The panel is centered and never larger than
//! the window it sits in.

use crate::layout::Rect;

/// Logical size of a panel, before the display scale.
pub const PANEL_WIDTH: f64 = 560.0;
pub const PANEL_HEIGHT: f64 = 360.0;
pub const PANEL_MARGIN: f64 = 40.0;
pub const TITLE_HEIGHT: f64 = 30.0;
pub const QUERY_HEIGHT: f64 = 26.0;
pub const FOOTER_HEIGHT: f64 = 24.0;

/// Where each part of a panel sits, in physical pixels.
#[derive(Clone, Copy, Debug)]
pub struct PanelGeometry {
    pub panel: Rect,
    pub title: Rect,
    pub query: Option<Rect>,
    pub rows: Rect,
    pub footer: Rect,
}

/// Lay a panel out in the middle of `window`.
pub fn geometry(window: Rect, scale: f64, has_query: bool) -> PanelGeometry {
    let scale = scale.max(0.5);
    let margin = (PANEL_MARGIN * scale) as usize;
    let width = ((PANEL_WIDTH * scale) as usize)
        .min(window.w.saturating_sub(margin * 2))
        .max(120);
    let height = ((PANEL_HEIGHT * scale) as usize)
        .min(window.h.saturating_sub(margin * 2))
        .max(120);
    let panel = Rect::new(
        window.x + (window.w.saturating_sub(width)) / 2,
        window.y + (window.h.saturating_sub(height)) / 2,
        width,
        height,
    );
    let title_height = (TITLE_HEIGHT * scale) as usize;
    let query_height = if has_query {
        (QUERY_HEIGHT * scale) as usize
    } else {
        0
    };
    let footer_height = (FOOTER_HEIGHT * scale) as usize;
    let title = Rect::new(panel.x, panel.y, panel.w, title_height);
    let query = has_query.then(|| {
        Rect::new(
            panel.x,
            title.bottom(),
            panel.w,
            query_height.min(panel.h.saturating_sub(title_height)),
        )
    });
    let footer = Rect::new(
        panel.x,
        panel.bottom().saturating_sub(footer_height),
        panel.w,
        footer_height,
    );
    let rows = Rect::new(
        panel.x,
        title.bottom().saturating_add(query_height),
        panel.w,
        footer
            .y
            .saturating_sub(title.bottom().saturating_add(query_height)),
    );
    PanelGeometry {
        panel,
        title,
        query,
        rows,
        footer,
    }
}

/// One selectable line in a panel.
pub struct PanelRow {
    pub label: String,
    pub detail: String,
    /// Accent for the detail text; `None` uses the muted colour.
    pub color: Option<u32>,
}

/// How many rows fit in the rows area.
pub fn visible_rows(rows: Rect, row_height: usize) -> usize {
    (rows.h / row_height.max(1)).max(1)
}

/// First row to paint so `selected` stays in view.
pub fn first_visible(selected: usize, len: usize, visible: usize) -> usize {
    let len = len.max(1);
    let visible = visible.max(1);
    selected
        .saturating_add(1)
        .saturating_sub(visible)
        .min(len.saturating_sub(1))
}

/// Row rectangles for `count` entries, clipped to the rows area.
pub fn row_rects(rows: Rect, count: usize, row_height: usize) -> Vec<Rect> {
    let height = row_height.max(1);
    let visible = rows.h / height;
    (0..count.min(visible))
        .map(|index| Rect::new(rows.x, rows.y + index * height, rows.w, height))
        .collect()
}

/// Subsequence match, case-insensitive: "sp" matches "Split Right".
pub fn fuzzy_match(query: &str, text: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let mut characters = text.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .all(|wanted| characters.any(|candidate| candidate == wanted))
}

/// Keep the selection inside a list of `len` entries.
pub fn clamp_selection(selected: usize, len: usize) -> usize {
    selected.min(len.saturating_sub(1))
}

/// Move the selection by `delta`, wrapping at both ends.
pub fn move_selection(selected: usize, delta: i32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let len = len as i32;
    (selected as i32 + delta).rem_euclid(len) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Rect {
        Rect::new(0, 0, 1200, 800)
    }

    #[test]
    fn given_a_window_when_the_panel_is_laid_out_then_it_is_centered_and_inside() {
        let geo = geometry(window(), 1.0, true);
        assert!(geo.panel.w <= 1200 && geo.panel.h <= 800);
        assert_eq!(geo.panel.x, (1200 - geo.panel.w) / 2);
        assert_eq!(geo.panel.y, (800 - geo.panel.h) / 2);
        assert!(window().contains(geo.panel.x, geo.panel.y));
        assert!(
            geo.panel
                .contains(geo.panel.right() - 1, geo.panel.bottom() - 1)
        );
    }

    #[test]
    fn given_a_small_window_when_the_panel_is_laid_out_then_it_still_fits() {
        let geo = geometry(Rect::new(0, 0, 260, 200), 1.0, true);
        assert!(geo.panel.w <= 260 && geo.panel.h <= 200);
        assert!(geo.panel.w >= 120);
    }

    #[test]
    fn given_a_query_row_when_laid_out_then_the_rows_start_below_it() {
        let geo = geometry(window(), 1.0, true);
        let query = geo.query.expect("palette has a query line");
        assert_eq!(geo.rows.y, query.bottom());
        let without = geometry(window(), 1.0, false);
        assert_eq!(without.rows.y, without.title.bottom());
        assert!(without.rows.h > geo.rows.h);
    }

    #[test]
    fn given_more_entries_than_fit_when_layout_runs_then_rows_are_clipped() {
        let rows = Rect::new(0, 0, 100, 60);
        assert_eq!(row_rects(rows, 2, 30).len(), 2);
        assert_eq!(row_rects(rows, 10, 30).len(), 2);
        assert_eq!(row_rects(rows, 0, 30).len(), 0);
    }

    #[test]
    fn given_a_query_when_matched_then_subsequences_count() {
        assert!(fuzzy_match("", "Split Right"));
        assert!(fuzzy_match("sr", "Split Right"));
        assert!(fuzzy_match("SPLIT", "split right"));
        assert!(fuzzy_match("jumpatt", "Jump to Attention"));
        assert!(!fuzzy_match("zz", "Split Right"));
        assert!(!fuzzy_match("rigsp", "Split Right"));
    }

    #[test]
    fn given_more_rows_than_fit_when_the_selection_moves_then_the_window_follows() {
        let rows = Rect::new(0, 0, 100, 100);
        assert_eq!(visible_rows(rows, 25), 4);
        assert_eq!(first_visible(0, 10, 4), 0);
        assert_eq!(first_visible(3, 10, 4), 0);
        assert_eq!(first_visible(4, 10, 4), 1);
        assert_eq!(first_visible(9, 10, 4), 6);
        assert_eq!(first_visible(0, 1, 4), 0);
    }

    #[test]
    fn given_a_selection_when_moved_past_an_end_then_it_wraps() {
        assert_eq!(move_selection(0, -1, 3), 2);
        assert_eq!(move_selection(2, 1, 3), 0);
        assert_eq!(move_selection(0, 0, 0), 0);
        assert_eq!(clamp_selection(7, 3), 2);
        assert_eq!(clamp_selection(1, 0), 0);
    }
}
