# ztx

**ztx** (Zed / Terminal session / eXchange) is a PTY-proxy wrapper that makes AI agent CLIs
— Claude Code, antigravity-cli, and others — feel at home inside Zed's
terminal sessions (Terminal Threads in the agent panel).

Zed's Terminal Threads run the real CLI with all of its native features, but
lose the conveniences of Zed's ACP-based agent sessions. ztx restores
them:

| Feature | How |
|---------|-----|
| **Session names that follow the work** | ztx injects OSC titles, so the thread name in the agent panel shows what the session is doing (via CLI-specific adapters) |
| **Open files from the log** | `ctrl-] f` overlays hint labels on file paths in the recent output; picking one opens `zed <path>:<line>`. cmd+click works via Zed's built-in path detection |
| **Open the session log as Markdown** | `ctrl-] e` (or `ztx export`) converts the session transcript to Markdown and opens it in the editor |
| **Send editor selections into the session** | `ztx send` (bound to `cmd-alt-z` by `ztx setup zed`) injects `file:line` references and selected text into the running CLI |

## Installation

Download a binary from [releases](https://github.com/handlename/ztx/releases),
or build from source:

```sh
cargo install --path .
```

## Usage

In a Zed Terminal Thread (or any terminal):

```sh
# Wrap an agent CLI. The adapter is auto-detected from the command name.
ztx run -- claude
ztx run -- agy

# Wrap anything else: every feature still works at PTY-capture quality.
ztx run --adapter none -- bash
```

Tip: set `agent.terminal_init_command` in Zed settings to
`"ztx run -- claude"` to wrap every new Terminal Thread automatically.

### Key bindings (inside a wrapped session)

All ztx bindings hide behind a prefix key, `ctrl-]` by default (configurable
via config.toml), so the wrapped CLI keeps its entire keymap. Press the prefix
twice to send a literal prefix byte.

| Keys | Action |
|------|--------|
| `ctrl-] f` | Hint mode: labels appear on the file paths visible on screen; press one to open that path in the editor. Falls back to a list view for primary-screen sessions |
| `ctrl-] e` | Export the session log as Markdown and open it |
| `ctrl-] d` | Dump ztx's internal state to a file (diagnostics) |

### Subcommands

```sh
ztx run [--adapter auto|claude|antigravity|none]
             [--title-mode passthrough|managed|prefix] -- <cli> [args...]
ztx export [--stdout]        # export the latest session for this cwd
ztx send --from-zed-env      # inject the Zed selection (reads ZED_* env)
ztx send --file F --line N --text "..."   # inject explicitly
ztx send --socket PATH ...   # target a specific session's socket
ztx sessions                 # list running sessions (pid + socket + cwd)
ztx notify --from-hook       # refresh title/transcript from a hook (plugin)
ztx notify [--wake] [--transcript F]   # or drive it explicitly
ztx setup zed [--yes]        # install the Zed task + keybinding
ztx setup zed --preview      # show what would change, write nothing
ztx setup zed --scope project   # install into ./.zed instead of ~/.config/zed
```

### Zed integration

Run `ztx setup zed` once. It merges (with confirmation and a backup):

- a task `ztx: send selection` into `~/.config/zed/tasks.json`
- a keybinding `cmd-alt-z` into `~/.config/zed/keymap.json`

Pass `--preview` to print the additions without writing any files. Pass
`--scope project` to install into the project-local `<worktree>/.zed/`
(rooted at `ZED_WORKTREE_ROOT`, else the current directory) instead of the
global config. Because Zed has no project-local keymap, project scope writes
only the task and prints the keybinding for you to add to the global
`~/.config/zed/keymap.json` yourself.

After that, selecting text in the editor and pressing `cmd-alt-z` sends
`file:line` plus the selected text into the ztx session running in the
same project. Zed's built-in `agent::AddSelectionToThread` (`cmd->`) also
works with Terminal Threads and needs no setup.

One session per project. A bare `ztx send` routes to the session whose
working directory matches the editor's project root (`ZED_WORKTREE_ROOT`, else
the current directory), so sessions in different projects each receive their
own selections. Starting a second `ztx run` in a project that already has
a live session reports that session (pid + socket + cwd); when run
interactively it offers to terminate it and start fresh in the current
terminal — useful for reclaiming a session orphaned by an editor restart.
`ztx sessions` lists running sessions (pid + socket + cwd); `ztx send
--socket <PATH>` targets one explicitly.

### Claude Code plugin (optional, instant titles)

Without any plugin, the Claude adapter polls Claude's session state every two
seconds, so the title's status emoji (🔄/⏳/🔔) can lag a beat and exports rely
on deriving the transcript path. Installing the bundled Claude Code plugin
removes both compromises: its hooks nudge the running ztx session to refresh
the moment Claude changes state — starts working, finishes, or blocks on a
prompt — and hand ztx the exact transcript path for exports. Polling stays
the source of truth, so the plugin only *accelerates* what already works.

The plugin is entirely optional; without it everything still works via polling.

```sh
# In Claude Code:
/plugin marketplace add handlename/ztx
/plugin install ztx@ztx
```

The hooks run `ztx notify --from-hook`, which is a silent no-op unless a
ztx session is running in the same project — so it never interferes with a
plain `claude` started outside ztx. `ztx` must be on your `PATH`.

#### Desktop notifications (macOS)

With the plugin installed, ztx also raises a macOS desktop notification when
the session **waits for input** or **finishes** responding. The notification is
titled with the session (`<repo>/<worktree>`), carries the Zed icon, and
**clicking it focuses that worktree's Zed workspace**.

This needs [`terminal-notifier`](https://github.com/julienXX/terminal-notifier)
on your `PATH` (`brew install terminal-notifier`). It is strictly additive and
best-effort: when `terminal-notifier` is absent — or the host is not macOS, or
no ztx session is live — the desktop notification is simply skipped and every
other behavior (title refresh, transcript hand-off) is unaffected. Set the
notification style to *Banner* (System Settings → Notifications →
terminal-notifier) so notifications auto-dismiss; disable the feature or change
the sound via `[notify]` in `config.toml`.

## Configuration

### Configuration file

An optional `~/.config/ztx/config.toml` (honoring `$XDG_CONFIG_HOME`) sets a
few defaults. Every key is optional; the precedence is **CLI argument >
config.toml > built-in default**. A missing, unreadable, or malformed file is
ignored — each setting falls back to its default and startup is never blocked.

```toml
prefix = "ctrl-]"        # ztx prefix key, as a Ctrl chord (e.g. "ctrl-a")
editor = "zed"           # editor for exports / hint "open" (split on spaces)

[status_emoji]           # Claude session-title status prefixes
busy = "🔄"
idle = "⏳"
waiting = "🔔"           # Claude is waiting for user input (choices, prompts)

[notify]                 # macOS desktop notifications (needs terminal-notifier)
desktop = true           # fire on waiting/finished; false disables
sound = "Glass"          # sound name (see Sound Preferences); "" for silent
```

### Environment variables

| Variable | Meaning |
|----------|---------|
| `ZTX_EDITOR` | Editor command used to open files/exports. The config-file `editor` takes precedence; otherwise this overrides the built-in `zed` default, which falls back to `$EDITOR` |
| `ZTX_LOG` | Tracing filter (e.g. `debug`); logging is off when unset |
| `ZTX_LOG_FILE` | Log file path (default: `~/.local/state/ztx/ztx.log`) |
| `ZTX_RUNTIME_DIR` | Socket directory override |

## Debugging

ztx never writes logs to the terminal (that would corrupt the wrapped
TUI). To investigate any misbehavior:

1. Run with `ZTX_LOG=debug` — all subsystems (screen-state changes,
   title emissions, prefix-key actions, IPC injections, exports, hint
   candidate counts) trace to the log file (`ZTX_LOG_FILE`, default
   `~/.local/state/ztx/ztx.log`).
2. Press `ctrl-] d` in the session to write a state dump — the primary
   scrollback, the visible alternate-screen frame, screen-mode flags, and
   the child's last title — to `$TMPDIR/ztx/state-<pid>-<n>.txt`.
   Hint mode also dumps automatically when it finds no candidates.

## Notes and limitations

- Session-name quality depends on the adapter. The Claude Code adapter titles
  the session with the worktree (or branch) name, prefixed by a status emoji
  (🔄 busy, ⏳ idle, 🔔 waiting for input — all configurable in config.toml); antigravity-cli
  exposes conversation titles. Without an adapter, the child CLI's own terminal
  titles pass through.
- Markdown export uses the CLI's native transcript when an adapter can locate
  one (Claude Code). Otherwise it falls back to the ANSI-stripped terminal
  capture, which excludes alternate-screen (full-screen TUI) content.
- Exported Markdown files are written under `$TMPDIR/ztx/` (owner-only) so
  the editor can keep them open. ztx prunes exports older than 7 days at
  `run` startup (best-effort); the OS also cleans the temp directory
  periodically.
- See [DESIGN.md](DESIGN.md) for architecture, [REQUIREMENTS.md](REQUIREMENTS.md)
  for the requirements this tool answers, and [GLOSSARY.md](GLOSSARY.md) for
  terminology.

## License

[MIT](LICENSE)
