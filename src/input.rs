//! Input-side prefix key handling.
//!
//! zediator's own key bindings live behind a single prefix key (default
//! `ctrl-]`, 0x1d) so that the wrapped CLI keeps its entire keymap. Pressing
//! the prefix twice forwards a literal prefix byte to the child.

/// Default prefix key: `ctrl-]`.
pub const DEFAULT_PREFIX: u8 = 0x1d;

/// A zediator action requested from the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputAction {
    /// `prefix + e`: export the session log as Markdown.
    Export,
    /// `prefix + f`: enter hint mode (file-path picking).
    Hint,
    /// `prefix + d`: dump internal state for diagnostics.
    DumpState,
}

/// Splits the stdin stream into bytes for the child and zediator actions.
pub struct InputFilter {
    prefix: u8,
    pending: bool,
}

impl InputFilter {
    pub fn new(prefix: u8) -> Self {
        Self {
            prefix,
            pending: false,
        }
    }

    /// Processes `input`, pushing child-bound bytes to `out` and returning
    /// any triggered actions.
    pub fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Vec<InputAction> {
        let mut actions = Vec::new();
        for &byte in input {
            if self.pending {
                self.pending = false;
                match byte {
                    b'e' => actions.push(InputAction::Export),
                    b'f' => actions.push(InputAction::Hint),
                    b'd' => actions.push(InputAction::DumpState),
                    b if b == self.prefix => out.push(self.prefix), // literal
                    other => {
                        out.push(self.prefix);
                        out.push(other);
                    }
                }
            } else if byte == self.prefix {
                self.pending = true;
            } else {
                out.push(byte);
            }
        }
        actions
    }

    /// Releases a pending prefix byte (call when stdin reaches EOF so the
    /// byte is not silently dropped).
    pub fn flush(&mut self, out: &mut Vec<u8>) {
        if self.pending {
            self.pending = false;
            out.push(self.prefix);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(chunks: &[&[u8]]) -> (Vec<u8>, Vec<InputAction>) {
        let mut filter = InputFilter::new(DEFAULT_PREFIX);
        let mut out = Vec::new();
        let mut actions = Vec::new();
        for chunk in chunks {
            actions.extend(filter.feed(chunk, &mut out));
        }
        (out, actions)
    }

    #[test]
    fn plain_bytes_pass_through() {
        let (out, actions) = feed_all(&[b"hello"]);
        assert_eq!(out, b"hello");
        assert!(actions.is_empty());
    }

    #[test]
    fn prefix_e_triggers_export() {
        let (out, actions) = feed_all(&[b"a\x1deb"]);
        assert_eq!(out, b"ab");
        assert_eq!(actions, [InputAction::Export]);
    }

    #[test]
    fn prefix_f_triggers_hint() {
        let (out, actions) = feed_all(&[b"\x1df"]);
        assert_eq!(out, b"");
        assert_eq!(actions, [InputAction::Hint]);
    }

    #[test]
    fn prefix_split_across_reads() {
        let (out, actions) = feed_all(&[b"\x1d", b"e"]);
        assert_eq!(out, b"");
        assert_eq!(actions, [InputAction::Export]);
    }

    #[test]
    fn double_prefix_forwards_literal() {
        let (out, actions) = feed_all(&[b"\x1d\x1d"]);
        assert_eq!(out, b"\x1d");
        assert!(actions.is_empty());
    }

    #[test]
    fn unknown_binding_forwards_both_bytes() {
        let (out, actions) = feed_all(&[b"\x1dx"]);
        assert_eq!(out, b"\x1dx");
        assert!(actions.is_empty());
    }

    #[test]
    fn flush_releases_pending_prefix() {
        let mut filter = InputFilter::new(DEFAULT_PREFIX);
        let mut out = Vec::new();
        filter.feed(b"\x1d", &mut out);
        assert!(out.is_empty());
        filter.flush(&mut out);
        assert_eq!(out, b"\x1d");
        // Idempotent.
        filter.flush(&mut out);
        assert_eq!(out, b"\x1d");
    }
}
