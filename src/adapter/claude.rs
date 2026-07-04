//! Claude Code adapter.
//!
//! Claude Code maintains a per-process session registry at
//! `~/.claude/sessions/<pid>.json` (undocumented; observed in v2.1.x):
//!
//! ```json
//! {"pid":10302,"sessionId":"...","cwd":"/path","startedAt":1783169354502,
//!  "name":"dotfiles-c0","nameSource":"derived", ...}
//! ```
//!
//! `name` is the auto-generated session title — exactly what feature 1 wants.
//! Everything here is best-effort: when the registry is missing or the schema
//! changed, the adapter returns `None` and the caller falls back to the
//! child's own OSC titles.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;

use super::Adapter;

#[derive(Debug, Deserialize)]
struct SessionMeta {
    pid: Option<u32>,
    // TODO(step 5): read by the export subcommand; drop the allow then.
    #[allow(dead_code)]
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    cwd: Option<String>,
    name: Option<String>,
}

pub struct ClaudeCodeAdapter {
    sessions_dir: PathBuf,
    // TODO(step 5): read by the export subcommand; drop the allow then.
    #[allow(dead_code)]
    projects_dir: PathBuf,
    child_pid: Option<u32>,
    cwd: PathBuf,
    started_at: SystemTime,
}

impl ClaudeCodeAdapter {
    pub fn from_env(child_pid: Option<u32>) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let claude_dir = PathBuf::from(home).join(".claude");
        Self::new(
            claude_dir.join("sessions"),
            claude_dir.join("projects"),
            child_pid,
        )
    }

    pub fn new(sessions_dir: PathBuf, projects_dir: PathBuf, child_pid: Option<u32>) -> Self {
        Self {
            sessions_dir,
            projects_dir,
            child_pid,
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            started_at: SystemTime::now(),
        }
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
}

impl Adapter for ClaudeCodeAdapter {
    fn current_activity(&mut self) -> Option<String> {
        let name = self.find_session()?.name?;
        if name.is_empty() { None } else { Some(name) }
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
// TODO(step 5): called via transcript_path from export; drop the allow then.
#[allow(dead_code)]
fn project_slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect()
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
        )
    }

    #[test]
    fn matches_session_by_child_pid() {
        let dir = tempfile::tempdir().unwrap();
        write_session(
            &dir.path().join("sessions"),
            "42.json",
            r#"{"pid":42,"sessionId":"s-1","cwd":"/elsewhere","name":"fix login bug"}"#,
        );
        let mut adapter = adapter_with(&dir, Some(42));
        assert_eq!(adapter.current_activity().as_deref(), Some("fix login bug"));
    }

    #[test]
    fn falls_back_to_cwd_match() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = std::env::current_dir().unwrap();
        write_session(
            &dir.path().join("sessions"),
            "7.json",
            &format!(r#"{{"pid":7,"sessionId":"s-2","cwd":{:?},"name":"by-cwd"}}"#, cwd),
        );
        // started_at is rewound because the session file above predates the
        // adapter within this test.
        let mut adapter = adapter_with(&dir, Some(999_999));
        adapter.started_at = SystemTime::UNIX_EPOCH;
        assert_eq!(adapter.current_activity().as_deref(), Some("by-cwd"));
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
