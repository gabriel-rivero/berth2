//! Extended styled text that `vt100` does not model: strikethrough, styled and
//! coloured underlines, and OSC 8 hyperlinks.
//!
//! The PTY reader records the extended attributes active for each cell that
//! changes, keyed by absolute grid line. `screen_view` overlays them on the
//! cells `vt100` renders. Style-only SGR changes do not repaint existing
//! cells, so the map is updated only where the grid changed.

use std::collections::HashMap;
use std::sync::Arc;

use unicode_width::UnicodeWidthChar;

/// The SGR 4:x underline variants.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UnderlineStyle {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// The attributes a cell carries beyond `vt100`'s model.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtendedStyle {
    pub strikethrough: bool,
    pub underline: UnderlineStyle,
    pub underline_color: Option<[u8; 3]>,
    pub hyperlink: Option<Arc<str>>,
}

impl ExtendedStyle {
    pub fn is_default(&self) -> bool {
        !self.strikethrough
            && self.underline == UnderlineStyle::None
            && self.underline_color.is_none()
            && self.hyperlink.is_none()
    }
}

/// The styles of one surface. The main and alternate screens keep separate
/// maps, and cells are keyed by absolute line so scrollback keeps them.
#[derive(Debug, Default)]
pub struct StyleState {
    current: ExtendedStyle,
    columns: u16,
    is_alternate_screen: bool,
    main: HashMap<(usize, u16), ExtendedStyle>,
    alternate: HashMap<(usize, u16), ExtendedStyle>,
    pruned_at: usize,
}

impl StyleState {
    pub fn current(&self) -> &ExtendedStyle {
        &self.current
    }

    pub fn is_empty(&self) -> bool {
        self.main.is_empty() && self.alternate.is_empty()
    }

    /// Drop every recorded style. A resize invalidates the column mapping.
    pub fn clear(&mut self) {
        self.main.clear();
        self.alternate.clear();
        self.pruned_at = 0;
    }

    /// The style recorded for one absolute line and column.
    pub fn get(&self, line: usize, column: u16) -> Option<&ExtendedStyle> {
        self.map().get(&(line, column))
    }

    /// Switch maps with the alternate screen. A program that changes the
    /// screen does not carry attributes across.
    pub fn set_alternate_screen(&mut self, is_alternate: bool) {
        if self.is_alternate_screen == is_alternate {
            return;
        }
        self.is_alternate_screen = is_alternate;
        self.current = ExtendedStyle::default();
    }

    /// Apply one SGR sequence body (the bytes before the final `m`).
    pub fn apply_sgr(&mut self, body: &[u8]) {
        let groups = sgr_groups(body);
        let mut index = 0;
        while index < groups.len() {
            let group = &groups[index];
            let Some(code) = group.first().copied() else {
                index += 1;
                continue;
            };
            match code {
                0 => {
                    let hyperlink = self.current.hyperlink.take();
                    self.current = ExtendedStyle {
                        hyperlink,
                        ..ExtendedStyle::default()
                    };
                }
                4 => {
                    let value = group.get(1).copied().unwrap_or(1);
                    self.current.underline = underline_from_sgr(value);
                }
                9 => self.current.strikethrough = true,
                24 => self.current.underline = UnderlineStyle::None,
                29 => self.current.strikethrough = false,
                58 => {
                    if let Some((color, consumed)) = parse_color(&groups[index..]) {
                        self.current.underline_color = color;
                        index += consumed.saturating_sub(1);
                    }
                }
                59 => self.current.underline_color = None,
                38 | 48 => {
                    // A foreground or background colour owns its parameters;
                    // skip them so a colour index is not read as a style.
                    if let Some((_, consumed)) = parse_color(&groups[index..]) {
                        index += consumed.saturating_sub(1);
                    }
                }
                _ => {}
            }
            index += 1;
        }
    }

    /// Apply one OSC payload. Only OSC 8 changes anything here.
    pub fn apply_osc(&mut self, payload: &[u8]) {
        let Some(rest) = payload.strip_prefix(b"8;") else {
            return;
        };
        let Some(position) = rest.iter().position(|byte| *byte == b';') else {
            return;
        };
        let uri = &rest[position + 1..];
        self.current.hyperlink = if uri.is_empty() {
            None
        } else {
            Some(Arc::from(String::from_utf8_lossy(uri).into_owned()))
        };
    }

    /// Record the current style on every cell that changed between snapshots.
    pub fn record(&mut self, before: &GridSnapshot, after: &GridSnapshot, scrollback: usize) {
        if self.columns != after.cols {
            self.clear();
            self.columns = after.cols;
        }

        let dimensions_match = before.cols == after.cols
            && before.rows == after.rows
            && before.cells.len() == after.cells.len();
        let reset = !dimensions_match || before.scrollback_len > after.scrollback_len;
        if reset {
            self.map_mut().clear();
            self.pruned_at = 0;
        }
        let delta = if reset {
            usize::from(after.rows)
        } else {
            after.scrollback_len.saturating_sub(before.scrollback_len)
        };

        for row in 0..usize::from(after.rows) {
            for column in 0..after.cols {
                let index = row * usize::from(after.cols) + usize::from(column);
                let old_row = row + delta;
                let changed = old_row >= usize::from(after.rows)
                    || before.cells[old_row * usize::from(before.cols) + usize::from(column)]
                        != after.cells[index];
                if !changed {
                    continue;
                }
                let key = (after.scrollback_len + row, column);
                self.put(key);
            }
        }

        if after.scrollback_len > self.pruned_at.saturating_add(1024) {
            let oldest = after.scrollback_len.saturating_sub(scrollback);
            self.map_mut().retain(|(line, _), _| *line >= oldest);
            self.pruned_at = after.scrollback_len;
        }
    }

    fn map(&self) -> &HashMap<(usize, u16), ExtendedStyle> {
        if self.is_alternate_screen {
            &self.alternate
        } else {
            &self.main
        }
    }

    fn map_mut(&mut self) -> &mut HashMap<(usize, u16), ExtendedStyle> {
        if self.is_alternate_screen {
            &mut self.alternate
        } else {
            &mut self.main
        }
    }

    fn put(&mut self, key: (usize, u16)) {
        let current = self.current.clone();
        if current.is_default() {
            self.map_mut().remove(&key);
        } else {
            self.map_mut().insert(key, current);
        }
    }

    /// Record the current style on the cells a text segment wrote, following
    /// carriage returns, line feeds, tabs, wide characters and cursor moves.
    pub fn record_text(
        &mut self,
        cursor: (u16, u16),
        rows: u16,
        cols: u16,
        scrollback: usize,
        bytes: &[u8],
    ) {
        let rows = usize::from(rows);
        let cols = usize::from(cols);
        let (mut row, mut column) = (usize::from(cursor.0), usize::from(cursor.1));
        let mut line = scrollback;
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == 0x1b {
                let Some(&next) = bytes.get(index + 1) else {
                    break;
                };
                if next != b'[' {
                    break;
                }
                let start = index + 2;
                let mut end = start;
                while end < bytes.len() && !(0x40..=0x7e).contains(&bytes[end]) {
                    end += 1;
                }
                if end >= bytes.len() {
                    break;
                }
                apply_cursor(
                    &mut row,
                    &mut column,
                    rows,
                    cols,
                    bytes[end],
                    &bytes[start..end],
                );
                index = end + 1;
                continue;
            }
            let Ok(text) = std::str::from_utf8(&bytes[index..]) else {
                break;
            };
            let Some(character) = text.chars().next() else {
                break;
            };
            index += character.len_utf8();
            match character {
                '\r' => column = 0,
                '\n' | '\u{0b}' | '\u{0c}' => {
                    if row + 1 < rows {
                        row += 1;
                    } else {
                        line = line.saturating_add(1);
                    }
                }
                '\t' => {
                    let next = (column / 8 + 1) * 8;
                    column = if next < cols { next } else { 0 };
                }
                '\u{8}' => column = column.saturating_sub(1),
                character if is_control(character) => {}
                character => {
                    let width = character.width().unwrap_or(1);
                    if width == 0 {
                        continue;
                    }
                    if column + width > cols {
                        column = 0;
                        if row + 1 < rows {
                            row += 1;
                        } else {
                            line = line.saturating_add(1);
                        }
                    }
                    let absolute = line.saturating_add(row);
                    self.put((absolute, column as u16));
                    if width > 1 && column + 1 < cols {
                        self.put((absolute, column as u16 + 1));
                    }
                    column += width;
                    if column >= cols {
                        column = 0;
                        if row + 1 < rows {
                            row += 1;
                        } else {
                            line = line.saturating_add(1);
                        }
                    }
                }
            }
        }
    }
}

/// Track the screen cursor through the CSI moves a text segment may contain.
fn apply_cursor(
    row: &mut usize,
    column: &mut usize,
    rows: usize,
    cols: usize,
    final_byte: u8,
    body: &[u8],
) {
    if body.first() == Some(&b'?') {
        return;
    }
    let mut params = body.split(|byte| *byte == b';').map(|part| {
        std::str::from_utf8(part)
            .ok()
            .and_then(|text| text.parse::<usize>().ok())
            .unwrap_or(0)
    });
    let first = params.next().unwrap_or(0);
    let second = params.next().unwrap_or(0);
    let count = if first == 0 { 1 } else { first };
    match final_byte {
        b'H' | b'f' => {
            *row = first.saturating_sub(1).min(rows.saturating_sub(1));
            *column = second.saturating_sub(1).min(cols.saturating_sub(1));
        }
        b'A' => *row = (*row).saturating_sub(count),
        b'B' => *row = (*row + count).min(rows.saturating_sub(1)),
        b'C' => *column = (*column + count).min(cols.saturating_sub(1)),
        b'D' => *column = (*column).saturating_sub(count),
        b'G' | b'`' => *column = first.saturating_sub(1).min(cols.saturating_sub(1)),
        b'd' => *row = first.saturating_sub(1).min(rows.saturating_sub(1)),
        b'E' => {
            *row = (*row + count).min(rows.saturating_sub(1));
            *column = 0;
        }
        b'F' => {
            *row = (*row).saturating_sub(count);
            *column = 0;
        }
        _ => {}
    }
}

fn is_control(character: char) -> bool {
    character < '\u{20}' || ('\u{7f}'..='\u{9f}').contains(&character)
}

/// A cheap fingerprint of the grid for change detection.
#[derive(Debug, Clone, Default)]
pub struct GridSnapshot {
    pub cols: u16,
    pub rows: u16,
    pub scrollback_len: usize,
    cells: Vec<u64>,
}

impl GridSnapshot {
    pub fn take(screen: &vt100::Screen) -> Self {
        let (rows, cols) = screen.size();
        let (rows, cols) = (rows.get(), cols.get());
        let mut cells = Vec::with_capacity(usize::from(rows) * usize::from(cols));
        for row in 0..rows {
            for column in 0..cols {
                cells.push(fingerprint(screen.cell(row, column)));
            }
        }
        Self {
            cols,
            rows,
            scrollback_len: screen.scrollback_len(),
            cells,
        }
    }
}

fn fingerprint(cell: Option<&vt100::Cell>) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    let Some(cell) = cell else {
        return 0;
    };
    let mut hash = OFFSET;
    for byte in cell.contents().as_bytes() {
        mix(&mut hash, u64::from(*byte));
    }
    let mut flags = 0u64;
    flags |= u64::from(cell.bold());
    flags |= u64::from(cell.dim()) << 1;
    flags |= u64::from(cell.italic()) << 2;
    flags |= u64::from(cell.underline()) << 3;
    flags |= u64::from(cell.inverse()) << 4;
    flags |= u64::from(cell.is_wide()) << 5;
    flags |= u64::from(cell.is_wide_continuation()) << 6;
    mix(&mut hash, flags);
    mix(&mut hash, color_code(cell.fgcolor()));
    mix(&mut hash, color_code(cell.bgcolor()));
    hash
}

fn mix(hash: &mut u64, value: u64) {
    *hash ^= value;
    *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
}
fn color_code(color: vt100::Color) -> u64 {
    match color {
        vt100::Color::Default => 0,
        vt100::Color::Idx(index) => 1 | (u64::from(index) << 8),
        vt100::Color::Rgb(red, green, blue) => {
            2 | (u64::from(red) << 8) | (u64::from(green) << 16) | (u64::from(blue) << 24)
        }
    }
}

fn sgr_groups(body: &[u8]) -> Vec<Vec<u16>> {
    body.split(|byte| *byte == b';')
        .map(|group| {
            group
                .split(|byte| *byte == b':')
                .map(parse_number)
                .collect()
        })
        .collect()
}

fn parse_number(bytes: &[u8]) -> u16 {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(0)
}

fn underline_from_sgr(value: u16) -> UnderlineStyle {
    match value {
        0 => UnderlineStyle::None,
        2 => UnderlineStyle::Double,
        3 => UnderlineStyle::Curly,
        4 => UnderlineStyle::Dotted,
        5 => UnderlineStyle::Dashed,
        _ => UnderlineStyle::Single,
    }
}

/// Parse the colour parameters of SGR 58 (or 38/48) from `groups`.
fn parse_color(groups: &[Vec<u16>]) -> Option<(Option<[u8; 3]>, usize)> {
    let group = groups.first()?;
    if group.len() > 1 {
        return Some(match group.get(1).copied()? {
            5 => (group.get(2).map(|index| indexed_rgb(*index as u8)), 1),
            2 => (
                Some([
                    group.get(3).copied().unwrap_or(0) as u8,
                    group.get(4).copied().unwrap_or(0) as u8,
                    group.get(5).copied().unwrap_or(0) as u8,
                ]),
                1,
            ),
            _ => (None, 1),
        });
    }
    Some(match groups.get(1)?.first().copied()? {
        5 => (
            groups
                .get(2)
                .and_then(|group| group.first())
                .map(|index| indexed_rgb(*index as u8)),
            3,
        ),
        2 => (
            Some([
                groups
                    .get(2)
                    .and_then(|group| group.first())
                    .copied()
                    .unwrap_or(0) as u8,
                groups
                    .get(3)
                    .and_then(|group| group.first())
                    .copied()
                    .unwrap_or(0) as u8,
                groups
                    .get(4)
                    .and_then(|group| group.first())
                    .copied()
                    .unwrap_or(0) as u8,
            ]),
            5,
        ),
        _ => (None, 2),
    })
}

/// The RGB of an xterm 256-colour index, matching `ansi_rgb` in the host.
pub fn indexed_rgb(index: u8) -> [u8; 3] {
    match index {
        0 => [0x1c, 0x1c, 0x22],
        1 => [0xd7, 0x5f, 0x5f],
        2 => [0x8f, 0xb3, 0x5f],
        3 => [0xd7, 0xa5, 0x5f],
        4 => [0x6c, 0x99, 0xc9],
        5 => [0xb0, 0x7c, 0xc9],
        6 => [0x5f, 0xb3, 0xb3],
        7 => [0xc9, 0xc9, 0xd0],
        8 => [0x5c, 0x5c, 0x66],
        9 => [0xff, 0x7a, 0x7a],
        10 => [0xa9, 0xd4, 0x6a],
        11 => [0xff, 0xd7, 0x66],
        12 => [0x82, 0xbb, 0xff],
        13 => [0xd7, 0x9a, 0xff],
        14 => [0x7a, 0xdb, 0xdb],
        15 => [0xff, 0xff, 0xff],
        n if n < 232 => {
            let n = n - 16;
            let level = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            [level(n / 36), level((n % 36) / 6), level(n % 6)]
        }
        n => {
            let level = 8 + (n - 232) * 10;
            [level, level, level]
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use super::*;

    fn parser() -> vt100::Parser {
        vt100::Parser::new(
            NonZeroU16::new(2).expect("rows"),
            NonZeroU16::new(4).expect("cols"),
            10,
        )
    }

    fn style(body: &[u8]) -> ExtendedStyle {
        let mut state = StyleState::default();
        state.apply_sgr(body);
        state.current().clone()
    }

    #[test]
    fn given_each_underline_form_then_it_maps_to_the_right_style() {
        assert_eq!(style(b"4:0").underline, UnderlineStyle::None);
        assert_eq!(style(b"4").underline, UnderlineStyle::Single);
        assert_eq!(style(b"4:1").underline, UnderlineStyle::Single);
        assert_eq!(style(b"4:2").underline, UnderlineStyle::Double);
        assert_eq!(style(b"4:3").underline, UnderlineStyle::Curly);
        assert_eq!(style(b"4:4").underline, UnderlineStyle::Dotted);
        assert_eq!(style(b"4:5").underline, UnderlineStyle::Dashed);
    }

    #[test]
    fn given_strikethrough_then_it_sets_and_clears() {
        assert!(style(b"9").strikethrough);
        let mut state = StyleState::default();
        state.apply_sgr(b"9");
        state.apply_sgr(b"29");
        assert!(!state.current().strikethrough);
    }

    #[test]
    fn given_a_reset_then_extended_attributes_clear_but_the_hyperlink_stays() {
        let mut state = StyleState::default();
        state.apply_osc(b"8;;https://example.com");
        state.apply_sgr(b"4:3;9");
        state.apply_sgr(b"0");
        assert_eq!(state.current().underline, UnderlineStyle::None);
        assert!(!state.current().strikethrough);
        assert_eq!(
            state.current().hyperlink.as_deref(),
            Some("https://example.com")
        );
    }

    #[test]
    fn given_an_underline_colour_in_both_forms_then_it_is_stored() {
        assert_eq!(style(b"58;5;196").underline_color, Some(indexed_rgb(196)));
        assert_eq!(style(b"58;2;1;2;3").underline_color, Some([1, 2, 3]));
        assert_eq!(style(b"58:2::1:2:3").underline_color, Some([1, 2, 3]));
        let mut state = StyleState::default();
        state.apply_sgr(b"58;5;196");
        state.apply_sgr(b"59");
        assert_eq!(state.current().underline_color, None);
    }

    #[test]
    fn given_a_colour_index_nine_then_strikethrough_is_not_set() {
        assert!(!style(b"38;5;9").strikethrough);
        assert!(!style(b"48;2;9;9;9").strikethrough);
    }

    #[test]
    fn given_osc8_then_the_hyperlink_sets_and_an_empty_uri_clears_it() {
        let mut state = StyleState::default();
        state.apply_osc(b"8;id=1;https://example.com/a");
        assert_eq!(
            state.current().hyperlink.as_deref(),
            Some("https://example.com/a")
        );
        state.apply_osc(b"8;;");
        assert_eq!(state.current().hyperlink, None);
        state.apply_osc(b"0;title");
        assert_eq!(state.current().hyperlink, None);
    }

    #[test]
    fn given_a_changed_cell_then_the_current_style_is_recorded() {
        let mut parser = parser();
        let mut state = StyleState::default();
        state.apply_sgr(b"4:3;58;5;196");
        let before = GridSnapshot::take(parser.screen());
        parser.process(b"ab");
        let after = GridSnapshot::take(parser.screen());
        state.record(&before, &after, 10);
        let recorded = state.get(0, 0).expect("styled cell");
        assert_eq!(recorded.underline, UnderlineStyle::Curly);
        assert_eq!(recorded.underline_color, Some(indexed_rgb(196)));
        assert!(state.get(0, 2).is_none());
    }

    #[test]
    fn given_a_default_style_then_a_rewritten_cell_loses_its_style() {
        let mut parser = parser();
        let mut state = StyleState::default();
        state.apply_sgr(b"4:1");
        let before = GridSnapshot::take(parser.screen());
        parser.process(b"a");
        let after = GridSnapshot::take(parser.screen());
        state.record(&before, &after, 10);
        assert!(state.get(0, 0).is_some());

        state.apply_sgr(b"0");
        let before = GridSnapshot::take(parser.screen());
        parser.process(b"\x1b[1;1Hb");
        let after = GridSnapshot::take(parser.screen());
        state.record(&before, &after, 10);
        assert!(state.get(0, 0).is_none());
    }

    #[test]
    fn given_the_same_text_rewritten_with_a_new_style_then_the_style_changes() {
        let mut parser = parser();
        let mut state = StyleState::default();
        state.apply_sgr(b"9");
        let before = GridSnapshot::take(parser.screen());
        parser.process(b"ab");
        let after = GridSnapshot::take(parser.screen());
        state.record(&before, &after, 10);
        assert!(state.get(0, 0).is_some_and(|style| style.strikethrough));

        state.apply_sgr(b"0");
        let before = GridSnapshot::take(parser.screen());
        parser.process(b"\rab");
        let after = GridSnapshot::take(parser.screen());
        state.record(&before, &after, 10);
        state.record_text((0, 2), 2, 4, 0, b"\rab");
        assert!(state.get(0, 0).is_none());
    }

    #[test]
    fn given_scrolling_then_styles_stay_on_their_absolute_lines() {
        let mut parser = parser();
        let mut state = StyleState::default();
        state.apply_sgr(b"4:1");
        for bytes in [&b"a"[..], &b"\r\nb"[..], &b"\r\nc"[..]] {
            let before = GridSnapshot::take(parser.screen());
            parser.process(bytes);
            let after = GridSnapshot::take(parser.screen());
            state.record(&before, &after, 10);
        }
        assert_eq!(
            state.get(0, 0).map(|style| style.underline),
            Some(UnderlineStyle::Single)
        );
        assert_eq!(
            state.get(1, 0).map(|style| style.underline),
            Some(UnderlineStyle::Single)
        );
        assert_eq!(
            state.get(2, 0).map(|style| style.underline),
            Some(UnderlineStyle::Single)
        );
    }

    #[test]
    fn given_the_alternate_screen_then_each_screen_keeps_its_own_map() {
        let mut parser = parser();
        let mut state = StyleState::default();
        state.apply_sgr(b"4:1");
        let before = GridSnapshot::take(parser.screen());
        parser.process(b"a");
        let after = GridSnapshot::take(parser.screen());
        state.record(&before, &after, 10);
        assert_eq!(
            state.get(0, 0).map(|style| style.underline),
            Some(UnderlineStyle::Single)
        );

        state.set_alternate_screen(true);
        state.apply_sgr(b"4:3");
        let before = GridSnapshot::take(parser.screen());
        parser.process(b"\x1b[1;1Hb");
        let after = GridSnapshot::take(parser.screen());
        state.record(&before, &after, 10);
        assert_eq!(
            state.get(0, 0).map(|style| style.underline),
            Some(UnderlineStyle::Curly)
        );

        state.set_alternate_screen(false);
        assert_eq!(
            state.get(0, 0).map(|style| style.underline),
            Some(UnderlineStyle::Single)
        );
    }
}
