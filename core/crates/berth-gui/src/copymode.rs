//! Copy mode: vi-style keyboard selection for the portable window.
//!
//! Copy mode is a modal capture in front of the key funnel, so `h`, `j` and `y`
//! never reach the shell.
//!
//! The cursor is an **absolute** line — the same index `set_scrollback` and the
//! prompt gutter use — not a visible row. A visible-row cursor would slide to a
//! different piece of text every time the view scrolled, and copy mode scrolls
//! the view on nearly every motion. Absolute lines survive both scrolling and
//! the panel being resized; the caller maps one to a row when it paints.
//!
//! The motion logic here is arithmetic over one line's text; the caller owns the
//! host, so every function returns what to do rather than doing it.

use crate::ui::is_word_char;

/// A position in the buffer: an absolute line and a column.
pub type Position = (usize, u16);

/// Where a word motion landed: on a column of this row, or off the end of it.
/// A `NextRow` from the last row means the motion continues on the row below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordStep {
    Col(u16),
    /// Past the end of this row: the motion continues on the row below.
    NextRow,
    /// Before the start of this row: the motion continues on the row above.
    PrevRow,
}

/// One copy-mode session.
///
/// The buffer is captured when the mode opens, so motions read line text
/// without locking the host per keystroke and a yank can span more than one
/// screen — `contents_between` only sees the visible grid, which is why the
/// mouse selection cannot do that. Output that arrives mid-session is not
/// indexed, the same trade search makes: a cursor that jumps under a motion is
/// worse than one that lags.
#[derive(Clone, Debug)]
pub struct CopyMode {
    pub surface_id: String,
    /// The cursor, as an absolute line and a column.
    pub cursor: Position,
    /// Where a visual selection started, in the same coordinates.
    pub anchor: Option<Position>,
    /// A motion count typed before the motion (`5j`), consumed by the next one.
    pub count: Option<usize>,
    /// Every line of the scrollback, oldest first, indexed by absolute line.
    pub lines: Vec<String>,
}

impl CopyMode {
    /// Start at `cursor`, with no selection yet.
    pub fn new(surface_id: impl Into<String>, lines: Vec<String>) -> Self {
        Self {
            surface_id: surface_id.into(),
            cursor: (0, 0),
            anchor: None,
            count: None,
            lines,
        }
    }

    /// The last absolute line a cursor may occupy.
    pub fn last_line(&self) -> usize {
        self.lines.len().saturating_sub(1)
    }

    /// One line's text, or an empty string past the end.
    pub fn line(&self, line: usize) -> &str {
        self.lines.get(line).map(String::as_str).unwrap_or("")
    }

    /// The cursor's line length in characters.
    pub fn cursor_line_end(&self) -> u16 {
        line_end(self.line(self.cursor.0))
    }

    /// Clamp the column to the current line, for motions that do not set it
    /// themselves.
    pub fn clamp_column(&mut self) {
        let end = self.cursor_line_end();
        self.cursor.1 = self.cursor.1.min(end);
    }

    /// Whether a visual selection is in progress.
    pub fn is_visual(&self) -> bool {
        self.anchor.is_some()
    }

    /// The selection in reading order, when visual.
    pub fn selection(&self) -> Option<(Position, Position)> {
        let anchor = self.anchor?;
        if anchor <= self.cursor {
            Some((anchor, self.cursor))
        } else {
            Some((self.cursor, anchor))
        }
    }

    /// Take the pending count for the next motion, defaulting to one.
    pub fn take_count(&self) -> usize {
        self.count.unwrap_or(1).max(1)
    }

    /// Add a digit to the pending count.
    pub fn push_digit(&mut self, digit: u8) {
        let digit = usize::from(digit);
        self.count = Some(
            self.count
                .unwrap_or(0)
                .saturating_mul(10)
                .saturating_add(digit),
        );
    }

    /// Toggle the visual selection. Starting one anchors at the cursor; ending
    /// one keeps the cursor where it is.
    pub fn toggle_visual(&mut self) {
        self.anchor = match self.anchor {
            Some(_) => None,
            None => Some(self.cursor),
        };
    }

    /// Move the cursor by `lines`, clamped to the buffer. `last_line` is the
    /// newest absolute line, so a motion cannot land past the live screen.
    pub fn move_lines(&mut self, delta: i64, last_line: usize) {
        let line = self.cursor.0 as i64 + delta;
        self.cursor.0 = line.clamp(0, last_line as i64) as usize;
        self.clamp_column();
    }

    /// Word motions walk the buffer, so they cross a row boundary rather than
    /// stopping on it. The count repeats the motion.
    pub fn move_word(&mut self, action: CopyAction, count: usize) {
        for _ in 0..count {
            match action {
                CopyAction::WordBack => self.move_word_back(),
                CopyAction::WordEnd => self.move_word_end(),
                _ => self.move_word_forward(),
            }
        }
        self.clamp_column();
    }

    /// `w`: the next word start. At a row end it lands on the next row's first
    /// non-blank, as vim does; re-running the motion from column zero would
    /// overshoot that word entirely.
    fn move_word_forward(&mut self) {
        let line = self.cursor.0;
        match word_forward(self.line(line), self.cursor.1) {
            WordStep::Col(at) => self.cursor = (line, at),
            WordStep::NextRow => self.cursor = next_row_start(self, line),
            WordStep::PrevRow => {}
        }
    }

    /// `b`: the previous word start. Before a row start it lands on the start
    /// of the previous row's last word.
    fn move_word_back(&mut self) {
        let line = self.cursor.0;
        match word_back(self.line(line), self.cursor.1) {
            WordStep::Col(at) => self.cursor = (line, at),
            WordStep::PrevRow if line > 0 => {
                let prev = line - 1;
                let end = self.line(prev).chars().count() as u16;
                self.cursor = match word_back(self.line(prev), end) {
                    WordStep::Col(at) => (prev, at),
                    _ => (prev, 0),
                };
            }
            _ => {}
        }
    }

    /// `e`: the end of the current or next word.
    fn move_word_end(&mut self) {
        let line = self.cursor.0;
        match word_end(self.line(line), self.cursor.1) {
            WordStep::Col(at) => self.cursor = (line, at),
            WordStep::NextRow if line < self.last_line() => {
                let next = line + 1;
                let first = first_column(self.line(next));
                self.cursor = match word_end(self.line(next), first) {
                    WordStep::Col(at) => (next, at),
                    _ => (next, first),
                };
            }
            _ => {}
        }
    }
}

/// The first non-blank column of the next row, or the cursor's row end when
/// there is no next row.
fn next_row_start(mode: &CopyMode, line: usize) -> Position {
    if line >= mode.last_line() {
        return (line, mode.cursor_line_end());
    }
    let next = line + 1;
    (next, first_column(mode.line(next)))
}

fn first_column(text: &str) -> u16 {
    text.chars().position(|ch| !ch.is_whitespace()).unwrap_or(0) as u16
}

/// The text between two positions, for the clipboard. A single-line selection
/// takes the columns it covers; a multi-line one takes the rest of the first
/// line, every line between, and the start of the last. Trailing whitespace is
/// stripped per line, the same as a mouse copy.
pub fn yank_text(lines: &[String], start: Position, end: Position) -> String {
    let line = |index: usize| lines.get(index).map(String::as_str).unwrap_or("");
    let slice = |text: &str, from: u16, to: Option<u16>| -> String {
        let chars: Vec<char> = text.chars().collect();
        let from = usize::from(from).min(chars.len());
        let to = to.map_or(chars.len(), usize::from).min(chars.len());
        chars[from..to.max(from)].iter().collect()
    };
    let mut out = Vec::new();
    if start.0 == end.0 {
        out.push(slice(line(start.0), start.1, Some(end.1.saturating_add(1))));
    } else {
        out.push(slice(line(start.0), start.1, None));
        for index in start.0.saturating_add(1)..end.0 {
            out.push(line(index).to_string());
        }
        out.push(slice(line(end.0), 0, Some(end.1.saturating_add(1))));
    }
    out.iter()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// How the view must scroll so `line` is visible, given the window `top` line
/// and its height. `None` keeps the offset; otherwise the new absolute top line.
pub fn scroll_to_show(
    line: usize,
    top: usize,
    rows: usize,
    scrollback_len: usize,
) -> Option<usize> {
    let rows = rows.max(1);
    let bottom = top.saturating_add(rows);
    if line < top {
        return Some(line);
    }
    if line >= bottom {
        // One line of context below the cursor, like `zz`-less vim.
        let top = line.saturating_add(1).saturating_sub(rows);
        return Some(top.min(scrollback_len));
    }
    None
}

/// Start of the next word at or after `col`, or the row boundary the motion
/// crosses when there is none. A word is the class a double-click selects.
pub fn word_forward(text: &str, col: u16) -> WordStep {
    let chars: Vec<char> = text.chars().collect();
    let mut index = usize::from(col);
    if index >= chars.len() {
        return WordStep::NextRow;
    }
    while index < chars.len() && is_word_char(chars[index]) {
        index += 1;
    }
    while index < chars.len() && !is_word_char(chars[index]) {
        index += 1;
    }
    if index >= chars.len() {
        WordStep::NextRow
    } else {
        WordStep::Col(index as u16)
    }
}

/// Start of the word before `col`, or the row boundary crossed when there is
/// none.
pub fn word_back(text: &str, col: u16) -> WordStep {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() || col == 0 {
        return WordStep::PrevRow;
    }
    let mut index = usize::from(col).min(chars.len());
    while index > 0 && !is_word_char(chars[index - 1]) {
        index -= 1;
    }
    while index > 0 && is_word_char(chars[index - 1]) {
        index -= 1;
    }
    if index == 0 && chars.first().is_some_and(|ch| !is_word_char(*ch)) {
        // Only punctuation before the cursor: there is no earlier word here.
        return WordStep::PrevRow;
    }
    WordStep::Col(index as u16)
}

/// End of the word at or after `col`, or the row boundary crossed when there is
/// none. Landing on the last character is the vi behaviour, so `e` is useful
/// for extending a selection.
pub fn word_end(text: &str, col: u16) -> WordStep {
    let chars: Vec<char> = text.chars().collect();
    let mut index = usize::from(col);
    if index >= chars.len() {
        return WordStep::NextRow;
    }
    if is_word_char(chars[index]) {
        while index + 1 < chars.len() && is_word_char(chars[index + 1]) {
            index += 1;
        }
        return WordStep::Col(index as u16);
    }
    while index < chars.len() && !is_word_char(chars[index]) {
        index += 1;
    }
    while index + 1 < chars.len() && is_word_char(chars[index + 1]) {
        index += 1;
    }
    if index >= chars.len() {
        WordStep::NextRow
    } else {
        WordStep::Col(index as u16)
    }
}

/// The last column with content on the row, or 0 for an empty one.
pub fn line_end(text: &str) -> u16 {
    let trimmed = text.trim_end();
    trimmed.chars().count().saturating_sub(1) as u16
}

/// What a key does in copy mode. The caller owns the pane, so a motion is
/// described rather than performed: `Move` is relative, `Word` needs the lines,
/// and `Scroll` changes the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyAction {
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    /// `w`, `b` and `e`: one word, forward, back or to its end.
    WordForward,
    WordBack,
    WordEnd,
    LineStart,
    LineEnd,
    /// `g` — jump to the oldest line, and `G` to the newest.
    Top,
    Bottom,
    /// Half and full page, in either direction.
    PageUp(bool),
    PageDown(bool),
    Visual,
    Yank,
    Exit,
    /// A digit, accumulated as the count for the next motion.
    Digit(u8),
    /// A key copy mode does not use; it is swallowed so it cannot reach the shell.
    Ignore,
}

/// Map a key to an action. Copy mode follows vi, so it is case-sensitive: `g`
/// is top where `G` is bottom. `ctrl` pages, because a terminal can receive a
/// Ctrl chord without the window claiming it elsewhere.
pub fn action_for(key: &str, shift: bool, ctrl: bool) -> CopyAction {
    match key {
        "escape" => return CopyAction::Exit,
        "enter" => return CopyAction::Yank,
        "left" => return CopyAction::MoveLeft,
        "right" => return CopyAction::MoveRight,
        "up" => return CopyAction::MoveUp,
        "down" => return CopyAction::MoveDown,
        "home" => return CopyAction::LineStart,
        "end" => return CopyAction::LineEnd,
        "pageup" => return CopyAction::PageUp(true),
        "pagedown" => return CopyAction::PageDown(true),
        _ => {}
    }
    if ctrl {
        return match key.to_ascii_lowercase().as_str() {
            "d" => CopyAction::PageDown(false),
            "u" => CopyAction::PageUp(false),
            "f" => CopyAction::PageDown(true),
            "b" => CopyAction::PageUp(true),
            "v" => CopyAction::Visual,
            "c" => CopyAction::Yank,
            _ => CopyAction::Ignore,
        };
    }
    match key {
        "G" => CopyAction::Bottom,
        "V" => CopyAction::Visual,
        "h" => CopyAction::MoveLeft,
        "j" => CopyAction::MoveDown,
        "k" => CopyAction::MoveUp,
        "l" => CopyAction::MoveRight,
        "w" => CopyAction::WordForward,
        "b" => CopyAction::WordBack,
        "e" => CopyAction::WordEnd,
        "g" => CopyAction::Top,
        "v" => CopyAction::Visual,
        "y" => CopyAction::Yank,
        "q" => CopyAction::Exit,
        "0" => CopyAction::LineStart,
        "$" => CopyAction::LineEnd,
        digit if !shift && digit.len() == 1 => match digit.parse::<u8>() {
            Ok(value) if value <= 9 => CopyAction::Digit(value),
            _ => CopyAction::Ignore,
        },
        _ => CopyAction::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(lines: &[&str]) -> CopyMode {
        CopyMode::new("srf", lines.iter().map(|line| line.to_string()).collect())
    }

    #[test]
    fn given_a_word_then_forward_lands_on_the_next_word_start() {
        assert_eq!(word_forward("alpha beta", 0), WordStep::Col(6));
        assert_eq!(word_forward("alpha beta", 2), WordStep::Col(6));
    }

    #[test]
    fn given_no_word_after_the_cursor_then_forward_crosses_the_row() {
        assert_eq!(word_forward("alpha", 0), WordStep::NextRow);
        assert_eq!(word_forward("alpha", 10), WordStep::NextRow);
        assert_eq!(word_forward("", 0), WordStep::NextRow);
    }

    #[test]
    fn given_leading_space_then_forward_skips_to_the_word() {
        // `-` is a word character (it is part of a flag or a path), so `--flag`
        // is one word: only the leading spaces are skipped.
        assert_eq!(word_forward("  --flag", 0), WordStep::Col(2));
        assert_eq!(word_forward("   flag", 0), WordStep::Col(3));
    }

    #[test]
    fn given_a_cursor_inside_a_word_then_back_lands_on_its_start() {
        assert_eq!(word_back("alpha beta", 8), WordStep::Col(6));
        assert_eq!(word_back("alpha beta", 6), WordStep::Col(0));
    }

    #[test]
    fn given_the_row_start_then_back_crosses_the_row() {
        assert_eq!(word_back("alpha", 0), WordStep::PrevRow);
        assert_eq!(word_back("", 0), WordStep::PrevRow);
    }

    #[test]
    fn given_only_spaces_before_the_cursor_then_back_crosses_the_row() {
        assert_eq!(word_back("   ", 2), WordStep::PrevRow);
    }

    #[test]
    fn given_a_cursor_in_a_word_then_end_stops_on_its_last_character() {
        assert_eq!(word_end("alpha beta", 0), WordStep::Col(4));
        assert_eq!(word_end("alpha beta", 2), WordStep::Col(4));
    }

    #[test]
    fn given_punctuation_then_end_finds_the_following_word() {
        assert_eq!(word_end("alpha beta", 5), WordStep::Col(9));
    }

    #[test]
    fn given_a_line_then_end_is_its_last_non_space_column() {
        assert_eq!(line_end("hello   "), 4);
        assert_eq!(line_end(""), 0);
        assert_eq!(line_end("x"), 0);
    }

    #[test]
    fn given_a_visual_selection_dragged_upwards_then_the_span_is_in_reading_order() {
        let mut session = mode(&["one", "two", "three", "four", "five"]);
        session.cursor = (4, 2);
        session.toggle_visual();
        session.cursor = (1, 7);
        assert_eq!(session.selection(), Some(((1, 7), (4, 2))));
        assert!(session.is_visual());
        session.toggle_visual();
        assert_eq!(session.selection(), None);
    }

    #[test]
    fn given_digits_then_they_accumulate_into_a_count() {
        let mut session = mode(&["one"]);
        assert_eq!(session.take_count(), 1);
        session.push_digit(1);
        session.push_digit(2);
        assert_eq!(session.take_count(), 12);
    }

    #[test]
    fn given_vertical_motion_then_it_stops_at_the_buffer_ends() {
        let mut session = mode(&["a", "b", "c"]);
        session.move_lines(10, session.last_line());
        assert_eq!(session.cursor, (2, 0));
        session.move_lines(-10, session.last_line());
        assert_eq!(session.cursor, (0, 0));
        session.move_lines(1, session.last_line());
        assert_eq!(session.cursor, (1, 0));
    }

    #[test]
    fn given_a_longer_line_above_then_a_vertical_motion_clamps_the_column() {
        let mut session = mode(&["abcdef", "x"]);
        session.cursor = (0, 5);
        session.move_lines(1, session.last_line());
        assert_eq!(session.cursor, (1, 0));
    }

    #[test]
    fn given_word_forward_at_a_line_end_then_it_crosses_to_the_next_line() {
        let mut session = mode(&["alpha", "beta gamma"]);
        session.cursor = (0, 2);
        session.move_word(CopyAction::WordForward, 1);
        assert_eq!(session.cursor, (1, 0));
    }

    #[test]
    fn given_word_forward_with_a_count_then_it_repeats_across_lines() {
        let mut session = mode(&["alpha", "beta gamma"]);
        session.move_word(CopyAction::WordForward, 2);
        assert_eq!(session.cursor, (1, 5));
    }

    #[test]
    fn given_word_back_at_a_line_start_then_it_crosses_to_the_previous_line() {
        let mut session = mode(&["alpha", "beta"]);
        session.cursor = (1, 0);
        session.move_word(CopyAction::WordBack, 1);
        assert_eq!(session.cursor, (0, 0));
    }

    #[test]
    fn given_word_back_inside_a_word_then_it_lands_on_that_word() {
        let mut session = mode(&["alpha", "beta"]);
        session.cursor = (1, 2);
        session.move_word(CopyAction::WordBack, 1);
        assert_eq!(session.cursor, (1, 0));
    }

    #[test]
    fn given_a_scrolled_window_when_the_cursor_leaves_it_then_the_view_follows() {
        // The cursor is on line 12 and the window shows 5..10.
        assert_eq!(scroll_to_show(12, 5, 5, 20), Some(8));
        // Above the window scrolls up to the cursor.
        assert_eq!(scroll_to_show(2, 5, 5, 20), Some(2));
        // Inside the window keeps the offset.
        assert_eq!(scroll_to_show(7, 5, 5, 20), None);
    }

    #[test]
    fn given_a_single_line_selection_then_only_its_columns_are_taken() {
        let lines = vec!["hello world".to_string()];
        assert_eq!(yank_text(&lines, (0, 6), (0, 10)), "world");
        assert_eq!(yank_text(&lines, (0, 0), (0, 4)), "hello");
    }

    #[test]
    fn given_a_multi_line_selection_then_the_rows_between_are_included() {
        let lines = vec![
            "one two".to_string(),
            "three".to_string(),
            "four five".to_string(),
        ];
        assert_eq!(yank_text(&lines, (0, 4), (2, 3)), "two\nthree\nfour");
    }

    #[test]
    fn given_trailing_whitespace_then_the_yank_drops_it_per_line() {
        let lines = vec!["ab   ".to_string(), "cd   ".to_string()];
        assert_eq!(yank_text(&lines, (0, 0), (1, 1)), "ab\ncd");
    }

    #[test]
    fn given_a_column_past_the_line_end_then_the_yank_stops_at_the_text() {
        let lines = vec!["ab".to_string(), "c".to_string()];
        assert_eq!(yank_text(&lines, (0, 0), (1, 9)), "ab\nc");
    }

    #[test]
    fn given_the_vi_motions_then_each_maps_to_its_action() {
        assert_eq!(action_for("h", false, false), CopyAction::MoveLeft);
        assert_eq!(action_for("j", false, false), CopyAction::MoveDown);
        assert_eq!(action_for("k", false, false), CopyAction::MoveUp);
        assert_eq!(action_for("l", false, false), CopyAction::MoveRight);
        assert_eq!(action_for("w", false, false), CopyAction::WordForward);
        assert_eq!(action_for("b", false, false), CopyAction::WordBack);
        assert_eq!(action_for("e", false, false), CopyAction::WordEnd);
        assert_eq!(action_for("0", false, false), CopyAction::LineStart);
        assert_eq!(action_for("$", false, false), CopyAction::LineEnd);
        assert_eq!(action_for("v", false, false), CopyAction::Visual);
        assert_eq!(action_for("y", false, false), CopyAction::Yank);
        assert_eq!(action_for("q", false, false), CopyAction::Exit);
    }

    #[test]
    fn given_shifted_letters_then_case_selects_the_other_motion() {
        assert_eq!(action_for("g", false, false), CopyAction::Top);
        assert_eq!(action_for("G", true, false), CopyAction::Bottom);
        assert_eq!(action_for("V", true, false), CopyAction::Visual);
    }

    #[test]
    fn given_a_ctrl_chord_then_it_pages() {
        assert_eq!(action_for("d", false, true), CopyAction::PageDown(false));
        assert_eq!(action_for("u", false, true), CopyAction::PageUp(false));
        assert_eq!(action_for("f", false, true), CopyAction::PageDown(true));
        assert_eq!(action_for("b", false, true), CopyAction::PageUp(true));
    }

    #[test]
    fn given_a_digit_then_it_becomes_a_count() {
        assert_eq!(action_for("5", false, false), CopyAction::Digit(5));
        assert_eq!(action_for("0", false, false), CopyAction::LineStart);
    }

    #[test]
    fn given_an_unmapped_key_then_it_is_swallowed() {
        assert_eq!(action_for("z", false, false), CopyAction::Ignore);
        assert_eq!(action_for("x", false, true), CopyAction::Ignore);
    }
}
