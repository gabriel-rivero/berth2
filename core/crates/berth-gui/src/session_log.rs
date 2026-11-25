//! Per-pane session logging.
//!
//! The log is the pane's plain text, appended as the pane produces output, to
//! one file per surface under the log directory. It is opt-in per session
//! (`sessionLog`), because a log is exactly the thing that outlives the
//! terminal: a session's output can contain a token, and writing it by default
//! would put it where a clipboard never does.
//!
//! The writer is a thin wrapper over a file handle. It appends the text the
//! headless engine already produces for a pane's screen, so it needs no second
//! parser and cannot drift from what is displayed.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// How large one pane's log grows before it is rotated once. Matches the app
/// log's one-step rotation: enough to be useful, bounded so a long agent run
/// cannot fill a disk.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// One surface's log file.
struct PaneLog {
    path: PathBuf,
    file: File,
    written: u64,
}

/// Every open pane log for this session.
#[derive(Default)]
pub struct SessionLog {
    logs: HashMap<String, PaneLog>,
    directory: Option<PathBuf>,
}

impl SessionLog {
    /// A log set that writes nothing. Used when `sessionLog` is off.
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Start logging under `directory`. The directory is created lazily on the
    /// first surface, so a session that never opens a pane writes nothing.
    pub fn new(directory: PathBuf) -> Self {
        Self {
            logs: HashMap::new(),
            directory: Some(directory),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.directory.is_some()
    }

    /// The file a surface's log goes to, whether or not it is open yet.
    pub fn path_for(&self, surface_id: &str) -> Option<PathBuf> {
        self.directory
            .as_ref()
            .map(|directory| directory.join(format!("{}.log", safe_name(surface_id))))
    }

    /// Append text to a surface's log. Output that arrives before the file can
    /// be opened is dropped rather than buffered: a log is a convenience, and
    /// buffering it in memory would undo the point of writing it out.
    pub fn append(&mut self, surface_id: &str, text: &str) {
        if text.is_empty() {
            return;
        }
        let Some(path) = self.path_for(surface_id) else {
            return;
        };
        if !self.logs.contains_key(surface_id) {
            let Some(log) = open(&path) else {
                return;
            };
            self.logs.insert(surface_id.to_string(), log);
        }
        let Some(log) = self.logs.get_mut(surface_id) else {
            return;
        };
        let bytes = text.as_bytes();
        if log.written.saturating_add(bytes.len() as u64) > MAX_BYTES {
            // One rotation step: move the current file aside and start fresh,
            // so the newest output is never the part that is lost.
            if let Some(log) = self.logs.get_mut(surface_id) {
                rotate(log);
            }
        }
        let Some(log) = self.logs.get_mut(surface_id) else {
            return;
        };
        if log.file.write_all(bytes).is_ok() {
            log.written = log.written.saturating_add(bytes.len() as u64);
        }
    }

    /// Close a surface's log, as its pane is closed or it exits.
    pub fn close(&mut self, surface_id: &str) {
        self.logs.remove(surface_id);
    }

    /// Close every log. Called on exit so buffered writes reach the disk.
    pub fn close_all(&mut self) {
        for (_, log) in self.logs.drain() {
            let _ = log.file.sync_all();
        }
    }

    /// The open log paths. Tests use it; nothing else needs it.
    #[cfg(test)]
    pub fn open_paths(&self) -> Vec<&Path> {
        self.logs.values().map(|log| log.path.as_path()).collect()
    }
}

/// Open (or create) a log file in append mode.
fn open(path: &Path) -> Option<PaneLog> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()?;
    let written = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    Some(PaneLog {
        path: path.to_path_buf(),
        file,
        written,
    })
}

/// Move the current file to `<name>.1` and reopen a fresh one.
fn rotate(log: &mut PaneLog) {
    let _ = log.file.sync_all();
    let rotated = log.path.with_extension("log.1");
    let _ = std::fs::rename(&log.path, &rotated);
    if let Some(fresh) = open(&log.path) {
        *log = fresh;
    }
}

/// A surface id is safe in a filename already (`srf_` plus hex), but a shell
/// can be given any id, so anything outside the safe set is replaced.
fn safe_name(surface_id: &str) -> String {
    surface_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("berth-log-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn given_logging_then_output_lands_in_the_pane_file() {
        let dir = temp_dir("append");
        let mut log = SessionLog::new(dir.clone());
        log.append("srf_a", "hello\n");
        log.append("srf_a", "world\n");
        log.close_all();
        let path = dir.join("srf_a.log");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello\nworld\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn given_two_panes_then_each_gets_its_own_file() {
        let dir = temp_dir("two");
        let mut log = SessionLog::new(dir.clone());
        log.append("srf_a", "a\n");
        log.append("srf_b", "b\n");
        log.close_all();
        assert_eq!(
            std::fs::read_to_string(dir.join("srf_a.log")).unwrap(),
            "a\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("srf_b.log")).unwrap(),
            "b\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn given_logging_disabled_then_nothing_is_written() {
        let mut log = SessionLog::disabled();
        assert!(!log.is_enabled());
        log.append("srf_a", "hello");
        assert!(log.open_paths().is_empty());
        assert!(log.path_for("srf_a").is_none());
    }

    #[test]
    fn given_an_unsafe_surface_id_then_the_filename_is_sanitised() {
        let log = SessionLog::new(PathBuf::from("/tmp/logs"));
        let path = log.path_for("../../etc/passwd").unwrap();
        // `../` and `/` are replaced, so the id cannot escape the directory.
        let name = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(name, "______etc_passwd.log");
        assert_eq!(path.parent(), Some(Path::new("/tmp/logs")));
    }

    #[test]
    fn given_the_size_cap_then_the_file_rotates_once() {
        let dir = temp_dir("rotate");
        let mut log = SessionLog::new(dir.clone());
        let chunk = "x".repeat(64 * 1024);
        // Write past the cap.
        for _ in 0..((MAX_BYTES / chunk.len() as u64) + 2) {
            log.append("srf_a", &chunk);
        }
        log.close_all();
        assert!(dir.join("srf_a.log").exists(), "the fresh log is missing");
        assert!(dir.join("srf_a.log.1").exists(), "nothing rotated");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn given_a_closed_pane_then_the_log_is_dropped() {
        let dir = temp_dir("close");
        let mut log = SessionLog::new(dir.clone());
        log.append("srf_a", "one\n");
        log.close("srf_a");
        assert!(log.open_paths().is_empty());
        // A later append reopens it rather than losing the output.
        log.append("srf_a", "two\n");
        log.close_all();
        assert_eq!(
            std::fs::read_to_string(dir.join("srf_a.log")).unwrap(),
            "one\ntwo\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
