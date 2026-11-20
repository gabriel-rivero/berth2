//! Frame painting: one CPU buffer, real glyphs, no GPU.
//!
//! Everything here works on pixels and view models; it never reads `AppState`.
//! Alpha blending is done per pixel, so glyph edges and the selection tint look
//! the same on every platform.

use berth_headless::cursor::CursorShape;
use berth_headless::style::UnderlineStyle;
use berth_headless::{CellView, ScreenView};

use crate::font::{Bitmap, CellSize, FontRenderer, Raster, TextStyle};
use crate::layout::Rect;
use crate::theme::Theme;
use crate::ui::{PaneView, RowKind, Selection, SidebarRow, WorkspaceView};

/// Cubes used to tint the selection and the inactive window.
const SELECTION_ALPHA: u8 = 96;

/// Row and column diacritics from the kitty spec, in codepoint order.
const DIACRITICS: [char; 297] = [
    '\u{0305}',
    '\u{030D}',
    '\u{030E}',
    '\u{0310}',
    '\u{0312}',
    '\u{033D}',
    '\u{033E}',
    '\u{033F}',
    '\u{0346}',
    '\u{034A}',
    '\u{034B}',
    '\u{034C}',
    '\u{0350}',
    '\u{0351}',
    '\u{0352}',
    '\u{0357}',
    '\u{035B}',
    '\u{0363}',
    '\u{0364}',
    '\u{0365}',
    '\u{0366}',
    '\u{0367}',
    '\u{0368}',
    '\u{0369}',
    '\u{036A}',
    '\u{036B}',
    '\u{036C}',
    '\u{036D}',
    '\u{036E}',
    '\u{036F}',
    '\u{0483}',
    '\u{0484}',
    '\u{0485}',
    '\u{0486}',
    '\u{0487}',
    '\u{0592}',
    '\u{0593}',
    '\u{0594}',
    '\u{0595}',
    '\u{0597}',
    '\u{0598}',
    '\u{0599}',
    '\u{059C}',
    '\u{059D}',
    '\u{059E}',
    '\u{059F}',
    '\u{05A0}',
    '\u{05A1}',
    '\u{05A8}',
    '\u{05A9}',
    '\u{05AB}',
    '\u{05AC}',
    '\u{05AF}',
    '\u{05C4}',
    '\u{0610}',
    '\u{0611}',
    '\u{0612}',
    '\u{0613}',
    '\u{0614}',
    '\u{0615}',
    '\u{0616}',
    '\u{0617}',
    '\u{0657}',
    '\u{0658}',
    '\u{0659}',
    '\u{065A}',
    '\u{065B}',
    '\u{065D}',
    '\u{065E}',
    '\u{06D6}',
    '\u{06D7}',
    '\u{06D8}',
    '\u{06D9}',
    '\u{06DA}',
    '\u{06DB}',
    '\u{06DC}',
    '\u{06DF}',
    '\u{06E0}',
    '\u{06E1}',
    '\u{06E2}',
    '\u{06E4}',
    '\u{06E7}',
    '\u{06E8}',
    '\u{06EB}',
    '\u{06EC}',
    '\u{0730}',
    '\u{0732}',
    '\u{0733}',
    '\u{0735}',
    '\u{0736}',
    '\u{073A}',
    '\u{073D}',
    '\u{073F}',
    '\u{0740}',
    '\u{0741}',
    '\u{0743}',
    '\u{0745}',
    '\u{0747}',
    '\u{0749}',
    '\u{074A}',
    '\u{07EB}',
    '\u{07EC}',
    '\u{07ED}',
    '\u{07EE}',
    '\u{07EF}',
    '\u{07F0}',
    '\u{07F1}',
    '\u{07F3}',
    '\u{0816}',
    '\u{0817}',
    '\u{0818}',
    '\u{0819}',
    '\u{081B}',
    '\u{081C}',
    '\u{081D}',
    '\u{081E}',
    '\u{081F}',
    '\u{0820}',
    '\u{0821}',
    '\u{0822}',
    '\u{0823}',
    '\u{0825}',
    '\u{0826}',
    '\u{0827}',
    '\u{0829}',
    '\u{082A}',
    '\u{082B}',
    '\u{082C}',
    '\u{082D}',
    '\u{0951}',
    '\u{0953}',
    '\u{0954}',
    '\u{0F82}',
    '\u{0F83}',
    '\u{0F86}',
    '\u{0F87}',
    '\u{135D}',
    '\u{135E}',
    '\u{135F}',
    '\u{17DD}',
    '\u{193A}',
    '\u{1A17}',
    '\u{1A75}',
    '\u{1A76}',
    '\u{1A77}',
    '\u{1A78}',
    '\u{1A79}',
    '\u{1A7A}',
    '\u{1A7B}',
    '\u{1A7C}',
    '\u{1B6B}',
    '\u{1B6D}',
    '\u{1B6E}',
    '\u{1B6F}',
    '\u{1B70}',
    '\u{1B71}',
    '\u{1B72}',
    '\u{1B73}',
    '\u{1CD0}',
    '\u{1CD1}',
    '\u{1CD2}',
    '\u{1CDA}',
    '\u{1CDB}',
    '\u{1CE0}',
    '\u{1DC0}',
    '\u{1DC1}',
    '\u{1DC3}',
    '\u{1DC4}',
    '\u{1DC5}',
    '\u{1DC6}',
    '\u{1DC7}',
    '\u{1DC8}',
    '\u{1DC9}',
    '\u{1DCB}',
    '\u{1DCC}',
    '\u{1DD1}',
    '\u{1DD2}',
    '\u{1DD3}',
    '\u{1DD4}',
    '\u{1DD5}',
    '\u{1DD6}',
    '\u{1DD7}',
    '\u{1DD8}',
    '\u{1DD9}',
    '\u{1DDA}',
    '\u{1DDB}',
    '\u{1DDC}',
    '\u{1DDD}',
    '\u{1DDE}',
    '\u{1DDF}',
    '\u{1DE0}',
    '\u{1DE1}',
    '\u{1DE2}',
    '\u{1DE3}',
    '\u{1DE4}',
    '\u{1DE5}',
    '\u{1DE6}',
    '\u{1DFE}',
    '\u{20D0}',
    '\u{20D1}',
    '\u{20D4}',
    '\u{20D5}',
    '\u{20D6}',
    '\u{20D7}',
    '\u{20DB}',
    '\u{20DC}',
    '\u{20E1}',
    '\u{20E7}',
    '\u{20E9}',
    '\u{20F0}',
    '\u{2CEF}',
    '\u{2CF0}',
    '\u{2CF1}',
    '\u{2DE0}',
    '\u{2DE1}',
    '\u{2DE2}',
    '\u{2DE3}',
    '\u{2DE4}',
    '\u{2DE5}',
    '\u{2DE6}',
    '\u{2DE7}',
    '\u{2DE8}',
    '\u{2DE9}',
    '\u{2DEA}',
    '\u{2DEB}',
    '\u{2DEC}',
    '\u{2DED}',
    '\u{2DEE}',
    '\u{2DEF}',
    '\u{2DF0}',
    '\u{2DF1}',
    '\u{2DF2}',
    '\u{2DF3}',
    '\u{2DF4}',
    '\u{2DF5}',
    '\u{2DF6}',
    '\u{2DF7}',
    '\u{2DF8}',
    '\u{2DF9}',
    '\u{2DFA}',
    '\u{2DFB}',
    '\u{2DFC}',
    '\u{2DFD}',
    '\u{2DFE}',
    '\u{2DFF}',
    '\u{A66F}',
    '\u{A67C}',
    '\u{A67D}',
    '\u{A6F0}',
    '\u{A6F1}',
    '\u{A8E0}',
    '\u{A8E1}',
    '\u{A8E2}',
    '\u{A8E3}',
    '\u{A8E4}',
    '\u{A8E5}',
    '\u{A8E6}',
    '\u{A8E7}',
    '\u{A8E8}',
    '\u{A8E9}',
    '\u{A8EA}',
    '\u{A8EB}',
    '\u{A8EC}',
    '\u{A8ED}',
    '\u{A8EE}',
    '\u{A8EF}',
    '\u{A8F0}',
    '\u{A8F1}',
    '\u{AAB0}',
    '\u{AAB2}',
    '\u{AAB3}',
    '\u{AAB7}',
    '\u{AAB8}',
    '\u{AABE}',
    '\u{AABF}',
    '\u{AAC1}',
    '\u{FE20}',
    '\u{FE21}',
    '\u{FE22}',
    '\u{FE23}',
    '\u{FE24}',
    '\u{FE25}',
    '\u{FE26}',
    '\u{10A0F}',
    '\u{10A38}',
    '\u{1D185}',
    '\u{1D186}',
    '\u{1D187}',
    '\u{1D188}',
    '\u{1D189}',
    '\u{1D1AA}',
    '\u{1D1AB}',
    '\u{1D1AC}',
    '\u{1D1AD}',
    '\u{1D242}',
    '\u{1D243}',
    '\u{1D244}',
];

/// Tile of an image for one placeholder cell, as `(cols, rows, col, row)`.
/// Kitty encodes the position in two diacritics after U+10EEEE. When they are
/// absent the cells are taken in reading order on a grid derived from the
/// image size. `order` counts the cells of this image seen so far.
fn placeholder_tile(
    text: &str,
    extents: Option<(usize, usize)>,
    order: &mut usize,
    image: (u32, u32),
    cell: (usize, usize),
) -> (usize, usize, usize, usize) {
    let mut chars = text.chars();
    chars.next();
    let row = chars.next().and_then(diacritic_index);
    let col = chars.next().and_then(diacritic_index);
    if let (Some(row), Some(col)) = (row, col) {
        let (max_row, max_col) = extents.unwrap_or((row, col));
        return ((max_col + 1).max(1), (max_row + 1).max(1), col, row);
    }
    let cols = (image.0 as usize).div_ceil(cell.0.max(1)).max(1);
    let rows = (image.1 as usize).div_ceil(cell.1.max(1)).max(1);
    let index = *order;
    *order += 1;
    (cols, rows, index % cols, index / cols)
}

/// Index of a placeholder diacritic, which encodes a row or column number.
fn diacritic_index(ch: char) -> Option<usize> {
    DIACRITICS.iter().position(|candidate| *candidate == ch)
}

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

    /// Draw a decoded RGBA image scaled into a box, clipped to a pane.
    #[allow(clippy::too_many_arguments)]
    pub fn image(
        &mut self,
        x: i32,
        y: i32,
        width: usize,
        height: usize,
        pixels: &[u8],
        source_width: u32,
        source_height: u32,
        clip: Rect,
    ) {
        if width == 0 || height == 0 || source_width == 0 || source_height == 0 {
            return;
        }
        if pixels.len() < source_width as usize * source_height as usize * 4 {
            return;
        }
        let Some(clip) = Rect::new(x.max(0) as usize, y.max(0) as usize, width, height)
            .intersect(clip)
            .and_then(|rect| rect.intersect(self.bounds()))
        else {
            return;
        };
        for gy in clip.y..clip.bottom() {
            let sy = ((gy as i32 - y) as usize * source_height as usize / height)
                .min(source_height as usize - 1);
            for gx in clip.x..clip.right() {
                let sx = ((gx as i32 - x) as usize * source_width as usize / width)
                    .min(source_width as usize - 1);
                let index = (sy * source_width as usize + sx) * 4;
                let alpha = pixels[index + 3];
                if alpha == 0 {
                    continue;
                }
                self.blend_pixel(
                    gx,
                    gy,
                    pixels[index],
                    pixels[index + 1],
                    pixels[index + 2],
                    alpha,
                );
            }
        }
    }

    /// Draw one source rectangle of an RGBA image into a box, clipped.
    #[allow(clippy::too_many_arguments)]
    fn image_region(
        &mut self,
        origin: (usize, usize),
        width: usize,
        height: usize,
        pixels: &[u8],
        source_width: u32,
        source_height: u32,
        sx: usize,
        sy: usize,
        sw: usize,
        sh: usize,
        clip: Rect,
    ) {
        if width == 0
            || height == 0
            || source_width == 0
            || source_height == 0
            || sw == 0
            || sh == 0
        {
            return;
        }
        if pixels.len() < source_width as usize * source_height as usize * 4 {
            return;
        }
        let Some(clip) = Rect::new(origin.0, origin.1, width, height)
            .intersect(clip)
            .and_then(|rect| rect.intersect(self.bounds()))
        else {
            return;
        };
        let sx = sx.min(source_width as usize - 1);
        let sy = sy.min(source_height as usize - 1);
        let sw = sw.min(source_width as usize - sx);
        let sh = sh.min(source_height as usize - sy);
        for gy in clip.y..clip.bottom() {
            let sy_pix = sy + (gy - origin.1) * sh / height;
            for gx in clip.x..clip.right() {
                let sx_pix = sx + (gx - origin.0) * sw / width;
                let index = (sy_pix.min(source_height as usize - 1) * source_width as usize
                    + sx_pix.min(source_width as usize - 1))
                    * 4;
                let alpha = pixels[index + 3];
                if alpha == 0 {
                    continue;
                }
                self.blend_pixel(
                    gx,
                    gy,
                    pixels[index],
                    pixels[index + 1],
                    pixels[index + 2],
                    alpha,
                );
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

    /// Draw one underline variant in a cell. The width may span a wide glyph.
    fn underline(
        &mut self,
        origin: (usize, usize),
        width: usize,
        style: UnderlineStyle,
        color: u32,
        height: usize,
    ) {
        let baseline = origin.1 + height.saturating_sub(3);
        match style {
            UnderlineStyle::None => {}
            UnderlineStyle::Single => {
                self.fill(Rect::new(origin.0, baseline, width, 1), color);
            }
            UnderlineStyle::Double => {
                self.fill(
                    Rect::new(origin.0, baseline.saturating_sub(2), width, 1),
                    color,
                );
                self.fill(Rect::new(origin.0, baseline, width, 1), color);
            }
            UnderlineStyle::Curly => {
                let mut x = 0;
                while x < width {
                    let y = baseline + ((x / 4) % 2);
                    self.fill(Rect::new(origin.0 + x, y, (width - x).min(2), 1), color);
                    x += 2;
                }
            }
            UnderlineStyle::Dotted => {
                for x in (0..width).step_by(3) {
                    self.fill(Rect::new(origin.0 + x, baseline, 1, 1), color);
                }
            }
            UnderlineStyle::Dashed => {
                for x in (0..width).step_by(4) {
                    self.fill(
                        Rect::new(origin.0 + x, baseline, (width - x).min(2), 1),
                        color,
                    );
                }
            }
        }
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
        let cursor_cell = cursor_visible.then(|| {
            usize::from(view.cursor.0) * usize::from(view.cols) + usize::from(view.cursor.1)
        });
        let mut placeholder_counts: std::collections::HashMap<u32, usize> =
            std::collections::HashMap::new();
        // Placeholder areas carry no explicit size, so the grid extent comes
        // from the cells themselves: (max row, max column) per image id.
        let mut placeholder_extents: std::collections::HashMap<u32, (usize, usize)> =
            std::collections::HashMap::new();
        for cell_view in &view.cells {
            let mut chars = cell_view.contents.chars();
            if chars.next() != Some('\u{10EEEE}') {
                continue;
            }
            let Some(fg) = cell_view.fg else {
                continue;
            };
            let id = (u32::from(fg[0]) << 16) | (u32::from(fg[1]) << 8) | u32::from(fg[2]);
            if !view.placeholders.iter().any(|image| image.id == id) {
                continue;
            }
            let row = chars.next().and_then(diacritic_index);
            let col = chars.next().and_then(diacritic_index);
            if let (Some(row), Some(col)) = (row, col) {
                let entry = placeholder_extents.entry(id).or_insert((0, 0));
                entry.0 = entry.0.max(row);
                entry.1 = entry.1.max(col);
            }
        }
        for row in 0..rows {
            let base = row * usize::from(view.cols);
            let runs = shaping_runs(&view.cells, base, cols, cursor_cell);
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
                let block_cursor = on_cursor && view.cursor_style.shape == CursorShape::Block;
                if cell_view.inverse || block_cursor {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if !options.focused {
                    fg = options.theme.inactive(fg);
                }
                let decoration = fg;
                let blink_off = cell_view.blink && !options.blink_on;
                self.fill(Rect::new(origin.0, origin.1, cell.width, cell.height), bg);
                if cell_view.wide {
                    // Fill both halves before the glyph: a wide glyph such as an
                    // emoji extends into the continuation cell, so filling that
                    // cell after the draw cut the right half off.
                    let next = Rect::new(origin.0 + cell.width, origin.1, cell.width, cell.height);
                    self.fill(next, bg);
                }
                let search_hit = options.search.is_some_and(|(line, start, len)| {
                    let absolute = view.scrollback_len - view.scrollback + row;
                    absolute == line && col >= start && col < start + len
                });
                if selected || search_hit {
                    self.blend(
                        Rect::new(origin.0, origin.1, cell.width, cell.height),
                        options.theme.selection,
                        SELECTION_ALPHA,
                    );
                }
                let text = cell_view.contents.as_str();
                if !blink_off
                    && text.starts_with('\u{10EEEE}')
                    && let Some(fg) = cell_view.fg
                {
                    let id = (u32::from(fg[0]) << 16) | (u32::from(fg[1]) << 8) | u32::from(fg[2]);
                    if let Some(image) = view.placeholders.iter().find(|image| image.id == id) {
                        let order = placeholder_counts.entry(id).or_insert(0);
                        let (cols, rows, col_index, row_index) = placeholder_tile(
                            text,
                            placeholder_extents.get(&id).copied(),
                            order,
                            (image.width, image.height),
                            (cell.width, cell.height),
                        );
                        let sw = (image.width as usize / cols).max(1);
                        let sh = (image.height as usize / rows).max(1);
                        self.image_region(
                            origin,
                            cell.width,
                            cell.height,
                            &image.pixels,
                            image.width,
                            image.height,
                            col_index * sw,
                            row_index * sh,
                            sw,
                            sh,
                            area,
                        );
                        continue;
                    }
                }
                let style = TextStyle {
                    bold: cell_view.bold,
                    italic: cell_view.italic,
                };
                let in_run = runs.iter().any(|run| col >= run.start && col < run.end);
                let mut painted = false;
                if !blink_off {
                    if !in_run && let Some(ch) = single_char(text) {
                        painted = self.block_glyph(origin, cell, ch, fg);
                    }
                    if !in_run && !painted && !text.is_empty() && text != " " {
                        self.glyph(font, origin, text, style, fg);
                    }
                }
                let hovered = options.hover_link.is_some()
                    && cell_view.hyperlink.as_deref() == options.hover_link;
                let underline_style = if hovered {
                    UnderlineStyle::Single
                } else if cell_view.underline_style != UnderlineStyle::None {
                    cell_view.underline_style
                } else if cell_view.underline {
                    UnderlineStyle::Single
                } else {
                    UnderlineStyle::None
                };
                let width = if cell_view.wide {
                    cell.width * 2
                } else {
                    cell.width
                };
                if !blink_off && underline_style != UnderlineStyle::None {
                    let color = if hovered && cell_view.underline_style == UnderlineStyle::None {
                        options.theme.accent
                    } else {
                        cell_view.underline_color.map(rgb).unwrap_or(decoration)
                    };
                    self.underline(origin, width, underline_style, color, cell.height);
                }
                if !blink_off && cell_view.strikethrough {
                    let y = origin.1 + cell.height / 2;
                    self.fill(Rect::new(origin.0, y, width, 1), decoration);
                }
                if on_cursor && let Some(rect) = cursor_rect(view.cursor_style.shape, origin, cell)
                {
                    self.fill(rect, options.theme.terminal_fg);
                }
            }
            for run in &runs {
                let first = &view.cells[base + run.start];
                // A run is all one blink state, so the off phase hides it whole.
                if first.blink && !options.blink_on {
                    continue;
                }
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
            self.fill(track, options.theme.scrollbar);
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
        self.fill(banner, theme.banner_bg);
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
                name,
                collapsed,
                color,
                ..
            } => {
                self.fill(row.rect, theme.sidebar_bg);
                if let Some(color) = crate::ui::parse_color(color.as_deref()) {
                    self.fill(
                        Rect::new(
                            row.rect.x + 2,
                            row.rect.y + 4,
                            3,
                            row.rect.h.saturating_sub(8),
                        ),
                        color,
                    );
                }
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
        if let Some(color) = crate::ui::parse_color(workspace.color.as_deref()) {
            self.fill(
                Rect::new(
                    rect.right().saturating_sub(4),
                    rect.y + 4,
                    3,
                    rect.h.saturating_sub(8),
                ),
                color,
            );
        }
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
        self.fill(geometry.panel, theme.panel_bg);
        self.stroke(geometry.panel, theme.accent);
        self.fill(geometry.title, theme.panel_title_bg);
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
    /// Absolute line, column and length of a search match to highlight.
    pub search: Option<(usize, usize, usize)>,
    /// The URL under the pointer, so every cell of it underlines on hover.
    pub hover_link: Option<&'a str>,
    /// The text blink phase: false hides blinking cells for this frame.
    pub blink_on: bool,
}

fn rgb(color: [u8; 3]) -> u32 {
    (u32::from(color[0]) << 16) | (u32::from(color[1]) << 8) | u32::from(color[2])
}

/// The bar or underline to draw for a non-block cursor.
fn cursor_rect(shape: CursorShape, origin: (usize, usize), cell: CellSize) -> Option<Rect> {
    match shape {
        CursorShape::Block => None,
        CursorShape::Underline => Some(Rect::new(
            origin.0,
            origin.1 + cell.height.saturating_sub(2),
            cell.width,
            2,
        )),
        CursorShape::Bar => Some(Rect::new(origin.0, origin.1, 2, cell.height)),
    }
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

/// One shaped run on a row, drawn as a single string.
#[derive(Debug, PartialEq, Eq)]
struct ShapeRun {
    start: usize,
    end: usize,
    text: String,
}

/// Every run that should be shaped as one string: right-to-left script first,
/// then adjacent left-to-right cells with the same style. A ligature needs the
/// latter; a space or a style change breaks it.
fn shaping_runs(
    cells: &[CellView],
    base: usize,
    cols: usize,
    cursor: Option<usize>,
) -> Vec<ShapeRun> {
    let mut runs = rtl_runs(cells, base, cols, cursor);
    let row = &cells[base.min(cells.len())..(base + cols).min(cells.len())];
    let mut left = 0;
    while left < row.len() {
        if runs.iter().any(|run| left >= run.start && left < run.end) {
            left += 1;
            continue;
        }
        if !can_shape_ltr(&row[left], base + left, cursor) {
            left += 1;
            continue;
        }
        let first = &row[left];
        let mut text = String::new();
        let mut right = left;
        while right < row.len()
            && !runs.iter().any(|run| right >= run.start && right < run.end)
            && can_shape_ltr(&row[right], base + right, cursor)
            && same_run_style(first, &row[right])
        {
            text.push_str(&row[right].contents);
            right += 1;
        }
        if right - left >= 2 {
            runs.push(ShapeRun {
                start: left,
                end: right,
                text,
            });
        }
        left = right.max(left + 1);
    }
    runs.sort_by_key(|run| run.start);
    runs
}

fn can_shape_ltr(cell: &CellView, index: usize, cursor: Option<usize>) -> bool {
    let first = cell.contents.chars().next();
    cursor != Some(index)
        && !cell.wide
        && !cell.wide_continuation
        && !cell.blink
        && !cell.contents.is_empty()
        && cell.contents != " "
        && !cell.contents.starts_with('\u{10EEEE}')
        && first
            .is_some_and(|character| block_coverage(character).is_none() && !is_rtl_char(character))
}

fn same_run_style(first: &CellView, cell: &CellView) -> bool {
    first.bold == cell.bold
        && first.italic == cell.italic
        && first.dim == cell.dim
        && first.inverse == cell.inverse
        && first.fg == cell.fg
        // The background is the glyph colour only under inverse.
        && (!first.inverse || first.bg == cell.bg)
        && first.blink == cell.blink
}

/// Group a row into right-to-left runs.
///
/// A run starts at an RTL letter and carries the spaces between RTL words and
/// punctuation glued to the preceding letter. Table padding ends it, so the
/// phrase stays right-aligned inside its own field instead of the whole row.
fn rtl_runs(cells: &[CellView], base: usize, cols: usize, cursor: Option<usize>) -> Vec<ShapeRun> {
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
                || (first.inverse && cell.bg != first.bg)
                || cell.blink != first.blink
                || cursor == Some(base + j)
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
            runs.push(ShapeRun {
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
    fn given_a_blink_cell_when_the_phase_is_off_then_no_glyph_is_painted() {
        let pixels = paint_blink_row(vec![CellView {
            contents: "B".to_string(),
            blink: true,
            ..CellView::default()
        }]);
        assert!(
            pixels.iter().all(|pixel| *pixel == 0),
            "a blinking cell painted during the off phase"
        );
    }

    /// An RTL run forms even when every cell blinks, so the phase has to hide
    /// the whole run, not just the per-cell glyphs.
    #[test]
    fn given_a_blinking_rtl_run_then_the_off_phase_hides_it() {
        let cells = vec![
            CellView {
                contents: "\u{05D0}".to_string(),
                blink: true,
                ..CellView::default()
            },
            CellView {
                contents: "\u{05D1}".to_string(),
                blink: true,
                ..CellView::default()
            },
        ];
        assert_eq!(rtl_runs(&cells, 0, cells.len(), None).len(), 1);
        let pixels = paint_blink_row(cells);
        assert!(
            pixels.iter().all(|pixel| *pixel == 0),
            "a blinking RTL run painted during the off phase"
        );
    }

    /// Paint one row at the blink-off phase, returning the pixel buffer.
    fn paint_blink_row(cells: Vec<CellView>) -> Vec<u32> {
        let mut font = FontRenderer::new(16.0, &[], crate::font::Options::default());
        let cell = font.cell();
        let cols = cells.len();
        let mut buffer = vec![0u32; cell.width * cols * cell.height];
        let view = ScreenView {
            cols: u16::try_from(cols).expect("a short row"),
            rows: 1,
            cursor_hidden: true,
            cells,
            ..ScreenView::default()
        };
        let theme = Theme::default();
        let options = ScreenOptions {
            theme: &theme,
            focused: true,
            cursor_visible: false,
            selection: None,
            exited: false,
            search: None,
            hover_link: None,
            blink_on: false,
        };
        let mut painter = Painter::new(&mut buffer, cell.width * cols, cell.height);
        painter.screen(
            &mut font,
            &view,
            Rect::new(0, 0, cell.width * cols, cell.height),
            &options,
        );
        buffer
    }

    #[test]
    fn given_each_cursor_shape_then_the_rect_matches_the_shape() {
        let cell = CellSize {
            width: 10,
            height: 20,
        };
        assert_eq!(cursor_rect(CursorShape::Block, (0, 0), cell), None);
        let underline = cursor_rect(CursorShape::Underline, (0, 0), cell).expect("underline");
        assert_eq!((underline.y, underline.w, underline.h), (18, 10, 2));
        let bar = cursor_rect(CursorShape::Bar, (1, 2), cell).expect("bar");
        assert_eq!((bar.x, bar.y, bar.w, bar.h), (1, 2, 2, 20));
    }

    #[test]
    fn given_each_underline_style_then_it_paints_its_own_pattern() {
        let cell = CellSize {
            width: 12,
            height: 10,
        };
        let pixels = |style| {
            let mut buffer = vec![0u32; cell.width * cell.height];
            let mut painter = Painter::new(&mut buffer, cell.width, cell.height);
            painter.underline((0, 0), cell.width, style, 0x00ff_ffff, cell.height);
            buffer.iter().filter(|pixel| **pixel != 0).count()
        };
        assert_eq!(pixels(UnderlineStyle::Single), 12);
        assert_eq!(pixels(UnderlineStyle::Double), 24);
        assert_eq!(pixels(UnderlineStyle::Dotted), 4);
        assert_eq!(pixels(UnderlineStyle::Dashed), 6);
        assert_eq!(pixels(UnderlineStyle::Curly), 12);
    }

    #[test]
    fn given_an_arabic_phrase_then_spaces_and_a_trailing_period_join_one_run() {
        let cells = row(&[
            "ا", "ل", "ت", "ب", "غ", " ", "ض", "ا", "ر", " ", "ب", "ا", "ل", "ص", "ح", "ة", ".",
            " ", " ", "x",
        ]);
        let runs = rtl_runs(&cells, 0, cells.len(), None);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].start, 0);
        assert_eq!(runs[0].end, 17);
        assert_eq!(runs[0].text, "التبغ ضار بالصحة.");
    }

    #[test]
    fn given_a_latin_row_then_no_rtl_run_is_grouped() {
        let cells = row(&["a", "r", "_", "S", "A", " ", "x"]);
        assert!(rtl_runs(&cells, 0, cells.len(), None).is_empty());
    }

    #[test]
    fn given_adjacent_latin_cells_then_they_shape_as_one_run() {
        let cells = row(&["f", "i", "x"]);
        let runs = shaping_runs(&cells, 0, cells.len(), None);
        assert_eq!(runs.len(), 1);
        assert_eq!(
            (runs[0].start, runs[0].end, runs[0].text.as_str()),
            (0, 3, "fix")
        );
    }

    #[test]
    fn given_a_space_or_style_change_then_the_run_breaks() {
        let cells = row(&["a", "b", " ", "c", "d"]);
        let runs = shaping_runs(&cells, 0, cells.len(), None);
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert_eq!(runs[0].text, "ab");
        assert_eq!(runs[1].text, "cd");

        let mut cells = row(&["a", "b", "c", "d"]);
        cells[2].bold = true;
        cells[3].bold = true;
        let runs = shaping_runs(&cells, 0, cells.len(), None);
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert_eq!(runs[0].text, "ab");
        assert_eq!(runs[1].text, "cd");
    }

    #[test]
    fn given_a_background_change_outside_inverse_then_the_run_still_shapes() {
        let mut cells = row(&["a", "b", "c", "d"]);
        cells[2].bg = Some([4, 5, 6]);
        cells[3].bg = Some([4, 5, 6]);
        let runs = shaping_runs(&cells, 0, cells.len(), None);
        assert_eq!(runs.len(), 1, "{runs:?}");
        assert_eq!(runs[0].text, "abcd");
    }

    #[test]
    fn given_inverse_cells_with_different_backgrounds_then_the_run_breaks() {
        // Under inverse the glyph colour is the cell background, so merging
        // these would paint the second cell in the first one's colour.
        let mut cells = row(&["a", "b", "c", "d"]);
        for cell in cells.iter_mut() {
            cell.inverse = true;
            cell.fg = Some([1, 2, 3]);
        }
        cells[2].bg = Some([4, 5, 6]);
        cells[3].bg = Some([4, 5, 6]);
        let runs = shaping_runs(&cells, 0, cells.len(), None);
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert_eq!(runs[0].text, "ab");
        assert_eq!(runs[1].text, "cd");
    }

    #[test]
    fn given_a_blink_or_cursor_cell_then_it_is_not_shaped_in_a_run() {
        let mut cells = row(&["a", "b", "c", "d"]);
        cells[1].blink = true;
        let runs = shaping_runs(&cells, 0, cells.len(), None);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "cd");

        let cells = row(&["a", "b", "c", "d"]);
        let runs = shaping_runs(&cells, 0, cells.len(), Some(1));
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "cd");
    }

    #[test]
    fn given_a_block_glyph_then_it_is_not_shaped_in_a_run() {
        let cells = row(&["█", "█"]);
        assert!(shaping_runs(&cells, 0, cells.len(), None).is_empty());
    }

    #[test]
    fn given_arabic_followed_by_table_padding_then_the_padding_stays_out() {
        let cells = row(&["ا", "ب", " ", " ", " ", "│"]);
        let runs = rtl_runs(&cells, 0, cells.len(), None);
        assert_eq!(runs.len(), 1);
        assert_eq!((runs[0].start, runs[0].end), (0, 2));
    }

    #[test]
    fn given_a_style_change_inside_the_phrase_then_the_run_splits_there() {
        let mut cells = row(&["ا", "ب", "ج"]);
        cells[1].bold = true;
        cells[2].bold = true;
        assert_eq!(rtl_runs(&cells, 0, cells.len(), None).len(), 2);
    }

    #[test]
    fn given_a_background_change_inside_an_arabic_phrase_then_it_stays_one_run() {
        let mut cells = row(&["ا", "ب", "ج"]);
        cells[1].bg = Some([1, 2, 3]);
        cells[2].bg = Some([1, 2, 3]);
        assert_eq!(rtl_runs(&cells, 0, cells.len(), None).len(), 1);
    }

    #[test]
    fn given_inverse_arabic_cells_with_different_backgrounds_then_the_run_splits() {
        let mut cells = row(&["ا", "ب", "ج"]);
        for cell in cells.iter_mut() {
            cell.inverse = true;
            cell.fg = Some([1, 2, 3]);
        }
        cells[1].bg = Some([4, 5, 6]);
        cells[2].bg = Some([4, 5, 6]);
        assert_eq!(rtl_runs(&cells, 0, cells.len(), None).len(), 2);
    }

    #[test]
    fn given_a_row_offset_then_run_indices_are_relative_to_the_row() {
        let mut cells = row(&["x", "ا", "ب"]);
        cells.extend(row(&["c"]));
        let runs = rtl_runs(&cells, 1, 2, None);
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

#[cfg(test)]
mod placeholder_tests {
    use super::placeholder_tile;

    fn text(row: usize, col: usize) -> String {
        let mut value = "\u{10EEEE}".to_string();
        value.push(super::DIACRITICS[row]);
        value.push(super::DIACRITICS[col]);
        value
    }

    #[test]
    fn given_row_and_column_diacritics_then_the_tile_follows_them() {
        let mut order = 0;
        let tile = placeholder_tile(&text(2, 5), Some((4, 9)), &mut order, (320, 180), (10, 20));
        assert_eq!(tile, (10, 5, 5, 2));
        assert_eq!(order, 0, "diacritics do not advance the fallback order");
    }

    #[test]
    fn given_a_bare_placeholder_then_the_tile_follows_reading_order() {
        let mut order = 0;
        let first = placeholder_tile("\u{10EEEE}", None, &mut order, (30, 40), (10, 20));
        let second = placeholder_tile("\u{10EEEE}", None, &mut order, (30, 40), (10, 20));
        assert_eq!(first, (3, 2, 0, 0));
        assert_eq!(second, (3, 2, 1, 0));
    }
}
