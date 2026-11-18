//! Frame painting: one CPU buffer, real glyphs, no GPU.
//!
//! Everything here works on pixels and view models; it never reads `AppState`.
//! Alpha blending is done per pixel, so glyph edges and the selection tint look
//! the same on every platform.

use berth_headless::ScreenView;

use crate::font::{Bitmap, FontRenderer, TextStyle};
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
        cell.width
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
                if !text.is_empty() && text != " " {
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
        for (index, (row, rect)) in style.rows.iter().zip(rects.iter()).enumerate() {
            if index == style.selected {
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
            "↑↓ move · enter select · esc close",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn painter<'a>(buffer: &'a mut [u32]) -> Painter<'a> {
        Painter::new(buffer, 10, 10)
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
