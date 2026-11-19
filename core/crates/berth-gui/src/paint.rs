//! Frame painting: one CPU buffer, real glyphs, no GPU.
//!
//! Everything here works on pixels and view models; it never reads `AppState`.
//! Alpha blending is done per pixel, so glyph edges and the selection tint look
//! the same on every platform.

use berth_headless::{CellView, ScreenView};

use crate::font::{Bitmap, CellSize, FontRenderer, Raster, TextStyle};
use crate::layout::Rect;
use crate::theme::Theme;
use crate::ui::{PaneView, RowKind, Selection, SidebarRow, WorkspaceView};

/// Cubes used to tint the selection and the inactive window.
const SELECTION_ALPHA: u8 = 96;

pub struct Painter<'a> {
    buffer: &'a mut [u32],
    width: usize,
    height: usize,
}

impl<'a> Painter<'a> {
    pub fn new(buffer: &'a mut [u32], width: usize, height: usize) -> Self {
        Self {
            buffer,
            width,
            height,
        }
    }

    pub fn fill(&mut self, rect: Rect, color: u32) {
        let Some(clip) = rect.intersect(self.bounds()) else {
            return;
        };
        for y in clip.y..clip.bottom() {
            let start = y * self.width + clip.x;
            let end = start + clip.w;
            if let Some(row) = self.buffer.get_mut(start..end) {
                row.fill(color);
            }
        }
    }

    pub fn blend(&mut self, rect: Rect, color: u32, alpha: u8) {
        let Some(clip) = rect.intersect(self.bounds()) else {
            return;
        };
        let r = ((color >> 16) & 0xff) as u8;
        let g = ((color >> 8) & 0xff) as u8;
        let b = (color & 0xff) as u8;
        for y in clip.y..clip.bottom() {
            for x in clip.x..clip.right() {
                self.blend_pixel(x, y, r, g, b, alpha);
            }
        }
    }

    pub fn stroke(&mut self, rect: Rect, color: u32) {
        if rect.w == 0 || rect.h == 0 {
            return;
        }
        self.fill(Rect::new(rect.x, rect.y, rect.w, 1), color);
        self.fill(
            Rect::new(rect.x, rect.bottom().saturating_sub(1), rect.w, 1),
            color,
        );
        self.fill(Rect::new(rect.x, rect.y, 1, rect.h), color);
        self.fill(
            Rect::new(rect.right().saturating_sub(1), rect.y, 1, rect.h),
            color,
        );
    }

    fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    fn blend_pixel(&mut self, x: usize, y: usize, r: u8, g: u8, b: u8, alpha: u8) {
        if x >= self.width || y >= self.height || alpha == 0 {
            return;
        }
        let index = y * self.width + x;
        let Some(target) = self.buffer.get_mut(index) else {
            return;
        };
        let a = u32::from(alpha);
        let inv = 255 - a;
        let mix = |dst: u8, src: u8| (u32::from(src) * a + u32::from(dst) * inv) / 255;
        let dr = ((*target >> 16) & 0xff) as u8;
        let dg = ((*target >> 8) & 0xff) as u8;
        let db = (*target & 0xff) as u8;
        *target = (mix(dr, r) << 16) | (mix(dg, g) << 8) | mix(db, b);
    }

    /// Draw one shaped cell with its top-left at `origin`. Returns the advance
    /// in pixels.
    pub fn glyph(
        &mut self,
        font: &mut FontRenderer,
        origin: (usize, usize),
        text: &str,
        style: TextStyle,
        color: u32,
    ) -> usize {
        let cell = font.cell();
        let Some(raster) = font.raster(text, style) else {
            return cell.width;
        };
        let x0 = origin.0 as i32 + raster.left;
        let y0 = origin.1 as i32 + raster.top;
        self.blit(&raster, x0, y0, color);
        cell.width
    }

    /// Draw a shaped run of cells at `origin`, `width` pixels wide.
    fn glyph_run(
        &mut self,
        font: &mut FontRenderer,
        origin: (usize, usize),
        text: &str,
        width: usize,
        style: TextStyle,
        color: u32,
    ) {
        let Some(raster) = font.raster_run(text, width, style) else {
            return;
        };
        let x0 = origin.0 as i32 + raster.left;
        let y0 = origin.1 as i32 + raster.top;
        self.blit(&raster, x0, y0, color);
    }

    /// Fill a Unicode block element with the cell's foreground. Returns false
    /// when the character is not a block element.
    fn block_glyph(
        &mut self,
        origin: (usize, usize),
        cell: CellSize,
        ch: char,
        color: u32,
    ) -> bool {
        let Some(coverage) = block_coverage(ch) else {
            return false;
        };
        for &(x0, y0, x1, y1) in coverage {
            let left = origin.0 + cell.width * x0 as usize / 8;
            let top = origin.1 + cell.height * y0 as usize / 8;
            let right = origin.0 + cell.width * x1 as usize / 8;
            let bottom = origin.1 + cell.height * y1 as usize / 8;
            self.fill(Rect::new(left, top, right - left, bottom - top), color);
        }
        true
    }

    /// Blend one shaped raster with its top-left at (`x0`, `y0`).
    fn blit(&mut self, raster: &Raster, x0: i32, y0: i32, color: u32) {
        match &raster.bitmap {
            Bitmap::Mask(alpha) => {
                let r = ((color >> 16) & 0xff) as u8;
                let g = ((color >> 8) & 0xff) as u8;
                let b = (color & 0xff) as u8;
                for row in 0..raster.height {
                    for col in 0..raster.width {
                        let a = alpha[row * raster.width + col];
                        if a == 0 {
                            continue;
                        }
                        let x = x0 + col as i32;
                        let y = y0 + row as i32;
                        if x < 0 || y < 0 {
                            continue;
                        }
                        self.blend_pixel(x as usize, y as usize, r, g, b, a);
                    }
                }
            }
            Bitmap::Color(rgba) => {
                for row in 0..raster.height {
                    for col in 0..raster.width {
                        let index = (row * raster.width + col) * 4;
                        let a = rgba[index + 3];
                        if a == 0 {
                            continue;
                        }
                        let x = x0 + col as i32;
                        let y = y0 + row as i32;
                        if x < 0 || y < 0 {
                            continue;
                        }
                        self.blend_pixel(
                            x as usize,
                            y as usize,
                            rgba[index],
                            rgba[index + 1],
                            rgba[index + 2],
                            a,
                        );
                    }
                }
            }
        }
    }

    /// Draw a single line of text, clipped to `max_width` pixels. Returns the
    /// width used.
    pub fn text(
        &mut self,
        font: &mut FontRenderer,
        origin: (usize, usize),
        text: &str,
        style: TextStyle,
        color: u32,
        max_width: usize,
    ) -> usize {
        let cell = font.cell();
        let limit = (max_width / cell.width).max(1);
        let mut x = origin.0;
        for (index, character) in text.chars().enumerate() {
            if index >= limit {
                break;
            }
            let start = x;
            x += self.glyph(font, (x, origin.1), &character.to_string(), style, color);
            if x == start {
                x += cell.width;
            }
        }
        x - origin.0
    }

    /// Draw text with a trailing ellipsis when it does not fit.
    pub fn text_elided(
        &mut self,
        font: &mut FontRenderer,
        origin: (usize, usize),
        text: &str,
        style: TextStyle,
        color: u32,
        max_width: usize,
    ) {
        let cell = font.cell();
        let limit = (max_width / cell.width).max(1);
        if text.chars().count() <= limit {
            self.text(font, origin, text, style, color, max_width);
            return;
        }
        if limit <= 1 {
            self.text(font, origin, "…", style, color, max_width);
            return;
        }
        let head: String = text.chars().take(limit - 1).collect();
        let used = self.text(font, origin, &head, style, color, max_width);
        self.glyph(font, (origin.0 + used, origin.1), "…", style, color);
    }

    /// Draw one terminal screen inside `area`.
    pub fn screen(
        &mut self,
        font: &mut FontRenderer,
        view: &ScreenView,
        area: Rect,
        options: &ScreenOptions<'_>,
    ) {
        let cell = font.cell();
        let cols = usize::from(view.cols).min(area.w / cell.width.max(1));
        let rows = usize::from(view.rows).min(area.h / cell.height.max(1));
        let cursor_visible = options.cursor_visible && !view.cursor_hidden && view.scrollback == 0;
        for row in 0..rows {
            let base = row * usize::from(view.cols);
            let runs = rtl_runs(&view.cells, base, cols);
            for col in 0..cols {
                let index = row * usize::from(view.cols) + col;
                let Some(cell_view) = view.cells.get(index) else {
                    continue;
                };
                if cell_view.wide_continuation {
                    continue;
                }
                let origin = (area.x + col * cell.width, area.y + row * cell.height);
                let on_cursor = cursor_visible && view.cursor == (row as u16, col as u16);
                let selected = options.selection.is_some_and(|selection| {
                    selection.contains(crate::ui::Cell {
                        row: row as u16,
                        col: col as u16,
                    })
                });
                let mut fg = cell_view.fg.map(rgb).unwrap_or(options.theme.terminal_fg);
                let mut bg = cell_view.bg.map(rgb).unwrap_or(options.theme.terminal_bg);
                if cell_view.dim {
                    fg = options.theme.inactive(fg);
                }
                if cell_view.inverse || on_cursor {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if !options.focused {
                    fg = options.theme.inactive(fg);
                }
                self.fill(Rect::new(origin.0, origin.1, cell.width, cell.height), bg);
                if selected {
                    self.blend(
                        Rect::new(origin.0, origin.1, cell.width, cell.height),
                        options.theme.selection,
                        SELECTION_ALPHA,
                    );
                }
                let text = cell_view.contents.as_str();
                let style = TextStyle {
                    bold: cell_view.bold,
                    italic: cell_view.italic,
                };
                let in_run = runs.iter().any(|run| col >= run.start && col < run.end);
                let mut painted = false;
                if !in_run && let Some(ch) = single_char(text) {
                    painted = self.block_glyph(origin, cell, ch, fg);
                }
                if !in_run && !painted && !text.is_empty() && text != " " {
                    self.glyph(font, origin, text, style, fg);
                }
                if cell_view.underline {
                    let y = origin.1 + cell.height.saturating_sub(3);
                    let width = if cell_view.wide {
                        cell.width * 2
                    } else {
                        cell.width
                    };
                    self.fill(Rect::new(origin.0, y, width, 1), fg);
                }
                if cell_view.wide {
                    let next = Rect::new(origin.0 + cell.width, origin.1, cell.width, cell.height);
                    self.fill(next, bg);
                }
            }
            for run in &runs {
                let first = &view.cells[base + run.start];
                let origin = (area.x + run.start * cell.width, area.y + row * cell.height);
                let mut fg = first.fg.map(rgb).unwrap_or(options.theme.terminal_fg);
                if first.dim {
                    fg = options.theme.inactive(fg);
                }
                if first.inverse {
                    fg = first.bg.map(rgb).unwrap_or(options.theme.terminal_bg);
                }
                if !options.focused {
                    fg = options.theme.inactive(fg);
                }
                let style = TextStyle {
                    bold: first.bold,
                    italic: first.italic,
                };
                self.glyph_run(
                    font,
                    origin,
                    &run.text,
                    (run.end - run.start) * cell.width,
                    style,
                    fg,
                );
            }
        }
        if view.scrollback > 0 && view.scrollback_len > 0 {
            let track = Rect::new(area.right().saturating_sub(4), area.y, 3, area.h);
            self.fill(track, 0x001c_1c24);
            let total = view.scrollback_len + rows;
            let thumb = (area.h * rows / total.max(1)).max(16).min(area.h);
            let travel = area.h.saturating_sub(thumb);
            let offset = (view.scrollback_len - view.scrollback) * travel / view.scrollback_len;
            self.fill(
                Rect::new(track.x, area.y + offset, 3, thumb),
                options.theme.text_tertiary,
            );
        }
        if options.exited {
            self.paint_exited_banner(font, area, *options.theme);
        }
    }

    fn paint_exited_banner(&mut self, font: &mut FontRenderer, area: Rect, theme: Theme) {
        let cell = font.cell();
        let height = cell.height + 6;
        if area.h < height + 4 {
            return;
        }
        let banner = Rect::new(area.x, area.bottom().saturating_sub(height), area.w, height);
        self.fill(banner, 0x0024_1418);
        self.fill(
            Rect::new(banner.x, banner.y, banner.w, 1),
            theme.attention_error,
        );
        self.text_elided(
            font,
            (banner.x + 8, banner.y + 3),
            "process exited · close the tab to remove it",
            TextStyle::NORMAL,
            theme.attention_error,
            banner.w.saturating_sub(16),
        );
    }

    /// Draw a workspace row and its agent line.
    pub fn sidebar_row(
        &mut self,
        font: &mut FontRenderer,
        row: &SidebarRow,
        theme: Theme,
        compact: bool,
    ) {
        let cell = font.cell();
        match &row.kind {
            RowKind::Group {
                name, collapsed, ..
            } => {
                self.fill(row.rect, theme.sidebar_bg);
                let chevron = if *collapsed { "▸" } else { "▾" };
                let x = row.rect.x + 8;
                let y = row.rect.y + (row.rect.h.saturating_sub(cell.height) / 2);
                let used = self.glyph(
                    font,
                    (x, y),
                    chevron,
                    TextStyle::NORMAL,
                    theme.text_tertiary,
                );
                self.text_elided(
                    font,
                    (x + used + 6, y),
                    &name.to_uppercase(),
                    TextStyle::NORMAL,
                    theme.text_tertiary,
                    row.rect.w.saturating_sub(30),
                );
            }
            RowKind::Workspace(workspace) => {
                self.workspace_row(font, workspace, theme, compact);
            }
        }
    }

    fn workspace_row(
        &mut self,
        font: &mut FontRenderer,
        workspace: &WorkspaceView,
        theme: Theme,
        compact: bool,
    ) {
        let cell = font.cell();
        let rect = workspace.rect;
        self.fill(rect, theme.sidebar_bg);
        if workspace.active {
            self.fill(rect, theme.sidebar_active);
            self.fill(Rect::new(rect.x, rect.y, 3, rect.h), theme.accent);
        }
        let marker = match workspace.attention {
            berth_core::Attention::Error => Some(theme.attention_error),
            berth_core::Attention::NeedsInput => Some(theme.attention_input),
            berth_core::Attention::Info => Some(theme.accent),
            berth_core::Attention::None => None,
        };
        if let Some(color) = marker {
            self.fill(Rect::new(rect.x + 3, rect.y, 2, rect.h), color);
        }
        let inner = rect.inset(1);
        let x = inner.x + 10;
        let mut y = inner.y + 5;
        let color = if workspace.active {
            theme.text_primary
        } else {
            theme.text_secondary
        };
        self.text_elided(
            font,
            (x, y),
            &workspace.label,
            TextStyle::BOLD,
            color,
            inner.w.saturating_sub(24),
        );
        y += cell.height;
        if let Some(agent) = &workspace.agent {
            let used = self.glyph(
                font,
                (x, y),
                agent.glyph,
                TextStyle::NORMAL,
                theme.text_secondary,
            );
            let used = used
                + self.text(
                    font,
                    (x + used + 4, y),
                    &format!("{} · ", agent.name),
                    TextStyle::NORMAL,
                    theme.text_secondary,
                    inner.w.saturating_sub(24),
                );
            let used = used
                + self.text(
                    font,
                    (x + used + 4, y),
                    agent.state_label,
                    TextStyle::BOLD,
                    agent.color,
                    inner.w.saturating_sub(24),
                );
            if !agent.detail.is_empty() {
                self.text_elided(
                    font,
                    (x + used + 4, y),
                    &format!(" · {}", agent.detail),
                    TextStyle::NORMAL,
                    theme.text_tertiary,
                    inner.w.saturating_sub(24),
                );
            }
        } else if compact {
            // Compact rows drop the detail line, so show it beside the title.
            self.text_elided(
                font,
                (x, y),
                &workspace.detail,
                TextStyle::NORMAL,
                theme.text_tertiary,
                inner.w.saturating_sub(24),
            );
        }
        if !compact {
            y += cell.height;
            self.text_elided(
                font,
                (x, y),
                &workspace.detail,
                TextStyle::NORMAL,
                theme.text_tertiary,
                inner.w.saturating_sub(24),
            );
        }
        if let Some(progress) = workspace.agent.as_ref().and_then(|agent| agent.progress) {
            let width = (inner.w as f32 * progress.clamp(0.0, 1.0)) as usize;
            self.fill(
                Rect::new(inner.x, rect.bottom().saturating_sub(2), width, 2),
                theme.accent,
            );
        }
        if workspace.unread > 0 || marker.is_some() {
            let dot = Rect::new(rect.right().saturating_sub(14), rect.y + 14, 6, 6);
            self.fill(dot, marker.unwrap_or(theme.accent));
        }
    }

    /// Draw a pane's tab strip: one box per tab, an active underline and a
    /// close cross.
    pub fn tab_strip(&mut self, font: &mut FontRenderer, pane: &PaneView, theme: Theme) {
        let cell = font.cell();
        self.fill(pane.tab_bar, theme.tab_bar_bg);
        for tab in &pane.tabs {
            if tab.active {
                self.fill(tab.rect, theme.tab_active_bg);
                self.fill(
                    Rect::new(
                        tab.rect.x,
                        tab.rect.bottom().saturating_sub(2),
                        tab.rect.w,
                        2,
                    ),
                    theme.accent,
                );
            }
            let y = tab.rect.y + (tab.rect.h.saturating_sub(cell.height) / 2);
            let color = if tab.active {
                theme.tab_text
            } else {
                theme.tab_text_dim
            };
            self.text_elided(
                font,
                (tab.rect.x + 8, y),
                &tab.title,
                TextStyle::NORMAL,
                color,
                tab.rect.w.saturating_sub(cell.width * 3),
            );
            self.text(
                font,
                (tab.close.x, y),
                "×",
                TextStyle::NORMAL,
                if tab.active {
                    theme.tab_text
                } else {
                    theme.tab_text_dim
                },
                cell.width,
            );
        }
        if pane.tabs.is_empty() {
            self.fill(pane.content, theme.terminal_bg);
        }
    }
}

pub struct PanelStyle<'a> {
    pub theme: &'a Theme,
    pub geometry: &'a crate::panel::PanelGeometry,
    pub title: &'a str,
    pub query: Option<&'a str>,
    pub rows: &'a [crate::panel::PanelRow],
    pub selected: usize,
    /// First row to paint, so a long list follows the selection.
    pub first: usize,
    pub hint: &'a str,
}

impl Painter<'_> {
    /// Dim the window and draw one modal panel.
    pub fn panel(&mut self, font: &mut FontRenderer, style: &PanelStyle<'_>) {
        let theme = *style.theme;
        let geometry = style.geometry;
        self.blend(self.bounds(), 0x0000_0000, 150);
        let cell = font.cell();
        self.fill(geometry.panel, 0x0020_2028);
        self.stroke(geometry.panel, theme.accent);
        self.fill(geometry.title, 0x0029_2934);
        self.text_elided(
            font,
            (geometry.title.x + 12, geometry.title.y + 6),
            style.title,
            TextStyle::BOLD,
            theme.text_primary,
            geometry.title.w.saturating_sub(24),
        );
        if let (Some(query), Some(rect)) = (style.query, geometry.query) {
            let text = if query.is_empty() {
                "type to filter"
            } else {
                query
            };
            let color = if query.is_empty() {
                theme.text_tertiary
            } else {
                theme.text_primary
            };
            self.text_elided(
                font,
                (rect.x + 12, rect.y + 4),
                text,
                TextStyle::NORMAL,
                color,
                rect.w.saturating_sub(24),
            );
        }
        let row_height = cell.height + 8;
        let rects = crate::panel::row_rects(geometry.rows, style.rows.len(), row_height);
        for (index, rect) in rects.iter().enumerate() {
            let Some(row) = style.rows.get(style.first + index) else {
                break;
            };
            if style.first + index == style.selected {
                self.fill(*rect, theme.sidebar_active);
                self.fill(Rect::new(rect.x, rect.y, 3, rect.h), theme.accent);
            }
            let y = rect.y + (rect.h.saturating_sub(cell.height) / 2);
            let detail_width = (rect.w / 2).max(cell.width * 6);
            self.text_elided(
                font,
                (rect.x + 12, y),
                &row.label,
                TextStyle::NORMAL,
                theme.text_primary,
                rect.w.saturating_sub(detail_width + 24),
            );
            if !row.detail.is_empty() {
                self.text_elided(
                    font,
                    (rect.right().saturating_sub(detail_width), y),
                    &row.detail,
                    TextStyle::NORMAL,
                    row.color.unwrap_or(theme.text_tertiary),
                    detail_width.saturating_sub(12),
                );
            }
        }
        self.text_elided(
            font,
            (geometry.footer.x + 12, geometry.footer.y + 4),
            style.hint,
            TextStyle::NORMAL,
            theme.text_tertiary,
            geometry.footer.w.saturating_sub(24),
        );
    }
}

pub struct ScreenOptions<'a> {
    pub theme: &'a Theme,
    pub focused: bool,
    /// Blinks off half the time; false for an unfocused pane.
    pub cursor_visible: bool,
    pub selection: Option<&'a Selection>,
    pub exited: bool,
}

fn rgb(color: [u8; 3]) -> u32 {
    (u32::from(color[0]) << 16) | (u32::from(color[1]) << 8) | u32::from(color[2])
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let ch = chars.next()?;
    chars.next().is_none().then_some(ch)
}

/// Sub-cell coverage of a Unicode block element, in eighths. Filling these
/// rectangles tiles seamlessly, which a font glyph from a fallback face does
/// not: pixel art such as a mascot needs every cell to be exactly covered.
fn block_coverage(ch: char) -> Option<&'static [(u8, u8, u8, u8)]> {
    Some(match ch {
        '\u{2580}' => &[(0, 0, 8, 4)],
        '\u{2581}' => &[(0, 7, 8, 8)],
        '\u{2582}' => &[(0, 6, 8, 8)],
        '\u{2583}' => &[(0, 5, 8, 8)],
        '\u{2584}' => &[(0, 4, 8, 8)],
        '\u{2585}' => &[(0, 3, 8, 8)],
        '\u{2586}' => &[(0, 2, 8, 8)],
        '\u{2587}' => &[(0, 1, 8, 8)],
        '\u{2588}' => &[(0, 0, 8, 8)],
        '\u{2589}' => &[(0, 0, 7, 8)],
        '\u{258A}' => &[(0, 0, 6, 8)],
        '\u{258B}' => &[(0, 0, 5, 8)],
        '\u{258C}' => &[(0, 0, 4, 8)],
        '\u{258D}' => &[(0, 0, 3, 8)],
        '\u{258E}' => &[(0, 0, 2, 8)],
        '\u{258F}' => &[(0, 0, 1, 8)],
        '\u{2590}' => &[(4, 0, 8, 8)],
        '\u{2594}' => &[(0, 0, 8, 1)],
        '\u{2595}' => &[(7, 0, 8, 8)],
        '\u{2596}' => &[(0, 4, 4, 8)],
        '\u{2597}' => &[(4, 4, 8, 8)],
        '\u{2598}' => &[(0, 0, 4, 4)],
        '\u{2599}' => &[(0, 0, 4, 8), (4, 4, 8, 8)],
        '\u{259A}' => &[(0, 0, 4, 4), (4, 4, 8, 8)],
        '\u{259B}' => &[(0, 0, 4, 8), (4, 0, 8, 4)],
        '\u{259C}' => &[(0, 0, 8, 4), (4, 4, 8, 8)],
        '\u{259D}' => &[(4, 0, 8, 4)],
        '\u{259E}' => &[(4, 0, 8, 4), (0, 4, 4, 8)],
        '\u{259F}' => &[(0, 4, 8, 8), (4, 0, 8, 4)],
        _ => return None,
    })
}

/// One right-to-left phrase on a row, shaped and drawn as a single string.
#[derive(Debug, PartialEq, Eq)]
struct RtlRun {
    start: usize,
    end: usize,
    text: String,
}

/// Group a row into right-to-left runs.
///
/// A run starts at an RTL letter and carries the spaces between RTL words and
/// punctuation glued to the preceding letter. Table padding ends it, so the
/// phrase stays right-aligned inside its own field instead of the whole row.
fn rtl_runs(cells: &[CellView], base: usize, cols: usize) -> Vec<RtlRun> {
    let row = &cells[base.min(cells.len())..(base + cols).min(cells.len())];
    let mut runs = Vec::new();
    let mut i = 0;
    while i < row.len() {
        let first = &row[i];
        if first.wide || first.wide_continuation || !starts_rtl(first) {
            i += 1;
            continue;
        }
        let mut text = String::new();
        let mut glued = false;
        let mut j = i;
        while j < row.len() {
            let cell = &row[j];
            if cell.wide
                || cell.wide_continuation
                || cell.bold != first.bold
                || cell.italic != first.italic
                || cell.dim != first.dim
                || cell.inverse != first.inverse
                || cell.fg != first.fg
            {
                break;
            }
            let content = cell.contents.as_str();
            let Some(ch) = content.chars().next() else {
                break;
            };
            if is_rtl_char(ch) {
                text.push_str(content);
                glued = true;
                j += 1;
                continue;
            }
            if content == " " {
                let mut k = j;
                while k < row.len() && row[k].contents == " " {
                    k += 1;
                }
                if k < row.len() && starts_rtl(&row[k]) {
                    text.push_str(&" ".repeat(k - j));
                    j = k;
                    continue;
                }
                break;
            }
            if glued && !ch.is_alphanumeric() {
                // Punctuation attached to the previous letter travels with it.
                text.push_str(content);
                glued = false;
                j += 1;
                continue;
            }
            break;
        }
        if j > i {
            runs.push(RtlRun {
                start: i,
                end: j,
                text,
            });
        }
        i = j.max(i + 1);
    }
    runs
}

fn starts_rtl(cell: &CellView) -> bool {
    cell.contents.chars().next().is_some_and(is_rtl_char)
}

/// RTL script blocks: Hebrew, Arabic and its supplements, Syriac, Thaana, NKo,
/// Samaritan, Mandaic, the presentation forms, and the supplementary planes.
fn is_rtl_char(ch: char) -> bool {
    matches!(
        u32::from(ch),
        0x0590..=0x05FF
            | 0x0600..=0x06FF
            | 0x0700..=0x074F
            | 0x0750..=0x077F
            | 0x0780..=0x07BF
            | 0x07C0..=0x07FF
            | 0x0800..=0x08FF
            | 0xFB1D..=0xFB4F
            | 0xFB50..=0xFDFF
            | 0xFE70..=0xFEFF
            | 0x10800..=0x10FFF
            | 0x1E800..=0x1EFFF
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn painter<'a>(buffer: &'a mut [u32]) -> Painter<'a> {
        Painter::new(buffer, 10, 10)
    }

    fn row(texts: &[&str]) -> Vec<CellView> {
        texts
            .iter()
            .map(|text| CellView {
                contents: (*text).to_string(),
                ..CellView::default()
            })
            .collect()
    }

    #[test]
    fn given_a_missing_quadrant_block_when_drawn_then_that_quadrant_stays_empty() {
        let mut buffer = vec![0u32; 100];
        let mut painter = painter(&mut buffer);
        let cell = CellSize {
            width: 10,
            height: 10,
        };
        assert!(painter.block_glyph((0, 0), cell, '▛', 0x00ff_ffff));
        assert_eq!(buffer[0], 0x00ff_ffff);
        assert_eq!(buffer[2 * 10 + 7], 0x00ff_ffff);
        assert_eq!(buffer[7 * 10 + 2], 0x00ff_ffff);
        assert_eq!(buffer[7 * 10 + 7], 0, "the lower right quadrant is empty");
    }

    #[test]
    fn given_a_non_block_character_then_the_block_painter_declines() {
        let mut buffer = vec![0u32; 100];
        let mut painter = painter(&mut buffer);
        let cell = CellSize {
            width: 10,
            height: 10,
        };
        assert!(!painter.block_glyph((0, 0), cell, 'a', 0x00ff_ffff));
        assert!(painter.block_glyph((0, 0), cell, '▁', 0x00ff_ffff));
    }

    #[test]
    fn given_an_arabic_phrase_then_spaces_and_a_trailing_period_join_one_run() {
        let cells = row(&[
            "ا", "ل", "ت", "ب", "غ", " ", "ض", "ا", "ر", " ", "ب", "ا", "ل", "ص", "ح", "ة", ".",
            " ", " ", "x",
        ]);
        let runs = rtl_runs(&cells, 0, cells.len());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].start, 0);
        assert_eq!(runs[0].end, 17);
        assert_eq!(runs[0].text, "التبغ ضار بالصحة.");
    }

    #[test]
    fn given_a_latin_row_then_no_rtl_run_is_grouped() {
        let cells = row(&["a", "r", "_", "S", "A", " ", "x"]);
        assert!(rtl_runs(&cells, 0, cells.len()).is_empty());
    }

    #[test]
    fn given_arabic_followed_by_table_padding_then_the_padding_stays_out() {
        let cells = row(&["ا", "ب", " ", " ", " ", "│"]);
        let runs = rtl_runs(&cells, 0, cells.len());
        assert_eq!(runs.len(), 1);
        assert_eq!((runs[0].start, runs[0].end), (0, 2));
    }

    #[test]
    fn given_a_style_change_inside_the_phrase_then_the_run_splits_there() {
        let mut cells = row(&["ا", "ب", "ج"]);
        cells[1].bold = true;
        cells[2].bold = true;
        assert_eq!(rtl_runs(&cells, 0, cells.len()).len(), 2);
    }

    #[test]
    fn given_a_row_offset_then_run_indices_are_relative_to_the_row() {
        let mut cells = row(&["x", "ا", "ب"]);
        cells.extend(row(&["c"]));
        let runs = rtl_runs(&cells, 1, 2);
        assert_eq!((runs[0].start, runs[0].end), (0, 2));
        assert_eq!(runs[0].text, "اب");
    }

    #[test]
    fn given_a_rect_when_filled_then_only_that_rect_changes() {
        let mut buffer = vec![0u32; 100];
        let mut painter = painter(&mut buffer);
        painter.fill(Rect::new(2, 3, 4, 2), 0x00ff_0000);
        assert_eq!(buffer[3 * 10 + 2], 0x00ff_0000);
        assert_eq!(buffer[3 * 10 + 5], 0x00ff_0000);
        assert_eq!(buffer[3 * 10 + 6], 0);
        assert_eq!(buffer[2 * 10 + 2], 0);
    }

    #[test]
    fn given_a_rect_outside_the_buffer_when_filled_then_nothing_panics() {
        let mut buffer = vec![0u32; 100];
        let mut painter = painter(&mut buffer);
        painter.fill(Rect::new(50, 50, 10, 10), 0x00ff_ffff);
        painter.fill(Rect::new(9, 9, 4, 4), 0x0000_ff00);
        assert_eq!(buffer[9 * 10 + 9], 0x0000_ff00);
    }

    #[test]
    fn given_a_half_alpha_blend_when_applied_then_the_result_is_between() {
        let mut buffer = vec![0u32; 100];
        let mut painter = painter(&mut buffer);
        painter.fill(Rect::new(0, 0, 10, 10), 0x0000_0000);
        painter.blend(Rect::new(0, 0, 1, 1), 0x00ff_ffff, 128);
        let pixel = buffer[0];
        let channel = (pixel >> 16) & 0xff;
        assert!(channel > 100 && channel < 160, "channel {channel}");
    }

    #[test]
    fn given_a_stroke_when_drawn_then_the_edges_are_set_and_the_middle_is_not() {
        let mut buffer = vec![0u32; 100];
        let mut painter = painter(&mut buffer);
        painter.stroke(Rect::new(1, 1, 6, 6), 0x00ff_ffff);
        assert_eq!(buffer[11], 0x00ff_ffff);
        assert_eq!(buffer[6 * 10 + 6], 0x00ff_ffff);
        assert_eq!(buffer[4 * 10 + 4], 0);
    }
}
