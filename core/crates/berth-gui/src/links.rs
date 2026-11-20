//! Link detection for the portable window.
//!
//! Only the visible row is scanned: a URL or an absolute path under the pointer
//! opens with the platform handler when the user holds the primary modifier.

/// The link under `column` in one row of terminal text, as `(start, end, text)`
/// in character columns. Trailing punctuation is not part of the link.
pub fn link_at(row: &str, column: usize) -> Option<(usize, usize, String)> {
    let characters: Vec<char> = row.chars().collect();
    if column >= characters.len() {
        return None;
    }
    let mut start = 0;
    while start < characters.len() {
        while start < characters.len() && characters[start].is_whitespace() {
            start += 1;
        }
        let mut end = start;
        while end < characters.len() && !characters[end].is_whitespace() {
            end += 1;
        }
        if end > start && column >= start && column < end {
            let mut text: String = characters[start..end].iter().collect();
            while text
                .chars()
                .last()
                .is_some_and(|c| ".,;:!?)]}'\"".contains(c))
            {
                text.pop();
            }
            if is_link(&text) {
                let len = text.chars().count();
                return Some((start, start + len, text));
            }
            return None;
        }
        start = end;
    }
    None
}

fn is_link(text: &str) -> bool {
    text.starts_with("http://")
        || text.starts_with("https://")
        || text.starts_with("www.")
        || (text.starts_with('/') && text.len() > 1)
        || text.starts_with("~/")
}

/// The address to hand the platform opener. A bare `www.` host gets a scheme.
pub fn target(link: &str) -> String {
    if link.starts_with("www.") {
        format!("https://{link}")
    } else {
        link.to_string()
    }
}

/// Only these targets reach the platform opener. Hyperlinks come from programs
/// in the terminal, so a custom scheme must not be handed to LaunchServices or
/// `cmd start` just because a cell carries it.
pub fn is_supported_target(target: &str) -> bool {
    let lower = target.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with("ftp://")
        || lower.starts_with("ftps://")
        || target.starts_with('/')
        || target.starts_with("~/")
}

/// Open a link with the platform handler. Spawned, so a slow browser cannot
/// block the window.
pub fn open(link: &str) -> std::io::Result<()> {
    let target = target(link);
    if !is_supported_target(&target) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("refusing to open an unsupported link: {target}"),
        ));
    }
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(&target);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(&target);
        command
    };
    #[cfg(windows)]
    let mut command = {
        // explorer goes through ShellExecute and does not parse a command line,
        // so an `&` in a URL cannot chain a command the way `cmd /C start` can.
        let mut command = std::process::Command::new("explorer.exe");
        command.arg(&target);
        command
    };
    command.spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_a_url_in_a_row_when_the_pointer_is_on_it_then_the_url_is_returned() {
        let row = "see https://example.com/a?b=1 for details";
        let (start, end, link) = link_at(row, 10).unwrap();
        assert_eq!(link, "https://example.com/a?b=1");
        assert_eq!(&row[start..end], link);
    }

    #[test]
    fn given_trailing_punctuation_when_matched_then_it_is_not_part_of_the_link() {
        let row = "open https://example.com.";
        assert_eq!(link_at(row, 8).unwrap().2, "https://example.com");
    }

    #[test]
    fn given_an_absolute_path_when_the_pointer_is_on_it_then_it_is_a_link() {
        let row = "error at /Users/me/project/src/main.rs:42";
        assert_eq!(
            link_at(row, 12).unwrap().2,
            "/Users/me/project/src/main.rs:42"
        );
        assert_eq!(link_at("cd ~/projects", 5).unwrap().2, "~/projects");
    }

    #[test]
    fn given_plain_text_when_scanned_then_there_is_no_link() {
        assert!(link_at("just some words here", 5).is_none());
        assert!(link_at("cargo test --workspace", 8).is_none());
        assert!(link_at("", 0).is_none());
        assert!(link_at("word", 99).is_none());
    }

    #[test]
    fn given_a_bare_host_when_targeted_then_it_gains_a_scheme() {
        assert_eq!(target("www.example.com"), "https://www.example.com");
        assert_eq!(target("https://x.dev"), "https://x.dev");
    }

    #[test]
    fn given_an_unsupported_scheme_then_it_is_refused() {
        assert!(is_supported_target("https://x.dev"));
        assert!(is_supported_target("mailto:a@b"));
        assert!(is_supported_target("ftps://x"));
        assert!(is_supported_target("/tmp/file"));
        assert!(is_supported_target("~/file"));
        assert!(!is_supported_target("ftp:anything"));
        assert!(!is_supported_target("ftps:anything"));
        assert!(!is_supported_target("file:///etc/passwd"));
        assert!(!is_supported_target("javascript:alert(1)"));
        assert!(!is_supported_target("custom:thing"));
    }
}
