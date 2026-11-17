//! Local IPC transport shared by every Berth client and shell.
//!
//! One API hides the platform split: Unix domain sockets on Unix, named pipes
//! on Windows. The wire format is unchanged NDJSON (docs/03-protocol.md), so a
//! Linux shell, a Windows shell and the macOS app all speak the same protocol.
//!
//! Key exports: [`connect`], [`bind`], [`LocalStream`], [`Framer`].

use std::io;
use std::path::Path;

use interprocess::local_socket::ListenerOptions;
use interprocess::local_socket::traits::{Listener as _, ListenerExt, Stream as _};
#[cfg(unix)]
use interprocess::local_socket::{GenericFilePath, ToFsName};
#[cfg(windows)]
use interprocess::local_socket::{GenericNamespaced, ToNsName};

pub use interprocess::local_socket::{Listener, Name, Stream as LocalStream};

/// Default cap for one NDJSON request line. Matches the Swift server and
/// `docs/03-protocol.md` (8 MiB).
pub const MAX_LINE_BYTES: usize = 8 << 20;

/// Convert a path (or, on Windows, a pipe name like `\\.\pipe\berth`) into a
/// transport name.
pub fn name_for(path: &Path) -> io::Result<Name<'static>> {
    #[cfg(unix)]
    {
        path.to_path_buf().to_fs_name::<GenericFilePath>()
    }
    #[cfg(windows)]
    {
        let raw = path.to_string_lossy();
        let bare = raw
            .strip_prefix(r"\\.\pipe\")
            .or_else(|| raw.strip_prefix(r"\\.\PIPE\"))
            .unwrap_or(&raw);
        bare.to_string().to_ns_name::<GenericNamespaced>()
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Berth transport is not implemented on this platform",
        ))
    }
}

/// Connect as a client.
pub fn connect(path: &Path) -> io::Result<LocalStream> {
    LocalStream::connect(name_for(path)?)
}

/// Clone a stream so one task can read while another writes.
pub fn try_clone(stream: &LocalStream) -> io::Result<LocalStream> {
    use interprocess::TryClone as _;
    stream.try_clone()
}

/// Bind a server endpoint. On Unix the parent directory is created, a stale
/// socket file is removed, and the socket is `0600`.
pub fn bind(path: &Path) -> io::Result<Listener> {
    #[cfg(unix)]
    {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        // A previous crash can leave the socket file behind; it is ours
        // because the name comes from the per-user data directory.
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }

    let options = ListenerOptions::new().name(name_for(path)?);
    let listener = options.create_sync()?;
    #[cfg(unix)]
    {
        // `.mode()` is not supported on macOS (it returns Unsupported), so the
        // socket is hardened right after bind, as the Swift server does.
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(listener)
}

/// Wait for one connection (blocking).
pub fn accept(listener: &Listener) -> io::Result<LocalStream> {
    listener.accept()
}

/// Iterator over incoming connections; see [`ListenerExt`].
pub fn incoming(listener: &Listener) -> impl Iterator<Item = io::Result<LocalStream>> + '_ {
    listener.incoming()
}

/// Result of feeding bytes to a [`Framer`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Framed {
    /// Complete lines, with the trailing `\n` stripped.
    pub lines: Vec<String>,
    /// A line exceeded [`MAX_LINE_BYTES`]; the connection is desynchronised
    /// and the server must answer `payload_too_large` and close.
    pub oversized: bool,
}

/// Incremental NDJSON framing.
#[derive(Debug)]
pub struct Framer {
    buffer: Vec<u8>,
    max_line_bytes: usize,
    discarding_oversized: bool,
}

impl Default for Framer {
    fn default() -> Self {
        Self::new(MAX_LINE_BYTES)
    }
}

impl Framer {
    pub fn new(max_line_bytes: usize) -> Self {
        Self {
            buffer: Vec::new(),
            max_line_bytes: max_line_bytes.max(1),
            discarding_oversized: false,
        }
    }

    /// Append bytes and return every complete line. Empty lines are ignored.
    pub fn append(&mut self, bytes: &[u8]) -> Framed {
        let mut out = Framed::default();

        if self.discarding_oversized {
            match bytes.iter().position(|b| *b == b'\n') {
                Some(index) => {
                    self.discarding_oversized = false;
                    self.buffer.clear();
                    self.consume(&bytes[index + 1..], &mut out);
                }
                None => return out,
            }
        } else {
            self.consume(bytes, &mut out);
        }
        out
    }

    fn consume(&mut self, bytes: &[u8], out: &mut Framed) {
        self.buffer.extend_from_slice(bytes);
        while let Some(index) = self.buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=index).collect();
            let line = &line[..line.len() - 1];
            if line.len() > self.max_line_bytes {
                out.oversized = true;
            } else if !line.is_empty() {
                // JSON is UTF-8; lossy conversion keeps the server answering a
                // canonical bad_request instead of dropping the connection.
                out.lines.push(String::from_utf8_lossy(line).into_owned());
            }
        }
        if self.buffer.len() > self.max_line_bytes {
            self.buffer.clear();
            self.discarding_oversized = true;
            out.oversized = true;
        }
    }

    /// Bytes that do not yet form a line.
    pub fn pending(&self) -> &[u8] {
        &self.buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_partial_and_multiple_lines() {
        let mut f = Framer::default();
        assert_eq!(f.append(b"{\"a\":1}").lines, Vec::<String>::new());
        assert_eq!(
            f.append(b"\n{\"b\":2}\n\n").lines,
            vec!["{\"a\":1}", "{\"b\":2}"]
        );
        assert!(f.pending().is_empty());
    }

    #[test]
    fn oversized_line_is_reported_and_discarded() {
        let mut f = Framer::new(8);
        let framed = f.append(b"{\"much\":\"too long\"}\nnext\n");
        assert!(framed.oversized);
        // The overlong line is rejected but framing resumes on the same chunk,
        // so a following complete line is still delivered.
        assert_eq!(framed.lines, vec!["next"]);
        assert_eq!(f.append(b"ok\n").lines, vec!["ok"]);
    }

    #[test]
    fn an_incomplete_oversized_line_is_dropped_until_newline() {
        let mut f = Framer::new(8);
        assert!(f.append(b"123456789").oversized);
        assert!(f.append(b"still the same line").lines.is_empty());
        assert_eq!(f.append(b"\nfine\n").lines, vec!["fine"]);
    }

    #[test]
    fn non_utf8_becomes_lossy_not_a_panic() {
        let mut f = Framer::default();
        let framed = f.append(b"\xff\xfe\n");
        assert_eq!(framed.lines.len(), 1);
        assert!(!framed.oversized);
    }
}
