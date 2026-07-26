# Send editor selections

`ztx send` injects a file reference and optional selected text into the
running ztx session for the current project. The session receives it as a
bracketed paste, so multi-line content arrives as a single atomic insert
rather than line-by-line.

The injected text follows this format:

```
path/to/file.rs:42 
```python
selected text here
```
```

The fence length is automatically extended beyond any backtick run in the
selection, so selecting Markdown that itself contains fenced blocks is handled
correctly.

## Zed keybinding (recommended)

Run `ztx setup zed` once to install the task and keybinding:

```sh
ztx setup zed          # install into ~/.config/zed/
ztx setup zed --scope project   # install into ./.zed/ (task only)
ztx setup zed --preview         # show changes without writing
```

After setup, select text in any Zed editor buffer and press `cmd-alt-z`. Zed
runs `ztx send --from-zed-env`, which reads the selection details from the
environment variables Zed injects into every task:

| Variable | Content |
|----------|---------|
| `ZED_RELATIVE_FILE` | Path of the active file (relative to worktree root) |
| `ZED_ROW` | Cursor line number |
| `ZED_SELECTED_TEXT` | Currently selected text |

Using `--from-zed-env` rather than passing the values on the command line
avoids the shell re-executing the selection text, which matters when the
selection contains shell metacharacters.

## Zed built-in: AddSelectionToThread

Zed's built-in `agent::AddSelectionToThread` action (default: `cmd->`) also
works with Terminal Threads and requires no ztx setup. It pastes the selection
directly into the active thread. Use whichever approach your workflow prefers;
both deliver the selection to the same session.

## Explicit flags

When calling `ztx send` outside Zed, or when you want to target a specific
file and line without Zed's environment, pass the values explicitly:

```sh
ztx send --file src/main.rs --line 42 --text "fn main() {}"

# File reference only (no text body)
ztx send --file src/main.rs --line 10

# Free-form message (no file reference)
ztx send "please review the last change"
```

## Targeting a specific session

By default, `ztx send` routes to the session whose project directory matches
`ZED_WORKTREE_ROOT` (or the current directory). To target a different session
explicitly:

```sh
ztx send --socket /path/to/session.sock --file foo.rs --line 1
```

Use `ztx sessions` to list socket paths for all running sessions.

See [One session per project](one-session-per-project.md) for how routing
works when multiple sessions are running.
