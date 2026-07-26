# Open files from the log

Press `ctrl-] f` inside a wrapped session to activate hint mode. ztx scans
the recent session output for file paths, labels each one, and opens the
chosen path in the editor.

## Basic flow

1. Press `ctrl-] f`.
2. Hint labels (single letters, then two-letter pairs) appear on or beside
   each file path found in the output.
3. Type the label for the path you want. The editor opens at that file and
   line.
4. Press `Esc`, `q`, or `ctrl-c` to cancel without opening anything.

The editor command follows the same resolution order as exports: the
`editor` key in `config.toml`, then `$ZTX_EDITOR`, then `zed`, then
`$EDITOR`. When the editor is Zed, the path is passed as `path:line:column`
so Zed jumps directly to the location.

## Overlay mode vs. list mode

ztx chooses between two hint-mode displays depending on the session's current
screen state.

**Alternate-screen sessions** (full-screen TUIs such as Claude Code's
interactive dialog) use the **in-place overlay**: hint labels are painted
directly over the visible terminal frame beside each detected path. Picking a
label restores the original characters before opening the file, so the
session's screen is undisturbed.

**Primary-screen sessions** (ordinary scrolling terminal output) fall back to
the **list view**: ztx enters its own alternate screen, shows a numbered list
of candidates extracted from the scrollback, and restores the session's screen
after the selection. The list is ordered most-recent-first so the path you
most likely want appears near the top.

## Path detection

ztx recognises three path formats:

- **Multi-segment paths** — anything containing a `/`, optionally followed
  by `:line` or `:line:column` (e.g. `src/main.rs:42:7`).
- **Bare filenames with an extension** — e.g. `main.rs`, also accepting an
  optional `:line` suffix.
- **Python tracebacks** — the `File "path", line N` format is recognised
  explicitly.

Relative paths are resolved against the session's working directory. Absolute
and `~/`-prefixed paths are resolved directly. Bare filenames and partial
paths that do not exist at the working directory are looked up in a worktree
index (a bounded scan of the project tree, skipping `target`, `node_modules`,
`vendor`, `dist`, `build`, and `__pycache__`). Only paths that exist on disk
appear as candidates.

## cmd+click

Zed's built-in path detection works in Terminal Threads without any ztx
involvement. Holding `cmd` and clicking a file path in the terminal output
opens it in the editor directly. Hint mode and cmd+click complement each
other: use cmd+click for a single visible path and hint mode when you want to
browse several candidates from the recent output.

## When no candidates are found

If hint mode finds zero file paths, it automatically writes a state dump to
`$TMPDIR/ztx/state-<pid>-<n>.txt` (the same file produced by `ctrl-] d`) and
shows a message. The dump includes the raw scrollback and the visible screen
frame, which is useful for reporting unexpected misses.

See [Troubleshooting](../troubleshooting.md) for how to read a state dump.
