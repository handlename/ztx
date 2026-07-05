//! Output tap: observes the child's byte stream without modifying it.
//!
//! Design constraint (see DESIGN.md): zediator does not maintain a full
//! terminal emulator. It keeps two lightweight views of the child's output:
//!
//! - **Primary screen**: an ANSI-stripped, append-oriented line buffer
//!   ([`Scrollback`]). Ink-style bottom-region redraws (erase + cursor-up +
//!   rewrite) are treated as replacements so frames do not flood the history.
//! - **Alternate screen**: a bounded row grid ([`AltGrid`]) tracking what is
//!   *currently visible*. Full-screen agent CLIs (Claude Code 2.x runs on the
//!   alternate screen) keep their own scrollback internally, so the visible
//!   frame is all zediator can — and needs to — capture for hint mode and
//!   export snapshots.

use std::collections::VecDeque;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

use vte::{Params, Perform};

/// Maximum number of lines kept in memory.
const RING_CAPACITY: usize = 10_000;

/// Maximum bytes written to the spill file (64 MiB).
const SPILL_CAP_BYTES: u64 = 64 * 1024 * 1024;

/// Upper bound for alternate-screen rows kept (top rows scroll away).
const ALT_MAX_ROWS: usize = 500;

/// State shared between the output pump thread (writer) and feature threads
/// (hint, export, title — readers).
pub struct TapShared {
    pub scrollback: Scrollback,
    /// Title most recently announced by the child via OSC 0/2.
    pub last_title: Option<String>,
    /// Whether the child is currently on the alternate screen.
    pub alt_screen: bool,
    /// Text of the alternate screen as currently drawn (empty on primary).
    pub alt_snapshot: Vec<String>,
    /// Mouse-tracking DECSET modes the child has enabled (1000, 1002, 1006,
    /// ...). Overlays disable these temporarily so mouse motion does not
    /// flood stdin while zediator reads a key.
    pub mouse_modes: std::collections::BTreeSet<u16>,
    /// Terminal height in rows; kept current by the resize handler and used
    /// to size the alternate-screen grid.
    pub screen_rows: u16,
}

impl TapShared {
    fn new(ring_capacity: usize) -> Self {
        Self {
            scrollback: Scrollback::new(ring_capacity),
            last_title: None,
            alt_screen: false,
            alt_snapshot: Vec::new(),
            mouse_modes: std::collections::BTreeSet::new(),
            screen_rows: 24,
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
    pub fn recent(&self, n: usize) -> Vec<String> {
        self.ring.iter().rev().take(n).rev().cloned().collect()
    }

    /// Number of lines currently captured (ring only, spill excluded).
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// Removes the most recent line (used when the child redraws it).
    fn pop_last(&mut self) {
        self.ring.pop_back();
    }

    /// Returns the full captured history: spilled lines, then the ring.
    /// Notes how many lines were dropped when the spill cap was exceeded.
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
        // Publish the alternate-screen text once per chunk (not per char).
        self.performer.publish_alt_snapshot();
    }

    /// Commits a pending partial line, if any (call once when the child exits).
    pub fn flush(&mut self) {
        if self.performer.alt.is_none() && !self.performer.line.is_empty() {
            self.performer.commit_line();
        }
        self.performer.publish_alt_snapshot();
    }
}

/// Fixed-height row grid for the alternate screen, with DECSTBM scroll
/// regions. Full-screen agent CLIs scroll their content area with
/// `CSI t;b r` + `CSI n S/T` (observed from Claude Code), so scroll
/// operations must shift the grid or hint-label coordinates drift. Wide
/// characters occupy one cell (display columns are recomputed with
/// unicode-width at draw time).
struct AltGrid {
    rows: Vec<Vec<char>>,
    height: usize,
    row: usize,
    col: usize,
    /// Scroll region, 0-based inclusive margins (DECSTBM).
    margin_top: usize,
    margin_bottom: usize,
    /// Cursor saved by DECSC (ESC 7), restored by DECRC (ESC 8).
    saved_cursor: Option<(usize, usize)>,
}

impl AltGrid {
    fn new(height: usize) -> Self {
        let height = height.clamp(1, ALT_MAX_ROWS);
        Self {
            rows: vec![Vec::new(); height],
            height,
            row: 0,
            col: 0,
            margin_top: 0,
            margin_bottom: height - 1,
            saved_cursor: None,
        }
    }

    fn put(&mut self, c: char) {
        let line = &mut self.rows[self.row];
        while line.len() < self.col {
            line.push(' ');
        }
        if self.col < line.len() {
            line[self.col] = c;
        } else {
            line.push(c);
        }
        self.col += 1;
    }

    fn move_to(&mut self, row: usize, col: usize) {
        self.row = row.min(self.height - 1);
        self.col = col;
    }

    /// Line feed: scrolls the region when the cursor sits on its bottom
    /// margin, otherwise just moves down.
    fn line_feed(&mut self) {
        if self.row == self.margin_bottom {
            self.scroll_up(1);
        } else {
            self.row = (self.row + 1).min(self.height - 1);
        }
    }

    /// DECSTBM. Per spec the cursor moves to home afterwards.
    fn set_margins(&mut self, top: u16, bottom: u16) {
        let top = (top.max(1) as usize - 1).min(self.height - 1);
        let bottom = match bottom {
            0 => self.height - 1,
            b => (b as usize - 1).min(self.height - 1),
        };
        if top < bottom {
            self.margin_top = top;
            self.margin_bottom = bottom;
        } else {
            self.margin_top = 0;
            self.margin_bottom = self.height - 1;
        }
        self.row = 0;
        self.col = 0;
    }

    /// SU: shifts the scroll region up, dropping its top rows.
    fn scroll_up(&mut self, n: usize) {
        for _ in 0..n.min(self.margin_bottom - self.margin_top + 1) {
            self.rows.remove(self.margin_top);
            self.rows.insert(self.margin_bottom, Vec::new());
        }
    }

    /// SD: shifts the scroll region down, dropping its bottom rows.
    fn scroll_down(&mut self, n: usize) {
        for _ in 0..n.min(self.margin_bottom - self.margin_top + 1) {
            self.rows.remove(self.margin_bottom);
            self.rows.insert(self.margin_top, Vec::new());
        }
    }

    /// IL: inserts blank lines at the cursor, pushing rows toward the bottom
    /// margin.
    fn insert_lines(&mut self, n: usize) {
        if self.row < self.margin_top || self.row > self.margin_bottom {
            return;
        }
        for _ in 0..n.min(self.margin_bottom - self.row + 1) {
            self.rows.remove(self.margin_bottom);
            self.rows.insert(self.row, Vec::new());
        }
    }

    /// DL: deletes lines at the cursor, pulling rows up from the bottom
    /// margin.
    fn delete_lines(&mut self, n: usize) {
        if self.row < self.margin_top || self.row > self.margin_bottom {
            return;
        }
        for _ in 0..n.min(self.margin_bottom - self.row + 1) {
            self.rows.remove(self.row);
            self.rows.insert(self.margin_bottom, Vec::new());
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let col = self.col;
        let line = &mut self.rows[self.row];
        match mode {
            0 => line.truncate(col),
            1 => {
                // ECMA-48: erase from line start THROUGH the cursor.
                for i in 0..(col + 1).min(line.len()) {
                    line[i] = ' ';
                }
            }
            2 => line.clear(),
            _ => {}
        }
    }

    fn erase_display(&mut self, mode: u16) {
        match mode {
            0 => {
                self.erase_line(0);
                for line in &mut self.rows[self.row + 1..] {
                    line.clear();
                }
            }
            1 => {
                for line in &mut self.rows[..self.row] {
                    line.clear();
                }
                self.erase_line(1);
            }
            2 | 3 => {
                for line in &mut self.rows {
                    line.clear();
                }
            }
            _ => {}
        }
    }

    /// Current screen text, trailing blank rows trimmed.
    fn lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.rows.iter().map(|r| r.iter().collect()).collect();
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }
        lines
    }
}

/// Line-oriented interpretation of the output stream. See the module docs
/// for the primary/alternate split.
struct Performer {
    shared: Arc<Mutex<TapShared>>,
    /// Primary-screen partial line and cursor column.
    line: Vec<char>,
    col: usize,
    /// Alternate-screen grid, present while the child is on the alt screen.
    alt: Option<AltGrid>,
    alt_dirty: bool,
}

impl Performer {
    fn new(shared: Arc<Mutex<TapShared>>) -> Self {
        Self {
            shared,
            line: Vec::new(),
            col: 0,
            alt: None,
            alt_dirty: false,
        }
    }

    fn commit_line(&mut self) {
        let text: String = self.line.iter().collect();
        self.line.clear();
        self.col = 0;
        self.shared
            .lock()
            .expect("tap lock poisoned")
            .scrollback
            .push(text);
    }

    fn set_alt_screen(&mut self, on: bool) {
        // The primary partial line is kept: DECSET 1049 saves and restores
        // the primary screen, so output resumes where it left off.
        let mut shared = self.shared.lock().expect("tap lock poisoned");
        shared.alt_screen = on;
        let height = shared.screen_rows as usize;
        drop(shared);
        self.alt = on.then(|| AltGrid::new(height));
        self.alt_dirty = true;
        tracing::debug!(alt_screen = on, height, "screen mode changed");
    }

    fn publish_alt_snapshot(&mut self) {
        if !self.alt_dirty {
            return;
        }
        self.alt_dirty = false;
        let snapshot = self.alt.as_ref().map(AltGrid::lines).unwrap_or_default();
        self.shared.lock().expect("tap lock poisoned").alt_snapshot = snapshot;
    }
}

impl Perform for Performer {
    fn print(&mut self, c: char) {
        if let Some(grid) = self.alt.as_mut() {
            grid.put(c);
            self.alt_dirty = true;
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
        if let Some(grid) = self.alt.as_mut() {
            match byte {
                b'\n' => grid.line_feed(),
                b'\r' => grid.col = 0,
                0x08 => grid.col = grid.col.saturating_sub(1),
                b'\t' => {
                    let next = (grid.col / 8 + 1) * 8;
                    while grid.col < next {
                        grid.put(' ');
                    }
                }
                _ => return,
            }
            self.alt_dirty = true;
            return;
        }
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
        let second = iter.next().and_then(|p| p.first().copied()).unwrap_or(0);

        // DECSET/DECRST private modes. A single CSI can carry several codes
        // (e.g. `CSI ? 1002;1006 h`), so every parameter is inspected.
        if intermediates.first() == Some(&b'?') && matches!(action, 'h' | 'l') {
            let on = action == 'h';
            let codes: Vec<u16> = params.iter().flatten().copied().collect();
            for code in codes {
                match code {
                    // Alternate screen (1047/1049, legacy 47).
                    47 | 1047 | 1049 => self.set_alt_screen(on),
                    // Mouse tracking family.
                    9 | 1000..=1003 | 1005 | 1006 | 1015 | 1016 => {
                        let mut shared = self.shared.lock().expect("tap lock poisoned");
                        if on {
                            shared.mouse_modes.insert(code);
                        } else {
                            shared.mouse_modes.remove(&code);
                        }
                    }
                    _ => {}
                }
            }
            return;
        }
        if intermediates.first() == Some(&b'?') {
            return;
        }
        if !intermediates.is_empty() {
            return;
        }

        if let Some(grid) = self.alt.as_mut() {
            let n = first.max(1) as usize;
            match action {
                'H' | 'f' => grid.move_to(n - 1, (second.max(1) as usize) - 1),
                'A' => grid.row = grid.row.saturating_sub(n),
                'B' => grid.row = (grid.row + n).min(grid.height - 1),
                'C' => grid.col += n,
                'D' => grid.col = grid.col.saturating_sub(n),
                'E' => {
                    grid.row = (grid.row + n).min(grid.height - 1);
                    grid.col = 0;
                }
                'F' => {
                    grid.row = grid.row.saturating_sub(n);
                    grid.col = 0;
                }
                'G' => grid.col = n - 1,
                'd' => grid.row = (n - 1).min(grid.height - 1), // VPA
                'K' => grid.erase_line(first),
                'J' => grid.erase_display(first),
                'r' => grid.set_margins(first, second), // DECSTBM
                'S' => grid.scroll_up(n),
                'T' => grid.scroll_down(n),
                'L' => grid.insert_lines(n),
                'M' => grid.delete_lines(n),
                _ => return,
            }
            self.alt_dirty = true;
            return;
        }

        // Primary screen. Cursor up (CUU 'A') / cursor previous line
        // (CPL 'F'): ink-style TUIs redraw their bottom region every frame by
        // moving up over already-emitted lines and rewriting them. Treat
        // those lines as *replaced*, not appended — otherwise redraw frames
        // flood the scrollback and push real content out of the hint/export
        // window.
        if matches!(action, 'A' | 'F') {
            let n = first.max(1) as usize;
            let mut shared = self.shared.lock().expect("tap lock poisoned");
            for _ in 0..n {
                shared.scrollback.pop_last();
            }
            drop(shared);
            self.line.clear();
            self.col = 0;
            return;
        }

        if action == 'K' {
            match first {
                0 => self.line.truncate(self.col), // erase to end of line
                1 => {
                    // ECMA-48: erase from line start THROUGH the cursor.
                    for i in 0..(self.col + 1).min(self.line.len()) {
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

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        // DECSC/DECRC: Claude Code saves and restores the cursor around its
        // partial redraws; without this the tracked cursor drifts.
        if !intermediates.is_empty() {
            return;
        }
        if let Some(grid) = self.alt.as_mut() {
            match byte {
                b'7' => grid.saved_cursor = Some((grid.row, grid.col)),
                b'8' => {
                    if let Some((row, col)) = grid.saved_cursor {
                        grid.row = row.min(grid.height - 1);
                        grid.col = col;
                        self.alt_dirty = true;
                    }
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
        feed_with_rows(24, input)
    }

    fn feed_with_rows(rows: u16, input: &[u8]) -> Arc<Mutex<TapShared>> {
        let shared = TermTap::shared(Some(8));
        shared.lock().unwrap().screen_rows = rows;
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

    fn snapshot(shared: &Arc<Mutex<TapShared>>) -> Vec<String> {
        shared.lock().unwrap().alt_snapshot.clone()
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
    fn ink_style_redraw_replaces_lines_instead_of_appending() {
        // Frame 1 commits three lines; the TUI then erases the bottom two
        // (erase-line + cursor-up pairs) and draws frame 2 in their place.
        let shared = feed(
            b"content src/main.rs\nspinner A\nbox B\n\
              \x1b[2K\x1b[1A\x1b[2K\x1b[1A\x1b[2K\
              spinner C\nbox D\n",
        );
        assert_eq!(
            lines(&shared),
            ["content src/main.rs", "spinner C", "box D"]
        );
    }

    #[test]
    fn cursor_up_with_count_pops_that_many_lines() {
        let shared = feed(b"keep\na\nb\nc\n\x1b[3Anew\n");
        assert_eq!(lines(&shared), ["keep", "new"]);
    }

    #[test]
    fn erase_to_line_start_includes_cursor_column() {
        // After overwriting "XY" the cursor sits on column 2 ('c');
        // EL 1 erases columns 0..=2 inclusive.
        let shared = feed(b"abcdef\rXY\x1b[1K\n");
        assert_eq!(lines(&shared), ["   def"]);
    }

    #[test]
    fn alternate_screen_content_stays_out_of_scrollback() {
        let shared = feed(b"before\n\x1b[?1049hall tui stuff\nmore\n\x1b[?1049lafter\n");
        assert_eq!(lines(&shared), ["before", "after"]);
    }

    #[test]
    fn alt_screen_snapshot_captures_visible_text() {
        let shared = feed(b"\x1b[?1049h\x1b[2J\x1b[Hfirst src/main.rs\r\nsecond\x1b[3;1Hthird");
        assert_eq!(snapshot(&shared), ["first src/main.rs", "second", "third"]);
        assert!(shared.lock().unwrap().alt_screen);
    }

    #[test]
    fn alt_screen_full_redraw_replaces_frame() {
        let shared = feed(
            b"\x1b[?1049h\x1b[Hframe one line\x1b[H\x1b[2Jframe two src/pty.rs\x1b[2;1Hstatus",
        );
        assert_eq!(snapshot(&shared), ["frame two src/pty.rs", "status"]);
    }

    #[test]
    fn leaving_alt_screen_clears_snapshot() {
        let shared = feed(b"\x1b[?1049h\x1b[Htransient\x1b[?1049lback\n");
        assert!(snapshot(&shared).is_empty());
        assert!(!shared.lock().unwrap().alt_screen);
        assert_eq!(lines(&shared), ["back"]);
    }

    #[test]
    fn alt_screen_erase_below_truncates_rows() {
        let shared = feed(b"\x1b[?1049h\x1b[Ha\r\nb\r\nc\x1b[2;1H\x1b[0J");
        assert_eq!(snapshot(&shared), ["a"]);
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

    #[test]
    fn decstbm_scroll_up_shifts_only_the_region() {
        // Rows L1..L6; region rows 2-4 (1-based); scroll up by one.
        let shared = feed_with_rows(
            6,
            b"\x1b[?1049h\
              \x1b[1;1HL1\x1b[2;1HL2\x1b[3;1HL3\x1b[4;1HL4\x1b[5;1HL5\x1b[6;1HL6\
              \x1b[2;4r\x1b[1S",
        );
        assert_eq!(snapshot(&shared), ["L1", "L3", "L4", "", "L5", "L6"]);
    }

    #[test]
    fn decstbm_scroll_down_shifts_only_the_region() {
        let shared = feed_with_rows(
            6,
            b"\x1b[?1049h\
              \x1b[1;1HL1\x1b[2;1HL2\x1b[3;1HL3\x1b[4;1HL4\x1b[5;1HL5\x1b[6;1HL6\
              \x1b[2;4r\x1b[1T",
        );
        assert_eq!(snapshot(&shared), ["L1", "", "L2", "L3", "L5", "L6"]);
    }

    #[test]
    fn line_feed_at_bottom_margin_scrolls() {
        let shared = feed_with_rows(4, b"\x1b[?1049hA\r\nB\r\nC\r\nD\r\nE");
        assert_eq!(snapshot(&shared), ["B", "C", "D", "E"]);
    }

    #[test]
    fn insert_and_delete_lines_respect_the_region() {
        // IL at row 2 pushes rows down within the region.
        let shared = feed_with_rows(
            4,
            b"\x1b[?1049h\x1b[1;1HA\x1b[2;1HB\x1b[3;1HC\x1b[4;1HD\x1b[2;1H\x1b[1L",
        );
        assert_eq!(snapshot(&shared), ["A", "", "B", "C"]);

        let shared = feed_with_rows(
            4,
            b"\x1b[?1049h\x1b[1;1HA\x1b[2;1HB\x1b[3;1HC\x1b[4;1HD\x1b[2;1H\x1b[1M",
        );
        assert_eq!(snapshot(&shared), ["A", "C", "D"]);
    }

    #[test]
    fn save_and_restore_cursor_keeps_positions() {
        // Write at row 3, save, jump to row 1 and write, restore, write more:
        // the continuation lands back on row 3.
        let shared = feed_with_rows(6, b"\x1b[?1049h\x1b[3;1Habc\x1b7\x1b[1;1Htop\x1b8def");
        assert_eq!(snapshot(&shared), ["top", "", "abcdef"]);
    }

    #[test]
    fn mouse_tracking_modes_are_tracked() {
        let shared = feed(b"\x1b[?1002;1006h\x1b[?1002l");
        let modes = shared.lock().unwrap().mouse_modes.clone();
        assert_eq!(modes.into_iter().collect::<Vec<_>>(), [1006]);
    }
}
