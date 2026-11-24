//! Scrollback search: literal or regular expression, whole-word and
//! case-insensitive, across every pane of a workspace.
//!
//! A query is compiled once per keystroke and run over each pane's lines.
//! Matches carry a surface id, so the panel can jump to a hit in another pane
//! and the window can highlight the one hit that is selected.

use regex::{Regex, RegexBuilder};

use crate::ui::is_word_char;

/// How many hits one search keeps before it stops looking. A broad pattern over
/// a workspace of deep scrollbacks would otherwise stall a keystroke.
pub const MAX_MATCHES: usize = 500;

/// One hit, with `col` and `len` counted in characters so the painter can map
/// them onto cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub surface_id: String,
    pub line: usize,
    pub col: usize,
    pub len: usize,
}

impl Hit {
    /// Where the hit paints: absolute line, column and length.
    pub fn span(&self) -> Span {
        (self.line, self.col, self.len)
    }
}

/// A hit's position on a line, as the painter indexes it.
pub type Span = (usize, usize, usize);

/// The toggles a search panel carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Treat the query as a regular expression instead of a literal.
    pub regex: bool,
    /// Only match when neither neighbour is a word character, so `error`
    /// matches `an error:` but not `errors`.
    pub whole_word: bool,
}

/// A compiled query, or the reason it could not be compiled.
pub struct Query {
    pattern: Option<Regex>,
    /// Why the last compilation failed, for the panel footer.
    pub error: Option<String>,
    options: Options,
}

impl Query {
    /// Compile `text`. An empty query matches nothing, and an invalid regular
    /// expression reports its error instead of matching nothing silently.
    pub fn new(text: &str, options: Options) -> Self {
        if text.is_empty() {
            return Self {
                pattern: None,
                error: None,
                options,
            };
        }
        let source = if options.regex {
            text.to_string()
        } else {
            // A literal query must not be read as a pattern: an unmatched
            // bracket is a character the user is looking for, not an error.
            regex::escape(text)
        };
        match RegexBuilder::new(&source).case_insensitive(true).build() {
            Ok(pattern) => Self {
                pattern: Some(pattern),
                error: None,
                options,
            },
            Err(error) => Self {
                pattern: None,
                error: Some(regex_error(&error.to_string())),
                options,
            },
        }
    }

    /// Whether the query can match anything.
    pub fn is_active(&self) -> bool {
        self.pattern.is_some()
    }

    /// Every hit in `text`, up to `limit`. `base` is the line number of the
    /// first string, so a pane's whole scrollback can be searched as one slice.
    pub fn find(&self, surface_id: &str, lines: &[String], base: usize, limit: usize) -> Vec<Hit> {
        let Some(pattern) = self.pattern.as_ref() else {
            return Vec::new();
        };
        let mut hits = Vec::new();
        for (offset, line) in lines.iter().enumerate() {
            if hits.len() >= limit {
                break;
            }
            collect(
                pattern,
                &self.options,
                surface_id,
                base + offset,
                line,
                limit,
                &mut hits,
            );
        }
        hits
    }
}

/// One pane's buffer, as the search panel holds it.
pub struct PaneBuffer {
    pub surface_id: String,
    /// The pane title, shown beside a hit so a cross-pane result is legible.
    pub title: String,
    /// Every line of the scrollback, oldest first. The index is the absolute
    /// line number the painter and `set_scrollback` use.
    pub lines: Vec<String>,
}

/// Search every pane in `panes`, in order, stopping at the match budget.
pub fn search(panes: &[PaneBuffer], query: &Query) -> Vec<Hit> {
    let mut hits = Vec::new();
    for pane in panes {
        if hits.len() >= MAX_MATCHES {
            break;
        }
        let remaining = MAX_MATCHES - hits.len();
        hits.extend(query.find(&pane.surface_id, &pane.lines, 0, remaining));
    }
    hits
}

fn collect(
    pattern: &Regex,
    options: &Options,
    surface_id: &str,
    line: usize,
    text: &str,
    limit: usize,
    hits: &mut Vec<Hit>,
) {
    for found in pattern.find_iter(text) {
        if hits.len() >= limit {
            return;
        }
        let (start, end) = (found.start(), found.end());
        if options.whole_word && !stands_alone(text, start, end) {
            continue;
        }
        if start == end {
            // An empty pattern match (`a*` at the end of a line) has nothing to
            // highlight, and an empty span would be invisible in the panel.
            continue;
        }
        hits.push(Hit {
            surface_id: surface_id.to_string(),
            line,
            col: text[..start].chars().count(),
            len: text[start..end].chars().count(),
        });
    }
}

/// Whether the span is bounded by non-word characters, the way a whole-word
/// search wants. `\b` alone is wrong for a query that starts or ends with a
/// symbol, so the neighbours are tested against the same class a double-click
/// selects.
fn stands_alone(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    before.is_none_or(|ch| !is_word_char(ch)) && after.is_none_or(|ch| !is_word_char(ch))
}

/// The first line of a regex error, without the pattern echoed back.
fn regex_error(text: &str) -> String {
    text.lines().next().unwrap_or("invalid pattern").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn literal(text: &str) -> Query {
        Query::new(text, Options::default())
    }

    #[test]
    fn given_a_literal_query_then_it_matches_case_insensitively() {
        let query = literal("error");
        let hits = query.find("srf", &lines(&["An ERROR here", "nothing"]), 0, MAX_MATCHES);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].line, hits[0].col, hits[0].len), (0, 3, 5));
    }

    #[test]
    fn given_a_literal_query_with_regex_syntax_then_it_is_not_read_as_a_pattern() {
        let query = literal("a[");
        let hits = query.find("srf", &lines(&["a[1]"]), 0, MAX_MATCHES);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].col, hits[0].len), (0, 2));
    }

    #[test]
    fn given_a_regex_query_then_it_matches_the_pattern() {
        let query = Query::new(
            r"err\w+",
            Options {
                regex: true,
                ..Options::default()
            },
        );
        let hits = query.find("srf", &lines(&["errors and errors"]), 0, MAX_MATCHES);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].len, 6);
    }

    #[test]
    fn given_an_invalid_regex_then_the_error_is_reported_and_nothing_matches() {
        let query = Query::new(
            "a(",
            Options {
                regex: true,
                ..Options::default()
            },
        );
        assert!(!query.is_active());
        assert!(query.error.is_some());
        assert!(
            query
                .find("srf", &lines(&["a("]), 0, MAX_MATCHES)
                .is_empty()
        );
    }

    #[test]
    fn given_whole_word_then_a_longer_word_does_not_match() {
        let query = Query::new(
            "error",
            Options {
                whole_word: true,
                ..Options::default()
            },
        );
        let hits = query.find(
            "srf",
            &lines(&["errors", "an error:", "error_code"]),
            0,
            MAX_MATCHES,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 1);
    }

    #[test]
    fn given_whole_word_then_punctuation_bounds_a_word() {
        let query = Query::new(
            "error",
            Options {
                whole_word: true,
                ..Options::default()
            },
        );
        let hits = query.find("srf", &lines(&["(error)"]), 0, MAX_MATCHES);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].col, 1);
    }

    #[test]
    fn given_an_empty_query_then_it_is_inactive_and_matches_nothing() {
        let query = literal("");
        assert!(!query.is_active());
        assert!(query.error.is_none());
        assert!(
            query
                .find("srf", &lines(&["anything"]), 0, MAX_MATCHES)
                .is_empty()
        );
    }

    #[test]
    fn given_matches_in_two_panes_then_each_hit_names_its_surface() {
        let panes = vec![
            PaneBuffer {
                surface_id: "srf_a".into(),
                title: "build".into(),
                lines: lines(&["error", "ok"]),
            },
            PaneBuffer {
                surface_id: "srf_b".into(),
                title: "tests".into(),
                lines: lines(&["fine", "error"]),
            },
        ];
        let hits = search(&panes, &literal("error"));
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].surface_id, "srf_a");
        assert_eq!(hits[1].surface_id, "srf_b");
        assert_eq!(hits[1].line, 1);
    }

    #[test]
    fn given_the_match_budget_then_searching_stops_at_it() {
        let query = literal("x");
        let many = vec!["x".repeat(MAX_MATCHES + 50)];
        let hits = query.find("srf", &many, 0, MAX_MATCHES);
        assert_eq!(hits.len(), MAX_MATCHES);
    }

    #[test]
    fn given_wide_characters_then_the_column_is_a_character_offset() {
        let query = literal("日");
        let hits = query.find("srf", &lines(&["ab日本語"]), 0, MAX_MATCHES);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].col, hits[0].len), (2, 1));
    }
}
