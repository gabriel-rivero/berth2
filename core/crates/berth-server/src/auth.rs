//! Socket authentication policy and token storage (docs/03-protocol.md §2).
//!
//! Modes: `off`, `local` (token required for mutating methods) and `trusted`
//! (same-user only, no token). This module is transport-level and testable
//! without a running shell.

use std::io;
use std::path::Path;

/// Socket authentication mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketMode {
    Off,
    Local,
    Trusted,
}

impl SocketMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Local => "local",
            Self::Trusted => "trusted",
        }
    }
}

/// Methods that never require a token in `local` mode. Reading terminal
/// contents is deliberately absent: screen text is sensitive.
pub fn is_read_only(method: &str) -> bool {
    if method.starts_with("system.") {
        return true;
    }
    matches!(
        method,
        "workspace.list"
            | "workspace.get"
            | "workspace.layout"
            | "group.list"
            | "pane.list"
            | "surface.list"
            | "agent.get"
            | "agent.list"
            | "notification.list"
            | "events.subscribe"
            | "events.unsubscribe"
            | "session.dump"
    )
}

pub fn requires_token(method: &str) -> bool {
    !is_read_only(method)
}

/// Only `local` mode authenticates mutations. `trusted` relies on the socket
/// being private to the user; `off` never reaches this code.
pub fn authorized(
    method: &str,
    presented: Option<&str>,
    token: Option<&str>,
    mode: SocketMode,
) -> bool {
    match mode {
        SocketMode::Local => {
            if !requires_token(method) {
                return true;
            }
            match (token, presented) {
                (Some(expected), Some(given)) => constant_time_eq(expected, given),
                _ => false,
            }
        }
        SocketMode::Trusted => true,
        SocketMode::Off => false,
    }
}

pub fn constant_time_eq(left: &str, right: &str) -> bool {
    let (a, b) = (left.as_bytes(), right.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for (x, y) in a.iter().zip(b) {
        difference |= x ^ y;
    }
    difference == 0
}

/// Read an existing token, creating one when absent. The file is `0600` on
/// Unix; on Windows it inherits the user-scoped ACL of `%APPDATA%`.
pub fn load_or_create_token(path: &Path) -> io::Result<String> {
    if let Some(existing) = read_token(path) {
        make_private(path);
        return Ok(existing);
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|e| io::Error::other(format!("cannot read random bytes: {e}")))?;
    let token = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    write_atomic(path, &token)?;
    make_private(path);
    Ok(token)
}

pub fn read_token(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    let temporary = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, path)
}

fn make_private(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_methods_need_no_token() {
        assert!(is_read_only("workspace.list"));
        assert!(is_read_only("system.ping"));
        assert!(!is_read_only("workspace.create"));
        assert!(requires_token("surface.send_text"));
        assert!(requires_token("session.save"));
    }

    #[test]
    fn local_mode_requires_the_token_only_for_mutations() {
        let mode = SocketMode::Local;
        assert!(authorized("workspace.list", None, Some("abc"), mode));
        assert!(!authorized("workspace.create", None, Some("abc"), mode));
        assert!(!authorized(
            "workspace.create",
            Some("wrong"),
            Some("abc"),
            mode
        ));
        assert!(authorized(
            "workspace.create",
            Some("abc"),
            Some("abc"),
            mode
        ));
    }

    #[test]
    fn trusted_mode_bypasses_the_token_but_off_denies_everything() {
        assert!(authorized(
            "workspace.create",
            None,
            None,
            SocketMode::Trusted
        ));
        assert!(!authorized("workspace.list", None, None, SocketMode::Off));
    }

    #[test]
    fn comparison_is_length_aware_and_correct() {
        assert!(constant_time_eq("secret", "secret"));
        assert!(!constant_time_eq("secret", "secreT"));
        assert!(!constant_time_eq("secret", "secret!"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn token_is_persisted_and_reused() {
        let dir = std::env::temp_dir().join(format!("berth-token-{}", std::process::id()));
        let path = dir.join("token");
        let _ = std::fs::remove_dir_all(&dir);
        let first = load_or_create_token(&path).unwrap();
        let second = load_or_create_token(&path).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
