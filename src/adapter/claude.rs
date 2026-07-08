//! Claude Code adapter.
//!
//! Claude Code maintains a per-process session registry at
//! `~/.claude/sessions/<pid>.json` (undocumented; observed in v2.1.x):
//!
//! ```json
//! {"pid":10302,"sessionId":"...","cwd":"/path","startedAt":1783169354502,
//!  "name":"dotfiles-c0","nameSource":"derived","status":"idle", ...}
//! ```
//!
//! The terminal title is built as `{status emoji} {worktree name}`: `status`
//! (`busy`/`idle`) selects the emoji, and the worktree name is derived from
//! `cwd`. Claude's own `name` (a `{repo}-{random}` slug) is intentionally not
//! used — it carries no useful information. Everything here is best-effort:
//! when the registry is missing or the schema changed, the adapter returns
//! `None` and the caller falls back to the child's own OSC titles.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;

use super::Adapter;
use crate::config::StatusEmoji;

#[derive(Debug, Deserialize)]
struct SessionMeta {
    pid: Option<u32>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    cwd: Option<String>,
    /// Claude's own activity flag; observed values are `"busy"` and `"idle"`
    /// (absent right after startup). Drives the title's status emoji.
    status: Option<String>,
}

pub struct ClaudeCodeAdapter {
    sessions_dir: PathBuf,
    projects_dir: PathBuf,
    child_pid: Option<u32>,
    cwd: PathBuf,
    started_at: SystemTime,
    /// The worktree name is derived from `cwd`, which never changes for a
    /// session, so it is computed once and reused on every poll.
    cached_title: Option<String>,
    /// Emoji prefixes for the `busy`/`idle` states, from user config.
    status_emoji: StatusEmoji,
}

impl ClaudeCodeAdapter {
    pub fn from_env(child_pid: Option<u32>, status_emoji: StatusEmoji) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let claude_dir = PathBuf::from(home).join(".claude");
        Self::new(
            claude_dir.join("sessions"),
            claude_dir.join("projects"),
            child_pid,
            status_emoji,
        )
    }

    /// Constructor for the standalone `export` subcommand: no child process
    /// exists, so sessions are matched purely by cwd regardless of age. The
    /// title (and thus the emoji) is never emitted here, so defaults suffice.
    pub fn for_export() -> Self {
        let mut adapter = Self::from_env(None, StatusEmoji::default());
        adapter.started_at = SystemTime::UNIX_EPOCH;
        adapter
    }

    pub fn new(
        sessions_dir: PathBuf,
        projects_dir: PathBuf,
        child_pid: Option<u32>,
        status_emoji: StatusEmoji,
    ) -> Self {
        Self {
            sessions_dir,
            projects_dir,
            child_pid,
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            started_at: SystemTime::now(),
            cached_title: None,
            status_emoji,
        }
    }

    /// Maps Claude's `status` field to the configured title prefix emoji.
    /// Unknown/absent statuses — and emojis configured to an empty string —
    /// yield no prefix (the bare worktree name is shown).
    fn status_prefix(&self, status: Option<&str>) -> Option<&str> {
        let emoji = match status {
            Some("busy") => self.status_emoji.busy.as_str(),
            Some("idle") => self.status_emoji.idle.as_str(),
            _ => return None,
        };
        (!emoji.is_empty()).then_some(emoji)
    }

    /// Finds this session's metadata. Match order (see spike results):
    /// 1. registry pid equals the direct child pid
    /// 2. registry cwd equals ours and the file appeared after we started
    ///    (claude may run behind a shim, so the pid does not always match)
    fn find_session(&self) -> Option<SessionMeta> {
        let entries = std::fs::read_dir(&self.sessions_dir).ok()?;
        let mut cwd_match: Option<(SystemTime, SessionMeta)> = None;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let Some(meta) = read_meta(&path) else {
                continue;
            };

            if self.child_pid.is_some() && meta.pid == self.child_pid {
                return Some(meta);
            }

            if meta.cwd.as_deref() == self.cwd.to_str() {
                let modified = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                if modified >= self.started_at
                    && cwd_match.as_ref().is_none_or(|(t, _)| modified > *t)
                {
                    cwd_match = Some((modified, meta));
                }
            }
        }
        cwd_match.map(|(_, meta)| meta)
    }

    /// The worktree name shown as the title body, computed once from `cwd`.
    fn worktree_title(&mut self) -> String {
        if let Some(title) = &self.cached_title {
            return title.clone();
        }
        let title = derive_title(&self.cwd);
        self.cached_title = Some(title.clone());
        title
    }
}

impl Adapter for ClaudeCodeAdapter {
    fn current_activity(&mut self) -> Option<String> {
        // No matching session -> nothing better than the child's own title.
        let meta = self.find_session()?;
        let title = self.worktree_title();
        if title.is_empty() {
            return None;
        }
        Some(match self.status_prefix(meta.status.as_deref()) {
            Some(emoji) => format!("{emoji} {title}"),
            None => title,
        })
    }

    fn transcript_path(&mut self) -> Option<PathBuf> {
        let session_id = self.find_session()?.session_id?;
        let slug = project_slug(&self.cwd);
        let path = self
            .projects_dir
            .join(slug)
            .join(format!("{session_id}.jsonl"));
        path.exists().then_some(path)
    }
}

fn read_meta(path: &Path) -> Option<SessionMeta> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Claude Code stores transcripts under a directory named after the project
/// cwd with every path separator (and dot) replaced by `-`.
fn project_slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect()
}

/// Derives the title body from `cwd`: the worktree name for a worktree
/// checkout, otherwise the git branch, otherwise the directory basename.
fn derive_title(cwd: &Path) -> String {
    worktree_name(cwd)
        .or_else(|| git_branch(cwd))
        .unwrap_or_else(|| basename(cwd))
}

/// Extracts the worktree name for the layout `.../worktrees/<repo>/<name>/<repo>`:
/// the leaf basename just repeats the repo, so the worktree name is the parent
/// directory. Returns `None` when `cwd` is not under a `worktrees/` tree.
fn worktree_name(cwd: &Path) -> Option<String> {
    let under_worktrees = cwd
        .components()
        .any(|c| c.as_os_str() == std::ffi::OsStr::new("worktrees"));
    if !under_worktrees {
        return None;
    }
    let name = cwd.parent()?.file_name()?.to_string_lossy().into_owned();
    (!name.is_empty()).then_some(name)
}

/// Current git branch of `cwd` (`None` when not a repo or on a detached HEAD).
fn git_branch(cwd: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!branch.is_empty() && branch != "HEAD").then_some(branch)
}

/// Last path component of `cwd` as a `String` (final fallback title).
fn basename(cwd: &Path) -> String {
    cwd.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_session(dir: &Path, file: &str, json: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(file), json).unwrap();
    }

    fn adapter_with(dir: &tempfile::TempDir, child_pid: Option<u32>) -> ClaudeCodeAdapter {
        ClaudeCodeAdapter::new(
            dir.path().join("sessions"),
            dir.path().join("projects"),
            child_pid,
            StatusEmoji::default(),
        )
    }

    const WORKTREE_CWD: &str = "/home/me/worktrees/nature-server/elder-reef/nature-server";

    #[test]
    fn busy_session_matched_by_pid_shows_emoji_and_worktree() {
        let dir = tempfile::tempdir().unwrap();
        write_session(
            &dir.path().join("sessions"),
            "42.json",
            r#"{"pid":42,"sessionId":"s-1","cwd":"/elsewhere","status":"busy"}"#,
        );
        let mut adapter = adapter_with(&dir, Some(42));
        adapter.cwd = PathBuf::from(WORKTREE_CWD);
        assert_eq!(adapter.current_activity().as_deref(), Some("🔄 elder-reef"));
    }

    #[test]
    fn idle_status_maps_to_hourglass() {
        let dir = tempfile::tempdir().unwrap();
        write_session(
            &dir.path().join("sessions"),
            "42.json",
            r#"{"pid":42,"sessionId":"s-1","cwd":"/elsewhere","status":"idle"}"#,
        );
        let mut adapter = adapter_with(&dir, Some(42));
        adapter.cwd = PathBuf::from(WORKTREE_CWD);
        assert_eq!(adapter.current_activity().as_deref(), Some("⏳ elder-reef"));
    }

    #[test]
    fn missing_status_yields_bare_worktree_name() {
        let dir = tempfile::tempdir().unwrap();
        write_session(
            &dir.path().join("sessions"),
            "42.json",
            r#"{"pid":42,"sessionId":"s-1","cwd":"/elsewhere"}"#,
        );
        let mut adapter = adapter_with(&dir, Some(42));
        adapter.cwd = PathBuf::from(WORKTREE_CWD);
        assert_eq!(adapter.current_activity().as_deref(), Some("elder-reef"));
    }

    #[test]
    fn falls_back_to_cwd_match() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = PathBuf::from(WORKTREE_CWD);
        write_session(
            &dir.path().join("sessions"),
            "7.json",
            &format!(r#"{{"pid":7,"sessionId":"s-2","cwd":{cwd:?},"status":"busy"}}"#),
        );
        // started_at is rewound because the session file above predates the
        // adapter within this test.
        let mut adapter = adapter_with(&dir, Some(999_999));
        adapter.cwd = cwd;
        adapter.started_at = SystemTime::UNIX_EPOCH;
        assert_eq!(adapter.current_activity().as_deref(), Some("🔄 elder-reef"));
    }

    #[test]
    fn worktree_name_uses_parent_of_repeated_leaf() {
        assert_eq!(
            worktree_name(Path::new(WORKTREE_CWD)).as_deref(),
            Some("elder-reef")
        );
    }

    #[test]
    fn worktree_name_is_none_outside_worktrees_tree() {
        assert_eq!(worktree_name(Path::new("/home/me/src/agent-skills")), None);
    }

    #[test]
    fn status_prefix_maps_known_values_only() {
        let dir = tempfile::tempdir().unwrap();
        let adapter = adapter_with(&dir, Some(1));
        assert_eq!(adapter.status_prefix(Some("busy")), Some("🔄"));
        assert_eq!(adapter.status_prefix(Some("idle")), Some("⏳"));
        assert_eq!(adapter.status_prefix(Some("something-new")), None);
        assert_eq!(adapter.status_prefix(None), None);
    }

    #[test]
    fn configured_emoji_overrides_defaults() {
        let dir = tempfile::tempdir().unwrap();
        write_session(
            &dir.path().join("sessions"),
            "42.json",
            r#"{"pid":42,"sessionId":"s-1","cwd":"/elsewhere","status":"busy"}"#,
        );
        let mut adapter = ClaudeCodeAdapter::new(
            dir.path().join("sessions"),
            dir.path().join("projects"),
            Some(42),
            StatusEmoji {
                busy: "🚀".into(),
                idle: "💤".into(),
            },
        );
        adapter.cwd = PathBuf::from(WORKTREE_CWD);
        assert_eq!(adapter.current_activity().as_deref(), Some("🚀 elder-reef"));
    }

    #[test]
    fn empty_configured_emoji_yields_bare_title() {
        let dir = tempfile::tempdir().unwrap();
        write_session(
            &dir.path().join("sessions"),
            "42.json",
            r#"{"pid":42,"sessionId":"s-1","cwd":"/elsewhere","status":"busy"}"#,
        );
        let mut adapter = ClaudeCodeAdapter::new(
            dir.path().join("sessions"),
            dir.path().join("projects"),
            Some(42),
            StatusEmoji {
                busy: String::new(),
                idle: String::new(),
            },
        );
        adapter.cwd = PathBuf::from(WORKTREE_CWD);
        assert_eq!(adapter.current_activity().as_deref(), Some("elder-reef"));
    }

    #[test]
    fn missing_registry_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let mut adapter = adapter_with(&dir, Some(1));
        assert_eq!(adapter.current_activity(), None);
        assert_eq!(adapter.transcript_path(), None);
    }

    #[test]
    fn malformed_json_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write_session(&dir.path().join("sessions"), "9.json", "{not json");
        let mut adapter = adapter_with(&dir, Some(9));
        assert_eq!(adapter.current_activity(), None);
    }

    #[test]
    fn transcript_path_resolves_project_slug() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = std::env::current_dir().unwrap();
        write_session(
            &dir.path().join("sessions"),
            "42.json",
            &format!(r#"{{"pid":42,"sessionId":"abc-123","cwd":{cwd:?},"name":"n"}}"#),
        );
        let slug = project_slug(&cwd);
        let project_dir = dir.path().join("projects").join(&slug);
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(project_dir.join("abc-123.jsonl"), "{}").unwrap();

        let mut adapter = adapter_with(&dir, Some(42));
        assert_eq!(
            adapter.transcript_path(),
            Some(project_dir.join("abc-123.jsonl"))
        );
    }

    #[test]
    fn project_slug_replaces_separators_and_dots() {
        assert_eq!(
            project_slug(Path::new("/Users/me/src/github.com/x/repo")),
            "-Users-me-src-github-com-x-repo"
        );
    }
}
