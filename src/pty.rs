use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::ipc::TitleSignal;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM, SIGWINCH};
use signal_hook::iterator::Signals;

use crate::adapter::AdapterKind;
use crate::term::{TapShared, TermTap};
use crate::title::{TitleFilter, TitleMode};

const IO_BUF_SIZE: usize = 8192;

/// Reconcile interval between adapter polls: the managed-title thread also
/// re-polls immediately whenever a plugin hook wakes it, so this is the upper
/// bound on staleness, not the common-case latency.
const TITLE_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Grace given to the child to exit on its own after the controlling terminal
/// is lost (SIGHUP) before the wrapper force-kills it. Bounds how long a child
/// that traps SIGHUP can keep the (now unusable) wrapper — and its socket —
/// alive before teardown.
const HANGUP_GRACE: Duration = Duration::from_secs(2);

pub struct RunOptions {
    pub title_mode: Option<TitleMode>,
    pub title_prefix: Option<String>,
    pub adapter: AdapterKind,
    /// ztx prefix key byte (from config, or `input::DEFAULT_PREFIX`).
    pub prefix: u8,
    /// Editor command line from config; `None` uses env/`zed`/`$EDITOR`.
    pub editor: Option<String>,
    /// Status-emoji prefixes for the managed title (from config).
    pub status_emoji: crate::config::StatusEmoji,
}

/// Runs `command` inside a PTY, passing terminal I/O through (unchanged
/// except for OSC 0/2 title handling per `--title-mode`), and returns the
/// child's exit code.
///
/// The parent terminal is switched to raw mode (when stdin is a TTY) so that
/// every key chord, including sequences like Shift+Enter encoded via the kitty
/// keyboard protocol, reaches the child as-is.
pub fn run(command: &[String], opts: RunOptions) -> io::Result<u32> {
    // Prune stale exports left in the temp dir; best-effort, never fatal.
    crate::export::cleanup_old_exports();

    // Claim this project's IPC socket before spawning the child, so a second
    // session in the same project is refused instead of launching the agent
    // CLI and then failing. A non-collision bind error (e.g. an unwritable
    // runtime dir) is non-fatal: the wrapper runs without selection sharing.
    let bound = match crate::ipc::IpcServer::bind_project() {
        Ok(bound) => Some(bound),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            // A session already owns this project's socket — often one orphaned
            // by an editor restart, still listening but detached from any
            // terminal. Report it and, when interactive, offer to replace it.
            match reclaim_project_socket()? {
                Some(bound) => Some(bound),
                None => return Ok(1),
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "failed to bind IPC socket; selection sharing disabled");
            None
        }
    };

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(current_size())
        .map_err(io::Error::other)?;

    let mut builder = CommandBuilder::new(&command[0]);
    builder.args(&command[1..]);
    builder.cwd(std::env::current_dir()?);
    let mut child = pair
        .slave
        .spawn_command(builder)
        .map_err(io::Error::other)?;
    // Close our copy of the slave end; the child holds its own.
    drop(pair.slave);

    let child_pid = child.process_id();
    let master = pair.master;
    let mut child_output = master.try_clone_reader().map_err(io::Error::other)?;
    // The child's input is shared between the stdin pump and the IPC server.
    let child_writer: crate::ipc::SharedWriter =
        Arc::new(Mutex::new(master.take_writer().map_err(io::Error::other)?));

    tracing::debug!(command = ?command, child_pid = ?child.process_id(), "spawned child in PTY");
    let _raw_mode = crate::term_guard::RawModeGuard::new(io::stdin().is_terminal())?;

    // Adapter and effective title mode. `managed` is the default only when an
    // adapter is available to supply meaningful titles. The adapter is shared
    // between the title thread (activity polling) and the input thread
    // (transcript export).
    let adapter = Arc::new(Mutex::new(crate::adapter::resolve(
        opts.adapter,
        command,
        child_pid,
        opts.status_emoji,
    )));
    let has_adapter = adapter.lock().expect("adapter lock poisoned").is_some();
    let title_mode = opts.title_mode.unwrap_or(if has_adapter {
        TitleMode::Managed
    } else {
        TitleMode::Passthrough
    });
    let title_prefix = opts.title_prefix.unwrap_or_else(|| {
        let program = Path::new(&command[0])
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| command[0].clone());
        format!("{program}: ")
    });

    // Writes to the parent terminal come from two threads (output pump and
    // title thread); the gate keeps escape sequences from interleaving.
    let stdout_gate = Arc::new(Mutex::new(()));

    let tap_shared: Arc<Mutex<TapShared>> = TermTap::shared(None);
    tap_shared.lock().expect("tap lock poisoned").screen_rows = current_size().rows;

    // Resize events and termination signals are handled on a dedicated thread.
    // The PTY master must live there for TIOCSWINSZ, so it moves into the closure.
    let mut signals = Signals::new([SIGWINCH, SIGTERM, SIGHUP, SIGINT])?;
    let signal_handle = signals.handle();
    let tap_for_signals = tap_shared.clone();
    let signal_thread = thread::spawn(move || {
        for signal in &mut signals {
            match signal {
                SIGWINCH => {
                    let size = current_size();
                    let _ = master.resize(size);
                    tap_for_signals
                        .lock()
                        .expect("tap lock poisoned")
                        .screen_rows = size.rows;
                }
                SIGHUP => {
                    // The controlling terminal is gone (e.g. the editor pane or
                    // window closed): this session can no longer be seen or
                    // driven. Forward the hangup so the child may exit and
                    // persist its own state, but never depend on it — a child
                    // that traps SIGHUP (Claude Code does, to survive
                    // disconnects) would otherwise strand this wrapper, leaving
                    // a listening-but-detached socket that blocks the next
                    // `ztx run` in the project. Escalate to SIGKILL after a
                    // short grace so `child.wait()` returns and we tear down.
                    // The pid cannot be recycled meanwhile: the main thread has
                    // not reaped the child yet, so it stays ours even as a
                    // zombie, making the delayed SIGKILL safe.
                    if let Some(pid) = child_pid.and_then(|p| i32::try_from(p).ok()) {
                        // SAFETY: single-process signals to our own child; the
                        // checked conversion rules out a negative (group) pid.
                        unsafe { libc::kill(pid, SIGHUP) };
                        thread::sleep(HANGUP_GRACE);
                        unsafe { libc::kill(pid, libc::SIGKILL) };
                    }
                }
                _ => {
                    if let Some(pid) = child_pid.and_then(|p| i32::try_from(p).ok()) {
                        // SAFETY: forwarding the received signal to the child
                        // process; a negative pid (group kill) is impossible
                        // thanks to the checked conversion above.
                        unsafe { libc::kill(pid, signal) };
                    }
                }
            }
        }
    });

    // Shared state fed by the IPC control frames the Claude Code plugin sends:
    // `title_tx` wakes the managed-title thread for an immediate re-poll, and
    // `transcript_store` holds the hook-supplied transcript path preferred by
    // export. Both exist regardless of adapter/title mode; when nothing
    // consumes them the sends and reads are simply inert.
    let (title_tx, title_rx) = mpsc::channel::<TitleSignal>();
    let transcript_store: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));

    // Start serving `ztx send` on the socket claimed above (if any).
    // Kept alive for the session; dropped on exit to clean up the socket.
    let _ipc = bound.map(|b| {
        b.serve(
            child_writer.clone(),
            crate::ipc::ControlChannels {
                wake: title_tx.clone(),
                transcript: transcript_store.clone(),
            },
        )
    });

    // stdin -> child, with ztx's prefix-key bindings peeled off when the
    // input is interactive. Left detached: reads from stdin cannot be
    // interrupted portably, and the thread dies with the process.
    let input_is_tty = io::stdin().is_terminal();
    let adapter_for_input = adapter.clone();
    let tap_for_input = tap_shared.clone();
    let gate_for_input = stdout_gate.clone();
    let child_input = child_writer.clone();
    let transcript_for_input = transcript_store.clone();
    let prefix = opts.prefix;
    let editor_for_input = opts.editor.clone();
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buf = [0u8; IO_BUF_SIZE];
        tracing::debug!(interactive = input_is_tty, "stdin pump started");
        let mut filter = input_is_tty.then(|| crate::input::InputFilter::new(prefix));
        let mut forwarded = Vec::with_capacity(IO_BUF_SIZE);
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let (bytes, actions): (&[u8], Vec<crate::input::InputAction>) =
                        match filter.as_mut() {
                            Some(filter) => {
                                forwarded.clear();
                                let actions = filter.feed(&buf[..n], &mut forwarded);
                                (&forwarded, actions)
                            }
                            None => (&buf[..n], Vec::new()),
                        };
                    if !bytes.is_empty() {
                        let mut writer = child_input.lock().expect("child writer poisoned");
                        if writer.write_all(bytes).is_err() || writer.flush().is_err() {
                            break;
                        }
                    }
                    for action in actions {
                        handle_action(
                            action,
                            &adapter_for_input,
                            &tap_for_input,
                            &gate_for_input,
                            &transcript_for_input,
                            &mut stdin,
                            editor_for_input.as_deref(),
                        );
                    }
                }
            }
        }
        // EOF: release a pending prefix byte so it is not silently dropped.
        if let Some(filter) = filter.as_mut() {
            forwarded.clear();
            filter.flush(&mut forwarded);
            if !forwarded.is_empty() {
                let mut writer = child_input.lock().expect("child writer poisoned");
                let _ = writer.write_all(&forwarded);
                let _ = writer.flush();
            }
        }
    });

    // child -> stdout. Joined after the child exits to drain remaining output.
    // The title filter transforms bytes on the way out; the tap observes the
    // ORIGINAL bytes (so suppressed child titles are still recorded), after
    // forwarding, so parsing never delays the passthrough.
    let mut tap = TermTap::new(tap_shared.clone());
    let mut filter = TitleFilter::new(title_mode, title_prefix);
    let gate_for_pump = stdout_gate.clone();
    let output_thread = thread::spawn(move || {
        let mut stdout = io::stdout();
        let mut buf = [0u8; IO_BUF_SIZE];
        let mut filtered = Vec::with_capacity(IO_BUF_SIZE + 64);
        loop {
            match child_output.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    filtered.clear();
                    filter.feed(&buf[..n], &mut filtered);
                    {
                        let _gate = gate_for_pump.lock().expect("stdout gate poisoned");
                        if stdout.write_all(&filtered).is_err() {
                            break;
                        }
                        if stdout.flush().is_err() {
                            break;
                        }
                    }
                    tap.advance(&buf[..n]);
                }
            }
        }
        filtered.clear();
        filter.flush(&mut filtered);
        if !filtered.is_empty() {
            let _gate = gate_for_pump.lock().expect("stdout gate poisoned");
            let _ = stdout.write_all(&filtered);
            let _ = stdout.flush();
        }
        tap.flush();
    });

    // Managed mode: a thread polls the adapter for the current activity and
    // re-emits it as the terminal title. It re-polls every TITLE_POLL_INTERVAL
    // or immediately when a plugin hook sends a wake over IPC, collapsing the
    // status latency to ~0 on hook events. Falls back to the child's own
    // (suppressed) title when the adapter has nothing.
    let title_thread = (title_mode == TitleMode::Managed).then(|| {
        let gate = stdout_gate.clone();
        let tap = tap_shared.clone();
        let adapter = adapter.clone();
        let initial = Path::new(&command[0])
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| command[0].clone());
        thread::spawn(move || {
            let mut stdout = io::stdout();
            let mut current = String::new();
            let mut emit = |title: &str, current: &mut String| {
                if title != current {
                    let _gate = gate.lock().expect("stdout gate poisoned");
                    if crate::title::emit_title(&mut stdout, title).is_ok() {
                        tracing::debug!(title, "emitted managed title");
                        *current = title.to_owned();
                    }
                }
            };
            emit(&initial, &mut current);

            loop {
                let title = adapter
                    .lock()
                    .expect("adapter lock poisoned")
                    .as_mut()
                    .and_then(|a| a.current_activity())
                    .or_else(|| tap.lock().expect("tap lock poisoned").last_title.clone());
                if let Some(title) = title {
                    emit(&title, &mut current);
                }
                match title_rx.recv_timeout(TITLE_POLL_INTERVAL) {
                    // Woken by a hook, or the regular reconcile interval elapsed.
                    Ok(TitleSignal::Wake) | Err(RecvTimeoutError::Timeout) => {}
                    // Shutdown requested, or the wrapper dropped every sender.
                    Ok(TitleSignal::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            // Clear the title on exit so a stale activity name does not stick
            // to the terminal after ztx is gone.
            let _gate = gate.lock().expect("stdout gate poisoned");
            let _ = crate::title::emit_title(&mut stdout, "");
        })
    });

    let status = child.wait().map_err(io::Error::other)?;
    let _ = output_thread.join();
    let _ = title_tx.send(TitleSignal::Stop);
    if let Some(handle) = title_thread {
        let _ = handle.join();
    }
    signal_handle.close();
    let _ = signal_thread.join();

    tracing::debug!(exit_code = status.exit_code(), "child exited");
    Ok(status.exit_code())
}

/// Handles a `bind_project` collision: describes the session blocking this
/// project and, when stdin is interactive, offers to terminate it and start
/// fresh here. Returns the rebound socket when the user agrees, or `None` when
/// nothing was started (the session was only reported, so `run` should exit).
fn reclaim_project_socket() -> io::Result<Option<crate::ipc::BoundSocket>> {
    let Some(existing) = crate::ipc::existing_project_session() else {
        // The owner vanished between the failed bind and now: just try again.
        return crate::ipc::IpcServer::bind_project().map(Some);
    };
    let pid_label = existing
        .pid
        .map(|p| p.to_string())
        .unwrap_or_else(|| "?".into());
    let cwd_label = existing
        .cwd
        .as_deref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    eprintln!(
        "ztx: a session is already running in this project\n  \
         pid:  {pid_label}\n  sock: {}\n  cwd:  {cwd_label}",
        existing.socket.display()
    );

    // Never terminate a session without a human's confirmation: a
    // non-interactive caller (Zed task, pipe) only gets the report above.
    if !io::stdin().is_terminal() {
        return Ok(None);
    }
    if !prompt_yes_no("terminate it and start a new session here?")? {
        return Ok(None);
    }
    let Some(pid) = existing.pid else {
        return Err(io::Error::other(
            "cannot terminate the existing session: its pid was not recorded",
        ));
    };
    crate::ipc::terminate_session(pid, &existing.socket)?;
    crate::ipc::IpcServer::bind_project().map(Some)
}

/// Prompts `question` on stderr and reads a yes/no answer from stdin. EOF or an
/// empty line is a safe "no". Mirrors `setup::confirm`; runs before raw mode is
/// entered and before the stdin pump thread starts, so the shared stdin is free.
fn prompt_yes_no(question: &str) -> io::Result<bool> {
    use std::io::BufRead;
    eprint!("{question} [y/N] ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

type SharedAdapter = Arc<Mutex<Option<Box<dyn crate::adapter::Adapter>>>>;

/// Executes a prefix-key action. Runs on the stdin thread. Terminal writes
/// are only allowed while holding the stdout gate (hint overlay); everything
/// else must stay silent to keep the child's screen intact.
fn handle_action(
    action: crate::input::InputAction,
    adapter: &SharedAdapter,
    tap: &Arc<Mutex<TapShared>>,
    gate: &Arc<Mutex<()>>,
    transcript: &Arc<Mutex<Option<PathBuf>>>,
    stdin: &mut impl crate::hint::HintInput,
    config_editor: Option<&str>,
) {
    tracing::debug!(?action, "prefix action triggered");
    match action {
        crate::input::InputAction::Export => {
            // Prefer the transcript path a plugin hook reported (authoritative),
            // then the adapter's best-effort derivation, then the PTY-capture
            // scrollback.
            let hook_path = transcript
                .lock()
                .expect("transcript lock poisoned")
                .clone()
                .and_then(|p| trusted_transcript(&p));
            let markdown = hook_path
                .or_else(|| {
                    adapter
                        .lock()
                        .expect("adapter lock poisoned")
                        .as_mut()
                        .and_then(|a| a.transcript_path())
                })
                .and_then(|path| crate::export::transcript_to_markdown(&path).ok())
                .unwrap_or_else(|| crate::export::scrollback_to_markdown(tap));
            let result = crate::export::write_export(&markdown).and_then(|path| {
                crate::export::open_in_editor(&path, config_editor).map(|()| path)
            });
            match result {
                Ok(path) => tracing::info!(path = %path.display(), "exported session log"),
                Err(err) => tracing::warn!(error = %err, "session export failed"),
            }
        }
        crate::input::InputAction::DumpState => {
            let message = match crate::debug::dump_state(tap, "manual dump (ctrl-] d)") {
                Ok(path) => format!("ztx: state dumped to {} (press any key)", path.display()),
                Err(err) => format!("ztx: state dump failed: {err} (press any key)"),
            };
            let mouse_modes = current_mouse_modes(tap);
            let _gate = gate.lock().expect("stdout gate poisoned");
            let mut stdout = io::stdout();
            let _ = crate::hint::show_message(stdin, &mut stdout, &message, &mouse_modes);
        }
        crate::input::InputAction::Hint => {
            let (scroll_lines, alt_rows, alt_screen, mouse_modes) = {
                let guard = tap.lock().expect("tap lock poisoned");
                (
                    guard.scrollback.recent(400),
                    guard.alt_snapshot.clone(),
                    guard.alt_screen,
                    guard.mouse_modes.iter().copied().collect::<Vec<u16>>(),
                )
            };
            let cwd = std::env::current_dir().unwrap_or_else(|_| "/".into());

            // Full-screen sessions: paint labels in place, directly over the
            // path positions on the visible frame. Holding the gate pauses
            // the output pump so the child cannot repaint over the labels.
            if alt_screen {
                let hints = crate::hint::extract_screen_hints(&alt_rows, &cwd, 26);
                if !hints.is_empty() {
                    tracing::debug!(hints = hints.len(), "hint mode (in-place overlay)");
                    let _gate = gate.lock().expect("stdout gate poisoned");
                    let mut stdout = io::stdout();
                    match crate::hint::pick_overlay(
                        stdin,
                        &mut stdout,
                        &hints,
                        &alt_rows,
                        &mouse_modes,
                    ) {
                        Ok(Some(index)) => open_candidate(&hints[index].candidate, config_editor),
                        Ok(None) => tracing::debug!("hint mode cancelled"),
                        Err(err) => tracing::warn!(error = %err, "hint mode failed"),
                    }
                    return;
                }
            }

            // Primary-screen sessions (no positional information for what is
            // visible): modal list over the captured history.
            let mut lines = scroll_lines;
            lines.extend(alt_rows.iter().cloned());
            let candidates = crate::hint::extract_candidates(&lines, &cwd, 40);
            tracing::debug!(
                window = lines.len(),
                alt_screen,
                candidates = candidates.len(),
                "hint mode (modal fallback)"
            );
            if candidates.is_empty() {
                let dump = crate::debug::dump_state(tap, "hint mode found no candidates")
                    .map(|p| format!(" — state dumped to {}", p.display()))
                    .unwrap_or_default();
                let _gate = gate.lock().expect("stdout gate poisoned");
                let mut stdout = io::stdout();
                let _ = crate::hint::show_message(
                    stdin,
                    &mut stdout,
                    &format!("ztx: no file paths found{dump} (press any key)"),
                    &mouse_modes,
                );
                return;
            }
            let _gate = gate.lock().expect("stdout gate poisoned");
            let mut stdout = io::stdout();
            match crate::hint::pick(stdin, &mut stdout, &candidates, &mouse_modes) {
                Ok(Some(index)) => open_candidate(&candidates[index], config_editor),
                Ok(None) => tracing::debug!("hint mode cancelled"),
                Err(err) => tracing::warn!(error = %err, "hint mode failed"),
            }
        }
    }
}

/// Validates a hook-supplied transcript path before trusting it for export: it
/// must resolve to a regular file under `~/.claude/projects/`. This keeps a
/// (same-user) writer on the IPC socket from steering export at an arbitrary
/// file. Returns the canonical path on success.
fn trusted_transcript(raw: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let projects = std::fs::canonicalize(Path::new(&home).join(".claude/projects")).ok()?;
    let path = std::fs::canonicalize(raw).ok()?;
    (path.starts_with(&projects) && path.is_file()).then_some(path)
}

fn open_candidate(chosen: &crate::hint::Candidate, config_editor: Option<&str>) {
    match crate::export::open_location(&chosen.path, chosen.line, chosen.column, config_editor) {
        Ok(()) => tracing::info!(
            path = %chosen.path.display(),
            line = ?chosen.line,
            "opened location from hint mode"
        ),
        Err(err) => tracing::warn!(error = %err, "failed to open location"),
    }
}

fn current_mouse_modes(tap: &Arc<Mutex<TapShared>>) -> Vec<u16> {
    tap.lock()
        .expect("tap lock poisoned")
        .mouse_modes
        .iter()
        .copied()
        .collect()
}

/// Reports the current terminal size, falling back to 80x24 when unavailable
/// or nonsensical (a PTY can report 0x0, e.g. under `expect`).
fn current_size() -> PtySize {
    let (cols, rows) = match crossterm::terminal::size() {
        Ok((cols, rows)) if cols > 0 && rows > 0 => (cols, rows),
        _ => (80, 24),
    };
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `test` with `HOME` set to `home`, serialized because it mutates a
    /// process-wide environment variable.
    fn with_home<T>(home: &Path, test: impl FnOnce() -> T) -> T {
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap();
        let prev = std::env::var_os("HOME");
        // SAFETY: guarded by ENV_LOCK; these tests run one at a time.
        unsafe { std::env::set_var("HOME", home) };
        let result = test();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
        result
    }

    #[test]
    fn trusted_transcript_accepts_file_under_claude_projects() {
        let home = tempfile::tempdir().unwrap();
        let projects = home.path().join(".claude/projects/slug");
        std::fs::create_dir_all(&projects).unwrap();
        let file = projects.join("session.jsonl");
        std::fs::write(&file, "{}").unwrap();
        with_home(home.path(), || {
            assert_eq!(
                trusted_transcript(&file),
                Some(std::fs::canonicalize(&file).unwrap())
            );
        });
    }

    #[test]
    fn trusted_transcript_rejects_path_outside_projects() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".claude/projects")).unwrap();
        // A real file, but outside ~/.claude/projects/.
        let outside = home.path().join("evil.jsonl");
        std::fs::write(&outside, "{}").unwrap();
        with_home(home.path(), || {
            assert_eq!(trusted_transcript(&outside), None);
        });
    }

    #[test]
    fn trusted_transcript_rejects_when_projects_dir_missing() {
        // No ~/.claude/projects: canonicalizing the base fails, so nothing is
        // trusted (also covers an effectively-unset HOME pointing nowhere).
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("x.jsonl");
        std::fs::write(&file, "{}").unwrap();
        with_home(home.path(), || {
            assert_eq!(trusted_transcript(&file), None);
        });
    }

    #[test]
    fn trusted_transcript_rejects_missing_file() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".claude/projects")).unwrap();
        let missing = home.path().join(".claude/projects/nope.jsonl");
        with_home(home.path(), || {
            assert_eq!(trusted_transcript(&missing), None);
        });
    }
}
