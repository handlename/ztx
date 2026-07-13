//! IPC channel (feature 4: pull editor selections into the session).
//!
//! Each wrapper listens on a Unix socket whose name is derived from the
//! project directory it runs in, so `zedic send` resolves the target in
//! O(1): both sides hash the project root (`ZED_WORKTREE_ROOT`, else the
//! current directory) to the same `<hash>.sock` path — no scanning, no
//! registry. This assumes one session per project: when a live session already
//! owns the project's socket, `zedic run` reports it and (interactively) offers
//! to terminate it and rebind, rather than silently launching a second one.
//! `--socket` overrides the target explicitly. A sibling `<hash>.info` records
//! pid and cwd; it feeds `zedic sessions` display and `run`'s collision report
//! (including the pid to terminate), but is never used for socket resolution.

use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

use serde::{Deserialize, Serialize};

/// Shared handle to the child's PTY writer, used by both the stdin pump and
/// the IPC accept loop.
pub type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// Upper bound for one IPC message; protects the wrapper (and the session it
/// hosts) from a runaway or malicious client exhausting memory.
const MAX_MESSAGE_LEN: u64 = 1024 * 1024;

/// Marks an IPC message as a control frame rather than text to paste. A normal
/// `send` payload is a file reference or selected text, which never begins with
/// NUL, so the two are unambiguous on the wire and `send` stays unchanged.
const CONTROL_PREFIX: u8 = 0x00;

/// A control message from `zedic notify` (emitted by the Claude Code plugin
/// hooks). Carried over the same per-project socket as `send`, distinguished
/// by the [`CONTROL_PREFIX`] byte.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Control {
    /// Force an immediate adapter re-poll (the session's status may have just
    /// changed); collapses the managed title's latency to ~0 on hook events.
    Wake,
    /// Record the authoritative transcript path so `export` need not guess it.
    Transcript { path: PathBuf },
}

/// Signal to the managed-title thread: re-poll now, or shut down.
pub enum TitleSignal {
    Wake,
    Stop,
}

/// Endpoints the IPC control handler writes to when a control frame arrives.
pub struct ControlChannels {
    /// Wakes the managed-title thread for an immediate re-poll.
    pub wake: Sender<TitleSignal>,
    /// Latest hook-supplied transcript path (preferred by `export`).
    pub transcript: Arc<Mutex<Option<PathBuf>>>,
}

/// Serializes `control` as an on-wire control frame (NUL prefix + JSON body).
/// Errors when the control cannot be serialized (e.g. a non-UTF-8 transcript
/// path) so the caller can react rather than send a bodyless frame the server
/// would discard.
pub fn encode_control(control: &Control) -> serde_json::Result<Vec<u8>> {
    let mut buf = vec![CONTROL_PREFIX];
    buf.extend_from_slice(&serde_json::to_vec(control)?);
    Ok(buf)
}

/// Handles a received control frame (the bytes after [`CONTROL_PREFIX`]).
/// Any valid control also wakes the title thread so a status change taking
/// effect alongside a transcript update is reflected immediately.
fn dispatch_control(payload: &[u8], control: &ControlChannels) {
    match serde_json::from_slice::<Control>(payload) {
        Ok(Control::Wake) => {}
        Ok(Control::Transcript { path }) => {
            *control.transcript.lock().expect("transcript lock poisoned") = Some(path);
            tracing::info!("recorded transcript path from hook");
        }
        Err(err) => {
            tracing::warn!(error = %err, "ignoring malformed control frame");
            return;
        }
    }
    let _ = control.wake.send(TitleSignal::Wake);
}

/// Per-user runtime directory holding the session sockets.
pub fn socket_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ZEDIC_RUNTIME_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir).join("zedic");
    }
    // macOS: $TMPDIR is per-user and mode 0700.
    std::env::temp_dir().join("zedic-run")
}

/// Canonicalizes a path, falling back to the input so symlink differences
/// (e.g. macOS `/tmp` vs `/private/tmp`) do not defeat matching.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// FNV-1a 64-bit hash. Deterministic across runs and builds (unlike
/// `DefaultHasher`), so `run` and `send` always agree on the socket name.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The project directory a bare `send`/`run` keys off: the editor's worktree
/// root when Zed provides it, otherwise the current directory.
fn project_cwd() -> PathBuf {
    let raw = std::env::var("ZED_WORKTREE_ROOT")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    canonical(&raw)
}

/// Deterministic socket path for a (canonical) project directory.
fn socket_path_for(cwd: &Path) -> PathBuf {
    let key = fnv1a(cwd.to_string_lossy().as_bytes());
    socket_dir().join(format!("{key:016x}.sock"))
}

/// A socket reserved for this project, before the accept loop is running.
/// Kept separate from [`IpcServer`] so `run` can claim the project (and fail
/// fast on a collision) before spawning the child.
pub struct BoundSocket {
    listener: UnixListener,
    socket_path: PathBuf,
}

/// Server side, owned by the wrapper process. Cleans its socket up on drop.
pub struct IpcServer {
    socket_path: PathBuf,
}

impl IpcServer {
    /// Reserves this project's socket. Returns [`io::ErrorKind::AlreadyExists`]
    /// when a live session already owns it (the caller should refuse to
    /// start); a stale socket is cleaned up and rebound.
    pub fn bind_project() -> io::Result<BoundSocket> {
        let dir = socket_dir();
        std::fs::create_dir_all(&dir)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;

        let cwd = project_cwd();
        let socket_path = socket_path_for(&cwd);
        if UnixStream::connect(&socket_path).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "a zedic session is already running in this project ({})",
                    cwd.display()
                ),
            ));
        }
        // Stale socket (owner gone): remove and take it over.
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path)?;

        // Record pid + cwd for `sessions` display only.
        let _ = std::fs::write(
            socket_path.with_extension("info"),
            format!("{}\n{}", std::process::id(), cwd.display()),
        );

        Ok(BoundSocket {
            listener,
            socket_path,
        })
    }
}

impl BoundSocket {
    /// Starts the accept loop that injects received text messages into
    /// `writer` as bracketed pastes (and routes control frames to `control`),
    /// and hands back an [`IpcServer`] for cleanup.
    pub fn serve(self, writer: SharedWriter, control: ControlChannels) -> IpcServer {
        let listener = self.listener;
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
                // A leading NUL marks a control frame (status wake / transcript
                // path); anything else is text to paste into the child.
                if message.first() == Some(&CONTROL_PREFIX) {
                    dispatch_control(&message[1..], &control);
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
        IpcServer {
            socket_path: self.socket_path,
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
        let _ = std::fs::remove_file(self.socket_path.with_extension("info"));
    }
}

/// Resolves the target socket for `zedic send`: an explicit `--socket`,
/// otherwise this project's deterministic socket. Errors when no live session
/// owns it.
pub fn resolve_socket(socket: Option<PathBuf>) -> io::Result<PathBuf> {
    if let Some(path) = socket {
        return Ok(path);
    }
    let cwd = project_cwd();
    let path = socket_path_for(&cwd);
    if UnixStream::connect(&path).is_ok() {
        Ok(path)
    } else {
        Err(io::Error::other(format!(
            "no zedic session running for this project ({}); \
             start one with `zedic run -- <cli>`",
            cwd.display()
        )))
    }
}

/// Resolves the socket for `zedic notify`. Unlike [`resolve_socket`], a
/// missing session is not an error: notify is best-effort so a plugin hook
/// never fails the agent. Returns `None` when no live session owns the socket.
/// `cwd` (the hook's reported working directory) keys the lookup when set,
/// otherwise the project cwd is used; `explicit` overrides both.
pub fn notify_target(cwd: Option<PathBuf>, explicit: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path);
    }
    let cwd = cwd.map(|c| canonical(&c)).unwrap_or_else(project_cwd);
    let path = socket_path_for(&cwd);
    UnixStream::connect(&path).is_ok().then_some(path)
}

/// Sends `message` to the wrapper listening on `socket`.
pub fn send(socket: &Path, message: &[u8]) -> io::Result<()> {
    let mut stream = UnixStream::connect(socket)?;
    stream.write_all(message)?;
    Ok(())
}

/// The live session owning this project's socket, if any. Used by `run` to
/// describe (and offer to replace) a session that is blocking a fresh start —
/// typically one orphaned by an editor restart, still listening but no longer
/// attached to any terminal. Returns `None` when the project's socket has no
/// live owner.
pub fn existing_project_session() -> Option<SessionInfo> {
    let cwd = project_cwd();
    let socket = socket_path_for(&cwd);
    if UnixStream::connect(&socket).is_err() {
        return None;
    }
    let (pid, info_cwd) = read_info(&socket.with_extension("info"));
    Some(SessionInfo {
        pid,
        alive: true,
        // Fall back to the resolved project dir when the `.info` file is
        // missing or unreadable, so callers always have a directory to show.
        cwd: info_cwd.or(Some(cwd)),
        socket,
    })
}

/// Terminates the wrapper process `pid` and waits for `socket` to be released,
/// so the project can be rebound. Sends SIGTERM first (letting the wrapper and
/// its child exit gracefully) and escalates to SIGKILL if the socket has not
/// been freed within a short window. Errors only when the socket is still owned
/// after the escalation.
pub fn terminate_session(pid: u32, socket: &Path) -> io::Result<()> {
    const GRACE: std::time::Duration = std::time::Duration::from_secs(2);
    signal(pid, libc::SIGTERM);
    if wait_socket_released(socket, GRACE) {
        return Ok(());
    }
    signal(pid, libc::SIGKILL);
    if wait_socket_released(socket, GRACE) {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "existing session (pid {pid}) did not release {} after SIGKILL",
        socket.display()
    )))
}

/// Sends `sig` to `pid`; best-effort (a vanished pid is not an error here).
fn signal(pid: u32, sig: libc::c_int) {
    if let Ok(pid) = i32::try_from(pid) {
        // SAFETY: a plain single-process signal; the checked conversion rules
        // out a negative pid (process-group) target.
        unsafe { libc::kill(pid, sig) };
    }
}

/// Polls until `socket` stops accepting connections, up to `timeout`. Returns
/// whether it was released. A socket file may linger on disk after the owner
/// dies; that is harmless because `bind_project` takes over a stale socket.
fn wait_socket_released(socket: &Path, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if UnixStream::connect(socket).is_err() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// A discovered session: its wrapper pid, whether it still accepts
/// connections, the working directory it was started in (if recorded), and the
/// socket path that identifies it.
pub struct SessionInfo {
    pub pid: Option<u32>,
    pub alive: bool,
    pub cwd: Option<PathBuf>,
    pub socket: PathBuf,
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
        if path.extension().is_none_or(|e| e != "sock") {
            continue;
        }
        let (pid, cwd) = read_info(&path.with_extension("info"));
        sessions.push(SessionInfo {
            pid,
            alive: UnixStream::connect(&path).is_ok(),
            cwd,
            socket: path,
        });
    }
    sessions.sort_unstable_by_key(|s| s.pid);
    sessions
}

/// Parses a `<hash>.info` file (`pid\ncwd`).
fn read_info(info_path: &Path) -> (Option<u32>, Option<PathBuf>) {
    let Ok(content) = std::fs::read_to_string(info_path) else {
        return (None, None);
    };
    let mut lines = content.lines();
    let pid = lines.next().and_then(|l| l.trim().parse().ok());
    let cwd = lines.next().map(PathBuf::from);
    (pid, cwd)
}

/// Builds the message text for `zedic send`.
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

    /// Control channels for tests that exercise text injection only and do not
    /// care about control frames (no wake is ever sent, so the dropped receiver
    /// is harmless).
    fn discard_channels() -> ControlChannels {
        let (wake, _rx) = std::sync::mpsc::channel();
        ControlChannels {
            wake,
            transcript: Arc::new(Mutex::new(None)),
        }
    }

    /// Runs `test` with an isolated runtime dir and project root. Serialized,
    /// because it mutates process-wide environment variables.
    fn with_env<T>(project: &Path, test: impl FnOnce() -> T) -> T {
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: guarded by ENV_LOCK; these tests run one at a time.
        unsafe {
            std::env::set_var("ZEDIC_RUNTIME_DIR", dir.path());
            std::env::set_var("ZED_WORKTREE_ROOT", project);
        }
        let result = test();
        unsafe {
            std::env::remove_var("ZEDIC_RUNTIME_DIR");
            std::env::remove_var("ZED_WORKTREE_ROOT");
        }
        result
    }

    #[test]
    fn server_injects_bracketed_paste() {
        let project = tempfile::tempdir().unwrap();
        with_env(project.path(), || {
            let capture = Capture::default();
            let sink = capture.0.clone();
            let writer: SharedWriter = Arc::new(Mutex::new(Box::new(capture)));
            let server = IpcServer::bind_project().unwrap().serve(writer, discard_channels());

            let socket = resolve_socket(None).unwrap();
            send(&socket, b"src/main.rs:4 ").unwrap();

            for _ in 0..50 {
                if !sink.lock().unwrap().is_empty() {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                sink.lock().unwrap().clone(),
                b"\x1b[200~src/main.rs:4 \x1b[201~"
            );
            drop(server);
        });
    }

    #[test]
    fn resolves_this_projects_socket() {
        let project = tempfile::tempdir().unwrap();
        with_env(project.path(), || {
            let writer: SharedWriter = Arc::new(Mutex::new(Box::new(Capture::default())));
            let server = IpcServer::bind_project().unwrap().serve(writer, discard_channels());
            let resolved = resolve_socket(None).unwrap();
            assert_eq!(resolved, socket_path_for(&canonical(project.path())));
            assert!(UnixStream::connect(&resolved).is_ok());
            drop(server);
        });
    }

    #[test]
    fn second_session_in_same_project_is_refused() {
        let project = tempfile::tempdir().unwrap();
        with_env(project.path(), || {
            let writer: SharedWriter = Arc::new(Mutex::new(Box::new(Capture::default())));
            let first = IpcServer::bind_project().unwrap().serve(writer, discard_channels());
            let err = IpcServer::bind_project()
                .err()
                .expect("second bind must fail");
            assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
            drop(first);
        });
    }

    #[test]
    fn different_projects_get_distinct_sockets() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        assert_ne!(
            socket_path_for(&canonical(a.path())),
            socket_path_for(&canonical(b.path()))
        );
    }

    #[test]
    fn resolve_errors_without_a_session() {
        let project = tempfile::tempdir().unwrap();
        with_env(project.path(), || {
            assert!(resolve_socket(None).is_err());
        });
    }

    #[test]
    fn resolve_prefers_explicit_socket() {
        let project = tempfile::tempdir().unwrap();
        with_env(project.path(), || {
            let explicit = PathBuf::from("/tmp/explicit.sock");
            assert_eq!(resolve_socket(Some(explicit.clone())).unwrap(), explicit);
        });
    }

    #[test]
    fn stale_socket_is_taken_over() {
        let project = tempfile::tempdir().unwrap();
        with_env(project.path(), || {
            // Leave a dead socket file behind, then bind: it should succeed.
            let path = socket_path_for(&canonical(project.path()));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            drop(UnixListener::bind(&path).unwrap()); // bound then closed -> dead
            let writer: SharedWriter = Arc::new(Mutex::new(Box::new(Capture::default())));
            let server = IpcServer::bind_project().unwrap().serve(writer, discard_channels());
            assert!(UnixStream::connect(socket_path_for(&canonical(project.path()))).is_ok());
            drop(server);
        });
    }

    #[test]
    fn drop_removes_socket_and_info() {
        let project = tempfile::tempdir().unwrap();
        with_env(project.path(), || {
            let writer: SharedWriter = Arc::new(Mutex::new(Box::new(Capture::default())));
            let server = IpcServer::bind_project().unwrap().serve(writer, discard_channels());
            let socket = socket_path_for(&canonical(project.path()));
            let info = socket.with_extension("info");
            assert!(socket.exists() && info.exists());
            drop(server);
            assert!(!socket.exists() && !info.exists());
        });
    }

    #[test]
    fn list_sessions_reports_pid_and_cwd() {
        let project = tempfile::tempdir().unwrap();
        with_env(project.path(), || {
            let writer: SharedWriter = Arc::new(Mutex::new(Box::new(Capture::default())));
            let server = IpcServer::bind_project().unwrap().serve(writer, discard_channels());
            let sessions = list_sessions();
            assert_eq!(sessions.len(), 1);
            assert!(sessions[0].alive);
            assert_eq!(sessions[0].pid, Some(std::process::id()));
            assert_eq!(sessions[0].cwd, Some(canonical(project.path())));
            assert_eq!(sessions[0].socket, socket_path_for(&canonical(project.path())));
            drop(server);
        });
    }

    #[test]
    fn existing_project_session_reports_socket_and_pid() {
        let project = tempfile::tempdir().unwrap();
        with_env(project.path(), || {
            // No session yet: nothing to report.
            assert!(existing_project_session().is_none());

            let writer: SharedWriter = Arc::new(Mutex::new(Box::new(Capture::default())));
            let server = IpcServer::bind_project().unwrap().serve(writer, discard_channels());
            let found = existing_project_session().expect("live session must be found");
            assert!(found.alive);
            assert_eq!(found.pid, Some(std::process::id()));
            assert_eq!(found.socket, socket_path_for(&canonical(project.path())));
            assert_eq!(found.cwd, Some(canonical(project.path())));
            drop(server);
        });
    }

    #[test]
    fn wait_socket_released_is_true_when_no_owner() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("absent.sock");
        // Nothing ever listened here, so it is already "released".
        assert!(wait_socket_released(
            &socket,
            std::time::Duration::from_millis(100)
        ));
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
        let text = "before\n```rust\nfn x() {}\n```\nafter";
        let msg = String::from_utf8(compose_message(Some("a.rs"), None, Some(text), &[])).unwrap();
        assert!(msg.contains("````\n"));
        assert!(msg.contains(text));
        assert_eq!(msg.matches("````").count(), 2);
    }

    #[test]
    fn encode_control_frames_start_with_nul_and_roundtrip() {
        let wake = encode_control(&Control::Wake).unwrap();
        assert_eq!(wake[0], CONTROL_PREFIX);
        assert_eq!(
            serde_json::from_slice::<Control>(&wake[1..]).unwrap(),
            Control::Wake
        );
        let transcript = encode_control(&Control::Transcript {
            path: PathBuf::from("/a/b.jsonl"),
        })
        .unwrap();
        assert_eq!(transcript[0], CONTROL_PREFIX);
        assert_eq!(
            serde_json::from_slice::<Control>(&transcript[1..]).unwrap(),
            Control::Transcript {
                path: PathBuf::from("/a/b.jsonl")
            }
        );
    }

    #[test]
    fn control_frame_updates_transcript_and_wakes_without_injecting() {
        let project = tempfile::tempdir().unwrap();
        with_env(project.path(), || {
            let capture = Capture::default();
            let sink = capture.0.clone();
            let writer: SharedWriter = Arc::new(Mutex::new(Box::new(capture)));
            let (wake, rx) = std::sync::mpsc::channel();
            let transcript = Arc::new(Mutex::new(None));
            let server = IpcServer::bind_project().unwrap().serve(
                writer,
                ControlChannels {
                    wake,
                    transcript: transcript.clone(),
                },
            );

            let socket = resolve_socket(None).unwrap();
            send(
                &socket,
                &encode_control(&Control::Transcript {
                    path: PathBuf::from("/x/y.jsonl"),
                })
                .unwrap(),
            )
            .unwrap();

            // The control handler pulses the title thread and stores the path.
            assert!(matches!(
                rx.recv_timeout(Duration::from_secs(1)),
                Ok(TitleSignal::Wake)
            ));
            assert_eq!(
                *transcript.lock().unwrap(),
                Some(PathBuf::from("/x/y.jsonl"))
            );
            // A control frame is never pasted into the child.
            assert!(sink.lock().unwrap().is_empty());
            drop(server);
        });
    }

    #[test]
    fn notify_target_is_none_without_a_session() {
        let project = tempfile::tempdir().unwrap();
        with_env(project.path(), || {
            assert!(notify_target(None, None).is_none());
        });
    }

    #[test]
    fn notify_target_prefers_explicit_socket() {
        let explicit = PathBuf::from("/tmp/explicit.sock");
        assert_eq!(
            notify_target(None, Some(explicit.clone())),
            Some(explicit)
        );
    }

    #[test]
    fn longest_backtick_run_counts_consecutive() {
        assert_eq!(longest_backtick_run("no ticks"), 0);
        assert_eq!(longest_backtick_run("a `b` c"), 1);
        assert_eq!(longest_backtick_run("```rust"), 3);
        assert_eq!(longest_backtick_run("`` then ````"), 4);
    }
}
