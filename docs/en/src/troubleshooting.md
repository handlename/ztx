# Troubleshooting

ztx never writes diagnostic output to the terminal. Because it shares the
terminal with the wrapped CLI, any stray output would corrupt the child's
screen. All investigation therefore relies on two out-of-band tools:

1. **Log file** — run with `ZTX_LOG=debug` to enable tracing for all
   subsystems (screen-state changes, title emissions, prefix-key actions, IPC
   injections, exports, hint candidate counts). Logs are appended to
   `ZTX_LOG_FILE`, which defaults to `$XDG_STATE_HOME/ztx/ztx.log` (falling
   back to `~/.local/state/ztx/ztx.log`).

   ```sh
   ZTX_LOG=debug ztx run -- claude
   ```

2. **State dump** — press `ctrl-] d` at any time to write a snapshot of the
   wrapper's internal state to `$TMPDIR/ztx/state-<pid>-<n>.txt`. The dump
   includes: whether the alternate screen is active, the child's last OSC title,
   the number of captured scrollback lines, the visible alternate-screen frame,
   and the recent primary scrollback. Hint mode also dumps automatically when it
   finds no file-path candidates.

---

## Session name not updating / unexpected name

**Symptoms:** The Terminal Thread's sidebar name in Zed is wrong, stale, or
does not reflect the current working branch or agent status.

**Causes and fixes:**

- **No adapter matched.** Without `--adapter`, ztx selects an adapter
  automatically. If the child command name is not recognized, the title mode
  defaults to `passthrough`: the child's own OSC title sequences are forwarded
  unchanged. Pass `--adapter claude` (or `agy`) explicitly if auto-detection
  misses.

- **`--title-mode passthrough` was set explicitly.** In `passthrough` mode ztx
  forwards the child's own OSC titles without modification. Change to `managed`
  (or omit `--title-mode` to let the adapter choose).

- **`--title-mode managed` but the session name is not changing.** The
  `managed` mode suppresses the child's OSC titles and has ztx emit its own.
  If the adapter cannot determine a meaningful name (e.g. the worktree root
  cannot be resolved), the title may be generic. Run with `ZTX_LOG=debug` and
  look for `title` log lines to see what the adapter is emitting.

- **The child CLI is not emitting OSC titles at all.** In `passthrough` and
  `prefix` modes, ztx depends on the child setting terminal titles. If the
  child does not emit `OSC 0` or `OSC 2` sequences, ztx has nothing to
  forward. Verify with `ZTX_LOG=debug` — a `title emissions` entry will
  appear whenever a title sequence is processed.

- **Adapter quality differences.** The Claude Code adapter sets the session
  title to the worktree (or branch) name prefixed by a status emoji (🔄 busy,
  ⏳ idle, 🔔 waiting for input). The antigravity-cli adapter exposes
  conversation titles. When no adapter applies, the child's own terminal title
  passes through. Session-name quality therefore depends on which adapter — if
  any — is active.

---

## `ctrl-] f` (hint mode) finds no files

**Symptoms:** Pressing `ctrl-] f` shows a message like "no file paths found",
or the in-place labels do not appear over paths that are visibly on screen.

**How hint mode works:**

- When the wrapped CLI is in **alternate screen** mode (a full-screen TUI),
  hint mode scans the visible frame and paints labels directly over path
  positions. If the visible frame contains no recognizable paths, hint mode
  falls through to the modal-list path below.

- When the wrapped CLI is in **primary screen** mode (scrolling output), hint
  mode shows a modal list built from the last 400 lines of the primary
  scrollback plus any captured alternate-screen rows. If that list is empty, a
  state dump is written automatically.

**Why candidates may be missing:**

- The file path does not exist on disk (ztx only labels paths it can resolve
  to an existing file).
- The path appeared only while the alternate screen was active and has since
  been replaced — historical alternate-screen content is not captured in the
  scrollback.
- The worktree index scan was capped (20 000 entries, depth 8, skipping
  `target`, `node_modules`, `vendor`, `dist`, `build`, `__pycache__`, and
  hidden directories starting with `.`).

**Reading the auto-dump:**

When the modal path finds zero candidates, the dump path is shown on screen.
Open the file and check:

- `alt_screen: true/false` — whether the alternate screen was active. If
  `true` and the in-place overlay showed nothing, the visible frame had no
  recognizable paths at the moment you pressed `ctrl-] f`.
- `scrollback_lines` — how many primary-screen lines are captured. A count of
  0 means the child has been running exclusively in alternate screen mode and
  nothing went to the primary scrollback.
- `## alternate screen (visible frame)` — the content ztx saw when hint mode
  ran. Verify the paths you expected are present in this section.
- `## primary scrollback` — the recent captured output on the primary screen.

---

## `ztx send` not arriving / arriving in the wrong session

**Symptoms:** `ztx send` exits with "no ztx session running for this project",
or the text is injected into a different session than expected.

**How socket routing works:**

`ztx run` and `ztx send` both key off the same project directory: the value of
`ZED_WORKTREE_ROOT` when set (Zed injects this into tasks), otherwise the
process's current directory. The path is canonicalized and hashed with FNV-1a
to produce `<hash>.sock` inside the socket directory (`ZTX_RUNTIME_DIR` →
`$XDG_RUNTIME_DIR/ztx` → `$TMPDIR/ztx-run`). There is no registry — both
sides compute the same path independently.

**Common causes:**

- **Directory mismatch.** If `ztx send` runs from a directory that differs
  (after canonicalization) from where `ztx run` was started, the hashes
  differ. Run `ztx sessions` to list live sessions and their working
  directories, then use `--socket <path>` to target one explicitly.

  ```sh
  ztx sessions
  ztx send --socket /path/to/<hash>.sock -- your message
  ```

- **Orphaned session from a previous editor session.** After a Zed restart
  the wrapper process may still be running (attached to a closed terminal).
  `ztx run` detects this and (interactively) offers to terminate the old
  session and rebind the socket. If `ztx send` points at the orphaned session,
  restart the wrapper or use `--socket` to target the new one.

- **Stale socket file.** A socket file whose owner process has exited is
  automatically taken over by the next `ztx run` in that project. If `ztx
  send` reports no session, the wrapper is not running — start one with
  `ztx run -- <cli>`.

---

## Export content is missing or unexpected

**Symptoms:** The exported Markdown is shorter than expected, missing agent
responses, or cut off mid-conversation.

**How export sources work:**

- **Native transcript (Claude Code adapter only).** When the Claude Code
  adapter locates the session JSONL under `~/.claude/projects/`, export uses
  that file. This is the highest-fidelity source.
- **Fallback: ANSI-stripped PTY capture.** When no native transcript is found,
  export falls back to ztx's own scrollback capture with ANSI escape sequences
  stripped. This is always available but has two limitations:
  - Content that appeared only on the **alternate screen** (inside a
    full-screen TUI) is not captured in the primary scrollback and will not
    appear in the export.
  - The capture is a ring buffer — very long sessions may lose early lines.

If you expect native-transcript quality but the export looks like a PTY
capture, check `ZTX_LOG=debug` for lines mentioning the transcript path. The
`ztx notify --from-hook` call from the Claude Code plugin hooks records the
authoritative transcript path over IPC; if the hooks are not firing, ztx
falls back to heuristic discovery.

---

## Desktop notifications not appearing

**Symptoms:** No system notifications when Claude finishes or asks for input.

**Requirements and settings:**

- Desktop notifications are **macOS only**. On other platforms the notification
  path is a silent no-op.
- The `terminal-notifier` tool must be installed and on `PATH`. Its absence is
  a silent no-op (logged at `debug` level with `ZTX_LOG=debug`).

  ```sh
  brew install terminal-notifier
  ```

- Notifications fire only for two events: **waiting for input**
  (`Notification` hook) and **finished responding** (`Stop` hook). Other
  hook events (session start, prompt submit, etc.) do not trigger a
  notification.
- Setting `desktop = false` in the `[notify]` section of
  `~/.config/ztx/config.toml` disables desktop notifications entirely.

  ```toml
  [notify]
  desktop = false
  ```

- The default notification sound is `"Glass"`. Set `sound = ""` for silent
  notifications.

---

## Configuration not taking effect

**Symptoms:** A setting in `~/.config/ztx/config.toml` appears to be ignored.

**Precedence order:** CLI argument > `config.toml` > built-in default. A CLI
flag always wins, so if you are passing `--title-mode` or `--adapter` on the
command line, those override the config file.

**Silent fallback on errors.** A missing file, an unreadable file, or
malformed TOML never prevents ztx from starting. When the whole file cannot
be parsed, all settings fall back to their built-in defaults. When only one
field is invalid (e.g. an unrecognized prefix value), that field falls back
to its default and the rest of the file is still applied.

This means a typo in `config.toml` can silently restore a default you did not
intend. Run with `ZTX_LOG=debug` to see whether the config was loaded:
a `no config loaded` or `ignoring malformed config.toml` log line indicates a
problem.

**Config file path.** The file is loaded from `$XDG_CONFIG_HOME/ztx/config.toml`
(falling back to `~/.config/ztx/config.toml`). If neither `XDG_CONFIG_HOME`
nor `HOME` is set, no config file is loaded.

**Valid prefix format.** Only `ctrl-<key>` or `c-<key>` (case-insensitive) is
accepted (e.g. `ctrl-]`, `ctrl-a`, `C-]`). A value like `meta-x` or a bare
`]` is silently ignored and the default prefix (`ctrl-]`) is used instead.
`ctrl-@` is also rejected because it maps to the null byte.

---

## Prefix key is intercepted by the wrapped CLI / ztx instead of the other

**Symptoms:** Pressing `ctrl-]` inside the wrapped CLI (e.g. to use one of the
CLI's own keybindings) triggers a ztx action instead.

All ztx keybindings live behind the prefix key so that the wrapped CLI's own
keymap is preserved for every other key. The one exception is the prefix key
itself: **pressing the prefix key twice** forwards a single literal prefix byte
to the child. For example, with the default prefix `ctrl-]`:

```
ctrl-]  ctrl-]   →  sends a literal ctrl-] to the wrapped CLI
ctrl-]  f        →  opens ztx hint mode
```

If you share the wrapped CLI's keybinding for `ctrl-]`, change ztx's prefix
to an unused key in `~/.config/ztx/config.toml`:

```toml
prefix = "ctrl-\\"
```

---

## Known limitations

- **Session-name quality depends on the adapter.** The Claude Code adapter
  provides worktree/branch names and status emoji. The antigravity-cli adapter
  exposes conversation titles. Without an adapter, only the child's own
  terminal titles are available; their quality varies by CLI.

- **Markdown export falls back to the PTY capture** when no adapter can locate
  a native transcript. The fallback excludes alternate-screen content (anything
  displayed inside a full-screen TUI). A note to this effect is included in the
  exported file.

- **Exports are written to `$TMPDIR/ztx/`** with owner-only permissions (mode
  `0700`). ztx removes exports older than 7 days at `run` startup
  (best-effort). The OS also cleans the temp directory periodically, so exports
  are not permanent storage.
