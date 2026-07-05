//! Generic runtime diagnostics.
//!
//! Two complementary mechanisms, usable for any feature (not tied to one
//! investigation):
//!
//! - **Event log**: every subsystem traces through `ZEDIATOR_LOG` /
//!   `ZEDIATOR_LOG_FILE` (see `logging.rs`) — screen-mode changes, title
//!   emissions, prefix actions, IPC injections, exports.
//! - **State dump** (`ctrl-] d`, or automatic when a feature comes up
//!   empty): snapshots the wrapper's internal state to a file so "why did X
//!   see nothing?" can be answered from real data after the fact.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::term::TapShared;

/// Distinguishes multiple dumps within one wrapper process.
static DUMP_SEQ: AtomicU32 = AtomicU32::new(0);

/// Writes a snapshot of the tap state (plus a caller-provided context
/// section) to a per-user file and returns its path.
pub fn dump_state(
    tap: &std::sync::Arc<Mutex<TapShared>>,
    context: &str,
) -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("zediator");
    std::fs::create_dir_all(&dir)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let seq = DUMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!("state-{}-{seq}.txt", std::process::id()));

    let (alt_screen, last_title, alt_snapshot, captured, recent, mouse_modes) = {
        let mut guard = tap.lock().expect("tap lock poisoned");
        (
            guard.alt_screen,
            guard.last_title.clone(),
            guard.alt_snapshot.clone(),
            guard.scrollback.len(),
            guard.scrollback.dump().unwrap_or_default(),
            guard.mouse_modes.iter().copied().collect::<Vec<u16>>(),
        )
    };

    let mut out = String::new();
    out.push_str(&format!(
        "# zediator state dump\n\npid: {}\nalt_screen: {alt_screen}\n\
         last_child_title: {last_title:?}\nscrollback_lines: {captured}\n\
         alt_snapshot_lines: {}\nmouse_modes: {mouse_modes:?}\n\n## context\n{context}\n",
        std::process::id(),
        alt_snapshot.len(),
    ));
    out.push_str("\n## alternate screen (visible frame)\n");
    for line in &alt_snapshot {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("\n## primary scrollback\n");
    out.push_str(&recent);

    std::fs::write(&path, out)?;
    tracing::info!(path = %path.display(), "state dumped");
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::TermTap;

    #[test]
    fn dump_contains_all_sections() {
        let shared = TermTap::shared(Some(8));
        let mut tap = TermTap::new(shared.clone());
        tap.advance(b"primary line\n\x1b[?1049h\x1b[Halt frame line");
        tap.flush();

        let path = dump_state(&shared, "unit test").unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("alt_screen: true"));
        assert!(content.contains("unit test"));
        assert!(content.contains("alt frame line"));
        assert!(content.contains("primary line"));
        std::fs::remove_file(path).unwrap();
    }
}
