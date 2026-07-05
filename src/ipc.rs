//! IPC channel (feature 4: pull editor selections into the session).
//!
//! Each wrapper listens on a Unix socket; `zediator send` (typically invoked
//! from a Zed task with `$ZED_RELATIVE_FILE` / `$ZED_ROW` /
//! `$ZED_SELECTED_TEXT`) connects and writes a message, which the wrapper
//! injects into the child's stdin as a bracketed paste.
//!
//! Discovery: sockets live in one per-user directory as `<pid>.sock`, each
//! with a `<pid>.cwd` sidecar recording the wrapper's working directory. A
//! bare `send` routes to the live session whose cwd matches the editor's
//! project (`ZED_WORKTREE_ROOT`), so concurrent sessions in different
//! projects each receive their own selections; it then falls back to the
//! `latest.sock` symlink and finally the newest live socket. `--pid`/
//! `--socket` select an explicit session. Stale sockets are swept
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

        // Record our working directory in a sidecar so `send` can route to the
        // session belonging to the editor's current project (see resolve_socket).
        if let Ok(cwd) = std::env::current_dir() {
            let _ = std::fs::write(
                socket_path.with_extension("cwd"),
                cwd.to_string_lossy().as_bytes(),
            );
        }

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
        let _ = std::fs::remove_file(self.socket_path.with_extension("cwd"));
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
            let _ = std::fs::remove_file(path.with_extension("cwd"));
        }
    }
}

/// Resolves the target socket for `zediator send`.
///
/// Precedence: explicit `--socket`, then `--pid`, then the session whose
/// working directory matches the editor's project (`ZED_WORKTREE_ROOT`, else
/// our own cwd), then `latest.sock`, then the newest live socket anywhere.
/// The cwd step routes selections to the right session when several run at
/// once in different projects.
pub fn resolve_socket(pid: Option<u32>, socket: Option<PathBuf>) -> io::Result<PathBuf> {
    if let Some(path) = socket {
        return Ok(path);
    }
    let dir = socket_dir();
    if let Some(pid) = pid {
        return Ok(dir.join(format!("{pid}.sock")));
    }

    if let Some(target) = target_cwd()
        && let Some(sock) = newest_live_socket_matching(&dir, Some(&target))
    {
        return Ok(sock);
    }

    let latest = dir.join(LATEST_LINK);
    if let Ok(target) = std::fs::read_link(&latest)
        && UnixStream::connect(&target).is_ok()
    {
        return Ok(target);
    }
    // Fall back to the newest live socket regardless of cwd.
    newest_live_socket_matching(&dir, None).ok_or_else(|| {
        io::Error::other("no running zediator session found (is one running in Zed?)")
    })
}

/// The directory `send` should route to: the editor's project root when Zed
/// provides it, otherwise our own working directory. Canonicalized so symlink
/// differences (e.g. macOS `/tmp` vs `/private/tmp`) do not defeat matching.
fn target_cwd() -> Option<PathBuf> {
    let raw = std::env::var("ZED_WORKTREE_ROOT")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())?;
    Some(canonical(&raw))
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Reads the working directory recorded for the session at `socket_path`.
fn session_cwd(socket_path: &Path) -> Option<PathBuf> {
    let raw = std::fs::read_to_string(socket_path.with_extension("cwd")).ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Some(canonical(Path::new(raw)))
}

/// Newest live socket, optionally restricted to sessions whose recorded cwd
/// equals `want_cwd`.
fn newest_live_socket_matching(dir: &Path, want_cwd: Option<&Path>) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "sock")
            || path.file_name().is_some_and(|n| n == LATEST_LINK)
            || UnixStream::connect(&path).is_err()
        {
            continue;
        }
        if let Some(want) = want_cwd
            && session_cwd(&path).as_deref() != Some(want)
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

/// A discovered session: its wrapper pid, whether it still accepts
/// connections, and the working directory it was started in (if recorded).
pub struct SessionInfo {
    pub pid: u32,
    pub alive: bool,
    pub cwd: Option<PathBuf>,
}

/// Lists known sessions, sorted by pid.
pub fn list_sessions() -> Vec<SessionInfo> {
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
        sessions.push(SessionInfo {
            pid,
            alive: UnixStream::connect(&path).is_ok(),
            cwd: session_cwd(&path),
        });
    }
    sessions.sort_unstable_by_key(|s| s.pid);
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
        // Use a fence longer than any backtick run in the selection, so
        // selecting Markdown that itself contains ``` blocks does not close
        // the fence early.
        let fence = "`".repeat(longest_backtick_run(text).max(2) + 1);
        message.push_str(&format!("\n{fence}\n{text}\n{fence}\n"));
    }
    if !rest.is_empty() {
        message.push_str(&rest.join(" "));
    }
    message.into_bytes()
}

/// Length of the longest consecutive backtick run in `s`.
fn longest_backtick_run(s: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for ch in s.chars() {
        if ch == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
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

    #[test]
    fn compose_message_fence_avoids_backtick_collision() {
        // Selection contains a ``` fence: the wrapper must use a longer one.
        let text = "before\n```rust\nfn x() {}\n```\nafter";
        let msg = String::from_utf8(compose_message(Some("a.rs"), None, Some(text), &[])).unwrap();
        assert!(msg.contains("````\n")); // 4-backtick fence
        assert!(msg.contains(text)); // selection preserved verbatim
        // The wrapping fence encloses the whole selection exactly once.
        assert_eq!(msg.matches("````").count(), 2);
    }

    #[test]
    fn longest_backtick_run_counts_consecutive() {
        assert_eq!(longest_backtick_run("no ticks"), 0);
        assert_eq!(longest_backtick_run("a `b` c"), 1);
        assert_eq!(longest_backtick_run("```rust"), 3);
        assert_eq!(longest_backtick_run("`` then ````"), 4);
    }

    /// Binds a live socket named `<pid>.sock` in the runtime dir and records
    /// `cwd` in its sidecar. Returns the listener (keep it alive) and path.
    fn spawn_fake_session(pid: u32, cwd: &Path) -> (UnixListener, PathBuf) {
        let dir = socket_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{pid}.sock"));
        let listener = UnixListener::bind(&path).unwrap();
        std::fs::write(path.with_extension("cwd"), cwd.to_string_lossy().as_bytes()).unwrap();
        (listener, path)
    }

    #[test]
    fn routes_to_session_matching_zed_worktree_root() {
        with_runtime_dir(|| {
            let proj_a = tempfile::tempdir().unwrap();
            let proj_b = tempfile::tempdir().unwrap();
            let (_a, sock_a) = spawn_fake_session(1001, proj_a.path());
            let (_b, _sock_b) = spawn_fake_session(1002, proj_b.path());

            // SAFETY: guarded by with_runtime_dir's ENV_LOCK.
            unsafe { std::env::set_var("ZED_WORKTREE_ROOT", proj_a.path()) };
            let resolved = resolve_socket(None, None).unwrap();
            unsafe { std::env::remove_var("ZED_WORKTREE_ROOT") };

            assert_eq!(canonical(&resolved), canonical(&sock_a));
        });
    }

    #[test]
    fn falls_back_to_newest_when_no_cwd_match() {
        with_runtime_dir(|| {
            let proj = tempfile::tempdir().unwrap();
            let other = tempfile::tempdir().unwrap();
            let (_s, sock) = spawn_fake_session(2001, proj.path());

            // Editor project has no matching session -> newest live wins.
            unsafe { std::env::set_var("ZED_WORKTREE_ROOT", other.path()) };
            let resolved = resolve_socket(None, None).unwrap();
            unsafe { std::env::remove_var("ZED_WORKTREE_ROOT") };

            assert_eq!(canonical(&resolved), canonical(&sock));
        });
    }

    #[test]
    fn list_sessions_reports_cwd() {
        with_runtime_dir(|| {
            let proj = tempfile::tempdir().unwrap();
            let (_s, _sock) = spawn_fake_session(3001, proj.path());
            let sessions = list_sessions();
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].pid, 3001);
            assert!(sessions[0].alive);
            assert_eq!(
                sessions[0].cwd.as_deref().map(canonical),
                Some(canonical(proj.path()))
            );
        });
    }

    #[test]
    fn drop_removes_cwd_sidecar() {
        with_runtime_dir(|| {
            let writer: SharedWriter = Arc::new(Mutex::new(Box::new(Capture::default())));
            let server = IpcServer::start(writer).unwrap();
            let sidecar = server.socket_path.with_extension("cwd");
            assert!(sidecar.exists());
            drop(server);
            assert!(!sidecar.exists());
        });
    }
}
