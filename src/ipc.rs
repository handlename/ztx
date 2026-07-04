//! IPC channel (feature 4: pull editor selections into the session).
//!
//! Each wrapper listens on a Unix socket; `zediator send` (typically invoked
//! from a Zed task with `$ZED_RELATIVE_FILE` / `$ZED_ROW` /
//! `$ZED_SELECTED_TEXT`) connects and writes a message, which the wrapper
//! injects into the child's stdin as a bracketed paste.
//!
//! Discovery: sockets live in one per-user directory as `<pid>.sock`, and a
//! `latest.sock` symlink always points at the most recently started wrapper.
//! `--pid`/`--socket` select an explicit session; stale sockets are removed
//! opportunistically.

use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

/// Shared handle to the child's PTY writer, used by both the stdin pump and
/// the IPC accept loop.
pub type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

const LATEST_LINK: &str = "latest.sock";

/// Upper bound for one IPC message; protects the wrapper (and the session it
/// hosts) from a runaway or malicious client exhausting memory.
const MAX_MESSAGE_LEN: u64 = 1024 * 1024;

/// Per-user runtime directory holding the session sockets.
pub fn socket_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ZEDIATOR_RUNTIME_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir).join("zediator");
    }
    // macOS: $TMPDIR is per-user and mode 0700.
    std::env::temp_dir().join("zediator-run")
}

/// Server side, owned by the wrapper process. Cleans its socket up on drop.
pub struct IpcServer {
    socket_path: PathBuf,
}

impl IpcServer {
    /// Binds this wrapper's socket, repoints `latest.sock`, sweeps stale
    /// sockets, and spawns the accept loop feeding `writer`.
    pub fn start(writer: SharedWriter) -> io::Result<Self> {
        let dir = socket_dir();
        std::fs::create_dir_all(&dir)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        sweep_stale(&dir);

        let socket_path = dir.join(format!("{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path)?;

        let latest = dir.join(LATEST_LINK);
        let _ = std::fs::remove_file(&latest);
        let _ = std::os::unix::fs::symlink(&socket_path, &latest);

        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let mut message = Vec::new();
                if stream
                    .take(MAX_MESSAGE_LEN)
                    .read_to_end(&mut message)
                    .is_err()
                    || message.is_empty()
                {
                    continue;
                }
                let mut guard = writer.lock().expect("child writer poisoned");
                // Bracketed paste keeps multi-line text as one prompt insert.
                let injected = guard
                    .write_all(b"\x1b[200~")
                    .and_then(|()| guard.write_all(&message))
                    .and_then(|()| guard.write_all(b"\x1b[201~"))
                    .and_then(|()| guard.flush());
                match injected {
                    Ok(()) => tracing::info!(bytes = message.len(), "injected IPC message"),
                    Err(err) => tracing::warn!(error = %err, "failed to inject IPC message"),
                }
            }
        });

        Ok(Self { socket_path })
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
        let latest = self.socket_path.with_file_name(LATEST_LINK);
        if std::fs::read_link(&latest).is_ok_and(|target| target == self.socket_path) {
            let _ = std::fs::remove_file(&latest);
        }
    }
}

/// Removes sockets whose wrapper no longer accepts connections.
fn sweep_stale(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "sock")
            || path.file_name().is_some_and(|n| n == LATEST_LINK)
        {
            continue;
        }
        if UnixStream::connect(&path).is_err() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Resolves the target socket for `zediator send`.
pub fn resolve_socket(pid: Option<u32>, socket: Option<PathBuf>) -> io::Result<PathBuf> {
    if let Some(path) = socket {
        return Ok(path);
    }
    let dir = socket_dir();
    if let Some(pid) = pid {
        return Ok(dir.join(format!("{pid}.sock")));
    }
    let latest = dir.join(LATEST_LINK);
    if let Ok(target) = std::fs::read_link(&latest)
        && UnixStream::connect(&target).is_ok()
    {
        return Ok(target);
    }
    // Fall back to the newest live socket.
    newest_live_socket(&dir).ok_or_else(|| {
        io::Error::other("no running zediator session found (is one running in Zed?)")
    })
}

fn newest_live_socket(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "sock")
            || path.file_name().is_some_and(|n| n == LATEST_LINK)
            || UnixStream::connect(&path).is_err()
        {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(t, _)| modified > *t) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, path)| path)
}

/// Sends `message` to the wrapper listening on `socket`.
pub fn send(socket: &Path, message: &[u8]) -> io::Result<()> {
    let mut stream = UnixStream::connect(socket)?;
    stream.write_all(message)?;
    Ok(())
}

/// Lists known sessions as `(pid, alive)` pairs.
pub fn list_sessions() -> Vec<(u32, bool)> {
    let dir = socket_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "sock")
            || path.file_name().is_some_and(|n| n == LATEST_LINK)
        {
            continue;
        }
        let Some(pid) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse().ok())
        else {
            continue;
        };
        sessions.push((pid, UnixStream::connect(&path).is_ok()));
    }
    sessions.sort_unstable();
    sessions
}

/// Builds the message text for `zediator send`.
///
/// The bare `file:line ` form mirrors Zed's built-in AddSelectionToThread
/// paste; the fenced block carries the selected text when provided.
pub fn compose_message(
    file: Option<&str>,
    line: Option<u32>,
    text: Option<&str>,
    rest: &[String],
) -> Vec<u8> {
    let mut message = String::new();
    if let Some(file) = file {
        message.push_str(file);
        if let Some(line) = line {
            message.push_str(&format!(":{line}"));
        }
        message.push(' ');
    }
    if let Some(text) = text
        && !text.is_empty()
    {
        message.push_str(&format!("\n```\n{text}\n```\n"));
    }
    if !rest.is_empty() {
        message.push_str(&rest.join(" "));
    }
    message.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Test writer capturing injected bytes.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn with_runtime_dir<T>(test: impl FnOnce() -> T) -> T {
        // Serialize env mutation across tests in this module.
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: guarded by ENV_LOCK; tests in this module run one at a time.
        unsafe { std::env::set_var("ZEDIATOR_RUNTIME_DIR", dir.path()) };
        let result = test();
        unsafe { std::env::remove_var("ZEDIATOR_RUNTIME_DIR") };
        result
    }

    #[test]
    fn server_injects_bracketed_paste() {
        with_runtime_dir(|| {
            let capture = Capture::default();
            let sink = capture.0.clone();
            let writer: SharedWriter = Arc::new(Mutex::new(Box::new(capture)));
            let server = IpcServer::start(writer).unwrap();

            let socket = resolve_socket(None, None).unwrap();
            send(&socket, b"src/main.rs:4 ").unwrap();

            // The accept loop is asynchronous; poll briefly.
            for _ in 0..50 {
                if !sink.lock().unwrap().is_empty() {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            let received = sink.lock().unwrap().clone();
            assert_eq!(received, b"\x1b[200~src/main.rs:4 \x1b[201~");
            drop(server);
        });
    }

    #[test]
    fn drop_removes_socket_and_latest_link() {
        with_runtime_dir(|| {
            let writer: SharedWriter = Arc::new(Mutex::new(Box::new(Capture::default())));
            let server = IpcServer::start(writer).unwrap();
            let socket = server.socket_path.clone();
            assert!(socket.exists());
            drop(server);
            assert!(!socket.exists());
            assert!(!socket.with_file_name(LATEST_LINK).exists());
        });
    }

    #[test]
    fn resolve_prefers_explicit_socket_and_pid() {
        with_runtime_dir(|| {
            let explicit = PathBuf::from("/tmp/explicit.sock");
            assert_eq!(
                resolve_socket(None, Some(explicit.clone())).unwrap(),
                explicit
            );
            let by_pid = resolve_socket(Some(1234), None).unwrap();
            assert!(by_pid.ends_with("1234.sock"));
        });
    }

    #[test]
    fn resolve_fails_when_no_session() {
        with_runtime_dir(|| {
            assert!(resolve_socket(None, None).is_err());
        });
    }

    #[test]
    fn compose_message_variants() {
        assert_eq!(
            compose_message(Some("a.rs"), Some(3), None, &[]),
            b"a.rs:3 "
        );
        assert_eq!(
            compose_message(Some("a.rs"), None, Some("let x;"), &[]),
            b"a.rs \n```\nlet x;\n```\n"
        );
        assert_eq!(compose_message(None, None, None, &["hi".into()]), b"hi");
    }
}
