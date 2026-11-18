//! Platform paths for Berth's runtime files.
//!
//! One place decides where the socket, token, session snapshot and logs live,
//! so the CLI, the server and every shell agree on every OS:
//!
//! | | macOS | Linux/BSD | Windows |
//! | --- | --- | --- | --- |
//! | data | `~/Library/Application Support/Berth` | `$XDG_DATA_HOME/berth` or `~/.local/share/berth` | `%APPDATA%\\Berth` |
//! | socket | `<data>/berth.sock` | `$XDG_RUNTIME_DIR/berth/berth.sock` or `<data>/berth.sock` | `\\.\pipe\berth-<user>` |
//! | token | `<data>/token` | `<data>/token` | `<data>\\token` |
//! | logs | `~/Library/Logs/Berth` | `$XDG_STATE_HOME/berth/logs` | `%LOCALAPPDATA%\\Berth\\Logs` |
//! | config | `~/.config/berth/config.json` | `$XDG_CONFIG_HOME/berth/config.json` | `%USERPROFILE%\\.config\\berth\\config.json` |
//!
//! `BERTH_SOCKET_PATH` overrides the socket; `BERTH_TOKEN_FILE` overrides the
//! token file. The macOS layout is unchanged so existing installs keep working.

use std::path::{Path, PathBuf};

/// The user's home directory, independent of the platform naming (`HOME` vs
/// `USERPROFILE`).
pub fn home_dir() -> Option<PathBuf> {
    if let Some(home) = env_path("HOME") {
        return Some(home);
    }
    if let Some(profile) = env_path("USERPROFILE") {
        return Some(profile);
    }
    // Windows permits HOME to be unset; HOMEDRIVE + HOMEPATH is the fallback.
    match (env_string("HOMEDRIVE"), env_string("HOMEPATH")) {
        (Some(drive), Some(path)) => Some(PathBuf::from(format!("{drive}{path}"))),
        _ => None,
    }
}

/// Root directory for persistent Berth state (session snapshot, token, logs
/// on platforms where there is no separate log location).
pub fn data_dir() -> Option<PathBuf> {
    if let Some(dir) = env_path("BERTH_DATA_DIR") {
        return Some(dir);
    }
    #[cfg(target_os = "macos")]
    {
        home_dir().map(|home| home.join("Library/Application Support/Berth"))
    }
    #[cfg(windows)]
    {
        env_path("APPDATA")
            .or_else(|| home_dir().map(|home| home.join("AppData/Roaming")))
            .map(|dir| dir.join("Berth"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        env_path("XDG_DATA_HOME")
            .or_else(|| home_dir().map(|home| home.join(".local/share")))
            .map(|dir| dir.join("berth"))
    }
    #[cfg(not(any(unix, windows)))]
    {
        home_dir().map(|home| home.join(".berth"))
    }
}

/// Directory for logs and crash diagnostics.
pub fn log_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        home_dir().map(|home| home.join("Library/Logs/Berth"))
    }
    #[cfg(windows)]
    {
        env_path("LOCALAPPDATA")
            .or_else(|| home_dir().map(|home| home.join("AppData/Local")))
            .map(|dir| dir.join("Berth").join("Logs"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        env_path("XDG_STATE_HOME")
            .or_else(|| home_dir().map(|home| home.join(".local/state")))
            .map(|dir| dir.join("berth").join("logs"))
    }
    #[cfg(not(any(unix, windows)))]
    {
        data_dir().map(|dir| dir.join("logs"))
    }
}

/// Local IPC endpoint. On Unix this is a filesystem socket; on Windows it is a
/// named pipe name. `BERTH_SOCKET_PATH` wins everywhere.
pub fn socket_path() -> PathBuf {
    if let Some(path) = env_path("BERTH_SOCKET_PATH") {
        return path;
    }
    default_socket_path()
}

fn default_socket_path() -> PathBuf {
    #[cfg(windows)]
    {
        let user = env_string("USERNAME").unwrap_or_else(|| "default".into());
        let user: String = user
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        return PathBuf::from(format!(r"\\.\pipe\berth-{user}"));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(runtime) = env_path("XDG_RUNTIME_DIR") {
            return runtime.join("berth").join("berth.sock");
        }
    }
    data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("berth.sock")
}

/// Shared secret for `local` socket mode.
pub fn token_path() -> PathBuf {
    if let Some(path) = env_path("BERTH_TOKEN_FILE") {
        return path;
    }
    data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("token")
}

/// User configuration file. `BERTH_CONFIG` overrides it for tests and
/// throwaway instances.
pub fn config_path() -> PathBuf {
    if let Some(path) = env_path("BERTH_CONFIG") {
        return path;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(config) = env_path("XDG_CONFIG_HOME") {
            return config.join("berth").join("config.json");
        }
    }
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("berth")
        .join("config.json")
}

/// Documented default for the session snapshot. The shell may supply its own.
pub fn snapshot_path() -> PathBuf {
    data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("session.json")
}

/// Render a path for display in protocol responses (`~` is expanded on Unix
/// only, so keep it absolute and deterministic).
pub fn display(path: &Path) -> String {
    path.display().to_string()
}

fn env_string(key: &str) -> Option<String> {
    std::env::var_os(key)
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.is_empty())
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .map(PathBuf::from)
        .filter(|value| !value.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_has_a_platform_appropriate_shape() {
        let Some(dir) = data_dir() else {
            return; // no home directory in this environment
        };
        let text = dir.to_string_lossy();
        #[cfg(target_os = "macos")]
        assert!(text.ends_with("Library/Application Support/Berth"));
        #[cfg(all(unix, not(target_os = "macos")))]
        assert!(text.ends_with("/berth") || text.ends_with("/Berth"));
        #[cfg(windows)]
        assert!(text.ends_with("Berth"));
    }

    #[test]
    fn snapshot_and_token_live_beside_the_data() {
        let Some(data) = data_dir() else { return };
        if std::env::var_os("BERTH_TOKEN_FILE").is_none() {
            assert_eq!(token_path(), data.join("token"));
        }
        assert_eq!(snapshot_path(), data.join("session.json"));
    }

    #[test]
    fn socket_names_are_never_empty() {
        assert!(!socket_path().as_os_str().is_empty());
    }

    #[test]
    fn display_is_lossless_for_unicode() {
        let path = PathBuf::from("café/ü");
        assert_eq!(display(&path), path.display().to_string());
    }
}
