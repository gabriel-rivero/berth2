//! Shell command construction.
//!
//! Ghostty's `command` field is a *string* it hands to `/bin/sh -c` (unless it
//! carries a `direct:` prefix, which splits naively on spaces). Berth receives
//! a proper `argv`, so the shell quoting has to happen here — one place,
//! tested, instead of string concatenation at the FFI boundary.

/// Join `argv` into a POSIX-shell-safe command string.
///
/// Every argument is single-quoted, so spaces, globs, `$`, backticks and
/// newlines survive intact. Empty arguments become `''`.
pub fn join_argv(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'_' | b'-' | b'.' | b'/' | b':' | b'@' | b'+' | b'=' | b','
                )
        })
    {
        // Keep the common case readable: `claude --resume abc`.
        return arg.to_string();
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('\'');
    for ch in arg.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn plain_arguments_stay_plain() {
        assert_eq!(
            join_argv(&argv(&["claude", "--resume", "abc-123"])),
            "claude --resume abc-123"
        );
    }

    #[test]
    fn spaces_and_globs_are_quoted() {
        assert_eq!(
            join_argv(&argv(&["echo", "hello world", "*.rs"])),
            "echo 'hello world' '*.rs'"
        );
    }

    #[test]
    fn single_quotes_are_escaped() {
        assert_eq!(join_argv(&argv(&["echo", "it's"])), r"echo 'it'\''s'");
    }

    #[test]
    fn empty_argument_survives() {
        assert_eq!(join_argv(&argv(&["printf", ""])), "printf ''");
    }

    #[test]
    fn shell_metacharacters_do_not_expand() {
        assert_eq!(
            join_argv(&argv(&["sh", "-c", "$HOME `id`"])),
            "sh -c '$HOME `id`'"
        );
    }
}
