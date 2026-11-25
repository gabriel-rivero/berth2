//! Scrollback restore: keeping the last screen of a pane across a relaunch.
//!
//! A relaunch respawns every pane, so without this the terminal comes back with
//! its cwd but an empty screen. Replaying the saved text into the new pane
//! gives the user the screen they left, and a program that redraws (a shell
//! printing a prompt) pushes it up into scrollback where it belongs.
//!
//! Two rules keep it honest:
//!
//! - **Opt-in and capped.** The text is a copy of what a program printed, which
//!   can include a token; and a session file that grows without bound is worse
//!   than one that forgets. The cap is in bytes.
//! - **Plain text only.** Only the text survives, not colours or attributes. A
//!   replay is a fresh write to a new terminal, so anything richer would have
//!   to be re-encoded as escape sequences that might not mean the same thing to
//!   the new program.

/// How much of a pane's screen is kept, in bytes. Enough for a full screen of
/// dense output, small enough that a workspace of panes stays a small file.
pub const MAX_BYTES: usize = 64 * 1024;

/// Trim `text` to at most `max_bytes`, cutting on a character boundary and
/// keeping the **end**: the last screen of a pane is the part worth restoring.
pub fn tail(text: &str, max_bytes: usize) -> String {
    if max_bytes == 0 || text.len() <= max_bytes {
        return text.to_string();
    }
    let mut start = text.len() - max_bytes;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

/// The text to persist for a pane: its visible screen as plain lines, trimmed
/// to the byte cap, with trailing blank lines dropped so an empty pane stores
/// nothing at all.
///
/// Each line ends `\r\n`, not `\n`. The replay is written straight into the
/// engine, so it never passes through the kernel's `ONLCR` translation, and
/// `vt100` treats a bare line feed as a row move with no carriage return: a
/// bare-`\n` screen comes back as a staircase. `reflow` does the same
/// translation for the same reason.
pub fn capture(screen: &str, max_bytes: usize) -> String {
    let mut text = tail(screen, max_bytes);
    while text.ends_with('\n') {
        text.pop();
    }
    while text.ends_with('\r') {
        text.pop();
    }
    if text.trim().is_empty() {
        return String::new();
    }
    // A trailing break leaves the cursor at the start of a fresh line, so the
    // shell's first prompt draws below the restored screen instead of over its
    // last line.
    // Normalise first: a screen that arrived as CRLF must not get a second
    // `\r`, which the engine would draw as a stray carriage return.
    let text = text.replace("\r\n", "\n");
    format!("{}\r\n", text.replace('\n', "\r\n"))
}

/// Whether a pane's screen is worth keeping. A full-screen program (an editor,
/// a pager) redraws on SIGWINCH, so its saved screen is stale the moment it
/// starts and replaying it would flash over the real one.
pub fn should_replay(alternate_screen: bool) -> bool {
    !alternate_screen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_text_under_the_cap_then_it_is_kept_whole() {
        assert_eq!(tail("hello", 100), "hello");
    }

    #[test]
    fn given_text_over_the_cap_then_the_end_is_kept() {
        let text = format!("{}tail", "x".repeat(100));
        let kept = tail(&text, 10);
        assert_eq!(kept.len(), 10);
        assert!(kept.ends_with("tail"));
    }

    #[test]
    fn given_a_cut_inside_a_character_then_the_boundary_is_respected() {
        // Three-byte characters; a naive byte slice would panic.
        let text = "日".repeat(10);
        let kept = tail(&text, 7);
        assert!(text.ends_with(&kept));
        assert!(kept.chars().all(|ch| ch == '日'));
    }

    #[test]
    fn given_a_zero_cap_then_the_cap_is_off_and_everything_is_kept() {
        // Zero means unlimited, matching `read_text`'s `max_bytes`.
        assert_eq!(tail("hello", 0), "hello");
    }

    #[test]
    fn given_a_screen_then_capture_drops_blank_tail_lines() {
        let screen = "one\ntwo\n\n\n";
        assert_eq!(capture(screen, MAX_BYTES), "one\r\ntwo\r\n");
    }

    #[test]
    fn given_a_multiline_screen_then_every_break_is_a_carriage_return_feed() {
        // A bare `\n` replay staircases: the engine moves the row but not the
        // column, so the second line would start where the first ended.
        let captured = capture("first\nsecond", MAX_BYTES);
        assert_eq!(captured, "first\r\nsecond\r\n");
        assert!(!captured.contains("\n\n"));
    }

    #[test]
    fn given_a_screen_that_already_has_crlf_then_it_is_not_doubled() {
        let captured = capture("one\r\ntwo\r\n", MAX_BYTES);
        assert_eq!(captured, "one\r\ntwo\r\n");
    }

    #[test]
    fn given_a_blank_screen_then_capture_stores_nothing() {
        assert!(capture("   \n\n", MAX_BYTES).is_empty());
        assert!(capture("", MAX_BYTES).is_empty());
    }

    #[test]
    fn given_a_captured_screen_then_it_ends_with_a_break() {
        // So the shell's first prompt draws below the restored text rather than
        // overwriting its last line.
        assert!(capture("last line", MAX_BYTES).ends_with("\r\n"));
    }

    #[test]
    fn given_an_alternate_screen_program_then_its_screen_is_not_replayed() {
        // A full-screen program redraws on SIGWINCH, so its saved screen is
        // stale the moment it starts.
        assert!(!should_replay(true));
        assert!(should_replay(false));
    }
}
