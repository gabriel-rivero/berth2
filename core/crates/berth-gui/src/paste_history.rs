//! Paste history: the last few clipboard entries, with a picker.
//!
//! This is an in-memory ring, deliberately never written to disk — a terminal
//! clipboard can hold a password or a token, and a file would outlive the
//! session that copied it.
//!
//! Entries are recorded by the user's own copy and paste commands, not by a
//! program writing the clipboard over OSC 52: a program can write it in a loop,
//! and letting that fill the ring would push the user's own entries out.

/// How many entries the history keeps. Enough to reach the thing copied two
/// commands ago without turning the picker into a list nobody scans.
pub const CAPACITY: usize = 12;

/// The largest entry kept. A pasted file or a huge selection must not pin its
/// bytes in memory for the rest of the session; the clipboard keeps the real
/// copy, and an entry this large is not one the picker can show anyway.
pub const MAX_ENTRY_BYTES: usize = 64 * 1024;

/// How many characters of the first line the picker row shows. Computing this
/// once at record time keeps the panel from re-walking a long line every frame.
pub const PREVIEW_CHARS: usize = 80;

/// One remembered clipboard entry.
///
/// Not `Debug`: a derived one would print whatever was copied into any panic
/// message or log line.
#[derive(Clone, PartialEq, Eq)]
pub struct Entry {
    pub text: String,
    preview: String,
    lines: usize,
}

impl Entry {
    fn new(text: String) -> Self {
        let first = text.lines().next().unwrap_or("").trim();
        let lines = text.lines().count().max(1);
        // A blank entry still needs a label, and one blank line must not read
        // as "1 blank lines".
        let preview = if first.is_empty() {
            match lines {
                1 => "1 blank line".to_string(),
                _ => format!("{lines} blank lines"),
            }
        } else {
            first.chars().take(PREVIEW_CHARS).collect()
        };
        Self {
            text,
            preview,
            lines,
        }
    }

    /// The picker row label, clipped when the entry was recorded.
    pub fn preview(&self) -> &str {
        &self.preview
    }

    /// How many lines the entry carries, for the row detail.
    pub fn line_count(&self) -> usize {
        self.lines
    }
}

/// The most recent entries, newest first.
#[derive(Default)]
pub struct PasteHistory {
    entries: Vec<Entry>,
}

impl PasteHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a clipboard entry. A blank entry is ignored, and an entry already
    /// in the ring moves to the front rather than appearing twice: the picker
    /// labels rows by their first line, so a duplicate would read as one entry
    /// at two ages and waste a slot.
    pub fn record(&mut self, text: &str) {
        // A trailing newline is invisible in the picker row, so `foo` and
        // `foo\n` must not become two rows that read the same. The stored text
        // is normalised too, so every copy spells an entry the same way.
        let text = text.trim_end();
        if text.trim().is_empty() || text.len() > MAX_ENTRY_BYTES {
            return;
        }
        if let Some(index) = self.entries.iter().position(|entry| entry.text == text) {
            let entry = self.entries.remove(index);
            self.entries.insert(0, entry);
            return;
        }
        self.entries.insert(0, Entry::new(text.to_string()));
        self.entries.truncate(CAPACITY);
    }

    /// Newest first.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&Entry> {
        self.entries.get(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(values: &[&str]) -> PasteHistory {
        let mut history = PasteHistory::new();
        for value in values {
            history.record(value);
        }
        history
    }

    #[test]
    fn given_several_pastes_then_the_newest_comes_first() {
        let history = history(&["one", "two", "three"]);
        let texts: Vec<_> = history.entries().iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, vec!["three", "two", "one"]);
    }

    #[test]
    fn given_a_repeated_paste_then_it_moves_to_the_front() {
        let history = history(&["a", "b", "a"]);
        let texts: Vec<_> = history.entries().iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts, vec!["a", "b"]);
    }

    #[test]
    fn given_more_than_capacity_then_the_oldest_falls_off() {
        let values: Vec<String> = (0..CAPACITY + 5).map(|n| format!("entry{n}")).collect();
        let refs: Vec<&str> = values.iter().map(String::as_str).collect();
        let history = history(&refs);
        assert_eq!(history.len(), CAPACITY);
        assert_eq!(
            history.get(0).unwrap().text,
            format!("entry{}", CAPACITY + 4)
        );
        assert_eq!(
            history.get(CAPACITY - 1).unwrap().text,
            format!("entry{}", 5)
        );
    }

    #[test]
    fn given_a_blank_or_whitespace_paste_then_nothing_is_recorded() {
        let history = history(&["", "   ", "\t\n "]);
        assert!(history.is_empty());
    }

    #[test]
    fn given_an_entry_over_the_byte_cap_then_it_is_not_kept() {
        let huge = "x".repeat(MAX_ENTRY_BYTES + 1);
        let history = history(&[&huge]);
        assert!(history.is_empty());
    }

    #[test]
    fn given_an_entry_exactly_at_the_cap_then_it_is_kept() {
        let at_cap = "x".repeat(MAX_ENTRY_BYTES);
        let history = history(&[&at_cap]);
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn given_a_trailing_newline_then_the_entry_reads_the_same_as_without_it() {
        let history = history(&["foo\n", "foo"]);
        assert_eq!(
            history.len(),
            1,
            "a trailing newline split one entry in two"
        );
        assert_eq!(history.get(0).unwrap().text, "foo");
    }

    #[test]
    fn given_a_windows_newline_then_the_line_count_is_right() {
        assert_eq!(Entry::new("a\r\nb".to_string()).line_count(), 2);
        assert_eq!(Entry::new("a\r\n".to_string()).line_count(), 1);
    }

    #[test]
    fn given_a_multiline_entry_then_the_preview_is_its_first_line() {
        let entry = Entry::new("first line\nsecond\nthird".to_string());
        assert_eq!(entry.preview(), "first line");
        assert_eq!(entry.line_count(), 3);
    }

    #[test]
    fn given_a_blank_entry_then_the_preview_is_singular_for_one_line() {
        assert_eq!(Entry::new("\n".to_string()).preview(), "1 blank line");
        assert_eq!(Entry::new("\n\n".to_string()).preview(), "2 blank lines");
    }

    #[test]
    fn given_a_long_first_line_then_the_preview_is_clipped() {
        let long = format!("{} tail", "y".repeat(500));
        let entry = Entry::new(long);
        assert_eq!(entry.preview().chars().count(), PREVIEW_CHARS);
        // The stored text keeps every byte; only the label is clipped.
        assert_eq!(entry.line_count(), 1);
    }
}
