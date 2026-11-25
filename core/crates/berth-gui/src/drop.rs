//! File drops: typing the dropped paths into the pane.
//!
//! The terminal cannot hand a file to a program, so a drop types the path the
//! way the user would: quoted, space-separated, with a trailing space so the
//! next thing typed continues the command. A directory drops the same way.
//!
//! Quoting is per platform, because the shell that receives it is not always
//! POSIX: a Windows pane usually runs `cmd` or PowerShell, where a single quote
//! is not a quote at all. The rule is chosen from the target shell rather than
//! the host OS where it can be, since a WSL pane on Windows is a POSIX shell.
//!
//! **Shell quoting is not enough on its own.** The text goes to the terminal's
//! line editor before any shell sees it, and the line editor acts on control
//! characters: a filename containing `^U` kills the line (opening quote
//! included), `DEL` erases backwards, and `ESC [ A` recalls a previous command
//! into the buffer. A path is not under this code's control — it is whatever
//! the file is called — so [`drop_text`] refuses to emit a control character at
//! all, and the caller wraps the result in a bracketed paste when the program
//! asked for one, which makes the line editor insert the bytes literally.

/// How a dropped path is quoted for the shell that will read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    /// POSIX shells: `sh`, `bash`, `zsh`, `fish`.
    Posix,
    /// `cmd.exe`: double quotes, no escaping of `%`.
    Cmd,
    /// PowerShell: double quotes, with a backtick escape and `$` doubled.
    PowerShell,
}

impl Syntax {
    /// The syntax a program name implies. Unknown programs fall back to the
    /// host's default, so a drop is never silently unquoted.
    pub fn for_program(program: &str, host_is_windows: bool) -> Self {
        let name = program
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(program)
            .to_ascii_lowercase();
        let name = name.strip_suffix(".exe").unwrap_or(&name);
        match name {
            "sh" | "bash" | "zsh" | "fish" | "dash" | "ksh" | "nu" | "wsl" => Syntax::Posix,
            "cmd" => Syntax::Cmd,
            "powershell" | "pwsh" => Syntax::PowerShell,
            _ if host_is_windows => Syntax::Cmd,
            _ => Syntax::Posix,
        }
    }
}

/// The text a drop types: every path quoted, space-separated, one trailing
/// space. An empty drop, or one whose paths cannot be typed safely, is nothing.
///
/// A path containing a control character (anything below `0x20`, `DEL`, or a
/// C1 control) is **skipped**: the closest safe rendering would be a lie about
/// which file was dropped, and the line editor would act on the raw byte.
pub fn drop_text(paths: &[String], syntax: Syntax) -> String {
    let safe: Vec<String> = paths
        .iter()
        .filter(|path| is_typable(path))
        .map(|path| quote(path, syntax))
        .collect();
    if safe.is_empty() {
        return String::new();
    }
    let mut text = safe.join(" ");
    text.push(' ');
    text
}

/// Whether a path can be typed into a terminal at all. Control characters are
/// interpreted by the line editor before the shell parses the quotes, so they
/// are refused rather than escaped: there is no quoting that survives `^U`.
pub fn is_typable(path: &str) -> bool {
    !path.chars().any(|ch| ch.is_control() && ch != '\t')
}

/// Quote one path for the target shell.
pub fn quote(path: &str, syntax: Syntax) -> String {
    match syntax {
        Syntax::Posix => quote_posix(path),
        Syntax::Cmd => quote_windows(path, false),
        Syntax::PowerShell => quote_windows(path, true),
    }
}

/// POSIX: single quotes, with an embedded quote closed and re-opened. Paths of
/// only safe characters stay bare so a drop reads like something typed, except
/// a leading `-` (a program would read it as a flag) and a leading `=` (zsh
/// expands `=cmd` to a path).
fn quote_posix(path: &str) -> String {
    let plain = !path.is_empty()
        && path.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'_' | b'-' | b'.' | b'/' | b':' | b'@' | b'+' | b'=' | b','
                )
        });
    if plain && !path.starts_with('-') && !path.starts_with('=') {
        return path.to_string();
    }
    let mut out = String::with_capacity(path.len() + 2);
    out.push('\'');
    for ch in path.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Windows quoting.
///
/// PowerShell gets a **single-quoted** string: it is fully literal, so a
/// backtick, `$`, `"` or `!` in the path needs no escape and only `'` doubles.
/// The double-quote form would need a backtick before each of those, and one
/// unescaped backtick at the end of the path would turn the closing quote into
/// a literal and make the rest of the line code.
///
/// `cmd` has no way to escape anything inside a double-quoted string, and it
/// expands `%VAR%` even inside quotes, so a path containing `"` or `%` cannot
/// be typed correctly; such a path is refused rather than emitted broken.
fn quote_windows(path: &str, powershell: bool) -> String {
    if powershell {
        if !needs_windows_quotes(path) && !path.contains('\'') {
            return path.to_string();
        }
        return format!("'{}'", path.replace('\'', "''"));
    }
    let needs_quotes = needs_windows_quotes(path);
    if !needs_quotes {
        return path.to_string();
    }
    let mut out = String::with_capacity(path.len() + 2);
    out.push('"');
    for ch in path.chars() {
        if ch == '"' {
            // Unrepresentable in cmd: drop the quote rather than close the
            // string and let the rest of the path run as a command.
            continue;
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// Whether a Windows path needs quoting at all: whitespace or a cmd
/// metacharacter. `%` and `` ` `` are included, since both expand.
fn needs_windows_quotes(path: &str) -> bool {
    path.is_empty()
        || path.chars().any(|ch| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '&' | '^' | '|' | '<' | '>' | '(' | ')' | '%' | '`' | '"'
                )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn given_a_plain_path_then_it_is_not_quoted() {
        assert_eq!(
            drop_text(&paths(&["/tmp/a.txt"]), Syntax::Posix),
            "/tmp/a.txt "
        );
    }

    #[test]
    fn given_a_path_with_a_space_then_it_is_quoted() {
        assert_eq!(
            drop_text(&paths(&["/tmp/my file.txt"]), Syntax::Posix),
            "'/tmp/my file.txt' "
        );
    }

    #[test]
    fn given_several_paths_then_they_are_space_separated_and_trailing_spaced() {
        let text = drop_text(&paths(&["/a", "/b c"]), Syntax::Posix);
        assert_eq!(text, "/a '/b c' ");
    }

    #[test]
    fn given_an_apostrophe_then_posix_quoting_survives_it() {
        assert_eq!(quote("it's", Syntax::Posix), "'it'\\''s'");
    }

    #[test]
    fn given_shell_metacharacters_then_posix_quotes_them() {
        for path in ["/tmp/a;b", "/tmp/$HOME", "/tmp/a*b", "/tmp/a`b`"] {
            let quoted = quote(path, Syntax::Posix);
            assert!(quoted.starts_with('\''), "{path} was not quoted: {quoted}");
            assert!(quoted.ends_with('\''), "{path} was not closed: {quoted}");
        }
    }

    #[test]
    fn given_a_control_character_then_the_path_is_refused() {
        // The line editor acts on these before the shell parses the quotes:
        // `^U` kills the line including the opening quote, `DEL` erases it, and
        // `ESC [ A` recalls a command into the buffer.
        for bad in [
            "/tmp/\x15echo INJECTED",
            "/tmp/\x17echo INJECTED",
            "/tmp/x\x7fecho INJECTED",
            "/tmp/a\x1b[A",
            "/tmp/a\nb",
            "/tmp/a\rb",
        ] {
            assert!(
                drop_text(&[bad.to_string()], Syntax::Posix).is_empty(),
                "{bad:?} was typed"
            );
            assert!(!is_typable(bad), "{bad:?} was called typable");
        }
        let text = drop_text(&paths(&["/tmp/a\nb", "/tmp/ok"]), Syntax::Posix);
        assert_eq!(
            text, "/tmp/ok ",
            "the bad path was not the only one dropped"
        );
    }

    #[test]
    fn given_a_leading_dash_then_the_path_is_quoted_so_it_is_not_a_flag() {
        // A file named `-rf` would otherwise reach the next program as a flag.
        assert_eq!(quote("-rf", Syntax::Posix), "'-rf'");
    }

    #[test]
    fn given_a_leading_equals_then_the_path_is_quoted_so_zsh_does_not_expand_it() {
        assert_eq!(quote("=ls", Syntax::Posix), "'=ls'");
    }

    #[test]
    fn given_cmd_then_a_space_is_double_quoted() {
        assert_eq!(
            quote(r"C:\Program Files\a.txt", Syntax::Cmd),
            r#""C:\Program Files\a.txt""#
        );
        assert_eq!(quote(r"C:\tmp\a.txt", Syntax::Cmd), r"C:\tmp\a.txt");
    }

    #[test]
    fn given_powershell_then_the_path_is_single_quoted_and_literal() {
        // A single-quoted string needs no escapes, so `$`, a backtick and `"`
        // all survive verbatim; only `'` doubles.
        assert_eq!(quote(r"C:\a$b c", Syntax::PowerShell), r"'C:\a$b c'");
        assert_eq!(quote(r"C:\a`b c", Syntax::PowerShell), r"'C:\a`b c'");
        assert_eq!(quote(r"C:\it's c", Syntax::PowerShell), r"'C:\it''s c'");
    }

    #[test]
    fn given_a_windows_path_with_percent_then_it_is_quoted_so_cmd_does_not_expand_it() {
        assert!(needs_windows_quotes(r"C:\a%b"));
        // cmd cannot escape a quote inside a path at all, so it is dropped
        // rather than closing the string and running the rest of the path.
        assert_eq!(quote(r#"C:\a"b"#, Syntax::Cmd), r#""C:\ab""#);
    }

    #[test]
    fn given_a_program_then_the_syntax_follows_it() {
        assert_eq!(Syntax::for_program("/bin/zsh", false), Syntax::Posix);
        assert_eq!(Syntax::for_program("wsl.exe", true), Syntax::Posix);
        assert_eq!(Syntax::for_program("pwsh", false), Syntax::PowerShell);
        assert_eq!(
            Syntax::for_program(r"C:\Windows\cmd.exe", true),
            Syntax::Cmd
        );
        // An unknown program falls back to the host, never to unquoted text.
        assert_eq!(Syntax::for_program("mystery", true), Syntax::Cmd);
        assert_eq!(Syntax::for_program("mystery", false), Syntax::Posix);
    }

    #[test]
    fn given_no_paths_then_nothing_is_typed() {
        assert!(drop_text(&[], Syntax::Posix).is_empty());
    }
}
