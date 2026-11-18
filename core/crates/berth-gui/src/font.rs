//! Real terminal text: system font discovery, shaping and glyph bitmaps.
//!
//! One [`FontRenderer`] owns the font database, the cell metrics and a cache
//! from styled cell text to a bitmap. The cache is what makes per-cell drawing
//! affordable: a screen repeats a few hundred distinct glyphs, and each one is
//! shaped and rasterised once.
//!
//! Fallback is cosmic-text's: a character the monospace face cannot draw is
//! shaped with a system face that can, so CJK, Cyrillic and emoji render
//! instead of becoming `?`.

use std::collections::HashMap;
use std::sync::Arc;

use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, SwashCache, SwashContent, Weight,
};

/// The uniform grid cell, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellSize {
    pub width: usize,
    pub height: usize,
}

/// Weight and slant of one cell. Colour is applied at paint time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextStyle {
    pub bold: bool,
    pub italic: bool,
}

impl TextStyle {
    pub const NORMAL: Self = Self {
        bold: false,
        italic: false,
    };

    pub const BOLD: Self = Self {
        bold: true,
        italic: false,
    };
}

/// A rasterised cell. Mask glyphs are tinted with the cell's foreground.
#[derive(Clone, Debug)]
pub enum Bitmap {
    Mask(Vec<u8>),
    /// Premultiplied-free RGBA, used by colour emoji.
    Color(Vec<u8>),
}

/// One shaped cell, ready to blit.
#[derive(Clone, Debug)]
pub struct Raster {
    pub width: usize,
    pub height: usize,
    /// Top-left corner relative to the cell origin.
    pub left: i32,
    pub top: i32,
    pub bitmap: Bitmap,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CacheKey {
    text: String,
    bold: bool,
    italic: bool,
}

/// The glyph cache ceiling. Beyond this the cache is dropped; a terminal screen
/// with more distinct cells than this is scrolling thousands of unique lines.
const CACHE_LIMIT: usize = 8_192;

pub struct FontRenderer {
    font_system: FontSystem,
    swash: SwashCache,
    buffer: Buffer,
    metrics: Metrics,
    cell: CellSize,
    cache: HashMap<CacheKey, Option<Arc<Raster>>>,
}

impl FontRenderer {
    /// Build a renderer from the system monospace face at `font_size` pixels.
    /// `font_size` is already scaled for the display, not points.
    pub fn new(font_size: f32) -> Self {
        let font_size = font_size.max(4.0);
        let metrics = Metrics::relative(font_size, 1.35);
        let mut font_system = FontSystem::new();
        let mut buffer = Buffer::new(&mut font_system, metrics);
        buffer.set_size(Some(f32::MAX), Some(metrics.line_height * 2.0));

        let mut renderer = Self {
            font_system,
            swash: SwashCache::new(),
            buffer,
            metrics,
            cell: CellSize {
                width: 1,
                height: 1,
            },
            cache: HashMap::new(),
        };
        renderer.cell = renderer.measure_cell();
        renderer
    }

    pub fn cell(&self) -> CellSize {
        self.cell
    }

    /// Width of one cell's advance and height of one line, from the font's own
    /// metrics for a shaped sample.
    fn measure_cell(&mut self) -> CellSize {
        let attrs = self.attrs(TextStyle::NORMAL);
        self.buffer
            .set_size(Some(f32::MAX), Some(self.metrics.line_height * 2.0));
        self.buffer
            .set_text("MMMMMMMMMM", &attrs, Shaping::Advanced, None);
        self.buffer.shape_until_scroll(&mut self.font_system, false);
        let mut advance = 0.0f32;
        if let Some(run) = self.buffer.layout_runs().next() {
            for glyph in run.glyphs {
                advance += glyph.w;
            }
        }
        let width = (advance / 10.0).round().max(1.0) as usize;
        CellSize {
            width,
            height: self
                .metrics
                .line_height
                .round()
                .max(self.metrics.font_size)
                .max(1.0) as usize,
        }
    }

    fn attrs(&self, style: TextStyle) -> Attrs<'static> {
        let mut attrs = Attrs::new().family(Family::Monospace);
        if style.bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        if style.italic {
            attrs = attrs.style(Style::Italic);
        }
        attrs
    }

    /// Shaped bitmap for one cell, or `None` when the text has no visible
    /// pixels (a space, a control character, a missing glyph in every face).
    pub fn raster(&mut self, text: &str, style: TextStyle) -> Option<Arc<Raster>> {
        let key = CacheKey {
            text: text.to_string(),
            bold: style.bold,
            italic: style.italic,
        };
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        if self.cache.len() >= CACHE_LIMIT {
            self.cache.clear();
        }
        let raster = self.render_cell(text, style).map(Arc::new);
        self.cache.insert(key, raster.clone());
        raster
    }

    /// Try the monospace face first, then a proportional one. Some platforms
    /// answer a missing CJK character with a monospace bitmap face that swash
    /// cannot rasterise; the second shaping finds a real outline.
    fn render_cell(&mut self, text: &str, style: TextStyle) -> Option<Raster> {
        for family in [Family::Monospace, Family::SansSerif, Family::Serif] {
            let attrs = self.attrs(style).family(family);
            if let Some(raster) = self.shape_cell(text, &attrs) {
                return Some(raster);
            }
        }
        None
    }

    fn shape_cell(&mut self, text: &str, attrs: &Attrs<'_>) -> Option<Raster> {
        let line_height = self.metrics.line_height;
        let width = self.cell.width * 2;
        let height = self.cell.height;
        self.buffer
            .set_size(Some(width as f32), Some(line_height * 2.0));
        self.buffer.set_text(text, attrs, Shaping::Advanced, None);
        self.buffer.shape_until_scroll(&mut self.font_system, false);

        let mut mask = vec![0u8; width * height];
        let mut color: Option<Vec<u8>> = None;
        let mut touched = false;
        let mut left = i32::MAX;
        let mut top = i32::MAX;
        let mut right = 0i32;
        let mut bottom = 0i32;

        let run = self.buffer.layout_runs().next()?;
        {
            for glyph in run.glyphs {
                let physical = glyph.physical((0.0, run.line_y), 1.0);
                let Some(image) = self
                    .swash
                    .get_image(&mut self.font_system, physical.cache_key)
                else {
                    continue;
                };
                if image.placement.width == 0 || image.placement.height == 0 {
                    continue;
                }
                let origin_x = physical.x + image.placement.left;
                let origin_y = physical.y - image.placement.top;
                let is_color = matches!(image.content, SwashContent::Color);
                if is_color && color.is_none() {
                    let mut pixels = vec![0u8; width * height * 4];
                    for (index, alpha) in mask.iter().enumerate() {
                        pixels[index * 4 + 3] = *alpha;
                    }
                    color = Some(pixels);
                }
                let stride = image.placement.width as usize;
                for row in 0..image.placement.height as i32 {
                    for col in 0..stride as i32 {
                        let x = origin_x + col;
                        let y = origin_y + row;
                        if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                            continue;
                        }
                        let index = (y as usize) * width + x as usize;
                        let source = (row as usize) * stride + col as usize;
                        if is_color {
                            let rgba = color.as_mut().expect("colour buffer");
                            rgba[index * 4] = image.data[source * 4];
                            rgba[index * 4 + 1] = image.data[source * 4 + 1];
                            rgba[index * 4 + 2] = image.data[source * 4 + 2];
                            rgba[index * 4 + 3] = image.data[source * 4 + 3];
                        } else {
                            mask[index] = mask[index].max(image.data[source]);
                        }
                        touched = true;
                        left = left.min(x);
                        top = top.min(y);
                        right = right.max(x + 1);
                        bottom = bottom.max(y + 1);
                    }
                }
            }
        }
        if !touched {
            return None;
        }
        Some(crop(
            (left, top, right - left, bottom - top),
            (width, height),
            color.unwrap_or_default(),
            mask,
        ))
    }
}

/// Cut a cell bitmap down to the painted box.
fn crop(
    (left, top, box_width, box_height): (i32, i32, i32, i32),
    (full_width, full_height): (usize, usize),
    rgba: Vec<u8>,
    mask: Vec<u8>,
) -> Raster {
    let width = box_width.max(1) as usize;
    let height = box_height.max(1) as usize;
    let mut bitmap = if rgba.is_empty() {
        Bitmap::Mask(vec![0u8; width * height])
    } else {
        Bitmap::Color(vec![0u8; width * height * 4])
    };
    for row in 0..height {
        let source_row = top as usize + row;
        if source_row >= full_height {
            break;
        }
        for col in 0..width {
            let source_col = left as usize + col;
            if source_col >= full_width {
                break;
            }
            let source = source_row * full_width + source_col;
            let target = row * width + col;
            match &mut bitmap {
                Bitmap::Mask(destination) => destination[target] = mask[source],
                Bitmap::Color(destination) => {
                    for channel in 0..4 {
                        destination[target * 4 + channel] = rgba[source * 4 + channel];
                    }
                }
            }
        }
    }
    Raster {
        width,
        height,
        left,
        top,
        bitmap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renderer() -> FontRenderer {
        FontRenderer::new(16.0)
    }

    #[test]
    fn given_a_real_font_when_measured_then_the_cell_is_plausibly_sized() {
        let font = renderer();
        let cell = font.cell();
        assert!(cell.width >= 4, "cell width {}", cell.width);
        assert!(cell.height >= 10, "cell height {}", cell.height);
        assert!(cell.width <= 40 && cell.height <= 60);
    }

    #[test]
    fn given_a_printable_glyph_when_rasterised_then_pixels_are_produced() {
        let mut font = renderer();
        let raster = font.raster("M", TextStyle::NORMAL).expect("M rasterises");
        assert!(raster.width > 0 && raster.height > 0);
        let painted = match &raster.bitmap {
            Bitmap::Mask(alpha) => alpha.iter().any(|value| *value > 0),
            Bitmap::Color(rgba) => rgba.iter().skip(3).step_by(4).any(|value| *value > 0),
        };
        assert!(painted);
    }

    #[test]
    fn given_a_space_when_rasterised_then_nothing_is_drawn() {
        let mut font = renderer();
        assert!(font.raster(" ", TextStyle::NORMAL).is_none());
    }

    #[test]
    fn given_the_same_glyph_twice_when_rasterised_then_the_cache_is_reused() {
        let mut font = renderer();
        let first = font.raster("A", TextStyle::NORMAL).unwrap();
        let second = font.raster("A", TextStyle::NORMAL).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(font.cache.len(), 1);
    }

    #[test]
    fn given_bold_and_regular_when_rasterised_then_they_are_different_entries() {
        let mut font = renderer();
        let regular = font.raster("B", TextStyle::NORMAL).unwrap();
        let bold = font.raster("B", TextStyle::BOLD).unwrap();
        assert!(!Arc::ptr_eq(&regular, &bold));
        assert_eq!(font.cache.len(), 2);
    }

    #[test]
    fn given_a_non_latin_character_when_rasterised_then_a_bitmap_is_produced() {
        let mut font = renderer();
        // A CJK glyph has no chance in a Latin-only monospace face, so this
        // only passes when shaping falls through to a face that can draw it.
        assert!(font.raster("日", TextStyle::NORMAL).is_some());
    }

    #[test]
    fn given_an_emoji_when_rasterised_then_colour_or_mask_pixels_exist() {
        let mut font = renderer();
        assert!(font.raster("🦀", TextStyle::NORMAL).is_some());
    }
}
