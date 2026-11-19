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
    Attrs, Buffer, Family, FamilyOwned, FontSystem, Metrics, Shaping, Stretch, Style, SwashCache,
    SwashContent, Weight,
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
    /// Shaping order: the configured faces that are installed, in order, then
    /// the generic families. Text comes from the first, icons and powerline
    /// glyphs from whichever face actually has them.
    families: Vec<FamilyOwned>,
    cache: HashMap<CacheKey, Option<Arc<Raster>>>,
}

/// Every configured name that is installed, in the user's order, then the
/// generic families. Shaping walks this list, so a Nerd Font later in
/// `fontFamily` still supplies the icons and powerline separators the first
/// face lacks. Names are matched case-insensitively; an unknown name is
/// skipped rather than fatal, so a config copied from another machine opens.
fn resolve_families(font_system: &FontSystem, names: &[String]) -> Vec<FamilyOwned> {
    let mut families: Vec<FamilyOwned> = Vec::new();
    for name in names {
        let wanted = name.trim();
        if wanted.is_empty() {
            continue;
        }
        // Keep the database's own spelling: the shaping lookup is not required
        // to be case-insensitive, so echoing the user's casing can miss.
        let installed = font_system.db().faces().find_map(|face| {
            face.families
                .iter()
                .find(|(family, _)| family.eq_ignore_ascii_case(wanted))
                .map(|(family, _)| family.clone())
        });
        if let Some(installed) = installed {
            let family = FamilyOwned::Name(installed.into());
            if !families.contains(&family) {
                families.push(family);
            }
        }
    }
    if families.is_empty() {
        families.push(FamilyOwned::Monospace);
    }
    for generic in [
        FamilyOwned::Monospace,
        FamilyOwned::SansSerif,
        FamilyOwned::Serif,
    ] {
        if !families.contains(&generic) {
            families.push(generic);
        }
    }
    families
}

/// Styled attributes for one cell. `family` is passed in because the caller
/// owns the name; the renderer must stay free to borrow itself mutably.
fn attrs_for(family: Family<'_>, style: TextStyle) -> Attrs<'_> {
    let mut attrs = Attrs::new().family(family);
    if style.bold {
        attrs = attrs.weight(Weight::BOLD);
    }
    if style.italic {
        attrs = attrs.style(Style::Italic);
    }
    attrs
}

impl FontRenderer {
    /// Build a renderer at `font_size` pixels. Text comes from the first
    /// configured name the system knows; the rest supply what it lacks.
    /// `font_size` is already scaled for the display, not points.
    pub fn new(font_size: f32, families: &[String]) -> Self {
        let font_size = font_size.max(4.0);
        let metrics = Metrics::relative(font_size, 1.35);
        let mut font_system = FontSystem::new();
        let resolved = resolve_families(&font_system, families);
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
            families: resolved,
            cache: HashMap::new(),
        };
        renderer.cell = renderer.measure_cell();
        // Which face the config actually resolved to; the first thing to check
        // when a window looks like it ignored `fontFamily`.
        crate::log::line(&format!(
            "font: {:?} at {font_size:.1}px cell {}x{}",
            renderer.families, renderer.cell.width, renderer.cell.height
        ));
        renderer
    }

    pub fn cell(&self) -> CellSize {
        self.cell
    }

    /// Width of one cell's advance and height of one line, from the font's own
    /// metrics for a shaped sample.
    fn measure_cell(&mut self) -> CellSize {
        let primary = self
            .families
            .first()
            .cloned()
            .unwrap_or(FamilyOwned::Monospace);
        let attrs = attrs_for(primary.as_family(), TextStyle::NORMAL);
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

    /// Walk the shaping order until a face draws the glyph. Later faces supply
    /// what the first lacks: icons and powerline separators from a Nerd Font,
    /// CJK from a system face. Some platforms answer a missing CJK character
    /// with a monospace bitmap face swash cannot rasterise, so a later face is
    /// tried rather than giving up.
    fn render_cell(&mut self, text: &str, style: TextStyle) -> Option<Raster> {
        let families = self.families.clone();
        for family in &families {
            let attrs = attrs_for(family.as_family(), style);
            // A named face must draw the glyph itself. cosmic-text otherwise
            // substitutes another installed font, which is how a powerline
            // separator ended up as a 6x7 box from an unrelated face.
            let required = self.face_id(family, style);
            if let Some(raster) = self.shape_cell(text, &attrs, required) {
                return Some(raster);
            }
        }
        None
    }

    /// The face `family` resolves to for this style, or `None` for the generic
    /// families, which are the fallback of last resort.
    fn face_id(&self, family: &FamilyOwned, style: TextStyle) -> Option<fontdb::ID> {
        if !matches!(family, FamilyOwned::Name(_)) {
            return None;
        }
        self.font_system.db().query(&fontdb::Query {
            families: &[family.as_family()],
            weight: if style.bold {
                Weight::BOLD
            } else {
                Weight::NORMAL
            },
            stretch: Stretch::Normal,
            style: if style.italic {
                Style::Italic
            } else {
                Style::Normal
            },
        })
    }

    fn shape_cell(
        &mut self,
        text: &str,
        attrs: &Attrs<'_>,
        required: Option<fontdb::ID>,
    ) -> Option<Raster> {
        let line_height = self.metrics.line_height;
        // Icons and powerline separators reach outside the cell: a separator
        // fills the whole line height and an icon can sit left of the pen. Pad
        // the box and keep the cell origin, so no ink is dropped.
        let margin_x = (self.cell.width / 2) as i32;
        let margin_y = (self.cell.height / 2) as i32;
        let width = self.cell.width * 2 + (margin_x * 2) as usize;
        let height = self.cell.height + (margin_y * 2) as usize;
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
        if let Some(required) = required
            && run.glyphs.iter().any(|glyph| glyph.font_id != required)
        {
            // The face we asked for does not have this character; a later
            // family in the list should draw it instead.
            return None;
        }
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
                        let x = origin_x + col + margin_x;
                        let y = origin_y + row + margin_y;
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
        let mut raster = crop(
            (left, top, right - left, bottom - top),
            (width, height),
            color.unwrap_or_default(),
            mask,
        );
        raster.left -= margin_x;
        raster.top -= margin_y;
        Some(raster)
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
        FontRenderer::new(16.0, &[])
    }

    /// Advance of a shaped string, so a face that silently fell back to a
    /// proportional one can be told apart from the configured monospace.
    fn advance_of(font: &mut FontRenderer, text: &str) -> f32 {
        let primary = font
            .families
            .first()
            .cloned()
            .unwrap_or(FamilyOwned::Monospace);
        let attrs = attrs_for(primary.as_family(), TextStyle::NORMAL);
        font.buffer
            .set_size(Some(f32::MAX), Some(font.metrics.line_height * 2.0));
        font.buffer.set_text(text, &attrs, Shaping::Advanced, None);
        font.buffer.shape_until_scroll(&mut font.font_system, false);
        font.buffer
            .layout_runs()
            .next()
            .map(|run| run.glyphs.iter().map(|glyph| glyph.w).sum())
            .unwrap_or(0.0)
    }

    #[test]
    fn given_a_configured_face_when_shaped_then_every_glyph_advances_equally() {
        let installed = {
            let system = FontSystem::new();
            system.db().faces().any(|face| {
                face.families
                    .iter()
                    .any(|(name, _)| name == "VictorMono NFM")
            })
        };
        if !installed {
            // Without the face this test only re-checks the generic fallback.
            return;
        }
        let mut font = FontRenderer::new(16.0, &["VictorMono NFM".to_string()]);
        assert!(matches!(font.families.first(), Some(FamilyOwned::Name(_))));
        assert_eq!(
            advance_of(&mut font, "M"),
            advance_of(&mut font, "i"),
            "the resolved face is not monospaced"
        );
        let mut fallback = renderer();
        assert_eq!(
            advance_of(&mut fallback, "M"),
            advance_of(&mut fallback, "i"),
            "the system monospace face is not monospaced"
        );
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

    #[test]
    fn given_no_family_names_when_resolved_then_the_system_monospace_leads() {
        let system = FontSystem::new();
        assert_eq!(
            resolve_families(&system, &[]).first(),
            Some(&FamilyOwned::Monospace)
        );
    }

    #[test]
    fn given_an_uninstalled_family_name_when_resolved_then_it_falls_back_to_monospace() {
        let system = FontSystem::new();
        let names = vec!["No Such Face 12345".to_string()];
        assert_eq!(
            resolve_families(&system, &names).first(),
            Some(&FamilyOwned::Monospace)
        );
    }

    #[test]
    fn given_an_installed_family_name_when_resolved_then_that_face_leads_ignoring_case() {
        let system = FontSystem::new();
        let installed = system
            .db()
            .faces()
            .find_map(|face| face.families.first().map(|(name, _)| name.clone()))
            .expect("the font database is never empty");
        let shouted = vec![installed.to_uppercase()];
        match resolve_families(&system, &shouted).first() {
            Some(FamilyOwned::Name(name)) => assert_eq!(name.as_str(), installed),
            other => panic!("expected {installed}, resolved {other:?}"),
        }
    }

    #[test]
    fn given_two_installed_names_when_resolved_then_both_are_kept_in_order() {
        let system = FontSystem::new();
        let mut names: Vec<String> = Vec::new();
        for face in system.db().faces() {
            if let Some((family, _)) = face.families.first()
                && !names.contains(family)
            {
                names.push(family.clone());
            }
            if names.len() == 2 {
                break;
            }
        }
        let resolved = resolve_families(&system, &names);
        assert!(resolved.len() >= 2);
        match (&resolved[0], &resolved[1]) {
            (FamilyOwned::Name(first), FamilyOwned::Name(second)) => {
                assert_eq!(first.as_str(), names[0]);
                assert_eq!(second.as_str(), names[1]);
            }
            other => panic!("expected two named faces, got {other:?}"),
        }
    }

    #[test]
    fn given_a_nerd_font_later_in_the_list_when_the_first_face_lacks_an_icon_then_it_draws_it() {
        let installed = {
            let system = FontSystem::new();
            system.db().faces().any(|face| {
                face.families
                    .iter()
                    .any(|(name, _)| name == "VictorMono NFM")
            })
        };
        if !installed {
            // CI images without the Nerd Font cannot say anything about the
            // hand-over; the ordering test above still covers resolution.
            return;
        }
        let mut font =
            FontRenderer::new(13.0, &["Monaco".to_string(), "VictorMono NFM".to_string()]);
        assert!(
            matches!(font.families.first(), Some(FamilyOwned::Name(name)) if name.as_str() == "Monaco"),
            "the icon-less face must lead the list: {:?}",
            font.families
        );
        let cell = font.cell();
        // U+E0B0 is a powerline separator: it fills the line height, so a
        // raster no taller than the cell means it was clipped or came from a
        // face that drew something else.
        let raster = font
            .raster("\u{e0b0}", TextStyle::NORMAL)
            .expect("the separator rasterises");
        assert!(
            raster.height >= cell.height,
            "separator {}x{} in a {}x{} cell",
            raster.width,
            raster.height,
            cell.width,
            cell.height
        );
    }

    #[test]
    fn given_a_configured_family_when_the_renderer_shapes_a_glyph_then_it_still_draws() {
        let installed = {
            let system = FontSystem::new();
            system
                .db()
                .faces()
                .find_map(|face| face.families.first().map(|(name, _)| name.clone()))
                .expect("the font database is never empty")
        };
        let mut font = FontRenderer::new(16.0, &[installed]);
        assert!(font.raster("M", TextStyle::NORMAL).is_some());
        assert!(font.cell().width >= 4);
    }
}
