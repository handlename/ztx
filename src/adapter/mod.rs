//! CLI-specific adapters.
//!
//! The zediator core is agent-CLI agnostic; adapters raise the experience for
//! known CLIs (session titles, structured transcript export). When no adapter
//! matches, every feature falls back to PTY-recording quality.

mod antigravity;
mod claude;

use std::path::Path;

pub use antigravity::AntigravityAdapter;
pub use claude::ClaudeCodeAdapter;

/// Which adapter to use for the wrapped CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum AdapterKind {
    /// Detect from the wrapped command's name.
    #[default]
    Auto,
    /// Claude Code (`claude`).
    Claude,
    /// antigravity-cli (`agy`).
    Antigravity,
    /// No adapter: PTY-recording fallback quality for every feature.
    None,
}

/// A source of CLI-specific session knowledge. Implementations must be cheap
/// to poll: they are consulted every couple of seconds while idle.
pub trait Adapter: Send {
    /// Human-meaningful description of what the session is doing right now,
    /// used as the terminal title. `None` means "nothing better than the
    /// child's own title".
    fn current_activity(&mut self) -> Option<String>;

    /// Path to the CLI's native transcript for the current session, if the
    /// adapter can locate one (consumed by `export`).
    fn transcript_path(&mut self) -> Option<std::path::PathBuf>;
}

/// Resolves an adapter for `command` (the argv of the wrapped CLI).
pub fn resolve(
    kind: AdapterKind,
    command: &[String],
    child_pid: Option<u32>,
) -> Option<Box<dyn Adapter>> {
    let program = command.first().map(|c| basename(c))?;
    match kind {
        AdapterKind::None => None,
        AdapterKind::Claude => Some(Box::new(ClaudeCodeAdapter::from_env(child_pid))),
        AdapterKind::Antigravity => Some(Box::new(AntigravityAdapter::from_env())),
        AdapterKind::Auto => match program {
            "claude" => Some(Box::new(ClaudeCodeAdapter::from_env(child_pid))),
            "agy" | "antigravity" => Some(Box::new(AntigravityAdapter::from_env())),
            _ => None,
        },
    }
}

/// Resolves an adapter for the standalone `export` subcommand, which runs
/// outside any wrapper process and matches sessions by cwd.
pub fn resolve_for_export(kind: AdapterKind) -> Option<Box<dyn Adapter>> {
    match kind {
        AdapterKind::None => None,
        AdapterKind::Auto | AdapterKind::Claude => Some(Box::new(ClaudeCodeAdapter::for_export())),
        // agy exposes no native transcript; see the antigravity module docs.
        AdapterKind::Antigravity => Some(Box::new(AntigravityAdapter::from_env())),
    }
}

fn basename(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
}
