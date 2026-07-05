//! Hint mode (feature 2, keyboard path): pick a file path out of the recent
//! session log and open it in the editor, tmux-thumbs style.
//!
//! Flow: `ctrl-] f` → the stdin thread takes the stdout gate (pausing the
//! output pump), switches to the alternate screen, lists path candidates with
//! home-row labels, reads a label from the keyboard, restores the screen, and
//! opens the chosen location via `zed <path>:<line>`.

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

/// Bounded worktree scan: entry cap and depth cap keep hint mode fast even
/// in large repositories.
const INDEX_MAX_ENTRIES: usize = 20_000;
const INDEX_MAX_DEPTH: usize = 8;
/// Directories that are never worth indexing.
const INDEX_SKIP_DIRS: &[&str] = &[
    "target",
    "node_modules",
    "vendor",
    "dist",
    "build",
    "__pycache__",
];

/// Maps file basenames to worktree paths, so bare names mentioned in agent
/// prose ("term.rs") resolve to real files ("src/term.rs").
pub struct FileIndex {
    by_name: HashMap<String, Vec<PathBuf>>,
}

impl FileIndex {
    pub fn scan(root: &Path) -> Self {
        let mut by_name: HashMap<String, Vec<PathBuf>> = HashMap::new();
        let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
        queue.push_back((root.to_path_buf(), 0));
        let mut entries = 0usize;

        while let Some((dir, depth)) = queue.pop_front() {
            let Ok(read) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in read.flatten() {
                entries += 1;
                if entries > INDEX_MAX_ENTRIES {
                    return Self { by_name };
                }
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                if name.starts_with('.') {
                    continue;
                }
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_dir() {
                    if depth < INDEX_MAX_DEPTH && !INDEX_SKIP_DIRS.contains(&name.as_str()) {
                        queue.push_back((entry.path(), depth + 1));
                    }
                } else if file_type.is_file() {
                    by_name.entry(name).or_default().push(entry.path());
                }
            }
        }
        Self { by_name }
    }

    /// Resolves `token` (a relative path or a bare filename) to an indexed
    /// file. Tokens containing directories must match as a path suffix;
    /// bare names pick the shallowest match.
    fn resolve(&self, token: &str) -> Option<PathBuf> {
        let name = token.rsplit('/').next()?;
        let matches = self.by_name.get(name)?;
        if token.contains('/') {
            matches.iter().find(|p| p.ends_with(token)).cloned()
        } else {
            matches
                .iter()
                .min_by_key(|p| p.components().count())
                .cloned()
        }
    }
}

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
    RE.get_or_init(|| Regex::new(r#"File "(?P<p>[^"]+)", line (?P<l>\d+)"#).expect("static regex"))
}

/// Extracts existing file paths from `lines` (scanned bottom-up so the most
/// recent mention comes first), resolving relative paths against `cwd`.
/// Bare filenames and partial paths fall back to a worktree index, built
/// lazily on the first miss.
pub fn extract_candidates(lines: &[String], cwd: &Path, limit: usize) -> Vec<Candidate> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut index: Option<FileIndex> = None;

    for line in lines.iter().rev() {
        for (display, path_text, lineno, col) in matches_in_line(line) {
            let resolved = resolve(&path_text, cwd).or_else(|| {
                index
                    .get_or_insert_with(|| FileIndex::scan(cwd))
                    .resolve(&path_text)
            });
            let Some(resolved) = resolved else {
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

/// Input source for overlays. `poll_readable` lets [`read_key`] distinguish
/// a bare ESC key press from an ESC-prefixed sequence (arrow keys, mouse
/// reports) without blocking forever.
pub trait HintInput: Read {
    /// Whether a byte can be read within a short window (~50ms).
    fn poll_readable(&self) -> bool {
        true
    }
}

impl HintInput for &[u8] {}

impl HintInput for std::io::StdinLock<'_> {
    fn poll_readable(&self) -> bool {
        let mut fd = libc::pollfd {
            fd: 0, // stdin
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: polling a single valid pollfd for stdin.
        unsafe { libc::poll(&mut fd, 1, 50) > 0 }
    }
}

enum Key {
    Byte(u8),
    Esc,
    /// An escape sequence (arrow key, function key, mouse report): ignored.
    Ignored,
    Eof,
}

/// Reads one logical key, consuming (and discarding) escape sequences so
/// that mouse reports or arrow keys never masquerade as key presses.
fn read_key(stdin: &mut impl HintInput) -> io::Result<Key> {
    let mut byte = [0u8; 1];
    if stdin.read(&mut byte)? == 0 {
        return Ok(Key::Eof);
    }
    if byte[0] != 0x1b {
        return Ok(Key::Byte(byte[0]));
    }
    if !stdin.poll_readable() {
        return Ok(Key::Esc); // a lone ESC key press
    }
    if stdin.read(&mut byte)? == 0 {
        return Ok(Key::Esc);
    }
    match byte[0] {
        // CSI: consume until the final byte (0x40..=0x7E). Covers cursor
        // keys and SGR mouse reports (final 'M'/'m').
        b'[' => loop {
            if stdin.read(&mut byte)? == 0 {
                return Ok(Key::Esc);
            }
            if (0x40..=0x7e).contains(&byte[0]) {
                return Ok(Key::Ignored);
            }
        },
        // SS3 (e.g. F1-F4): one more byte.
        b'O' => {
            let _ = stdin.read(&mut byte)?;
            Ok(Key::Ignored)
        }
        // Alt-modified key: ignore both bytes.
        _ => Ok(Key::Ignored),
    }
}

/// Temporarily disables the child's mouse-tracking modes for the duration of
/// an overlay (mouse motion would otherwise flood stdin), restoring them on
/// drop... callers re-enable explicitly since drop cannot write.
fn set_mouse_modes(stdout: &mut impl Write, modes: &[u16], on: bool) -> io::Result<()> {
    let toggle = if on { 'h' } else { 'l' };
    for mode in modes {
        stdout.write_all(format!("\x1b[?{mode}{toggle}").as_bytes())?;
    }
    Ok(())
}

/// Runs the interactive overlay: draws candidates, reads a label, restores
/// the screen. Returns the selected candidate, or `None` on cancel.
///
/// The caller must hold the stdout gate for the whole call so the output pump
/// cannot repaint over the overlay. `mouse_modes` is the child's active
/// mouse-tracking set (disabled while the overlay is up).
pub fn pick(
    stdin: &mut impl HintInput,
    stdout: &mut impl Write,
    candidates: &[Candidate],
    mouse_modes: &[u16],
) -> io::Result<Option<usize>> {
    let rows = crossterm::terminal::size()
        .map(|(_, r)| r as usize)
        .unwrap_or(24);
    // Single-key labels only: with mixed lengths, a one-key label would
    // shadow every two-key label sharing its prefix.
    let visible = candidates
        .len()
        .min(rows.saturating_sub(2))
        .min(LABEL_KEYS.len())
        .max(1);
    let labels: Vec<String> = (0..visible).map(label).collect();

    // Enter alternate screen, pause mouse reporting, hide cursor, draw.
    set_mouse_modes(stdout, mouse_modes, false)?;
    stdout.write_all(b"\x1b[?1049h\x1b[H\x1b[2J\x1b[?25l")?;
    stdout.write_all(b"zediator \xe2\x80\x94 open file (press label, ESC to cancel)\r\n\r\n")?;
    for (i, candidate) in candidates.iter().take(visible).enumerate() {
        // `display` already carries any :line:col suffix as it appeared.
        stdout.write_all(
            format!(
                "\x1b[1;33m{:>2}\x1b[0m  {}\r\n",
                labels[i], candidate.display
            )
            .as_bytes(),
        )?;
    }
    stdout.flush()?;

    let selection = read_selection(stdin, &labels);

    // Restore the child's screen and its mouse-tracking modes.
    stdout.write_all(b"\x1b[?25h\x1b[?1049l")?;
    set_mouse_modes(stdout, mouse_modes, true)?;
    stdout.flush()?;
    selection
}

/// Shows a one-line message on the alternate screen and waits for a real key
/// (escape sequences such as mouse reports are ignored). Used so hint mode
/// never returns silently. The caller must hold the stdout gate, exactly as
/// for [`pick`].
pub fn show_message(
    stdin: &mut impl HintInput,
    stdout: &mut impl Write,
    message: &str,
    mouse_modes: &[u16],
) -> io::Result<()> {
    set_mouse_modes(stdout, mouse_modes, false)?;
    stdout.write_all(b"\x1b[?1049h\x1b[H\x1b[2J\x1b[?25l")?;
    stdout.write_all(message.as_bytes())?;
    stdout.write_all(b"\r\n")?;
    stdout.flush()?;
    // Wait for a real key; skip escape sequences (mouse reports, arrows).
    while matches!(read_key(stdin)?, Key::Ignored) {}
    stdout.write_all(b"\x1b[?25h\x1b[?1049l")?;
    set_mouse_modes(stdout, mouse_modes, true)?;
    stdout.flush()
}

fn read_selection(stdin: &mut impl HintInput, labels: &[String]) -> io::Result<Option<usize>> {
    let mut typed = String::new();
    loop {
        match read_key(stdin)? {
            Key::Ignored => continue,
            Key::Esc | Key::Eof => return Ok(None),
            Key::Byte(0x03 | 0x1d | b'q') => return Ok(None), // ctrl-c / prefix / q
            Key::Byte(b) => {
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
        let picked = pick(&mut input, &mut output, &candidates, &[]).unwrap();
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
        assert_eq!(
            pick(&mut input, &mut output, &candidates, &[]).unwrap(),
            None
        );
    }

    #[test]
    fn pick_ignores_mouse_reports_and_arrow_keys() {
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
                line: None,
                column: None,
            },
        ];
        // SGR mouse motion, an arrow key, then the actual selection.
        let mut input: &[u8] = b"\x1b[<35;10;5M\x1b[Bs";
        let mut output = Vec::new();
        assert_eq!(
            pick(&mut input, &mut output, &candidates, &[]).unwrap(),
            Some(1)
        );
    }

    #[test]
    fn pick_toggles_child_mouse_modes() {
        let candidates = vec![Candidate {
            display: "x".into(),
            path: PathBuf::from("/tmp"),
            line: None,
            column: None,
        }];
        let mut input: &[u8] = b"q";
        let mut output = Vec::new();
        pick(&mut input, &mut output, &candidates, &[1002, 1006]).unwrap();
        let drawn = String::from_utf8_lossy(&output);
        assert!(drawn.contains("\x1b[?1002l"));
        assert!(drawn.contains("\x1b[?1006l"));
        assert!(drawn.contains("\x1b[?1002h"));
        assert!(drawn.contains("\x1b[?1006h"));
    }

    #[test]
    fn show_message_survives_mouse_noise() {
        let mut input: &[u8] = b"\x1b[<35;1;1M\x1b[<35;2;2Mx";
        let mut output = Vec::new();
        show_message(&mut input, &mut output, "hello", &[]).unwrap();
        // Only the real key press ends the message; two events were skipped.
        assert!(input.is_empty());
    }

    #[test]
    fn bare_filename_resolves_via_worktree_index() {
        let (_dir, cwd) = fixture();
        // "main.rs" does not exist at the cwd root; the index finds src/main.rs.
        let lines = vec!["as described in main.rs the flow is...".to_owned()];
        let found = extract_candidates(&lines, &cwd, 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, cwd.join("src/main.rs"));
    }

    #[test]
    fn partial_path_resolves_by_suffix() {
        let (_dir, cwd) = fixture();
        std::fs::create_dir_all(cwd.join("deep/adapter")).unwrap();
        std::fs::write(cwd.join("deep/adapter/claude.rs"), "x").unwrap();
        let lines = vec!["see adapter/claude.rs:12".to_owned()];
        let found = extract_candidates(&lines, &cwd, 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, cwd.join("deep/adapter/claude.rs"));
        assert_eq!(found[0].line, Some(12));
    }

    #[test]
    fn index_skips_hidden_and_heavy_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        std::fs::write(dir.path().join("target/debug/ghost.rs"), "x").unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/config.rs"), "x").unwrap();
        let index = FileIndex::scan(dir.path());
        assert!(index.resolve("ghost.rs").is_none());
        assert!(index.resolve("config.rs").is_none());
    }
}
