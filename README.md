# zediator

**zediator** (Zed + Mediator) is a PTY-proxy wrapper that makes AI agent CLIs
— Claude Code, antigravity-cli, and others — feel at home inside Zed's
terminal sessions (Terminal Threads in the agent panel).

Zed's Terminal Threads run the real CLI with all of its native features, but
lose the conveniences of Zed's ACP-based agent sessions. zediator restores
them:

| Feature | How |
|---------|-----|
| **Session names that follow the work** | zediator injects OSC titles, so the thread name in the agent panel shows what the session is doing (via CLI-specific adapters) |
| **Open files from the log** | `ctrl-] f` overlays hint labels on file paths in the recent output; picking one opens `zed <path>:<line>`. cmd+click works via Zed's built-in path detection |
| **Open the session log as Markdown** | `ctrl-] e` (or `zediator export`) converts the session transcript to Markdown and opens it in the editor |
| **Send editor selections into the session** | `zediator send` (bound to `cmd-alt-z` by `zediator setup zed`) injects `file:line` references and selected text into the running CLI |

## Installation

Download a binary from [releases](https://github.com/handlename/zediator/releases),
or build from source:

```sh
cargo install --path .
```

## Usage

In a Zed Terminal Thread (or any terminal):

```sh
# Wrap an agent CLI. The adapter is auto-detected from the command name.
zediator run -- claude
zediator run -- agy

# Wrap anything else: every feature still works at PTY-capture quality.
zediator run --adapter none -- bash
```

Tip: set `agent.terminal_init_command` in Zed settings to
`"zediator run -- claude"` to wrap every new Terminal Thread automatically.

### Key bindings (inside a wrapped session)

All zediator bindings hide behind a prefix key, `ctrl-]` by default, so the
wrapped CLI keeps its entire keymap. Press `ctrl-]` twice to send a literal
`ctrl-]`.

| Keys | Action |
|------|--------|
| `ctrl-] f` | Hint mode: labels appear on the file paths visible on screen; press one to open that path in the editor. Falls back to a list view for primary-screen sessions |
| `ctrl-] e` | Export the session log as Markdown and open it |
| `ctrl-] d` | Dump zediator's internal state to a file (diagnostics) |

### Subcommands

```sh
zediator run [--adapter auto|claude|antigravity|none]
             [--title-mode passthrough|managed|prefix] -- <cli> [args...]
zediator export [--stdout]        # export the latest session for this cwd
zediator send --file F --line N --text "..."   # inject into a running session
zediator sessions                 # list running wrapper sessions
zediator setup zed [--yes]        # install the Zed task + keybinding
```

### Zed integration

Run `zediator setup zed` once. It merges (with confirmation and a backup):

- a task `zediator: send selection` into `~/.config/zed/tasks.json`
- a keybinding `cmd-alt-z` into `~/.config/zed/keymap.json`

After that, selecting text in the editor and pressing `cmd-alt-z` sends
`file:line` plus the selected text into the most recent zediator session.
Zed's built-in `agent::AddSelectionToThread` (`cmd->`) also works with
Terminal Threads and needs no setup.

## Configuration

Environment variables:

| Variable | Meaning |
|----------|---------|
| `ZEDIATOR_EDITOR` | Editor command used to open files/exports (default: `zed`, falling back to `$EDITOR`) |
| `ZEDIATOR_LOG` | Tracing filter (e.g. `debug`); logging is off when unset |
| `ZEDIATOR_LOG_FILE` | Log file path (default: `~/.local/state/zediator/zediator.log`) |
| `ZEDIATOR_RUNTIME_DIR` | Socket directory override |

## Debugging

zediator never writes logs to the terminal (that would corrupt the wrapped
TUI). To investigate any misbehavior:

1. Run with `ZEDIATOR_LOG=debug` — all subsystems (screen-state changes,
   title emissions, prefix-key actions, IPC injections, exports, hint
   candidate counts) trace to the log file (`ZEDIATOR_LOG_FILE`, default
   `~/.local/state/zediator/zediator.log`).
2. Press `ctrl-] d` in the session to write a state dump — the primary
   scrollback, the visible alternate-screen frame, screen-mode flags, and
   the child's last title — to `$TMPDIR/zediator/state-<pid>-<n>.txt`.
   Hint mode also dumps automatically when it finds no candidates.

## Notes and limitations

- Session-name quality depends on the adapter. Claude Code exposes derived
  session names; antigravity-cli exposes conversation titles. Without an
  adapter, the child CLI's own terminal titles pass through.
- Markdown export uses the CLI's native transcript when an adapter can locate
  one (Claude Code). Otherwise it falls back to the ANSI-stripped terminal
  capture, which excludes alternate-screen (full-screen TUI) content.
- Exported Markdown files accumulate under `$TMPDIR/zediator/` (owner-only)
  so the editor can keep them open; the OS cleans the temp directory
  periodically, or delete them manually.
- See [DESIGN.md](DESIGN.md) for architecture, [REQUIREMENTS.md](REQUIREMENTS.md)
  for the requirements this tool answers, and [GLOSSARY.md](GLOSSARY.md) for
  terminology.

## License

[MIT](LICENSE)
