//! Desktop notifications for terminal and agent events.
//!
//! Each platform has its own one-liner; the window shells out so a missing or
//! slow notification daemon can never block a frame.

/// Show a desktop notification. Errors are returned for logging, not retried.
pub fn notify(title: &str, body: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification {} with title {}",
            apple_script_string(body),
            apple_script_string(title)
        );
        std::process::Command::new("osascript")
            .args(["-e", &script])
            .spawn()
            .map(|_| ())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("notify-send")
            .args([title, body])
            .spawn()
            .map(|_| ())
    }
    #[cfg(windows)]
    {
        // No bundled toast dependency: the attention panel is the Windows
        // surface for now.
        let _ = (title, body);
        Ok(())
    }
}

/// Quote a string for AppleScript, escaping what would end it early.
pub fn apple_script_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_quotes_and_newlines_when_quoted_then_they_are_escaped() {
        assert_eq!(apple_script_string("hi"), "\"hi\"");
        assert_eq!(
            apple_script_string("say \"hi\"\\\n"),
            "\"say \\\"hi\\\"\\\\\\n\""
        );
    }
}
