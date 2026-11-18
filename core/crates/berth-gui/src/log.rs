//! Rolling breadcrumbs for the portable window.
//!
//! `berth.log` lives beside the macOS app's log. It answers the one question a
//! GUI cannot: what did the window do before it died.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Rotate once past this size, keeping `berth.log.1`.
const MAX_BYTES: u64 = 1 << 20;

static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// Open (or create) the log, rotating a full file first.
pub fn init() {
    let Some(dir) = berth_core::paths::log_dir() else {
        return;
    };
    let path = dir.join("berth.log");
    if let Err(error) = open(&path) {
        eprintln!("berth-gui: log {}: {error}", path.display());
        return;
    }
    line(&format!(
        "berth-gui {} start socket={}",
        env!("CARGO_PKG_VERSION"),
        berth_core::paths::socket_path().display()
    ));
}

fn open(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let size = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    if size > MAX_BYTES {
        let rotated = rotated_path(path);
        let _ = std::fs::rename(path, rotated);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    *LOG.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(file);
    Ok(())
}

fn rotated_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".1");
    PathBuf::from(name)
}

/// Append one breadcrumb. Never fails loudly: a broken log must not break the
/// window.
pub fn line(message: &str) {
    let mut guard = LOG.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(file) = guard.as_mut() else {
        return;
    };
    let _ = writeln!(file, "{} {message}", berth_core::time::now_ms());
    let _ = file.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_a_full_log_when_opened_then_it_rotates_once() {
        let dir = std::env::temp_dir().join(format!("berth-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("berth.log");
        std::fs::write(&path, vec![b'x'; (MAX_BYTES + 1) as usize]).unwrap();
        open(&path).unwrap();
        assert!(path.exists());
        assert!(rotated_path(&path).exists());
        let rotated = std::fs::metadata(rotated_path(&path)).unwrap().len();
        assert_eq!(rotated, MAX_BYTES + 1);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
