//! OSC 133/633 semantic prompt marks.
//!
//! A shell that emits shell integration brackets its prompt and every command
//! with OSC 133 sequences. `A` is the prompt start, `B` the start of the
//! command the user sent, `C` the start of the command's output and `D` the
//! end of it, carrying an exit status. OSC 633 is VS Code's dialect and is
//! parsed the same way.
//!
//! The reader records marks against the absolute grid line the cursor was on,
//! exactly as [`crate::style::StyleState`] records attributes, so a mark
//! survives scrolling and is pruned with the scrollback it belongs to.
//!
//! Two row kinds are stored: a prompt row and the row a command's output
//! starts on. `D` is not a row of its own. A shell emits it on the line the
//! output ended on and the next `A` immediately after, often on that same
//! line, so the exit status is held until the next prompt claims it. That is
//! also the shape a reader wants: the gutter mark on a prompt says whether the
//! command above it failed.

use std::collections::BTreeMap;

/// What a program said about a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// `A` or `B`: the prompt, where the user starts typing.
    Prompt,
    /// `C`: the command's output begins on this row.
    CommandStart,
}

/// One stored mark, placed on an absolute grid line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptMark {
    pub line: usize,
    pub kind: PromptKind,
    /// The exit status of the command that ran above this prompt, when the
    /// shell reported one and there was a command to report on.
    pub status: Option<i32>,
    /// How long the command above this prompt took, in milliseconds, when the
    /// shell reported it. OSC 133 `D` may carry a duration after the status
    /// (`133;D;0;1234`), and OSC 633 always does.
    pub duration_ms: Option<u64>,
}

impl PromptMark {
    /// Whether the command above this prompt failed.
    pub fn failed(&self) -> bool {
        self.kind == PromptKind::Prompt && self.status.is_some_and(|status| status != 0)
    }
}

/// What `D` reported about the command that just finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CommandReport {
    pub status: Option<i32>,
    /// Wall-clock duration in milliseconds. OSC 633 puts it after the status
    /// (`633;D;0;1234`); a plain OSC 133 `D` usually carries only the status.
    pub duration_ms: Option<u64>,
}

/// One OSC 133/633 sequence, before it is placed on a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// `A` or `B`.
    PromptStart,
    /// `C`.
    CommandStart,
    /// `D`, with the exit status and duration the shell reported.
    CommandEnd(CommandReport),
}

/// Prompt marks for one surface, keyed by absolute line so scrollback keeps
/// them. The alternate screen is skipped by the reader: a full-screen program
/// has no prompt to mark, and its redraws would leave marks on the wrong rows.
#[derive(Debug, Default)]
pub struct PromptMarks {
    marks: BTreeMap<usize, PromptMark>,
    /// The last `D`, waiting for the prompt that follows it.
    pending: Option<CommandReport>,
    pruned_below: usize,
}

impl PromptMarks {
    /// Record one event at `line`.
    pub fn record(&mut self, line: usize, event: Event) {
        match event {
            Event::PromptStart => {
                let report = self.pending.take().unwrap_or_default();
                let entry = self.marks.entry(line).or_insert(PromptMark {
                    line,
                    kind: PromptKind::Prompt,
                    status: report.status,
                    duration_ms: report.duration_ms,
                });
                // `A` and `B` land on the same row; keep the row a prompt and
                // keep the status the first of them claimed.
                entry.kind = PromptKind::Prompt;
                if entry.status.is_none() {
                    entry.status = report.status;
                }
                if entry.duration_ms.is_none() {
                    entry.duration_ms = report.duration_ms;
                }
            }
            Event::CommandStart => {
                // A row that already holds a prompt is a prompt: `C` on the
                // prompt row would otherwise hide it from jump-to-prompt.
                self.marks.entry(line).or_insert(PromptMark {
                    line,
                    kind: PromptKind::CommandStart,
                    status: None,
                    duration_ms: None,
                });
            }
            Event::CommandEnd(report) => self.pending = Some(report),
        }
    }

    /// Drop marks that scrolled past the configured scrollback.
    pub fn prune(&mut self, scrollback_len: usize, scrollback: usize) {
        let oldest = scrollback_len.saturating_sub(scrollback);
        if oldest <= self.pruned_below {
            return;
        }
        self.marks.retain(|line, _| *line >= oldest);
        self.pruned_below = oldest;
    }

    /// Every mark, oldest line first.
    pub fn all(&self) -> impl Iterator<Item = &PromptMark> {
        self.marks.values()
    }

    /// The marks inside a window starting at `first_line`.
    pub fn visible(&self, first_line: usize, rows: u16) -> Vec<PromptMark> {
        let end = first_line.saturating_add(usize::from(rows));
        self.marks
            .range(first_line..end)
            .map(|(_, mark)| *mark)
            .collect()
    }

    /// The absolute lines a jump-to-prompt chord walks, oldest first.
    pub fn prompt_lines(&self) -> Vec<usize> {
        self.marks
            .values()
            .filter(|mark| mark.kind == PromptKind::Prompt)
            .map(|mark| mark.line)
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    pub fn clear(&mut self) {
        self.marks.clear();
        self.pending = None;
        self.pruned_below = 0;
    }
}

/// Parse one OSC payload (`133;A`, `633;D;1`) into an event. Returns `None`
/// for an OSC that is not shell integration.
pub fn parse(payload: &[u8]) -> Option<Event> {
    let body = payload
        .strip_prefix(b"133;")
        .or_else(|| payload.strip_prefix(b"633;"))?;
    let mut fields = body.split(|byte| *byte == b';');
    match fields.next()? {
        b"A" | b"B" => Some(Event::PromptStart),
        b"C" => Some(Event::CommandStart),
        // `D` carries the exit status first (`D;1`), then an optional duration
        // in milliseconds (`D;1;1234`).
        b"D" => {
            let mut numbers = fields
                .filter_map(|field| std::str::from_utf8(field).ok())
                .filter_map(|field| field.trim().parse::<u64>().ok());
            let status = numbers.next().map(|status| status as i32);
            Some(Event::CommandEnd(CommandReport {
                status,
                duration_ms: numbers.next(),
            }))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_an_osc_133_sequence_then_it_parses_as_an_event() {
        assert_eq!(parse(b"133;A"), Some(Event::PromptStart));
        assert_eq!(parse(b"133;B"), Some(Event::PromptStart));
        assert_eq!(parse(b"133;C"), Some(Event::CommandStart));
    }

    #[test]
    fn given_a_finished_command_then_its_exit_status_is_read() {
        assert_eq!(
            parse(b"133;D;0"),
            Some(Event::CommandEnd(CommandReport {
                status: Some(0),
                duration_ms: None
            }))
        );
        assert_eq!(
            parse(b"133;D;127"),
            Some(Event::CommandEnd(CommandReport {
                status: Some(127),
                duration_ms: None
            }))
        );
    }

    #[test]
    fn given_a_finished_command_with_a_duration_then_both_fields_are_read() {
        assert_eq!(
            parse(b"133;D;0;1234"),
            Some(Event::CommandEnd(CommandReport {
                status: Some(0),
                duration_ms: Some(1234)
            }))
        );
        assert_eq!(
            parse(b"633;D;2;45"),
            Some(Event::CommandEnd(CommandReport {
                status: Some(2),
                duration_ms: Some(45)
            }))
        );
    }

    #[test]
    fn given_a_vs_code_mark_with_a_non_numeric_field_then_the_duration_is_skipped() {
        // A field that is not a number is not a duration, so the report keeps
        // the status and no timing rather than shifting the wrong field in.
        assert_eq!(
            parse(b"133;D;7;none"),
            Some(Event::CommandEnd(CommandReport {
                status: Some(7),
                duration_ms: None
            }))
        );
    }

    #[test]
    fn given_a_command_that_reported_no_status_then_the_end_event_has_none() {
        assert_eq!(
            parse(b"133;D"),
            Some(Event::CommandEnd(CommandReport::default()))
        );
    }

    #[test]
    fn given_an_unrelated_osc_then_the_parser_declines() {
        assert_eq!(parse(b"7;file:///tmp"), None);
        assert_eq!(parse(b"133;Z"), None);
        assert_eq!(parse(b"133"), None);
    }

    #[test]
    fn given_a_prompt_with_no_command_before_it_then_it_has_no_status() {
        let mut marks = PromptMarks::default();
        marks.record(0, Event::PromptStart);
        let all: Vec<_> = marks.all().copied().collect();
        assert_eq!(
            all,
            vec![PromptMark {
                line: 0,
                kind: PromptKind::Prompt,
                status: None,
                duration_ms: None
            }]
        );
    }

    #[test]
    fn given_a_prompt_then_a_command_start_then_output_then_a_prompt_then_the_marks_split_by_row() {
        let mut marks = PromptMarks::default();
        marks.record(0, Event::PromptStart);
        marks.record(1, Event::CommandStart);
        marks.record(
            1,
            Event::CommandEnd(CommandReport {
                status: Some(0),
                duration_ms: None,
            }),
        );
        marks.record(2, Event::PromptStart);
        let all: Vec<_> = marks.all().copied().collect();
        assert_eq!(
            all,
            vec![
                PromptMark {
                    line: 0,
                    kind: PromptKind::Prompt,
                    status: None,
                    duration_ms: None
                },
                PromptMark {
                    line: 1,
                    kind: PromptKind::CommandStart,
                    status: None,
                    duration_ms: None
                },
                PromptMark {
                    line: 2,
                    kind: PromptKind::Prompt,
                    status: Some(0),
                    duration_ms: None
                },
            ]
        );
    }

    #[test]
    fn given_a_failed_command_then_the_next_prompt_carries_its_status() {
        let mut marks = PromptMarks::default();
        marks.record(0, Event::PromptStart);
        marks.record(
            1,
            Event::CommandEnd(CommandReport {
                status: Some(1),
                duration_ms: Some(2500),
            }),
        );
        marks.record(1, Event::PromptStart);
        let all: Vec<_> = marks.all().copied().collect();
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].status, Some(1));
        assert_eq!(all[1].duration_ms, Some(2500));
        assert!(all[1].failed());
    }

    #[test]
    fn given_the_prompt_start_after_a_command_end_on_the_same_row_then_the_row_is_a_prompt() {
        let mut marks = PromptMarks::default();
        marks.record(
            4,
            Event::CommandEnd(CommandReport {
                status: Some(130),
                duration_ms: None,
            }),
        );
        marks.record(4, Event::PromptStart);
        marks.record(4, Event::PromptStart);
        let all: Vec<_> = marks.all().copied().collect();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].kind, PromptKind::Prompt);
        assert_eq!(all[0].status, Some(130));
    }

    #[test]
    fn given_a_command_start_on_the_prompt_row_then_the_prompt_is_kept() {
        let mut marks = PromptMarks::default();
        marks.record(0, Event::PromptStart);
        marks.record(0, Event::CommandStart);
        let all: Vec<_> = marks.all().copied().collect();
        assert_eq!(all[0].kind, PromptKind::Prompt);
    }

    #[test]
    fn given_a_scrolled_prompt_then_pruning_drops_marks_past_the_scrollback() {
        let mut marks = PromptMarks::default();
        marks.record(10, Event::PromptStart);
        marks.record(200, Event::PromptStart);
        marks.prune(250, 100);
        let lines: Vec<_> = marks.all().map(|mark| mark.line).collect();
        assert_eq!(lines, vec![200]);
    }

    #[test]
    fn given_a_timed_command_then_the_next_prompt_carries_its_duration() {
        let mut marks = PromptMarks::default();
        marks.record(0, Event::PromptStart);
        marks.record(1, Event::CommandStart);
        marks.record(
            1,
            Event::CommandEnd(CommandReport {
                status: Some(0),
                duration_ms: Some(900),
            }),
        );
        marks.record(2, Event::PromptStart);
        let all: Vec<_> = marks.all().copied().collect();
        assert_eq!(all[2].status, Some(0));
        assert_eq!(all[2].duration_ms, Some(900));
        // A command start has no timing of its own.
        assert_eq!(all[1].duration_ms, None);
    }

    #[test]
    fn given_a_visible_window_then_only_its_marks_are_returned() {
        let mut marks = PromptMarks::default();
        for line in [0, 5, 9, 10, 20] {
            marks.record(line, Event::PromptStart);
        }
        let visible: Vec<_> = marks.visible(5, 5).iter().map(|mark| mark.line).collect();
        assert_eq!(visible, vec![5, 9]);
        assert_eq!(marks.prompt_lines(), vec![0, 5, 9, 10, 20]);
    }
}
