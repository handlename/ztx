//! User configuration file (`~/.config/ztx/config.toml`).
//!
//! Every setting is optional and only fills in where an explicit CLI argument
//! is absent: the precedence is **CLI argument > config.toml > built-in
//! default**. A missing file, unreadable file, malformed TOML, or an
//! unrecognized value is never fatal — the affected setting simply falls back
//! to its default so a broken config can never stop the wrapper from starting.
//!
//! ```toml
//! prefix = "ctrl-]"       # ztx prefix key (see `parse_prefix`)
//! editor = "zed --wait"   # editor command for export / hint "open"
//!
//! [status_emoji]          # Claude session-title status prefixes
//! busy = "🔄"
//! idle = "⏳"
//! waiting = "🔔"          # Claude is waiting for user input (choices, prompts)
//!
//! [notify]                # macOS desktop notifications (via terminal-notifier)
//! desktop = true          # fire on waiting/finished; needs terminal-notifier
//! sound = "Glass"         # notification sound name; "" for silent
//! ```

use std::path::PathBuf;

use serde::Deserialize;

/// Default Claude "busy" status emoji (mirrors `adapter::claude`).
const DEFAULT_BUSY_EMOJI: &str = "🔄";
/// Default Claude "idle" status emoji (mirrors `adapter::claude`).
const DEFAULT_IDLE_EMOJI: &str = "⏳";
/// Default Claude "waiting" status emoji (mirrors `adapter::claude`).
const DEFAULT_WAITING_EMOJI: &str = "🔔";

/// Resolved ztx configuration. Fields that map to a CLI-overridable setting
/// are `Option`: `None` means "no preference, use the built-in default".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    /// Prefix key byte (e.g. `0x1d` for `ctrl-]`). `None` -> `DEFAULT_PREFIX`.
    pub prefix: Option<u8>,
    /// Editor command line, whitespace-split into program + args.
    pub editor: Option<String>,
    /// Status-emoji prefixes for the managed session title.
    pub status_emoji: StatusEmoji,
    /// Desktop-notification behavior for hook events.
    pub notify: NotifyConfig,
}

/// macOS desktop-notification settings. Enabled by default so that installing
/// the plugin is enough to get notifications; `terminal-notifier` is still
/// required at runtime and its absence is a silent no-op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyConfig {
    /// Whether to emit a desktop notification when the session starts waiting
    /// for input or finishes responding.
    pub desktop: bool,
    /// Notification sound name (as in Sound Preferences). `None` is silent.
    pub sound: Option<String>,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            desktop: true,
            sound: Some("Glass".to_owned()),
        }
    }
}

/// The busy/idle/waiting emoji prefixes used by the Claude adapter's title.
/// Defaults match the adapter's built-in emoji so an absent `[status_emoji]`
/// section is indistinguishable from omitting the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEmoji {
    pub busy: String,
    pub idle: String,
    pub waiting: String,
}

impl Default for StatusEmoji {
    fn default() -> Self {
        Self {
            busy: DEFAULT_BUSY_EMOJI.to_owned(),
            idle: DEFAULT_IDLE_EMOJI.to_owned(),
            waiting: DEFAULT_WAITING_EMOJI.to_owned(),
        }
    }
}

impl Config {
    /// Loads the config from the default path, returning defaults on any error.
    pub fn load() -> Self {
        Self::load_from(config_path())
    }

    /// Loads from an explicit path (used by tests). Any failure -> defaults.
    fn load_from(path: Option<PathBuf>) -> Self {
        let Some(path) = path else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text),
            Err(err) => {
                tracing::debug!(path = %path.display(), error = %err, "no config loaded");
                Self::default()
            }
        }
    }

    /// Parses TOML text into a `Config`. Pure and total: malformed TOML yields
    /// defaults, and each field independently falls back when absent/invalid.
    pub fn parse(text: &str) -> Self {
        let raw = match toml::from_str::<RawConfig>(text) {
            Ok(raw) => raw,
            Err(err) => {
                tracing::warn!(error = %err, "ignoring malformed config.toml");
                return Self::default();
            }
        };

        let prefix = raw.prefix.as_deref().and_then(|s| {
            let parsed = parse_prefix(s);
            if parsed.is_none() {
                tracing::warn!(prefix = s, "ignoring unrecognized config prefix");
            }
            parsed
        });

        let editor = raw.editor.filter(|s| !s.trim().is_empty());

        let mut status_emoji = StatusEmoji::default();
        if let Some(raw_emoji) = raw.status_emoji {
            if let Some(busy) = raw_emoji.busy {
                status_emoji.busy = busy;
            }
            if let Some(idle) = raw_emoji.idle {
                status_emoji.idle = idle;
            }
            if let Some(waiting) = raw_emoji.waiting {
                status_emoji.waiting = waiting;
            }
        }

        let mut notify = NotifyConfig::default();
        if let Some(raw_notify) = raw.notify {
            if let Some(desktop) = raw_notify.desktop {
                notify.desktop = desktop;
            }
            // An explicit empty string means "silent"; otherwise keep the value.
            if let Some(sound) = raw_notify.sound {
                notify.sound = (!sound.trim().is_empty()).then_some(sound);
            }
        }

        Self {
            prefix,
            editor,
            status_emoji,
            notify,
        }
    }
}

/// Raw TOML shape; every field optional so partial files parse cleanly.
#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    prefix: Option<String>,
    editor: Option<String>,
    status_emoji: Option<RawStatusEmoji>,
    notify: Option<RawNotify>,
}

#[derive(Debug, Default, Deserialize)]
struct RawStatusEmoji {
    busy: Option<String>,
    idle: Option<String>,
    waiting: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawNotify {
    desktop: Option<bool>,
    sound: Option<String>,
}

/// Parses a prefix-key spec into its control byte. Only the `ctrl-<key>` form
/// (or its `c-<key>` shorthand) is supported (e.g. `"ctrl-]"` -> `0x1d`,
/// `"ctrl-a"` -> `0x01`): the control byte is the key's ASCII code masked to
/// its low 5 bits, matching how terminals encode Ctrl chords. The whole spec
/// is case-insensitive. Anything else — including `ctrl-@`, which would mask
/// to an unusable `0x00` — yields `None`.
fn parse_prefix(spec: &str) -> Option<u8> {
    // ASCII-lowercasing the whole spec makes both the scheme and the key
    // case-insensitive; the key is upper-masked below, so its case is moot.
    let spec = spec.trim().to_ascii_lowercase();
    let key = spec
        .strip_prefix("ctrl-")
        .or_else(|| spec.strip_prefix("c-"))?;
    let mut chars = key.chars();
    let c = chars.next()?;
    if chars.next().is_some() || !c.is_ascii() {
        return None;
    }
    let byte = (c.to_ascii_uppercase() as u8) & 0x1f;
    (byte != 0).then_some(byte)
}

/// The config file path: `$XDG_CONFIG_HOME/ztx/config.toml`, falling back to
/// `~/.config/ztx/config.toml`. `None` when neither is resolvable.
fn config_path() -> Option<PathBuf> {
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".config"))
        })?;
    Some(config_home.join("ztx").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_yields_defaults() {
        assert_eq!(Config::parse(""), Config::default());
    }

    #[test]
    fn malformed_toml_yields_defaults() {
        assert_eq!(Config::parse("this is = = not toml"), Config::default());
    }

    #[test]
    fn parses_all_fields() {
        let cfg = Config::parse(
            r#"
            prefix = "ctrl-a"
            editor = "zed --wait"
            [status_emoji]
            busy = "B"
            idle = "I"
            waiting = "W"
            "#,
        );
        assert_eq!(cfg.prefix, Some(0x01));
        assert_eq!(cfg.editor.as_deref(), Some("zed --wait"));
        assert_eq!(cfg.status_emoji.busy, "B");
        assert_eq!(cfg.status_emoji.idle, "I");
        assert_eq!(cfg.status_emoji.waiting, "W");
    }

    #[test]
    fn partial_status_emoji_keeps_defaults_for_missing() {
        let cfg = Config::parse("[status_emoji]\nbusy = \"X\"\n");
        assert_eq!(cfg.status_emoji.busy, "X");
        assert_eq!(cfg.status_emoji.idle, DEFAULT_IDLE_EMOJI);
        assert_eq!(cfg.status_emoji.waiting, DEFAULT_WAITING_EMOJI);
    }

    #[test]
    fn notify_defaults_to_enabled_with_sound() {
        assert_eq!(Config::parse("").notify, NotifyConfig::default());
        assert!(Config::parse("").notify.desktop);
        assert_eq!(Config::parse("").notify.sound.as_deref(), Some("Glass"));
    }

    #[test]
    fn notify_can_be_disabled_and_silenced() {
        let cfg = Config::parse("[notify]\ndesktop = false\nsound = \"\"\n");
        assert!(!cfg.notify.desktop);
        assert_eq!(cfg.notify.sound, None);
    }

    #[test]
    fn notify_custom_sound_is_kept() {
        let cfg = Config::parse("[notify]\nsound = \"Ping\"\n");
        assert!(cfg.notify.desktop);
        assert_eq!(cfg.notify.sound.as_deref(), Some("Ping"));
    }

    #[test]
    fn invalid_prefix_falls_back_to_none() {
        assert_eq!(Config::parse(r#"prefix = "meta-x""#).prefix, None);
        assert_eq!(Config::parse(r#"prefix = "ctrl-ab""#).prefix, None);
        assert_eq!(Config::parse(r#"prefix = "]""#).prefix, None);
        // `ctrl-@` masks to 0x00, which is unusable as a prefix key.
        assert_eq!(Config::parse(r#"prefix = "ctrl-@""#).prefix, None);
    }

    #[test]
    fn blank_editor_is_ignored() {
        assert_eq!(Config::parse(r#"editor = "   ""#).editor, None);
    }

    #[test]
    fn parse_prefix_matches_terminal_encoding() {
        assert_eq!(parse_prefix("ctrl-]"), Some(0x1d));
        assert_eq!(parse_prefix("ctrl-a"), Some(0x01));
        assert_eq!(parse_prefix("Ctrl-A"), Some(0x01));
        assert_eq!(parse_prefix("CTRL-]"), Some(0x1d));
        assert_eq!(parse_prefix("C-]"), Some(0x1d));
        assert_eq!(parse_prefix("ctrl-@"), None);
        assert_eq!(parse_prefix("nonsense"), None);
    }

    #[test]
    fn missing_file_yields_defaults() {
        assert_eq!(
            Config::load_from(Some(PathBuf::from("/no/such/ztx/config.toml"))),
            Config::default()
        );
        assert_eq!(Config::load_from(None), Config::default());
    }
}
