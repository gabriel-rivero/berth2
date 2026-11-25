//! Output rules: regexes that style the lines they match, like ConEmu's
//! Highlight and iTerm2's Triggers.
//!
//! A rule is matched against a whole visible row at paint time rather than
//! against the byte stream. That is deliberate: the grid already holds the
//! final text, so a rule sees what the user sees — after wrapping, overstrikes
//! and colours — and it applies to scrollback as well as new output without a
//! second copy of the line being kept anywhere.
//!
//! Only the row's tint is set. A rule cannot change a glyph or its size, so it
//! cannot break the cell grid or widen a row.

use regex::{Regex, RegexBuilder};

/// The styling an output rule applies to a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tint {
    pub fg: Option<u32>,
    pub bg: Option<u32>,
    pub bold: bool,
}

impl Rule {
    pub fn tint(&self) -> Tint {
        Tint {
            fg: self.fg,
            bg: self.bg,
            bold: self.bold,
        }
    }
}

/// One configured rule.
pub struct Rule {
    #[allow(dead_code)]
    /// What the rule is called, kept for diagnostics.
    name: String,
    pattern: Regex,
    /// Foreground and background tints, packed 0x00RRGGBB.
    pub fg: Option<u32>,
    pub bg: Option<u32>,
    pub bold: bool,
}

impl Rule {
    /// Whether the rule matches a row's text.
    pub fn is_match(&self, line: &str) -> bool {
        // A row of spaces matches `\s*` and every similar pattern, which would
        // tint every blank line; skip the empty ones before the regex runs.
        if line.trim().is_empty() {
            return false;
        }
        self.pattern.is_match(line)
    }
}

/// The compiled rule set. An invalid pattern is reported and skipped, so one
/// bad rule cannot stop the others or the window.
#[derive(Default)]
pub struct Rules {
    rules: Vec<Rule>,
    /// Patterns that did not compile, for the startup log.
    pub rejected: Vec<String>,
}

impl Rules {
    pub fn new() -> Self {
        Self::default()
    }

    /// Compile from `config.json` shapes.
    pub fn from_specs(specs: &[crate::config::OutputRule]) -> Self {
        let mut rules = Self::new();
        for spec in specs {
            let name = if spec.name.is_empty() {
                spec.pattern.clone()
            } else {
                spec.name.clone()
            };
            let built = RegexBuilder::new(&spec.pattern)
                .case_insensitive(true)
                .build();
            match built {
                Ok(pattern) => {
                    if spec.fg.is_none() && spec.bg.is_none() && !spec.bold {
                        // A rule that changes nothing is a config mistake; say
                        // so rather than silently doing nothing forever.
                        rules
                            .rejected
                            .push(format!("{name:?}: rule sets no fg, bg or bold"));
                        continue;
                    }
                    rules.rules.push(Rule {
                        name,
                        pattern,
                        fg: spec.fg.as_deref().and_then(parse_color),
                        bg: spec.bg.as_deref().and_then(parse_color),
                        bold: spec.bold,
                    });
                }
                Err(error) => rules.rejected.push(format!(
                    "{name:?}: {}",
                    error
                        .to_string()
                        .lines()
                        .next()
                        .unwrap_or("invalid pattern")
                )),
            }
        }
        rules
    }

    /// Only used by the tests, which assert which rules survived compiling.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// The first rule matching `line`, or `None`. First match wins, so the
    /// order in the config file is the priority.
    pub fn first_match(&self, line: &str) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.is_match(line))
    }

    /// The tint for each row of a screen. `None` per row when no rule matches,
    /// and an empty vector when there are no rules at all, so the painter can
    /// skip the work entirely.
    pub fn row_tints(&self, rows: &[String]) -> Vec<Option<Tint>> {
        if self.rules.is_empty() {
            return Vec::new();
        }
        rows.iter()
            .map(|line| self.first_match(line).map(Rule::tint))
            .collect()
    }
}

/// Parse `#rrggbb` or `rrggbb` into a packed colour.
pub fn parse_color(text: &str) -> Option<u32> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OutputRule;

    fn rule(pattern: &str, fg: Option<&str>, bold: bool) -> OutputRule {
        OutputRule {
            name: String::new(),
            pattern: pattern.to_string(),
            fg: fg.map(str::to_string),
            bg: None,
            bold,
        }
    }

    #[test]
    fn given_a_colour_then_both_hex_spellings_parse() {
        assert_eq!(parse_color("#ff5555"), Some(0x00ff_5555));
        assert_eq!(parse_color("ff5555"), Some(0x00ff_5555));
        assert_eq!(parse_color("#fff"), None);
        assert_eq!(parse_color("nope"), None);
    }

    #[test]
    fn given_a_matching_row_then_the_rule_is_found() {
        let rules = Rules::from_specs(&[rule("error", Some("#ff5555"), false)]);
        let matched = rules.first_match("an ERROR happened").expect("a match");
        assert_eq!(matched.fg, Some(0x00ff_5555));
    }

    #[test]
    fn given_a_non_matching_row_then_nothing_matches() {
        let rules = Rules::from_specs(&[rule("error", Some("#ff5555"), false)]);
        assert!(rules.first_match("all good").is_none());
    }

    #[test]
    fn given_blank_rows_then_a_rule_does_not_tint_them() {
        let rules = Rules::from_specs(&[rule(r"^\s*$", None, true)]);
        assert!(rules.first_match("    ").is_none());
        assert!(rules.first_match("").is_none());
    }

    #[test]
    fn given_several_rules_then_the_first_in_config_order_wins() {
        let rules = Rules::from_specs(&[
            rule("error", Some("#ff0000"), false),
            rule("error", Some("#00ff00"), false),
        ]);
        assert_eq!(
            rules.first_match("error").map(|rule| rule.fg),
            Some(Some(0x00ff_0000))
        );
    }

    #[test]
    fn given_an_invalid_pattern_then_it_is_rejected_and_the_rest_survive() {
        let rules = Rules::from_specs(&[
            rule("a(", Some("#ff0000"), false),
            rule("ok", Some("#00ff00"), false),
        ]);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules.rejected.len(), 1);
        assert!(rules.first_match("ok").is_some());
    }

    #[test]
    fn given_a_rule_that_styles_nothing_then_it_is_rejected() {
        let rules = Rules::from_specs(&[rule("error", None, false)]);
        assert!(rules.is_empty());
        assert_eq!(rules.rejected.len(), 1);
    }

    #[test]
    fn given_an_unparsable_colour_then_the_rule_still_matches_without_it() {
        let rules = Rules::from_specs(&[rule("error", Some("not-a-colour"), true)]);
        let matched = rules.first_match("error").expect("the rule still matches");
        assert_eq!(matched.fg, None);
        assert!(matched.bold);
    }

    #[test]
    fn given_rows_then_only_the_matching_ones_get_a_tint() {
        let rules = Rules::from_specs(&[rule("error", Some("#ff0000"), false)]);
        let rows = vec!["all good".to_string(), "an error".to_string()];
        let tints = rules.row_tints(&rows);
        assert_eq!(tints.len(), 2);
        assert_eq!(tints[0], None);
        assert_eq!(tints[1].map(|tint| tint.fg), Some(Some(0x00ff_0000)));
    }

    #[test]
    fn given_no_rules_then_row_tints_is_empty_so_the_painter_skips_it() {
        let rules = Rules::new();
        assert!(rules.row_tints(&["anything".to_string()]).is_empty());
    }

    #[test]
    fn given_an_empty_config_then_the_set_is_empty() {
        let rules = Rules::from_specs(&[]);
        assert!(rules.is_empty());
        assert!(rules.rejected.is_empty());
    }
}
