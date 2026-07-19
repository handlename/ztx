//! antigravity-cli (`agy`) adapter.
//!
//! Observed storage layout under `~/.gemini/antigravity-cli/` (v1.0.x,
//! undocumented):
//! - `cache/last_conversations.json` — JSON map of workspace cwd to the most
//!   recent conversation id.
//! - `conversation_summaries.db` — SQLite table `conversation_summaries`
//!   with `conversation_id` and `title` columns.
//!
//! The conversation *content* lives in per-conversation SQLite files whose
//! payloads are opaque protobuf blobs, so this adapter provides titles only;
//! Markdown export intentionally stays at PTY-capture fallback quality.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::Adapter;

pub struct AntigravityAdapter {
    data_dir: PathBuf,
    cwd: PathBuf,
}

impl AntigravityAdapter {
    pub fn from_env() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        Self::new(PathBuf::from(home).join(".gemini/antigravity-cli"))
    }

    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    fn conversation_id(&self) -> Option<String> {
        let path = self.data_dir.join("cache/last_conversations.json");
        let content = std::fs::read_to_string(path).ok()?;
        let map: HashMap<String, String> = serde_json::from_str(&content).ok()?;
        map.get(self.cwd.to_str()?).cloned()
    }

    fn title_for(&self, conversation_id: &str) -> Option<String> {
        let db_path = self.data_dir.join("conversation_summaries.db");
        let conn = open_read_only(&db_path)?;
        conn.query_row(
            "SELECT title FROM conversation_summaries WHERE conversation_id = ?1",
            [conversation_id],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .filter(|title| !title.is_empty())
    }
}

fn open_read_only(path: &Path) -> Option<rusqlite::Connection> {
    rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
}

impl Adapter for AntigravityAdapter {
    fn current_activity(&mut self) -> Option<String> {
        let id = self.conversation_id()?;
        self.title_for(&id)
    }

    fn transcript_path(&mut self) -> Option<PathBuf> {
        // Conversation payloads are opaque protobuf blobs; no native
        // transcript is exposed, so export falls back to the PTY capture.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(cwd: &Path, title: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("cache")).unwrap();
        std::fs::write(
            dir.path().join("cache/last_conversations.json"),
            format!(r#"{{{:?}: "conv-1"}}"#, cwd.to_str().unwrap()),
        )
        .unwrap();
        let conn =
            rusqlite::Connection::open(dir.path().join("conversation_summaries.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE conversation_summaries (
                conversation_id TEXT PRIMARY KEY,
                title TEXT NOT NULL DEFAULT ''
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO conversation_summaries (conversation_id, title) VALUES (?1, ?2)",
            ("conv-1", title),
        )
        .unwrap();
        dir
    }

    #[test]
    fn resolves_title_for_current_directory() {
        let cwd = std::env::current_dir().unwrap();
        let dir = fixture(&cwd, "refactor auth flow");
        let mut adapter = AntigravityAdapter::new(dir.path().to_path_buf());
        assert_eq!(
            adapter.current_activity().as_deref(),
            Some("refactor auth flow")
        );
    }

    #[test]
    fn unknown_directory_yields_none() {
        let dir = fixture(Path::new("/somewhere/else"), "other");
        let mut adapter = AntigravityAdapter::new(dir.path().to_path_buf());
        assert_eq!(adapter.current_activity(), None);
    }

    #[test]
    fn empty_title_yields_none() {
        let cwd = std::env::current_dir().unwrap();
        let dir = fixture(&cwd, "");
        let mut adapter = AntigravityAdapter::new(dir.path().to_path_buf());
        assert_eq!(adapter.current_activity(), None);
    }

    #[test]
    fn missing_data_dir_yields_none() {
        let mut adapter = AntigravityAdapter::new(PathBuf::from("/nonexistent/ztx-test"));
        assert_eq!(adapter.current_activity(), None);
        assert_eq!(adapter.transcript_path(), None);
    }
}
