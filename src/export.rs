//! Markdown export (feature 3: "Open thread as Markdown" for terminal
//! sessions).
//!
//! Two quality tiers, per the adapter-plus-fallback design:
//! - **Adapter**: the CLI's native transcript (e.g. Claude Code's session
//!   JSONL) converted into structured Markdown.
//! - **Fallback**: the ANSI-stripped PTY scrollback. Readable in order, but
//!   with no role separation, and alternate-screen content is absent.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::term::TapShared;

/// Converts a Claude Code session transcript (JSONL) into Markdown.
///
/// The schema is undocumented and changes between versions, so unknown lines
/// are skipped rather than treated as errors.
pub fn transcript_to_markdown(transcript: &Path) -> io::Result<String> {
    let content = std::fs::read_to_string(transcript)?;
    let mut out = String::new();
    out.push_str(&format!(
        "# Session transcript\n\nSource: `{}`\n",
        transcript.display()
    ));

    for line in content.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("user") => render_message(&mut out, "User", &value),
            Some("assistant") => render_message(&mut out, "Assistant", &value),
            _ => {}
        }
    }
    Ok(out)
}

fn render_message(out: &mut String, role: &str, value: &Value) {
    let Some(content) = value.pointer("/message/content") else {
        return;
    };
    let mut body = String::new();
    match content {
        Value::String(text) => push_text(&mut body, text),
        Value::Array(blocks) => {
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(text) = block.get("text").and_then(Value::as_str) {
                            push_text(&mut body, text);
                        }
                    }
                    Some("tool_use") => {
                        let name = block.get("name").and_then(Value::as_str).unwrap_or("?");
                        let input = block
                            .get("input")
                            .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
                            .unwrap_or_default();
                        body.push_str(&format!("**Tool: {name}**\n\n```json\n{input}\n```\n\n"));
                    }
                    Some("tool_result") => {
                        // Tool results are often huge; keep a marker only.
                        body.push_str("*(tool result)*\n\n");
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    if !body.trim().is_empty() {
        out.push_str(&format!("\n## {role}\n\n{}", body.trim_end()));
        out.push('\n');
    }
}

fn push_text(body: &mut String, text: &str) {
    body.push_str(text.trim_end());
    body.push_str("\n\n");
}

/// Renders the PTY scrollback as fallback Markdown, plus a snapshot of the
/// alternate screen when the child currently lives there (full-screen CLIs
/// keep their history internally, so only the visible frame is available).
pub fn scrollback_to_markdown(shared: &Arc<Mutex<TapShared>>) -> String {
    let mut guard = shared.lock().expect("tap lock poisoned");
    let dump = guard.scrollback.dump().unwrap_or_default();
    let title = guard.last_title.clone();
    let alt_snapshot = guard.alt_snapshot.clone();
    drop(guard);

    let mut out = String::from("# Session log (terminal capture)\n\n");
    if let Some(title) = title {
        out.push_str(&format!("Session: {title}\n\n"));
    }
    out.push_str(
        "> Captured from terminal output. For full-screen TUIs only the \
         currently visible frame is available (see the last section).\n\n```text\n",
    );
    out.push_str(&dump);
    out.push_str("```\n");
    if !alt_snapshot.is_empty() {
        out.push_str("\n## Current screen\n\n```text\n");
        for line in &alt_snapshot {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("```\n");
    }
    out
}

/// Writes `content` to a timestamped Markdown file in the temp directory and
/// returns its path. The file is intentionally not auto-deleted: the editor
/// opens it asynchronously.
pub fn write_export(content: &str) -> io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("zediator");
    std::fs::create_dir_all(&dir)?;
    // Exports can contain conversation content; keep them owner-only.
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut file = tempfile::Builder::new()
        .prefix("session-")
        .suffix(".md")
        .disable_cleanup(true)
        .tempfile_in(&dir)?;
    file.write_all(content.as_bytes())?;
    Ok(file.path().to_path_buf())
}

/// Opens `path` in the user's editor without touching the wrapped terminal.
///
/// Resolution order: `$ZEDIATOR_EDITOR`, then `zed`, then `$EDITOR`.
/// GUI editors detach; terminal editors in `$EDITOR` would fight over the
/// TTY, so output is nulled and failures only logged.
pub fn open_in_editor(path: &Path) -> io::Result<()> {
    let editor = editor_command();
    let (program, args) = editor
        .split_first()
        .ok_or_else(|| io::Error::other("no editor available (set ZEDIATOR_EDITOR or EDITOR)"))?;
    std::process::Command::new(program)
        .args(args)
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}

/// Opens a specific location in the editor. Zed understands the
/// `path:line:column` form; other editors receive the bare path.
pub fn open_location(path: &Path, line: Option<u32>, column: Option<u32>) -> io::Result<()> {
    let editor = editor_command();
    let (program, args) = editor
        .split_first()
        .ok_or_else(|| io::Error::other("no editor available (set ZEDIATOR_EDITOR or EDITOR)"))?;
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    let is_zed = Path::new(program)
        .file_name()
        .is_some_and(|name| name == "zed");
    match (is_zed, line) {
        (true, Some(line)) => {
            let mut target = format!("{}:{line}", path.display());
            if let Some(column) = column {
                target.push_str(&format!(":{column}"));
            }
            cmd.arg(target);
        }
        _ => {
            cmd.arg(path);
        }
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}

fn editor_command() -> Vec<String> {
    if let Ok(cmd) = std::env::var("ZEDIATOR_EDITOR")
        && !cmd.is_empty()
    {
        return cmd.split_whitespace().map(str::to_owned).collect();
    }
    if which("zed") {
        return vec!["zed".into()];
    }
    if let Ok(cmd) = std::env::var("EDITOR")
        && !cmd.is_empty()
    {
        return cmd.split_whitespace().map(str::to_owned).collect();
    }
    Vec::new()
}

fn which(program: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(program).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_user_and_assistant_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(
            &path,
            concat!(
                r#"{"type":"user","message":{"content":"fix the bug"}}"#,
                "\n",
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Looking."},{"type":"tool_use","name":"Read","input":{"file":"a.rs"}}]}}"#,
                "\n",
                r#"{"type":"file-history-snapshot","irrelevant":true}"#,
                "\n",
                "not json at all\n",
            ),
        )
        .unwrap();

        let md = transcript_to_markdown(&path).unwrap();
        assert!(md.contains("## User\n\nfix the bug"));
        assert!(md.contains("## Assistant\n\nLooking."));
        assert!(md.contains("**Tool: Read**"));
        assert!(!md.contains("irrelevant"));
    }

    #[test]
    fn tool_results_render_as_marker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"huge"},{"type":"text","text":"and my question"}]}}"#,
        )
        .unwrap();
        let md = transcript_to_markdown(&path).unwrap();
        assert!(md.contains("*(tool result)*"));
        assert!(md.contains("and my question"));
        assert!(!md.contains("huge"));
    }

    #[test]
    fn write_export_creates_readable_file() {
        let path = write_export("# hello\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# hello\n");
        assert_eq!(path.extension().unwrap(), "md");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn scrollback_fallback_includes_primary_capture() {
        let shared = crate::term::TermTap::shared(Some(8));
        let mut tap = crate::term::TermTap::new(shared.clone());
        tap.advance(b"line one\nline two\n");
        let md = scrollback_to_markdown(&shared);
        assert!(md.contains("line one\nline two\n"));
        assert!(!md.contains("## Current screen"));
    }

    #[test]
    fn scrollback_fallback_appends_visible_alt_frame() {
        let shared = crate::term::TermTap::shared(Some(8));
        let mut tap = crate::term::TermTap::new(shared.clone());
        tap.advance(b"before\n\x1b[?1049h\x1b[Hvisible frame src/x.rs");
        let md = scrollback_to_markdown(&shared);
        assert!(md.contains("## Current screen"));
        assert!(md.contains("visible frame src/x.rs"));
    }
}
