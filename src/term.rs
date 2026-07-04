//! Output tap: observes the child's byte stream without modifying it.
//!
//! Design constraint (see DESIGN.md): zediator does not maintain a cell grid.
//! It keeps an ANSI-stripped line buffer of the primary screen plus a few
//! screen-state flags. This is enough for hint-mode path extraction and
//! fallback Markdown export, at a fraction of a terminal emulator's cost.

use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

use vte::{Params, Perform};

/// Maximum number of lines kept in memory.
const RING_CAPACITY: usize = 10_000;

/// Maximum bytes written to the spill file (64 MiB).
const SPILL_CAP_BYTES: u64 = 64 * 1024 * 1024;

/// State shared between the output pump thread (writer) and feature threads
/// (hint, export, title — readers).
pub struct TapShared {
    pub scrollback: Scrollback,
    /// Title most recently announced by the child via OSC 0/2.
    pub last_title: Option<String>,
    /// Whether the child is currently on the alternate screen.
    pub alt_screen: bool,
}

impl TapShared {
    fn new(ring_capacity: usize) -> Self {
        Self {
            scrollback: Scrollback::new(ring_capacity),
            last_title: None,
            alt_screen: false,
        }
    }
}

/// ANSI-stripped line history: a bounded in-memory ring with overflow spilled
/// to an anonymous temp file (deleted by the OS when the process exits).
pub struct Scrollback {
    ring: VecDeque<String>,
    capacity: usize,
    spill: Option<std::fs::File>,
    spill_written: u64,
    /// Lines dropped after the spill cap was reached.
    dropped: u64,
}

impl Scrollback {
    fn new(capacity: usize) -> Self {
        Self {
            ring: VecDeque::with_capacity(capacity.min(1024)),
            capacity,
            spill: None,
            spill_written: 0,
            dropped: 0,
        }
    }

    fn push(&mut self, line: String) {
        if self.ring.len() == self.capacity
            && let Some(evicted) = self.ring.pop_front()
        {
            self.spill_line(&evicted);
        }
        self.ring.push_back(line);
    }

    fn spill_line(&mut self, line: &str) {
        if self.spill_written >= SPILL_CAP_BYTES {
            self.dropped += 1;
            return;
        }
        if self.spill.is_none() {
            self.spill = tempfile::tempfile().ok();
        }
        if let Some(file) = self.spill.as_mut() {
            let bytes = line.len() as u64 + 1;
            if file
                .write_all(line.as_bytes())
                .and_then(|()| file.write_all(b"\n"))
                .is_ok()
            {
                self.spill_written += bytes;
            }
        } else {
            self.dropped += 1;
        }
    }

    /// Returns up to `n` most recent lines (oldest first).
    // TODO(step 6): consumed by hint mode; drop the allow once wired.
    #[allow(dead_code)]
    pub fn recent(&self, n: usize) -> Vec<String> {
        self.ring.iter().rev().take(n).rev().cloned().collect()
    }

    /// Returns the full captured history: spilled lines, then the ring.
    /// Notes how many lines were dropped when the spill cap was exceeded.
    // TODO(step 5): consumed by fallback export; drop the allow once wired.
    #[allow(dead_code)]
    pub fn dump(&mut self) -> std::io::Result<String> {
        let mut out = String::new();
        if self.dropped > 0 {
            out.push_str(&format!(
                "[zediator: {} earliest lines dropped ({} MiB spill cap)]\n",
                self.dropped,
                SPILL_CAP_BYTES / 1024 / 1024,
            ));
        }
        if let Some(file) = self.spill.as_mut() {
            file.seek(SeekFrom::Start(0))?;
            file.read_to_string(&mut out)?;
            file.seek(SeekFrom::End(0))?;
        }
        for line in &self.ring {
            out.push_str(line);
            out.push('\n');
        }
        Ok(out)
    }
}

/// Feeds child output bytes through a VTE parser into [`TapShared`].
/// Owned by the output pump thread; never blocks the passthrough for long.
pub struct TermTap {
    parser: vte::Parser,
    performer: Performer,
}

impl TermTap {
    pub fn new(shared: Arc<Mutex<TapShared>>) -> Self {
        Self {
            parser: vte::Parser::new(),
            performer: Performer::new(shared),
        }
    }

    pub fn shared(ring_capacity: Option<usize>) -> Arc<Mutex<TapShared>> {
        Arc::new(Mutex::new(TapShared::new(
            ring_capacity.unwrap_or(RING_CAPACITY),
        )))
    }

    pub fn advance(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.performer, bytes);
    }

    /// Commits a pending partial line, if any (call once when the child exits).
    pub fn flush(&mut self) {
        if !self.performer.line.is_empty() {
            self.performer.commit_line();
        }
    }
}

/// Line-oriented interpretation of the output stream.
///
/// Cursor handling is deliberately minimal: `\r` rewinds within the current
/// line (spinner redraws overwrite in place), `\n` commits, and erase-in-line
/// truncates. Vertical cursor motion is ignored — full-frame TUI redraws on
/// the primary screen may therefore appear as repeated blocks, which is
/// acceptable for hint extraction (deduplicated) and fallback export.
struct Performer {
    shared: Arc<Mutex<TapShared>>,
    line: Vec<char>,
    col: usize,
}

impl Performer {
    fn new(shared: Arc<Mutex<TapShared>>) -> Self {
        Self {
            shared,
            line: Vec::new(),
            col: 0,
        }
    }

    fn commit_line(&mut self) {
        let text: String = self.line.iter().collect();
        self.line.clear();
        self.col = 0;
        let mut shared = self.shared.lock().expect("tap lock poisoned");
        if shared.alt_screen {
            return;
        }
        shared.scrollback.push(text);
    }

    fn in_alt_screen(&self) -> bool {
        self.shared.lock().expect("tap lock poisoned").alt_screen
    }

    fn set_alt_screen(&mut self, on: bool) {
        let mut shared = self.shared.lock().expect("tap lock poisoned");
        shared.alt_screen = on;
        // Entering or leaving the alternate screen discards the partial line:
        // it belongs to the screen being switched away from.
        drop(shared);
        self.line.clear();
        self.col = 0;
    }
}

impl Perform for Performer {
    fn print(&mut self, c: char) {
        if self.in_alt_screen() {
            return;
        }
        if self.col < self.line.len() {
            self.line[self.col] = c;
        } else {
            self.line.push(c);
        }
        self.col += 1;
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' => self.commit_line(),
            b'\r' => self.col = 0,
            0x08 => self.col = self.col.saturating_sub(1), // backspace
            b'\t' => {
                // Expand to the next 8-column stop with spaces.
                let next = (self.col / 8 + 1) * 8;
                while self.col < next {
                    self.print(' ');
                }
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        let mut iter = params.iter();
        let first = iter.next().and_then(|p| p.first().copied()).unwrap_or(0);

        // DECSET/DECRST alternate screen (1047/1049, legacy 47).
        if intermediates.first() == Some(&b'?') && matches!(first, 47 | 1047 | 1049) {
            match action {
                'h' => self.set_alt_screen(true),
                'l' => self.set_alt_screen(false),
                _ => {}
            }
            return;
        }

        if action == 'K' && !self.in_alt_screen() {
            match first {
                0 => self.line.truncate(self.col), // erase to end of line
                1 => {
                    for i in 0..self.col.min(self.line.len()) {
                        self.line[i] = ' ';
                    }
                }
                2 => {
                    self.line.clear();
                    self.col = 0;
                }
                _ => {}
            }
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        // OSC 0 (icon+title) / OSC 2 (title): remember the child's title so
        // the title module can re-emit or compose it.
        if params.len() >= 2 && matches!(params[0], b"0" | b"2") {
            let title = String::from_utf8_lossy(params[1]).into_owned();
            self.shared.lock().expect("tap lock poisoned").last_title = Some(title);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(input: &[u8]) -> Arc<Mutex<TapShared>> {
        let shared = TermTap::shared(Some(8));
        let mut tap = TermTap::new(shared.clone());
        tap.advance(input);
        tap.flush();
        shared
    }

    fn lines(shared: &Arc<Mutex<TapShared>>) -> Vec<String> {
        let mut guard = shared.lock().unwrap();
        guard
            .scrollback
            .dump()
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn plain_lines_are_captured() {
        let shared = feed(b"hello\r\nworld\r\n");
        assert_eq!(lines(&shared), ["hello", "world"]);
    }

    #[test]
    fn sgr_colors_are_stripped() {
        let shared = feed(b"\x1b[1;31mred\x1b[0m text\n");
        assert_eq!(lines(&shared), ["red text"]);
    }

    #[test]
    fn carriage_return_overwrites_in_place() {
        // Spinner-style redraw: only the final frame survives.
        let shared = feed(b"| loading\r/ loading\r- done   \r\n");
        assert_eq!(lines(&shared), ["- done   "]);
    }

    #[test]
    fn erase_to_end_of_line_truncates() {
        let shared = feed(b"abcdef\r\x1b[Kxy\n");
        assert_eq!(lines(&shared), ["xy"]);
    }

    #[test]
    fn alternate_screen_content_is_ignored() {
        let shared = feed(b"before\n\x1b[?1049hall tui stuff\nmore\n\x1b[?1049lafter\n");
        assert_eq!(lines(&shared), ["before", "after"]);
    }

    #[test]
    fn osc_title_is_observed_and_stripped() {
        let shared = feed(b"\x1b]2;my session\x07visible\n");
        assert_eq!(lines(&shared), ["visible"]);
        assert_eq!(
            shared.lock().unwrap().last_title.as_deref(),
            Some("my session")
        );
    }

    #[test]
    fn ring_overflow_spills_to_file_preserving_order() {
        let shared = TermTap::shared(Some(4));
        let mut tap = TermTap::new(shared.clone());
        for i in 0..10 {
            tap.advance(format!("line-{i}\n").as_bytes());
        }
        let all = lines(&shared);
        let expected: Vec<String> = (0..10).map(|i| format!("line-{i}")).collect();
        assert_eq!(all, expected);
    }

    #[test]
    fn recent_returns_tail_in_order() {
        let shared = feed(b"a\nb\nc\nd\n");
        let recent = shared.lock().unwrap().scrollback.recent(2);
        assert_eq!(recent, ["c", "d"]);
    }

    #[test]
    fn tabs_expand_to_spaces() {
        let shared = feed(b"ab\tcd\n");
        assert_eq!(lines(&shared), ["ab      cd"]);
    }
}
