use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// Environment variable holding a tracing filter directive (e.g. `debug`,
/// `ztx=trace`). Logging is disabled when unset.
const LOG_ENV: &str = "ZTX_LOG";

/// Environment variable overriding the log file path.
const LOG_FILE_ENV: &str = "ZTX_LOG_FILE";

/// Initializes file-based logging when `ZTX_LOG` is set.
///
/// Logs never go to stdout/stderr: the wrapper shares its terminal with the
/// wrapped TUI, and any stray output would corrupt the child's screen.
/// Returns a guard that must stay alive for the duration of the process.
pub fn init() -> Option<WorkerGuard> {
    let directive = std::env::var(LOG_ENV).ok()?;
    if directive.is_empty() {
        return None;
    }

    let path = log_file_path();
    let dir = path.parent()?;
    std::fs::create_dir_all(dir).ok()?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;

    let (writer, guard) = tracing_appender::non_blocking(file);
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(directive))
        .with_writer(writer)
        .with_ansi(false)
        .init();
    Some(guard)
}

/// Resolves the log file path: `$ZTX_LOG_FILE`, or
/// `$XDG_STATE_HOME/ztx/ztx.log`, falling back to
/// `~/.local/state/ztx/ztx.log`.
fn log_file_path() -> PathBuf {
    if let Ok(path) = std::env::var(LOG_FILE_ENV) {
        return PathBuf::from(path);
    }
    let state_home = std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".local/state")
        });
    state_home.join("ztx").join("ztx.log")
}
