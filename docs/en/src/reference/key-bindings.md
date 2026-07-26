# Key bindings

All ztx key bindings live behind a single Prefix key so the wrapped Agent CLI
keeps its entire keymap untouched. ztx only intercepts keystrokes that begin
with the prefix; everything else passes through to the child process unchanged.

## Prefix key

The default prefix key is **`ctrl-]`** (ASCII `0x1d`). It can be changed via
the `prefix` key in [`config.toml`](configuration.md).

Pressing the prefix twice forwards one literal prefix byte to the child. This
lets you send `ctrl-]` to a CLI that actually uses it — type it twice and the
wrapped process receives it once.

Any key sequence that starts with the prefix but does not match a known binding
forwards both the prefix byte and the unrecognized key to the child unchanged.

## Bindings

All bindings require the prefix key first.

| Keys | Action |
|------|--------|
| `ctrl-] f` | **Hint mode.** An overlay appears labeling every file path visible in the scrollback. Type a label to open that path in the editor at the referenced line (Zed receives `path:line`). Falls back to a list view when the session is on the alternate screen. Hint mode also dumps candidates automatically when it finds none, for diagnostics. |
| `ctrl-] e` | **Export.** Converts the session log to Markdown and opens it in the editor. Uses the CLI's native transcript when an adapter can locate one (Claude Code), otherwise uses the ANSI-stripped PTY capture. |
| `ctrl-] d` | **State dump.** Writes ztx's internal state — primary scrollback, visible alternate screen frame, screen-mode flags, and the child's last OSC title — to `$TMPDIR/ztx/state-<pid>-<n>.txt`. Useful for diagnostics. |

## Why all bindings are behind a prefix

Agent CLIs use the full keyboard. Tools such as Claude Code bind `ctrl-r`,
`ctrl-c`, `ctrl-k`, and many others. A prefix-key approach means ztx can add
any binding without risking a conflict with the wrapped CLI's keymap, now or
in the future. The prefix itself (`ctrl-]`) is rarely claimed by interactive
CLIs, making it a safe default.

## Changing the prefix key

Set `prefix` in `~/.config/ztx/config.toml`:

```toml
prefix = "ctrl-a"   # any Ctrl chord except ctrl-@
```

The value must be a `ctrl-<key>` or `c-<key>` chord (case-insensitive). Single
characters, meta chords, and `ctrl-@` (which maps to the null byte) are not
accepted — an unrecognized value is ignored and the built-in default is used
instead. See [Configuration](configuration.md) for the full syntax.
