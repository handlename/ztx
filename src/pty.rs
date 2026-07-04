use std::io::{self, IsTerminal, Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM, SIGWINCH};
use signal_hook::iterator::Signals;

use crate::term::{TapShared, TermTap};

const IO_BUF_SIZE: usize = 8192;

/// Runs `command` inside a PTY, passing terminal I/O through unchanged,
/// and returns the child's exit code.
///
/// The parent terminal is switched to raw mode (when stdin is a TTY) so that
/// every key chord, including sequences like Shift+Enter encoded via the kitty
/// keyboard protocol, reaches the child as-is.
pub fn run(command: &[String]) -> io::Result<u32> {
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
    // The tap observes bytes after they are forwarded so parsing never delays
    // the passthrough.
    let tap_shared: Arc<Mutex<TapShared>> = TermTap::shared(None);
    let mut tap = TermTap::new(tap_shared.clone());
    let output_thread = thread::spawn(move || {
        let mut stdout = io::stdout().lock();
        let mut buf = [0u8; IO_BUF_SIZE];
        loop {
            match child_output.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if stdout.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    if stdout.flush().is_err() {
                        break;
                    }
                    tap.advance(&buf[..n]);
                }
            }
        }
        tap.flush();
    });

    let status = child.wait().map_err(io::Error::other)?;
    let _ = output_thread.join();
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
