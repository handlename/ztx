//! Per-project IPC channel.
//!
//! Each wrapper listens on a Unix socket whose name is derived from the
//! project directory it runs in, so `ztx notify` resolves the target in
//! O(1): both sides hash the project root (`ZED_WORKTREE_ROOT`, else the
//! current directory) to the same `<hash>.sock` path — no scanning, no
//! registry. This assumes one session per project: when a live session already
//! owns the project's socket, `ztx run` reports it and (interactively) offers
//! to terminate it and rebind, rather than silently launching a second one.
//! `--socket` overrides the target explicitly. A sibling `<hash>.info` records
//! pid and cwd; it feeds `ztx sessions` display and `run`'s collision report
//! (including the pid to terminate), but is never used for socket resolution.

use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

use serde::{Deserialize, Serialize};

/// Shared handle to the child's PTY writer, used by the stdin pump.
pub type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// Upper bound for one IPC message; protects the wrapper (and the session it
/// hosts) from a runaway or malicious client exhausting memory.
const MAX_MESSAGE_LEN: u64 = 1024 * 1024;

/// Marks an IPC message as a control frame. Every frame ztx sends today is a
/// control frame, so the prefix is redundant on its own — it is kept because a
/// long-lived session started by an older ztx still pastes any non-NUL payload
/// straight into the agent's prompt. Dropping the prefix would make a new
/// `ztx notify` spray raw JSON into such a session.
const CONTROL_PREFIX: u8 = 0x00;

/// A control message from `ztx notify` (emitted by the Claude Code plugin
/// hooks), carried over the per-project socket and marked by the
/// [`CONTROL_PREFIX`] byte.
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
    if let Ok(dir) = std::env::var("ZTX_RUNTIME_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir).join("ztx");
    }
    // macOS: $TMPDIR is per-user and mode 0700.
    std::env::temp_dir().join("ztx-run")
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
                    "a ztx session is already running in this project ({})",
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
    /// Starts the accept loop that routes control frames to `control`, and
    /// hands back an [`IpcServer`] for cleanup.
    pub fn serve(self, control: ControlChannels) -> IpcServer {
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
                // Only control frames are understood. A payload without the
                // prefix is an older `ztx send` reaching a session that no
                // longer injects text; drop it rather than surprise the agent.
                if message.first() == Some(&CONTROL_PREFIX) {
                    dispatch_control(&message[1..], &control);
                } else {
                    tracing::warn!(bytes = message.len(), "ignoring non-control IPC frame");
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

/// Resolves the socket for `ztx notify`. A missing session is not an error:
/// notify is best-effort so a plugin hook never fails the agent. Returns
/// `None` when no live session owns the socket.
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

/// Sends an encoded control frame to the wrapper listening on `socket`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Control channels for tests that only need a served socket and do not
    /// care about control frames (no wake is ever sent, so the dropped
    /// receiver is harmless).
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
            std::env::set_var("ZTX_RUNTIME_DIR", dir.path());
            std::env::set_var("ZED_WORKTREE_ROOT", project);
        }
        let result = test();
        unsafe {
            std::env::remove_var("ZTX_RUNTIME_DIR");
            std::env::remove_var("ZED_WORKTREE_ROOT");
        }
        result
    }

    #[test]
    fn second_session_in_same_project_is_refused() {
        let project = tempfile::tempdir().unwrap();
        with_env(project.path(), || {
            let first = IpcServer::bind_project().unwrap().serve(discard_channels());
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
    fn stale_socket_is_taken_over() {
        let project = tempfile::tempdir().unwrap();
        with_env(project.path(), || {
            // Leave a dead socket file behind, then bind: it should succeed.
            let path = socket_path_for(&canonical(project.path()));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            drop(UnixListener::bind(&path).unwrap()); // bound then closed -> dead
            let server = IpcServer::bind_project().unwrap().serve(discard_channels());
            assert!(UnixStream::connect(socket_path_for(&canonical(project.path()))).is_ok());
            drop(server);
        });
    }

    #[test]
    fn drop_removes_socket_and_info() {
        let project = tempfile::tempdir().unwrap();
        with_env(project.path(), || {
            let server = IpcServer::bind_project().unwrap().serve(discard_channels());
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
            let server = IpcServer::bind_project().unwrap().serve(discard_channels());
            let sessions = list_sessions();
            assert_eq!(sessions.len(), 1);
            assert!(sessions[0].alive);
            assert_eq!(sessions[0].pid, Some(std::process::id()));
            assert_eq!(sessions[0].cwd, Some(canonical(project.path())));
            assert_eq!(
                sessions[0].socket,
                socket_path_for(&canonical(project.path()))
            );
            drop(server);
        });
    }

    #[test]
    fn existing_project_session_reports_socket_and_pid() {
        let project = tempfile::tempdir().unwrap();
        with_env(project.path(), || {
            // No session yet: nothing to report.
            assert!(existing_project_session().is_none());

            let server = IpcServer::bind_project().unwrap().serve(discard_channels());
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
    fn control_frame_updates_transcript_and_wakes() {
        let project = tempfile::tempdir().unwrap();
        with_env(project.path(), || {
            let (wake, rx) = std::sync::mpsc::channel();
            let transcript = Arc::new(Mutex::new(None));
            let server = IpcServer::bind_project().unwrap().serve(ControlChannels {
                wake,
                transcript: transcript.clone(),
            });

            let socket = socket_path_for(&canonical(project.path()));
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
            drop(server);
        });
    }

    /// A payload without the control prefix (an older `ztx send`) is dropped:
    /// it must neither reach the control handler nor reappear in the session.
    #[test]
    fn non_control_frame_is_ignored() {
        let project = tempfile::tempdir().unwrap();
        with_env(project.path(), || {
            let (wake, rx) = std::sync::mpsc::channel();
            let transcript = Arc::new(Mutex::new(None));
            let server = IpcServer::bind_project().unwrap().serve(ControlChannels {
                wake,
                transcript: transcript.clone(),
            });

            let socket = socket_path_for(&canonical(project.path()));
            send(&socket, b"src/main.rs:4 ").unwrap();

            assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
            assert!(transcript.lock().unwrap().is_none());
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
        assert_eq!(notify_target(None, Some(explicit.clone())), Some(explicit));
    }
}
