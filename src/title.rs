//! OSC title management (feature 1: session names in Zed).
//!
//! Zed's Terminal Threads display the OSC 0/2 title as the session name, so
//! controlling the title stream controls the name in the agent panel.
//!
//! [`TitleFilter`] transforms the child's output stream. OSC sequences may be
//! split across arbitrary `read()` boundaries, so the filter buffers from the
//! OSC introducer until its terminator (BEL or ST) before deciding whether to
//! pass, rewrite, or suppress the sequence. Everything that is not an OSC 0/2
//! title — including OSC 8 hyperlinks — passes through byte-for-byte.

use std::io::Write;

/// How the child's OSC 0/2 title sequences are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum TitleMode {
    /// Forward the child's titles unchanged.
    #[default]
    Passthrough,
    /// Suppress the child's titles; zedic emits its own (adapter-driven).
    Managed,
    /// Rewrite the child's titles with a fixed prefix.
    Prefix,
}

/// Upper bound for a buffered OSC sequence. Beyond this the sequence is
/// considered malformed and flushed through unchanged (fail-open).
const MAX_OSC_LEN: usize = 4096;

enum State {
    Normal,
    /// Saw ESC; the next byte decides whether an OSC starts.
    Esc,
    /// Inside an OSC sequence (buffer holds everything from the ESC on).
    Osc,
    /// Inside an OSC sequence and saw ESC (potential ST terminator).
    OscEsc,
}

pub struct TitleFilter {
    mode: TitleMode,
    prefix: String,
    state: State,
    /// Raw bytes of the OSC sequence being buffered, including the introducer.
    osc_buf: Vec<u8>,
}

impl TitleFilter {
    pub fn new(mode: TitleMode, prefix: impl Into<String>) -> Self {
        Self {
            mode,
            prefix: prefix.into(),
            state: State::Normal,
            osc_buf: Vec::new(),
        }
    }

    /// Feeds `input` through the filter, appending transformed bytes to `out`.
    pub fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) {
        for &byte in input {
            match self.state {
                State::Normal => {
                    if byte == 0x1b {
                        self.state = State::Esc;
                    } else {
                        out.push(byte);
                    }
                }
                State::Esc => {
                    if byte == b']' {
                        self.state = State::Osc;
                        self.osc_buf.clear();
                        self.osc_buf.extend_from_slice(b"\x1b]");
                    } else {
                        // Not an OSC: replay the ESC and handle this byte anew.
                        out.push(0x1b);
                        self.state = State::Normal;
                        if byte == 0x1b {
                            self.state = State::Esc;
                        } else {
                            out.push(byte);
                        }
                    }
                }
                State::Osc => {
                    self.osc_buf.push(byte);
                    if byte == 0x07 {
                        self.finish_osc(out);
                    } else if byte == 0x1b {
                        self.state = State::OscEsc;
                    } else if self.osc_buf.len() > MAX_OSC_LEN {
                        // Malformed or oversized: fail open.
                        out.extend_from_slice(&self.osc_buf);
                        self.osc_buf.clear();
                        self.state = State::Normal;
                    }
                }
                State::OscEsc => {
                    self.osc_buf.push(byte);
                    if byte == b'\\' {
                        self.finish_osc(out);
                    } else {
                        self.state = State::Osc;
                    }
                }
            }
        }
    }

    /// Flushes an unterminated buffer (call when the child exits) so no
    /// buffered bytes are ever lost.
    pub fn flush(&mut self, out: &mut Vec<u8>) {
        match self.state {
            State::Normal => {}
            State::Esc => out.push(0x1b),
            State::Osc | State::OscEsc => out.extend_from_slice(&self.osc_buf),
        }
        self.osc_buf.clear();
        self.state = State::Normal;
    }

    fn finish_osc(&mut self, out: &mut Vec<u8>) {
        self.state = State::Normal;
        match parse_title(&self.osc_buf) {
            Some(title) => match self.mode {
                TitleMode::Passthrough => out.extend_from_slice(&self.osc_buf),
                TitleMode::Managed => {} // suppressed; zedic emits its own
                TitleMode::Prefix => {
                    out.extend_from_slice(&format_title(&format!("{}{title}", self.prefix)));
                }
            },
            None => out.extend_from_slice(&self.osc_buf), // not a title OSC
        }
        self.osc_buf.clear();
    }
}

/// Extracts the title text when `seq` is a complete OSC 0/2 sequence.
fn parse_title(seq: &[u8]) -> Option<String> {
    let body = seq.strip_prefix(b"\x1b]")?;
    let body = body
        .strip_suffix(b"\x07")
        .or_else(|| body.strip_suffix(b"\x1b\\"))?;
    let (code, title) = {
        let sep = body.iter().position(|&b| b == b';')?;
        (&body[..sep], &body[sep + 1..])
    };
    if code == b"0" || code == b"2" {
        Some(String::from_utf8_lossy(title).into_owned())
    } else {
        None
    }
}

/// Formats an OSC 2 sequence announcing `title` (control chars stripped).
pub fn format_title(title: &str) -> Vec<u8> {
    let clean: String = title.chars().filter(|c| !c.is_control()).collect();
    format!("\x1b]2;{clean}\x07").into_bytes()
}

/// Writes an OSC 2 title directly to `writer` (used by the managed-mode
/// title thread, serialized with the output pump via the stdout gate).
pub fn emit_title(writer: &mut impl Write, title: &str) -> std::io::Result<()> {
    writer.write_all(&format_title(title))?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_filter(mode: TitleMode, prefix: &str, chunks: &[&[u8]]) -> Vec<u8> {
        let mut filter = TitleFilter::new(mode, prefix);
        let mut out = Vec::new();
        for chunk in chunks {
            filter.feed(chunk, &mut out);
        }
        filter.flush(&mut out);
        out
    }

    #[test]
    fn passthrough_keeps_title_sequences() {
        let out = run_filter(TitleMode::Passthrough, "", &[b"a\x1b]2;t\x07b"]);
        assert_eq!(out, b"a\x1b]2;t\x07b");
    }

    #[test]
    fn managed_suppresses_titles() {
        let out = run_filter(TitleMode::Managed, "", &[b"a\x1b]2;t\x07b"]);
        assert_eq!(out, b"ab");
    }

    #[test]
    fn managed_suppresses_osc0_with_st_terminator() {
        let out = run_filter(TitleMode::Managed, "", &[b"x\x1b]0;t\x1b\\y"]);
        assert_eq!(out, b"xy");
    }

    #[test]
    fn prefix_rewrites_title() {
        let out = run_filter(TitleMode::Prefix, "claude: ", &[b"\x1b]2;fix bug\x07"]);
        assert_eq!(out, b"\x1b]2;claude: fix bug\x07");
    }

    #[test]
    fn osc_split_across_chunks_is_reassembled() {
        let out = run_filter(
            TitleMode::Managed,
            "",
            &[b"a\x1b", b"]2;spl", b"it title", b"\x07b"],
        );
        assert_eq!(out, b"ab");
    }

    #[test]
    fn non_title_osc_passes_through() {
        // OSC 8 hyperlink must survive all modes untouched.
        let link = b"\x1b]8;;file:///tmp/x\x07text\x1b]8;;\x07";
        let out = run_filter(TitleMode::Managed, "", &[link]);
        assert_eq!(out, link);
    }

    #[test]
    fn non_osc_escapes_pass_through() {
        let input: &[u8] = b"\x1b[1;31mred\x1b[0m\x1b[2J";
        let out = run_filter(TitleMode::Managed, "", &[input]);
        assert_eq!(out, input);
    }

    #[test]
    fn unterminated_osc_is_flushed_on_exit() {
        let out = run_filter(TitleMode::Managed, "", &[b"a\x1b]2;never terminated"]);
        assert_eq!(out, b"a\x1b]2;never terminated");
    }

    #[test]
    fn oversized_osc_fails_open() {
        let mut big = Vec::from(&b"\x1b]2;"[..]);
        big.extend(std::iter::repeat_n(b'x', MAX_OSC_LEN + 10));
        let out = run_filter(TitleMode::Managed, "", &[&big]);
        assert_eq!(out, big);
    }

    #[test]
    fn format_title_strips_control_chars() {
        assert_eq!(format_title("a\x07b\nc"), b"\x1b]2;abc\x07");
    }
}
