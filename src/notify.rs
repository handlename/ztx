//! Best-effort macOS desktop notifications for Claude Code hook events.
//!
//! Fired from `ztx notify --from-hook` only when a live wrapper session exists
//! for the hook's `cwd`. Everything here is best-effort: a non-macOS host, a
//! missing `terminal-notifier`, or any spawn failure is a silent no-op, so the
//! notification path can never disturb the agent or the title-refresh work that
//! shares the same hook.

use std::path::Path;

/// Emits a desktop notification for `event`, if the event warrants user
/// attention and the platform supports it. `sound` is a Sound-Preferences name
/// (`None` is silent); `emoji` supplies the status prefix for the subtitle.
pub fn desktop(
    event: &str,
    cwd: Option<&Path>,
    message: Option<&str>,
    sound: Option<&str>,
    emoji: &crate::config::StatusEmoji,
) {
    #[cfg(target_os = "macos")]
    {
        if let Some(subtitle) = subtitle_for(event, emoji) {
            let cwd = cwd.unwrap_or_else(|| Path::new("."));
            macos::emit(&subtitle, event, cwd, message, sound);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (event, cwd, message, sound, emoji);
}

/// Maps a hook event to the notification subtitle, or `None` for events that
/// should not raise a desktop notification. This is the single gate on which
/// events notify: `Notification` (Claude wants input) and `Stop` (Claude
/// finished); everything else (SessionStart, UserPromptSubmit, …) is ignored.
// Only the macOS path and the tests call these pure helpers, so on other
// targets they are unused by the (non-test) build; allow that rather than
// gate them off, keeping them testable everywhere.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn event_subtitle(event: &str) -> Option<&'static str> {
    match event {
        "Notification" => Some("Waiting for input"),
        "Stop" => Some("Finished"),
        _ => None,
    }
}

/// The subtitle with the matching status emoji prefixed (mirroring the terminal
/// title's status prefix): `waiting` for `Notification`, `idle` for `Stop`. An
/// emoji configured to an empty string yields the bare subtitle. `None` for
/// events that should not notify.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn subtitle_for(event: &str, emoji: &crate::config::StatusEmoji) -> Option<String> {
    let base = event_subtitle(event)?;
    let prefix = match event {
        "Notification" => emoji.waiting.as_str(),
        "Stop" => emoji.idle.as_str(),
        _ => "",
    };
    Some(if prefix.is_empty() {
        base.to_owned()
    } else {
        format!("{prefix} {base}")
    })
}

/// The notification title: `<repo>/<session>` (e.g. `ztx/push-notification`),
/// where the session label is the same one ztx shows in the terminal thread.
/// The repo segment is dropped when it would merely repeat the label.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn session_label(cwd: &Path) -> String {
    let name = crate::adapter::derive_title(cwd);
    match cwd.file_name().map(|n| n.to_string_lossy().into_owned()) {
        Some(repo) if !repo.is_empty() && repo != name => format!("{repo}/{name}"),
        _ => name,
    }
}

/// Wraps `s` in single quotes for a POSIX `sh -c` command line (used by
/// `terminal-notifier -execute`), escaping any embedded single quotes.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use super::{session_label, shell_quote};

    pub(super) fn emit(
        subtitle: &str,
        event: &str,
        cwd: &Path,
        message: Option<&str>,
        sound: Option<&str>,
    ) {
        let Some(terminal_notifier) = which("terminal-notifier") else {
            tracing::debug!("terminal-notifier not found; desktop notification skipped");
            return;
        };

        let title = session_label(cwd);
        let body = message.unwrap_or(match event {
            "Stop" => "Response finished",
            _ => "Waiting for your input",
        });
        // One notification per session at a time: a fresh event replaces the
        // previous banner for the same session instead of stacking.
        let group = format!("ztx-{title}");

        let mut cmd = Command::new(terminal_notifier);
        cmd.args(["-title", &title])
            .args(["-subtitle", subtitle])
            .args(["-message", body])
            .args(["-group", &group]);
        if let Some(icon) = zed_icon() {
            // `-contentImage` (right-side attachment) shows on current macOS;
            // `-appIcon` (left app icon) is ignored there, so it is not used.
            cmd.args(["-contentImage", &icon.to_string_lossy()]);
        }
        if let Some(sound) = sound {
            cmd.args(["-sound", sound]);
        }
        // Clicking focuses the worktree's Zed workspace. `-sender` would put the
        // Zed icon in the app-icon slot but disables `-execute`, so the click
        // action wins and the Zed logo rides along via `-contentImage` instead.
        if let Some(zed) = which("zed") {
            let execute = format!(
                "{} {}",
                shell_quote(&zed.to_string_lossy()),
                shell_quote(&cwd.to_string_lossy()),
            );
            cmd.args(["-execute", &execute]);
        }

        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Err(err) = cmd.spawn() {
            tracing::debug!(error = %err, "failed to spawn terminal-notifier");
        }
    }

    /// Absolute path of `program` on `PATH`, or `None`. `terminal-notifier`
    /// `-execute` runs under a bare `sh` whose `PATH` may miss Homebrew, so the
    /// click command needs the resolved path rather than a bare name.
    fn which(program: &str) -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())
    }

    /// A PNG of the Zed app icon for `-contentImage`, generated once from
    /// `Zed.icns` and cached. `None` (no icon) when Zed, `sips`, or the cache
    /// directory is unavailable — the notification still fires, just plainer.
    fn zed_icon() -> Option<PathBuf> {
        let cache = cache_dir()?;
        let png = cache.join("zed-icon.png");
        if png.is_file() {
            return Some(png);
        }
        let icns = Path::new("/Applications/Zed.app/Contents/Resources/Zed.icns");
        if !icns.is_file() {
            return None;
        }
        std::fs::create_dir_all(&cache).ok()?;
        let status = Command::new("sips")
            .args(["-s", "format", "png"])
            .arg(icns)
            .arg("--out")
            .arg(&png)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()?;
        (status.success() && png.is_file()).then_some(png)
    }

    /// Cache directory for generated assets: `$XDG_CACHE_HOME/ztx`, else the
    /// macOS default `~/Library/Caches/ztx`.
    fn cache_dir() -> Option<PathBuf> {
        if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(xdg).join("ztx"));
        }
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join("Library/Caches/ztx"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn only_waiting_and_finished_events_notify() {
        assert_eq!(event_subtitle("Notification"), Some("Waiting for input"));
        assert_eq!(event_subtitle("Stop"), Some("Finished"));
        assert_eq!(event_subtitle("SessionStart"), None);
        assert_eq!(event_subtitle("UserPromptSubmit"), None);
        assert_eq!(event_subtitle(""), None);
    }

    #[test]
    fn subtitle_prefixes_the_matching_status_emoji() {
        let emoji = crate::config::StatusEmoji {
            busy: "B".into(),
            idle: "I".into(),
            waiting: "W".into(),
        };
        assert_eq!(
            subtitle_for("Notification", &emoji).as_deref(),
            Some("W Waiting for input")
        );
        assert_eq!(subtitle_for("Stop", &emoji).as_deref(), Some("I Finished"));
        assert_eq!(subtitle_for("SessionStart", &emoji), None);
    }

    #[test]
    fn subtitle_omits_prefix_for_empty_emoji() {
        let emoji = crate::config::StatusEmoji {
            busy: String::new(),
            idle: String::new(),
            waiting: String::new(),
        };
        assert_eq!(
            subtitle_for("Notification", &emoji).as_deref(),
            Some("Waiting for input")
        );
    }

    #[test]
    fn session_label_is_repo_slash_worktree() {
        // Worktree layout `.../worktrees/<repo>/<name>/<repo>` -> `<repo>/<name>`.
        let cwd = Path::new("/home/u/src/worktrees/ztx/push-notification/ztx");
        assert_eq!(session_label(cwd), "ztx/push-notification");
    }

    #[test]
    fn shell_quote_wraps_and_escapes() {
        assert_eq!(shell_quote("/a/b"), "'/a/b'");
        assert_eq!(shell_quote("/a b/c"), "'/a b/c'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }
}
