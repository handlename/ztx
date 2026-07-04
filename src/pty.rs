use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM, SIGWINCH};
use signal_hook::iterator::Signals;

use crate::adapter::AdapterKind;
use crate::term::{TapShared, TermTap};
use crate::title::{TitleFilter, TitleMode};

const IO_BUF_SIZE: usize = 8192;

/// Interval between adapter polls while the child is running.
const TITLE_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Granularity of the title thread's shutdown checks.
const TITLE_TICK: Duration = Duration::from_millis(250);

pub struct RunOptions {
    pub title_mode: Option<TitleMode>,
    pub title_prefix: Option<String>,
    pub adapter: AdapterKind,
}

/// Runs `command` inside a PTY, passing terminal I/O through (unchanged
/// except for OSC 0/2 title handling per `--title-mode`), and returns the
/// child's exit code.
///
/// The parent terminal is switched to raw mode (when stdin is a TTY) so that
/// every key chord, including sequences like Shift+Enter encoded via the kitty
/// keyboard protocol, reaches the child as-is.
pub fn run(command: &[String], opts: RunOptions) -> io::Result<u32> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(current_size()).map_err(io::Error::other)?;

    let mut builder = CommandBuilder::new(&command[0]);
    builder.args(&command[1..]);
    builder.cwd(std::env::current_dir()?);
    let mut child = pair.slave.spawn_command(builder).map_err(io::Error::other)?;
    // Close our copy of the slave end; the child holds its own.
    drop(pair.slave);

    let child_pid = child.process_id();
    let master = pair.master;
    let mut child_output = master.try_clone_reader().map_err(io::Error::other)?;
    let mut child_input = master.take_writer().map_err(io::Error::other)?;

    tracing::debug!(command = ?command, child_pid = ?child.process_id(), "spawned child in PTY");
    let _raw_mode = crate::term_guard::RawModeGuard::new(io::stdin().is_terminal())?;

    // Adapter and effective title mode. `managed` is the default only when an
    // adapter is available to supply meaningful titles.
    let adapter = crate::adapter::resolve(opts.adapter, command, child_pid);
    let title_mode = opts.title_mode.unwrap_or(if adapter.is_some() {
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

    // Resize events and termination signals are handled on a dedicated thread.
    // The PTY master must live there for TIOCSWINSZ, so it moves into the closure.
    let mut signals = Signals::new([SIGWINCH, SIGTERM, SIGHUP, SIGINT])?;
    let signal_handle = signals.handle();
    let signal_thread = thread::spawn(move || {
        for signal in &mut signals {
            match signal {
                SIGWINCH => {
                    let _ = master.resize(current_size());
                }
                _ => {
                    if let Some(pid) = child_pid {
                        // SAFETY: forwarding the received signal to the child process.
                        unsafe { libc::kill(pid as i32, signal) };
                    }
                }
            }
        }
    });

    // stdin -> child. Left detached: reads from stdin cannot be interrupted
    // portably, and the thread dies with the process.
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buf = [0u8; IO_BUF_SIZE];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if child_input.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    if child_input.flush().is_err() {
                        break;
                    }
                }
            }
        }
    });

    // child -> stdout. Joined after the child exits to drain remaining output.
    // The title filter transforms bytes on the way out; the tap observes the
    // ORIGINAL bytes (so suppressed child titles are still recorded), after
    // forwarding, so parsing never delays the passthrough.
    let tap_shared: Arc<Mutex<TapShared>> = TermTap::shared(None);
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

    // Managed mode: a low-frequency thread polls the adapter for the current
    // activity and re-emits it as the terminal title. Falls back to the
    // child's own (suppressed) title when the adapter has nothing.
    let title_stop = Arc::new(AtomicBool::new(false));
    let title_thread = (title_mode == TitleMode::Managed).then(|| {
        let stop = title_stop.clone();
        let gate = stdout_gate.clone();
        let tap = tap_shared.clone();
        let mut adapter = adapter;
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

            let ticks_per_poll =
                (TITLE_POLL_INTERVAL.as_millis() / TITLE_TICK.as_millis()).max(1) as u64;
            let mut tick: u64 = 0;
            while !stop.load(Ordering::Relaxed) {
                thread::sleep(TITLE_TICK);
                tick += 1;
                if !tick.is_multiple_of(ticks_per_poll) {
                    continue;
                }
                let title = adapter
                    .as_mut()
                    .and_then(|a| a.current_activity())
                    .or_else(|| tap.lock().expect("tap lock poisoned").last_title.clone());
                if let Some(title) = title {
                    emit(&title, &mut current);
                }
            }
            // Clear the title on exit so a stale activity name does not stick
            // to the terminal after zediator is gone.
            let _gate = gate.lock().expect("stdout gate poisoned");
            let _ = crate::title::emit_title(&mut stdout, "");
        })
    });

    let status = child.wait().map_err(io::Error::other)?;
    let _ = output_thread.join();
    title_stop.store(true, Ordering::Relaxed);
    if let Some(handle) = title_thread {
        let _ = handle.join();
    }
    signal_handle.close();
    let _ = signal_thread.join();

    tracing::debug!(exit_code = status.exit_code(), "child exited");
    Ok(status.exit_code())
}

/// Reports the current terminal size, falling back to 80x24 when unavailable
/// (e.g. when stdin is not a TTY).
fn current_size() -> PtySize {
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}
