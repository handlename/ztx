//! Hint mode (feature 2, keyboard path): pick a file path out of the recent
//! session log and open it in the editor, tmux-thumbs style.
//!
//! Flow: `ctrl-] f` → the stdin thread takes the stdout gate (pausing the
//! output pump), switches to the alternate screen, lists path candidates with
//! home-row labels, reads a label from the keyboard, restores the screen, and
//! opens the chosen location via `zed <path>:<line>`.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

/// A path found in the scrollback, most recent first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Path as it appeared in the log (for display).
    pub display: String,
    /// Absolute path (existence-checked).
    pub path: PathBuf,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

/// Keys used for hint labels, in assignment order.
const LABEL_KEYS: &[u8] = b"asdfghjklqwertyuiopzxcvbnm";

fn path_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // Multi-segment paths (contain `/`) or bare file names with an
        // extension, optionally followed by `:line[:col]`.
        Regex::new(
            r"(?x)
            (?P<p>
                (?:~|\.{1,2})?/?[\w@.+-]+(?:/[\w@.+-]+)+  # segment/segment...
              | [\w@+-][\w@.+-]*\.[A-Za-z0-9]{1,8}       # name.ext
            )
            (?: : (?P<l>\d+) (?: : (?P<c>\d+) )? )?
            ",
        )
        .expect("static regex")
    })
}

fn traceback_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"File "(?P<p>[^"]+)", line (?P<l>\d+)"#).expect("static regex")
    })
}

/// Extracts existing file paths from `lines` (scanned bottom-up so the most
/// recent mention comes first), resolving relative paths against `cwd`.
pub fn extract_candidates(lines: &[String], cwd: &Path, limit: usize) -> Vec<Candidate> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    for line in lines.iter().rev() {
        for (display, path_text, lineno, col) in matches_in_line(line) {
            let Some(resolved) = resolve(&path_text, cwd) else {
                continue;
            };
            if !seen.insert((resolved.clone(), lineno)) {
                continue;
            }
            out.push(Candidate {
                display,
                path: resolved,
                line: lineno,
                column: col,
            });
            if out.len() >= limit {
                return out;
            }
        }
    }
    out
}

fn matches_in_line(line: &str) -> Vec<(String, String, Option<u32>, Option<u32>)> {
    let mut found = Vec::new();
    for caps in traceback_regex().captures_iter(line) {
        let path = caps["p"].to_owned();
        let lineno = caps["l"].parse().ok();
        found.push((format!("{path}:{}", &caps["l"]), path, lineno, None));
    }
    for caps in path_regex().captures_iter(line) {
        let path = caps["p"].trim_end_matches(['.', ',', ')', ';']).to_owned();
        let lineno = caps.name("l").and_then(|m| m.as_str().parse().ok());
        let col = caps.name("c").and_then(|m| m.as_str().parse().ok());
        found.push((caps[0].to_owned(), path, lineno, col));
    }
    found
}

fn resolve(path_text: &str, cwd: &Path) -> Option<PathBuf> {
    let expanded = if let Some(rest) = path_text.strip_prefix("~/") {
        PathBuf::from(std::env::var("HOME").ok()?).join(rest)
    } else if path_text.starts_with('/') {
        PathBuf::from(path_text)
    } else {
        cwd.join(path_text)
    };
    expanded.is_file().then_some(expanded)
}

/// Assigns a unique label to each candidate index: single keys first, then
/// two-key combinations.
pub fn label(index: usize) -> String {
    let n = LABEL_KEYS.len();
    if index < n {
        (LABEL_KEYS[index] as char).to_string()
    } else {
        let index = index - n;
        format!(
            "{}{}",
            LABEL_KEYS[index / n] as char,
            LABEL_KEYS[index % n] as char
        )
    }
}

/// Runs the interactive overlay: draws candidates, reads a label, restores
/// the screen. Returns the selected candidate, or `None` on cancel.
///
/// The caller must hold the stdout gate for the whole call so the output pump
/// cannot repaint over the overlay.
pub fn pick(
    stdin: &mut impl Read,
    stdout: &mut impl Write,
    candidates: &[Candidate],
) -> io::Result<Option<usize>> {
    let rows = crossterm::terminal::size().map(|(_, r)| r as usize).unwrap_or(24);
    let visible = candidates.len().min(rows.saturating_sub(2)).max(1);
    let labels: Vec<String> = (0..visible).map(label).collect();

    // Enter alternate screen, hide cursor, draw.
    stdout.write_all(b"\x1b[?1049h\x1b[H\x1b[2J\x1b[?25l")?;
    stdout.write_all(b"zediator \xe2\x80\x94 open file (press label, ESC to cancel)\r\n\r\n")?;
    for (i, candidate) in candidates.iter().take(visible).enumerate() {
        // `display` already carries any :line:col suffix as it appeared.
        stdout.write_all(
            format!("\x1b[1;33m{:>2}\x1b[0m  {}\r\n", labels[i], candidate.display).as_bytes(),
        )?;
    }
    stdout.flush()?;

    let selection = read_selection(stdin, &labels);

    // Restore the child's screen.
    stdout.write_all(b"\x1b[?25h\x1b[?1049l")?;
    stdout.flush()?;
    selection
}

fn read_selection(stdin: &mut impl Read, labels: &[String]) -> io::Result<Option<usize>> {
    let mut typed = String::new();
    let mut byte = [0u8; 1];
    loop {
        if stdin.read(&mut byte)? == 0 {
            return Ok(None);
        }
        match byte[0] {
            0x1b | 0x03 | 0x1d | b'q' => return Ok(None), // ESC / ctrl-c / prefix / q
            b => {
                typed.push(b as char);
                if let Some(index) = labels.iter().position(|l| *l == typed) {
                    return Ok(Some(index));
                }
                if !labels.iter().any(|l| l.starts_with(&typed)) {
                    return Ok(None);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("script.py"), "pass").unwrap();
        let cwd = dir.path().to_path_buf();
        (dir, cwd)
    }

    #[test]
    fn extracts_path_with_line_and_column() {
        let (_dir, cwd) = fixture();
        let lines = vec!["error at src/main.rs:42:7 something".to_owned()];
        let found = extract_candidates(&lines, &cwd, 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, cwd.join("src/main.rs"));
        assert_eq!(found[0].line, Some(42));
        assert_eq!(found[0].column, Some(7));
    }

    #[test]
    fn extracts_python_traceback_location() {
        let (_dir, cwd) = fixture();
        let lines = vec![format!(
            r#"  File "{}/script.py", line 10, in <module>"#,
            cwd.display()
        )];
        let found = extract_candidates(&lines, &cwd, 10);
        assert!(found.iter().any(|c| c.line == Some(10)));
    }

    #[test]
    fn nonexistent_paths_are_dropped() {
        let (_dir, cwd) = fixture();
        let lines = vec!["see does/not/exist.rs:1 and src/main.rs".to_owned()];
        let found = extract_candidates(&lines, &cwd, 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, cwd.join("src/main.rs"));
    }

    #[test]
    fn recent_lines_come_first_and_dedup() {
        let (_dir, cwd) = fixture();
        let lines = vec![
            "old mention src/main.rs".to_owned(),
            "recent mention script.py".to_owned(),
            "again src/main.rs".to_owned(),
        ];
        let found = extract_candidates(&lines, &cwd, 10);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].path, cwd.join("src/main.rs"));
        assert_eq!(found[1].path, cwd.join("script.py"));
    }

    #[test]
    fn absolute_paths_resolve() {
        let (_dir, cwd) = fixture();
        let abs = cwd.join("script.py");
        let lines = vec![format!("wrote {}", abs.display())];
        let found = extract_candidates(&lines, &cwd, 10);
        assert_eq!(found[0].path, abs);
    }

    #[test]
    fn labels_are_unique() {
        let labels: Vec<String> = (0..60).map(label).collect();
        let unique: std::collections::HashSet<_> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len());
        assert_eq!(labels[0], "a");
    }

    #[test]
    fn pick_returns_selected_candidate() {
        let (_dir, cwd) = fixture();
        let candidates = vec![
            Candidate {
                display: "src/main.rs".into(),
                path: cwd.join("src/main.rs"),
                line: None,
                column: None,
            },
            Candidate {
                display: "script.py".into(),
                path: cwd.join("script.py"),
                line: Some(3),
                column: None,
            },
        ];
        let mut input: &[u8] = b"s";
        let mut output = Vec::new();
        let picked = pick(&mut input, &mut output, &candidates).unwrap();
        assert_eq!(picked, Some(1));
        let drawn = String::from_utf8_lossy(&output);
        assert!(drawn.contains("script.py"));
        assert!(drawn.contains("\x1b[?1049h"));
        assert!(drawn.contains("\x1b[?1049l"));
    }

    #[test]
    fn pick_cancels_on_escape() {
        let candidates = vec![Candidate {
            display: "x".into(),
            path: PathBuf::from("/tmp"),
            line: None,
            column: None,
        }];
        let mut input: &[u8] = b"\x1b";
        let mut output = Vec::new();
        assert_eq!(pick(&mut input, &mut output, &candidates).unwrap(), None);
    }
}
