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
    Attrs, Buffer, Family, FamilyOwned, FeatureTag, FontFeatures, FontSystem, Metrics, Shaping,
    Stretch, Style, SwashCache, SwashContent, Weight, Wrap,
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

/// Renderer options from `config.json`.
#[derive(Clone, Debug)]
pub struct Options {
    /// Base weight for normal text, 100..=900.
    pub weight: u16,
    /// Line height as a multiple of the font size.
    pub line_height: f32,
    /// OpenType features and whether each is enabled.
    pub features: Vec<([u8; 4], bool)>,
    /// Per-family cell overrides, keyed by the face name.
    pub metrics: Vec<(String, FaceMetrics)>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            weight: 400,
            line_height: 1.35,
            features: Vec::new(),
            metrics: Vec::new(),
        }
    }
}

/// `fontMetrics` for one face. Only the primary face changes the grid.
#[derive(Clone, Copy, Debug, Default)]
pub struct FaceMetrics {
    pub width: Option<usize>,
    pub height: Option<usize>,
}

/// Parse `"calt"` or `"-liga"` into a feature tag and its state.
pub fn parse_feature(text: &str) -> Option<([u8; 4], bool)> {
    let text = text.trim();
    let (enabled, tag) = match text.strip_prefix('-') {
        Some(tag) => (false, tag),
        None => (true, text.strip_prefix('+').unwrap_or(text)),
    };
    let tag: [u8; 4] = tag.as_bytes().try_into().ok()?;
    Some((tag, enabled))
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
    /// Pixel width of a shaped text run; `None` for a single cell.
    run_width: Option<usize>,
    /// Byte offset of each cell's text in a run, empty for a single cell.
    cells: Vec<usize>,
}

/// The glyph cache ceiling. Beyond this the cache is dropped; a terminal screen
/// with more distinct cells than this is scrolling thousands of unique lines.
const CACHE_LIMIT: usize = 8_192;

/// A font built on a worker thread while the window and the restored shells are
/// being set up. `take` joins it when the first frame needs it.
pub struct FontSlot {
    job: Option<std::thread::JoinHandle<FontRenderer>>,
    ready: Option<FontRenderer>,
}

impl FontSlot {
    pub fn spawn(pixels: f32, families: Vec<String>, options: Options) -> Self {
        let job = std::thread::Builder::new()
            .name("berth-font".into())
            .spawn(move || FontRenderer::new(pixels, &families, options))
            .ok();
        Self { job, ready: None }
    }

    pub fn take(&mut self) -> Option<FontRenderer> {
        if self.ready.is_none()
            && let Some(job) = self.job.take()
        {
            self.ready = job.join().ok();
        }
        self.ready.take()
    }
}

pub struct FontRenderer {
    font_system: FontSystem,
    swash: SwashCache,
    buffer: Buffer,
    metrics: Metrics,
    cell: CellSize,
    weight: u16,
    features: FontFeatures,
    metrics_overrides: Vec<(String, FaceMetrics)>,
    /// Shaping order: the configured faces that are installed, in order, then
    /// the generic families. Text comes from the first, icons and powerline
    /// glyphs from whichever face actually has them.
    families: Vec<FamilyOwned>,
    cache: HashMap<CacheKey, Option<Arc<Raster>>>,
    /// Face lookup is a database query; a glyph redraws often, so remember the
    /// answer per family and style.
    faces: HashMap<(usize, bool, bool), Option<fontdb::ID>>,
}

/// Faces tried in order when `fontFamily` names nothing the system has. Each
/// one carries programming ligatures, so the default draws them instead of
/// falling back to a system monospace that has none.
const DEFAULT_FAMILIES: &[&str] = &[
    "JetBrains Mono",
    "Fira Code",
    "MesloLGS NF",
    "Cascadia Code",
    "Hack",
    "VictorMono Nerd Font Mono",
    "Iosevka",
    "Monaspace Neon",
    "Hasklig",
    "Monoid",
];

/// The database's own spelling of `name` when an installed face carries it.
/// The shaping lookup is not required to be case-insensitive, so echoing the
/// user's casing can miss.
fn installed_family(database: &fontdb::Database, name: &str) -> Option<String> {
    let wanted = name.trim();
    if wanted.is_empty() {
        return None;
    }
    database.faces().find_map(|face| {
        face.families
            .iter()
            .find(|(family, _)| family.eq_ignore_ascii_case(wanted))
            .map(|(family, _)| family.clone())
    })
}

/// The family the settings row reports for an unset `fontFamily`: the first
/// preferred ligature face this system has, else the system monospace.
pub fn default_family_name() -> String {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        let (_, database) = shared_database();
        DEFAULT_FAMILIES
            .iter()
            .find_map(|name| installed_family(&database, name))
            .unwrap_or_else(|| system_monospace_name(&database))
    })
    .clone()
}

/// Every installed family that declares itself monospaced, sorted for the
/// settings picker. Hidden system faces (a leading dot) stay out of the list;
/// the default row already reaches them. The list is cached because the font
/// database only changes when the user installs a font and relaunches.
pub fn installed_monospace_families() -> Vec<String> {
    static NAMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    NAMES
        .get_or_init(|| {
            let (_, database) = shared_database();
            let mut names: Vec<String> = Vec::new();
            for face in database.faces() {
                if !face.monospaced {
                    continue;
                }
                let Some((name, _)) = face.families.first() else {
                    continue;
                };
                let hidden = name.starts_with('.')
                    || name.contains("Emoji")
                    || name.contains("emoji")
                    || name.contains("Bitmap");
                let known = names
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(name));
                if !hidden && !known {
                    names.push(name.clone());
                }
            }
            names.sort_by_key(|name| name.to_lowercase());
            names
        })
        .clone()
}

/// The family the generic `Monospace` resolves to, so the default row can name
/// the face the user already sees.
fn system_monospace_name(database: &fontdb::Database) -> String {
    database
        .query(&fontdb::Query {
            families: &[fontdb::Family::Monospace],
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        })
        .and_then(|id| database.face(id))
        .and_then(|face| face.families.first().map(|(name, _)| name.clone()))
        .unwrap_or_else(|| "System monospace".to_string())
}

/// Every configured name that is installed, in the user's order, then the
/// generic families. Shaping walks this list, so a Nerd Font later in
/// `fontFamily` still supplies the icons and powerline separators the first
/// face lacks. Names are matched case-insensitively; an unknown name is
/// skipped rather than fatal, so a config copied from another machine opens.
/// With nothing installed, the preferred ligature face leads.
fn resolve_families(font_system: &FontSystem, names: &[String]) -> Vec<FamilyOwned> {
    let database = font_system.db();
    let mut families: Vec<FamilyOwned> = Vec::new();
    for name in names {
        if let Some(installed) = installed_family(database, name) {
            let family = FamilyOwned::Name(installed.into());
            if !families.contains(&family) {
                families.push(family);
            }
        }
    }
    if families.is_empty()
        && let Some(default) = DEFAULT_FAMILIES
            .iter()
            .find_map(|name| installed_family(database, name))
    {
        families.push(FamilyOwned::Name(default.into()));
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
fn attrs_for<'a>(
    family: Family<'a>,
    style: TextStyle,
    weight: u16,
    features: &FontFeatures,
) -> Attrs<'a> {
    let weight = if style.bold {
        Weight(weight.max(700))
    } else {
        Weight(weight)
    };
    let mut attrs = Attrs::new()
        .family(family)
        .weight(weight)
        .font_features(features.clone());
    if style.italic {
        attrs = attrs.style(Style::Italic);
    }
    attrs
}

/// The system font database is expensive to build (a few thousand faces read
/// from disk) and the window builds two `FontSystem`s, one for the grid and one
/// for chrome. Load it once and clone the parsed database for each system.
pub fn preload() {
    std::thread::spawn(|| {
        let _ = shared_database();
    });
}

fn shared_database() -> (String, fontdb::Database) {
    static SHARED: std::sync::OnceLock<(String, fontdb::Database)> = std::sync::OnceLock::new();
    SHARED
        .get_or_init(|| {
            let system = FontSystem::new();
            (system.locale().to_string(), system.db().clone())
        })
        .clone()
}

impl FontRenderer {
    /// Build a renderer at `font_size` pixels. Text comes from the first
    /// configured name the system knows; the rest supply what it lacks.
    /// `font_size` is already scaled for the display, not points.
    pub fn new(font_size: f32, families: &[String], options: Options) -> Self {
        let font_size = font_size.max(4.0);
        let metrics = Metrics::relative(font_size, options.line_height.clamp(0.8, 3.0));
        let started = std::time::Instant::now();
        let (locale, database) = shared_database();
        let database_ms = started.elapsed().as_millis();
        let mut font_system = FontSystem::new_with_locale_and_db(locale, database);
        let resolved = resolve_families(&font_system, families);
        let mut buffer = Buffer::new(&mut font_system, metrics);
        buffer.set_wrap(Wrap::None);
        buffer.set_size(Some(f32::MAX), Some(metrics.line_height * 2.0));
        let mut features = FontFeatures::new();
        for (tag, enabled) in &options.features {
            features.set(FeatureTag::new(tag), u32::from(*enabled));
        }

        let mut renderer = Self {
            font_system,
            swash: SwashCache::new(),
            buffer,
            metrics,
            cell: CellSize {
                width: 1,
                height: 1,
            },
            weight: options.weight.clamp(100, 900),
            features,
            metrics_overrides: options.metrics,
            families: resolved,
            cache: HashMap::new(),
            faces: HashMap::new(),
        };
        renderer.cell = renderer.measure_cell();
        renderer.apply_primary_metrics();
        // Which face the config actually resolved to; the first thing to check
        // when a window looks like it ignored `fontFamily`.
        crate::log::line(&format!(
            "font: {:?} at {font_size:.1}px cell {}x{} (database {database_ms} ms, ready {} ms)",
            renderer.families,
            renderer.cell.width,
            renderer.cell.height,
            started.elapsed().as_millis()
        ));
        renderer
    }

    /// Apply a `fontMetrics` entry for the family that shapes normal text.
    fn apply_primary_metrics(&mut self) {
        let Some(FamilyOwned::Name(name)) = self.families.first() else {
            return;
        };
        let Some(metrics) = self
            .metrics_overrides
            .iter()
            .find(|(family, _)| family.eq_ignore_ascii_case(name))
            .map(|(_, metrics)| *metrics)
        else {
            return;
        };
        if let Some(width) = metrics.width {
            self.cell.width = width.max(1);
        }
        if let Some(height) = metrics.height {
            self.cell.height = height.max(1);
            self.metrics.line_height = self.cell.height as f32;
            self.buffer.set_metrics(self.metrics);
        }
    }

    pub fn cell(&self) -> CellSize {
        self.cell
    }

    /// Width of one cell's advance and height of one line, from the font's own
    /// metrics for a shaped sample.
    ///
    /// The advance rounds up: a cell narrower than its glyph made cosmic-text
    /// clip the tail of a shaped run, which cut the last letter of a word.
    fn measure_cell(&mut self) -> CellSize {
        let primary = self
            .families
            .first()
            .cloned()
            .unwrap_or(FamilyOwned::Monospace);
        let attrs = attrs_for(
            primary.as_family(),
            TextStyle::NORMAL,
            self.weight,
            &self.features,
        );
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
        let width = cell_width(advance / 10.0);
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
            run_width: None,
            cells: Vec::new(),
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

    /// Shaped bitmap for a run of cells, drawn as one string.
    ///
    /// A cell cannot be shaped alone when the text is right to left: Arabic
    /// joins across cell boundaries and the shaped line sits at the run's
    /// right edge, not at each cell's origin.
    /// `cells` holds the byte offset of each cell's text in `text`, so a glyph
    /// can be put back on its cell. Right-to-left runs keep the shaper's
    /// positions, which anchor the whole phrase to the box's right edge.
    pub fn raster_run(
        &mut self,
        text: &str,
        width: usize,
        cells: &[usize],
        rtl: bool,
        style: TextStyle,
    ) -> Option<Arc<Raster>> {
        let width = width.max(1);
        let key = CacheKey {
            text: text.to_string(),
            bold: style.bold,
            italic: style.italic,
            run_width: Some(width),
            cells: cells.to_vec(),
        };
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        if self.cache.len() >= CACHE_LIMIT {
            self.cache.clear();
        }
        let raster = self
            .render_run(text, width, cells, rtl, style)
            .map(Arc::new);
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
        for (index, family) in families.iter().enumerate() {
            let attrs = attrs_for(family.as_family(), style, self.weight, &self.features);
            // A named face must draw the glyph itself. cosmic-text otherwise
            // substitutes another installed font, which is how a powerline
            // separator ended up as a 6x7 box from an unrelated face.
            let required = self.face_for(index, family, style);
            if let Some(raster) = self.shape_cell(text, &attrs, required) {
                return Some(raster);
            }
        }
        None
    }

    /// The face `family` resolves to for this style, or `None` for the generic
    /// families, which are the fallback of last resort.
    fn face_for(
        &mut self,
        index: usize,
        family: &FamilyOwned,
        style: TextStyle,
    ) -> Option<fontdb::ID> {
        if !matches!(family, FamilyOwned::Name(_)) {
            return None;
        }
        let key = (index, style.bold, style.italic);
        if let Some(hit) = self.faces.get(&key) {
            return *hit;
        }
        let id = self.font_system.db().query(&fontdb::Query {
            families: &[family.as_family()],
            weight: if style.bold {
                Weight(self.weight.max(700))
            } else {
                Weight(self.weight)
            },
            stretch: Stretch::Normal,
            style: if style.italic {
                Style::Italic
            } else {
                Style::Normal
            },
        });
        self.faces.insert(key, id);
        id
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

    fn render_run(
        &mut self,
        text: &str,
        width: usize,
        cells: &[usize],
        rtl: bool,
        style: TextStyle,
    ) -> Option<Raster> {
        let families = self.families.clone();
        for family in &families {
            let attrs = attrs_for(family.as_family(), style, self.weight, &self.features);
            if let Some(raster) = self.shape_run(text, width, cells, rtl, &attrs) {
                return Some(raster);
            }
        }
        None
    }

    /// Shape a whole run into a box exactly `width` pixels wide. Unlike a cell,
    /// the box keeps its alignment: a right-to-left run is anchored to its
    /// right edge, which is where the first printed cell belongs.
    ///
    /// A left-to-right glyph moves to the cell that holds its cluster. Shaped
    /// advances differ from the whole cell width, so the natural layout drifted
    /// and left the cursor past the end of a long word.
    fn shape_run(
        &mut self,
        text: &str,
        width: usize,
        cells: &[usize],
        rtl: bool,
        attrs: &Attrs<'_>,
    ) -> Option<Raster> {
        let line_height = self.metrics.line_height;
        let margin_y = (self.cell.height / 2) as i32;
        let height = self.cell.height + (margin_y * 2) as usize;
        self.buffer
            .set_size(Some(width as f32), Some(line_height * 2.0));
        self.buffer.set_text(text, attrs, Shaping::Advanced, None);
        self.buffer.shape_until_scroll(&mut self.font_system, false);

        let mut mask = vec![0u8; width * height];
        let mut color: Option<Vec<u8>> = None;
        let mut touched = false;

        let cell_width = self.cell.width as f32;
        let run = self.buffer.layout_runs().next()?;
        for glyph in run.glyphs {
            // The shaper offsets a glyph from its shaping position plus its own
            // logical offset; both are undone to land on the cluster's cell.
            let logical = glyph.x + glyph.font_size * glyph.x_offset;
            let snap = if rtl {
                0.0
            } else {
                cell_at(cells, glyph.start) as f32 * cell_width - logical
            };
            let physical = glyph.physical((snap, run.line_y), 1.0);
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
            let origin_y = physical.y - image.placement.top + margin_y;
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
                }
            }
        }
        if !touched {
            return None;
        }
        Some(Raster {
            width,
            height,
            left: 0,
            top: -margin_y,
            bitmap: match color {
                Some(rgba) => Bitmap::Color(rgba),
                None => Bitmap::Mask(mask),
            },
        })
    }
}

/// Round a glyph advance up to whole pixels, so a cell never sits inside its
/// own glyph. Rounding down made a shaped run overhang its box and lose ink.
fn cell_width(advance: f32) -> usize {
    advance.ceil().max(1.0) as usize
}

/// Cell index of a glyph cluster, given the byte offset of each cell's text.
fn cell_at(cells: &[usize], byte: usize) -> usize {
    cells
        .partition_point(|&start| start <= byte)
        .saturating_sub(1)
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
        FontRenderer::new(16.0, &[], Options::default())
    }

    #[test]
    fn given_a_line_height_then_the_cell_height_follows_it() {
        let short = FontRenderer::new(
            16.0,
            &[],
            Options {
                line_height: 1.0,
                ..Options::default()
            },
        );
        let tall = FontRenderer::new(
            16.0,
            &[],
            Options {
                line_height: 2.0,
                ..Options::default()
            },
        );
        assert!(tall.cell().height > short.cell().height);
    }

    #[test]
    fn given_a_feature_string_then_it_parses_tag_and_state() {
        assert_eq!(parse_feature("calt"), Some((*b"calt", true)));
        assert_eq!(parse_feature("-liga"), Some((*b"liga", false)));
        assert_eq!(parse_feature("+ss01"), Some((*b"ss01", true)));
        assert_eq!(parse_feature("liga!"), None);
        assert_eq!(parse_feature("abc"), None);
    }

    #[test]
    fn given_a_primary_face_metric_override_then_the_cell_uses_it() {
        let installed = {
            let system = FontSystem::new();
            system
                .db()
                .faces()
                .find_map(|face| face.families.first().map(|(name, _)| name.clone()))
                .expect("the font database is never empty")
        };
        let font = FontRenderer::new(
            16.0,
            std::slice::from_ref(&installed),
            Options {
                metrics: vec![(
                    installed.clone(),
                    FaceMetrics {
                        width: Some(11),
                        height: Some(23),
                    },
                )],
                ..Options::default()
            },
        );
        assert_eq!((font.cell().width, font.cell().height), (11, 23));
    }

    #[test]
    fn given_a_run_that_fills_its_box_then_the_last_glyph_is_not_wrapped_away() {
        let mut font = renderer();
        let cell = font.cell();
        let raster = font
            .raster_run(
                "iiii",
                cell.width * 4,
                &[0, 1, 2, 3],
                false,
                TextStyle::NORMAL,
            )
            .expect("a run raster");
        let mut right = 0;
        match &raster.bitmap {
            Bitmap::Mask(mask) => {
                for y in 0..raster.height {
                    for x in 0..raster.width {
                        if mask[y * raster.width + x] > 0 {
                            right = right.max(x);
                        }
                    }
                }
            }
            Bitmap::Color(rgba) => {
                for y in 0..raster.height {
                    for x in 0..raster.width {
                        if rgba[(y * raster.width + x) * 4 + 3] > 0 {
                            right = right.max(x);
                        }
                    }
                }
            }
        }
        assert!(
            right >= cell.width * 3,
            "the last glyph was wrapped out of the run: ink ends at {right}"
        );
    }

    #[test]
    fn given_an_arabic_phrase_when_shaped_as_a_run_then_it_has_ink_at_the_run_width() {
        let mut font = renderer();
        let raster = font
            .raster_run("التبغ ضار بالصحة.", 200, &[], true, TextStyle::NORMAL)
            .expect("a system Arabic face should shape the run");
        assert_eq!(raster.width, 200);
        assert!(raster.height >= font.cell().height);
        let ink: u32 = match &raster.bitmap {
            Bitmap::Mask(mask) => mask.iter().map(|&a| u32::from(a)).sum(),
            Bitmap::Color(rgba) => rgba.iter().skip(3).step_by(4).map(|&a| u32::from(a)).sum(),
        };
        assert!(ink > 0, "the run raster is blank");
    }

    /// Advance of a shaped string, so a face that silently fell back to a
    /// proportional one can be told apart from the configured monospace.
    fn advance_of(font: &mut FontRenderer, text: &str) -> f32 {
        let primary = font
            .families
            .first()
            .cloned()
            .unwrap_or(FamilyOwned::Monospace);
        let attrs = attrs_for(
            primary.as_family(),
            TextStyle::NORMAL,
            font.weight,
            &font.features,
        );
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

    /// Glyph ids a string shapes to with the leading face. A ligature shows up
    /// as a substitution the font would not make for the characters alone.
    fn run_glyph_ids(font: &mut FontRenderer, text: &str) -> Vec<u16> {
        let primary = font
            .families
            .first()
            .cloned()
            .unwrap_or(FamilyOwned::Monospace);
        let attrs = attrs_for(
            primary.as_family(),
            TextStyle::NORMAL,
            font.weight,
            &font.features,
        );
        font.buffer
            .set_size(Some(f32::MAX), Some(font.metrics.line_height * 2.0));
        font.buffer.set_text(text, &attrs, Shaping::Advanced, None);
        font.buffer.shape_until_scroll(&mut font.font_system, false);
        font.buffer
            .layout_runs()
            .next()
            .map(|run| run.glyphs.iter().map(|glyph| glyph.glyph_id).collect())
            .unwrap_or_default()
    }

    /// Name of the preferred face this system has, if any.
    fn installed_default() -> Option<String> {
        let (_, database) = shared_database();
        DEFAULT_FAMILIES
            .iter()
            .find_map(|name| installed_family(&database, name))
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
        let mut font = FontRenderer::new(16.0, &["VictorMono NFM".to_string()], Options::default());
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
    fn given_a_fractional_advance_when_rounded_then_the_cell_never_shrinks_below_it() {
        assert_eq!(cell_width(14.4), 15);
        assert_eq!(cell_width(14.0), 14);
        assert_eq!(cell_width(0.0), 1);
    }

    #[test]
    fn given_a_face_with_a_fractional_advance_then_the_cell_fits_it() {
        let mut font = FontRenderer::new(24.0, &[], Options::default());
        let advance = advance_of(&mut font, "M");
        if advance.fract() > f32::EPSILON {
            assert!(
                font.cell().width as f32 >= advance,
                "cell {} is narrower than the {advance} advance",
                font.cell().width
            );
        }
    }

    #[test]
    fn given_a_byte_offset_when_the_cell_is_found_then_it_follows_the_map() {
        assert_eq!(cell_at(&[], 0), 0);
        assert_eq!(cell_at(&[0, 1, 2], 0), 0);
        assert_eq!(cell_at(&[0, 1, 2], 2), 2);
        // A combining mark extends its cell's bytes, not the cell index.
        assert_eq!(cell_at(&[0, 3], 3), 1);
    }

    #[test]
    fn given_a_long_run_when_shaped_then_its_last_glyph_sits_on_the_cell_grid() {
        let mut font = renderer();
        let cell = font.cell();
        let cells: Vec<usize> = (0..7).collect();
        let single = font.raster("m", TextStyle::NORMAL).expect("m raster");
        let mut single_right = 0;
        if let Bitmap::Mask(mask) = &single.bitmap {
            for (index, &alpha) in mask.iter().enumerate() {
                if alpha > 0 {
                    single_right = single_right.max((index % single.width) as i32);
                }
            }
        }
        let run = font
            .raster_run("mmmmmmm", 7 * cell.width, &cells, false, TextStyle::NORMAL)
            .expect("a run raster");
        let mut right = 0;
        if let Bitmap::Mask(mask) = &run.bitmap {
            for (index, &alpha) in mask.iter().enumerate() {
                if alpha > 0 {
                    right = right.max((index % run.width) as i32);
                }
            }
        }
        // The shipped bug left the run on the font's advance, which is narrower
        // than the cell, so the last letter ended short of the cursor's cell.
        assert!(
            right >= 6 * cell.width as i32 + single_right - 1,
            "the last glyph ended at {right}, off the cell grid"
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
    fn given_no_family_names_when_resolved_then_an_installed_ligature_face_leads() {
        let system = FontSystem::new();
        let expected = installed_default()
            .map(|name| FamilyOwned::Name(name.into()))
            .unwrap_or(FamilyOwned::Monospace);
        assert_eq!(resolve_families(&system, &[]).first(), Some(&expected));
    }

    #[test]
    fn given_an_uninstalled_family_name_when_resolved_then_the_default_face_leads() {
        let system = FontSystem::new();
        let names = vec!["No Such Face 12345".to_string()];
        let expected = installed_default()
            .map(|name| FamilyOwned::Name(name.into()))
            .unwrap_or(FamilyOwned::Monospace);
        assert_eq!(resolve_families(&system, &names).first(), Some(&expected));
    }

    #[test]
    fn given_the_default_face_then_a_run_uses_the_font_ligature() {
        let Some(default) = installed_default() else {
            // No preferred ligature face installed; the system monospace has
            // no ligature table, so there is nothing to assert.
            return;
        };
        let mut font = renderer();
        let pair = run_glyph_ids(&mut font, "=>");
        let mut separate = run_glyph_ids(&mut font, "=");
        separate.extend(run_glyph_ids(&mut font, ">"));
        assert_ne!(pair, separate, "{default} shaped `=>` without a ligature");
    }

    #[test]
    fn given_the_installed_monospace_list_then_it_is_sorted_and_skips_hidden_faces() {
        let names = installed_monospace_families();
        let mut sorted = names.clone();
        sorted.sort_by_key(|name| name.to_lowercase());
        assert_eq!(names, sorted);
        assert!(names.iter().all(|name| !name.starts_with('.')));
        let mut unique = names.clone();
        unique.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        assert_eq!(names.len(), unique.len());
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
        let installed = |name: &str| {
            FontSystem::new()
                .db()
                .faces()
                .any(|face| face.families.iter().any(|(family, _)| family == name))
        };
        if !installed("Monaco") || !installed("VictorMono NFM") {
            // The hand-over needs an icon-less first face and a Nerd Font
            // second; a machine with either missing cannot exercise it.
            return;
        }
        let mut font = FontRenderer::new(
            13.0,
            &["Monaco".to_string(), "VictorMono NFM".to_string()],
            Options::default(),
        );
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
            raster.height >= cell.height && raster.width >= cell.width,
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
        let mut font = FontRenderer::new(16.0, &[installed], Options::default());
        assert!(font.raster("M", TextStyle::NORMAL).is_some());
        assert!(font.cell().width >= 4);
    }
}
