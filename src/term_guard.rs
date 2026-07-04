use std::sync::atomic::{AtomicBool, Ordering};

/// Tracks whether the parent terminal is currently in raw mode so that the
/// panic hook can restore it from any thread.
static RAW_MODE_ACTIVE: AtomicBool = AtomicBool::new(false);

/// RAII guard that puts the parent terminal into raw mode and guarantees
/// restoration on drop. Combined with [`install_panic_hook`], the terminal is
/// restored even when a thread panics.
pub struct RawModeGuard {
    enabled: bool,
}

impl RawModeGuard {
    /// Enables raw mode when `enable` is true (no-op guard otherwise, so the
    /// caller can construct it unconditionally).
    pub fn new(enable: bool) -> std::io::Result<Self> {
        if enable {
            crossterm::terminal::enable_raw_mode()?;
            RAW_MODE_ACTIVE.store(true, Ordering::SeqCst);
        }
        Ok(Self { enabled: enable })
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.enabled {
            restore_terminal();
        }
    }
}

/// Restores the terminal to cooked mode if raw mode is active. Safe to call
/// multiple times.
pub fn restore_terminal() {
    if RAW_MODE_ACTIVE.swap(false, Ordering::SeqCst) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// Installs a panic hook that restores the terminal before the default hook
/// prints the panic message, so the message is readable and the user's shell
/// is not left in raw mode.
pub fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));
}
