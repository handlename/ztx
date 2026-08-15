# Configuration

ztx reads an optional configuration file at startup. Every setting is
independent: a missing key, an unrecognized value, or a completely absent file
simply falls back to the built-in default for that setting. A broken config
file never blocks ztx from starting.

## File location

```
~/.config/ztx/config.toml
```

ztx honors `$XDG_CONFIG_HOME`: if that variable is set and non-empty, the path
becomes `$XDG_CONFIG_HOME/ztx/config.toml`.

## Precedence

**CLI argument > `config.toml` > built-in default**

Flags passed on the command line always win. `config.toml` fills in where no
explicit flag was given. The built-in default applies when neither source
provides a value.

## Behavior on errors

| Situation | Result |
|-----------|--------|
| File does not exist | All settings use built-in defaults; startup continues normally. |
| File is unreadable (permissions, I/O error) | All settings use built-in defaults; startup continues normally. |
| File contains malformed TOML | All settings use built-in defaults; a warning is emitted to the log (`ZTX_LOG`). |
| A single key has an unrecognized value (e.g. an invalid `prefix` chord) | That key uses its built-in default; all other keys parse normally; a warning is emitted to the log. |

## Keys

### `prefix`

The Prefix key chord that activates ztx's key bindings.

| | |
|-|-|
| **Type** | string |
| **Default** | `"ctrl-]"` |
| **Syntax** | `ctrl-<key>` or `c-<key>` (case-insensitive) |

Only single-character Ctrl chords are accepted. The chord is encoded as the
key's ASCII value masked to its low 5 bits, matching how terminals encode Ctrl
sequences. `ctrl-@` (which maps to the null byte `0x00`) is explicitly
rejected. An unrecognized value is ignored and the default is used.

Valid examples: `"ctrl-]"`, `"ctrl-a"`, `"Ctrl-A"`, `"C-]"`.

---

### `editor`

The editor command used by Export and Hint mode to open files.

| | |
|-|-|
| **Type** | string |
| **Default** | _(none; falls through to `$ZTX_EDITOR`, then `zed`, then `$EDITOR`)_ |

The value is split on whitespace into a program and its arguments, so
`"zed --wait"` is valid. A blank or whitespace-only value is treated as absent.
See [Environment variables](environment-variables.md) for the full resolution
order.

---

### `[status_emoji]`

Emoji prefixes shown in the managed session title for the Claude Code Adapter.
Each key is optional; omitting a key keeps the built-in default for that state.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `busy` | string | `"🔄"` | Claude is actively generating a response. |
| `idle` | string | `"⏳"` | Claude is waiting for the next user message. |
| `waiting` | string | `"🔔"` | Claude has stopped to ask the user a question or present choices. |

---

### `[notify]`

macOS desktop notification behavior. Notifications fire when the Claude Code
session starts waiting for input or finishes responding. Requires
[`terminal-notifier`](https://github.com/julienXX/terminal-notifier) on
`$PATH`; its absence is a silent no-op.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `desktop` | bool | `true` | Whether to emit a desktop notification. Set to `false` to disable entirely. |
| `sound` | string | `"Glass"` | Notification sound name as listed in System Settings → Sound. Set to `""` for a silent notification. |

---

### `[run]`

Defaults for `ztx run`.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `force` | bool | `false` | Terminate a live session already running in this project without confirming, then start fresh here. Setting it to `true` is the same as passing `--force` on every `ztx run`. |

`force = true` kills the existing session's agent without asking. Enable it
only for workflows where editor restarts routinely orphan sessions. Pass
[`--no-force`](subcommands.md) to bring the confirmation back for a single run.

## Complete example

```toml
# ~/.config/ztx/config.toml

# Prefix key for all ztx key bindings (ctrl-] is the default).
prefix = "ctrl-]"

# Editor opened by Export (ctrl-] e) and Hint mode (ctrl-] f).
# Split on whitespace: "zed --wait" passes --wait to zed.
editor = "zed --wait"

[status_emoji]
# Emoji prefixes in the managed Terminal Thread title (Claude adapter).
busy    = "🔄"   # generating a response
idle    = "⏳"   # waiting for your next message
waiting = "🔔"   # blocked on a question or choice prompt

[notify]
# macOS desktop notifications via terminal-notifier (brew install terminal-notifier).
desktop = true    # set false to disable
sound   = "Glass" # sound name from Sound Preferences; "" for silent

[run]
# Replace a live session without confirming (same as passing --force every time).
force = false
```
