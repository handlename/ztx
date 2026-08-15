//! CLI-specific adapters.
//!
//! The ztx core is agent-CLI agnostic; adapters raise the experience for
//! known CLIs (session titles, structured transcript export). When no adapter
//! matches, every feature falls back to PTY-recording quality.

mod antigravity;
mod claude;

use std::path::Path;

pub use antigravity::AntigravityAdapter;
pub use claude::ClaudeCodeAdapter;
/// Session label derived from `cwd` (worktree name / branch / basename),
/// reused by desktop notifications so they match the terminal thread title.
pub(crate) use claude::derive_title;
use claude::worktree_name;

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
    status_emoji: crate::config::StatusEmoji,
) -> Option<Box<dyn Adapter>> {
    let program = command.first().map(|c| basename(c))?;
    match kind {
        AdapterKind::None => None,
        AdapterKind::Claude => Some(Box::new(ClaudeCodeAdapter::from_env(
            child_pid,
            status_emoji,
        ))),
        AdapterKind::Antigravity => Some(Box::new(AntigravityAdapter::from_env())),
        AdapterKind::Auto => match program {
            "claude" => Some(Box::new(ClaudeCodeAdapter::from_env(
                child_pid,
                status_emoji,
            ))),
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

/// Returns `command` with `-n <worktree name>` inserted right after the
/// program, so Claude Code names its session the same way ztx labels the
/// terminal thread and `claude --resume <worktree name>` resolves it.
///
/// Inserted *before* the caller's arguments rather than appended: `-n` is
/// last-wins, so `ztx run -- claude -n mine` keeps `mine` without ztx having
/// to detect the duplicate.
///
/// Outside a worktree checkout the command is returned unchanged. Not because
/// there is no name to offer — `derive_title` would fall back to the branch —
/// but because a session name is a machine-wide resume handle, and every
/// repository checked out on `main` would claim the same one.
pub fn with_session_name(kind: AdapterKind, command: &[String], cwd: &Path) -> Vec<String> {
    if !is_claude(kind, command) {
        return command.to_vec();
    }
    let Some(name) = worktree_name(cwd) else {
        return command.to_vec();
    };

    let mut argv = Vec::with_capacity(command.len() + 2);
    argv.push(command[0].clone());
    argv.push("-n".to_owned());
    argv.push(name);
    argv.extend_from_slice(&command[1..]);
    argv
}

/// Whether `resolve` would pick the Claude Code adapter for `command`.
fn is_claude(kind: AdapterKind, command: &[String]) -> bool {
    let Some(program) = command.first().map(|c| basename(c)) else {
        return false;
    };
    match kind {
        AdapterKind::Claude => true,
        AdapterKind::Auto => program == "claude",
        AdapterKind::None | AdapterKind::Antigravity => false,
    }
}

fn basename(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Worktree layout `.../worktrees/<repo>/<name>/<repo>`.
    const WORKTREE_CWD: &str = "/home/u/src/worktrees/ztx/push-notification/ztx";
    const PLAIN_CWD: &str = "/home/u/src/ztx";

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn worktree_checkout_gets_the_name_before_the_user_arguments() {
        assert_eq!(
            with_session_name(
                AdapterKind::Auto,
                &argv(&["claude", "--continue"]),
                Path::new(WORKTREE_CWD),
            ),
            argv(&["claude", "-n", "push-notification", "--continue"]),
        );
    }

    #[test]
    fn an_explicit_user_name_wins_by_coming_last() {
        let out = with_session_name(
            AdapterKind::Auto,
            &argv(&["claude", "-n", "mine"]),
            Path::new(WORKTREE_CWD),
        );
        assert_eq!(
            out,
            argv(&["claude", "-n", "push-notification", "-n", "mine"])
        );
        // `-n` is last-wins, so the trailing pair is the effective name.
        assert_eq!(out.last().unwrap(), "mine");
    }

    #[test]
    fn plain_checkout_is_left_alone() {
        let command = argv(&["claude"]);
        assert_eq!(
            with_session_name(AdapterKind::Auto, &command, Path::new(PLAIN_CWD)),
            command,
        );
    }

    #[test]
    fn other_adapters_are_left_alone() {
        let command = argv(&["agy"]);
        assert_eq!(
            with_session_name(AdapterKind::Auto, &command, Path::new(WORKTREE_CWD)),
            command,
        );
        assert_eq!(
            with_session_name(AdapterKind::Antigravity, &command, Path::new(WORKTREE_CWD)),
            command,
        );
    }

    #[test]
    fn adapter_none_is_left_alone_even_for_claude() {
        let command = argv(&["claude"]);
        assert_eq!(
            with_session_name(AdapterKind::None, &command, Path::new(WORKTREE_CWD)),
            command,
        );
    }

    #[test]
    fn an_explicit_claude_adapter_names_a_renamed_binary() {
        assert_eq!(
            with_session_name(
                AdapterKind::Claude,
                &argv(&["/opt/bin/claude-beta"]),
                Path::new(WORKTREE_CWD),
            ),
            argv(&["/opt/bin/claude-beta", "-n", "push-notification"]),
        );
    }

    #[test]
    fn an_empty_command_is_left_alone() {
        assert!(with_session_name(AdapterKind::Auto, &[], Path::new(WORKTREE_CWD)).is_empty());
    }
}
