# Subcommands

ztx exposes six subcommands. All persistent preferences live in
[`config.toml`](configuration.md); the flags below override them for a single
invocation.

---

## `ztx run`

Run an Agent CLI wrapped in the ztx PTY proxy. All ztx features (key
bindings, hint mode, export, IPC) are active for the lifetime of the child
process.

```sh
ztx run [OPTIONS] -- <command> [args...]
```

The `--` separator is required. Everything after it is passed verbatim to the
child as its argv.

| Flag | Values | Default | Description |
|------|--------|---------|-------------|
| `--adapter` | `auto` \| `claude` \| `antigravity` \| `none` | `auto` | Selects the Adapter. `auto` detects from the command name (`claude` → Claude Code adapter, `agy` / `antigravity` → Antigravity adapter, anything else → no adapter). |
| `--title-mode` | `passthrough` \| `managed` \| `prefix` | `managed` when an adapter matches, `passthrough` otherwise | Controls how OSC title sequences from the child are handled. `passthrough` forwards them unchanged; `managed` suppresses them and lets ztx emit adapter-driven titles; `prefix` rewrites them with a fixed string. |
| `--title-prefix` | string | `"<command>: "` | The prefix string used when `--title-mode prefix` is active. |

### Examples

```sh
# Wrap Claude Code. Adapter and title-mode are auto-detected.
ztx run -- claude

# Wrap an arbitrary shell; every feature still works at PTY-capture quality.
ztx run --adapter none -- bash

# Force managed titles with a custom prefix on an unlisted CLI.
ztx run --adapter none --title-mode prefix --title-prefix "work: " -- mycli
```

---

## `ztx export`

Export the latest session transcript for the current directory as Markdown and
open it in the editor. When called from inside a running wrapper session,
`ctrl-] e` does the same thing and also has access to the live PTY capture.

```sh
ztx export [OPTIONS]
```

| Flag | Values | Default | Description |
|------|--------|---------|-------------|
| `--adapter` | `auto` \| `claude` \| `antigravity` \| `none` | `auto` | Selects the Adapter used to locate the native transcript. `auto` and `claude` both try the Claude Code transcript; `none` skips the native transcript and relies on the PTY-capture scrollback. |
| `--stdout` | — | off | Write the Markdown to stdout instead of opening an editor. |

### Examples

```sh
# Export the current project's session and open it in the editor.
ztx export

# Pipe the Markdown to another tool.
ztx export --stdout | pbcopy
```

---

## `ztx send`

Send a file reference, line number, or selected text into a running ztx
session. The message is injected as a [Bracketed paste](../appendix/glossary.md) so
the Agent CLI receives it as a single unit. Designed to be called from a Zed
task (see `ztx setup zed`).

```sh
ztx send [OPTIONS] [message...]
```

| Flag | Values | Default | Description |
|------|--------|---------|-------------|
| `--from-zed-env` | — | off | Read file, line, and selected text from the `ZED_RELATIVE_FILE`, `ZED_ROW`, and `ZED_SELECTED_TEXT` environment variables instead of explicit flags. Preferred in Zed tasks to avoid shell injection via `$ZED_*` interpolation. |
| `--file` | path string | — | File path to include in the message. |
| `--line` | integer | — | Line number to attach to the file reference. |
| `--text` | string | — | Selected text to attach as a fenced code block. |
| `--socket` | path | project socket | Target a specific session by its Unix socket path. Without this flag, ztx routes to the session whose project directory matches `ZED_WORKTREE_ROOT` (or the current directory). |
| `message` | positional, multiple words | — | Free-form message text appended after any file/text context. |

### Examples

```sh
# Inject the current Zed selection (called by the Zed task installed by setup zed).
ztx send --from-zed-env

# Inject an explicit reference.
ztx send --file src/main.rs --line 42 --text "this panics on empty input"

# Target a specific session.
ztx send --socket ~/.local/share/ztx/abc123.sock "please review this"
```

---

## `ztx notify`

Notify a running session of an activity change. Used by the Claude Code plugin
hooks; a silent no-op when no ztx session is running in the same project.

```sh
ztx notify [OPTIONS]
```

| Flag | Values | Default | Description |
|------|--------|---------|-------------|
| `--from-hook` | — | off | Read hook JSON from stdin, derive the working directory and transcript path from it, and wake the session's managed title. Preferred in Claude Code plugin hooks. |
| `--wake` | — | off | Force the session's managed title to refresh immediately. |
| `--transcript` | path | — | Record the authoritative transcript path for the next `export`. |
| `--socket` | path | project socket | Target a specific session by its Unix socket path. |

### Examples

```sh
# Called automatically by the Claude Code plugin hook.
ztx notify --from-hook

# Manually wake the title (e.g. for testing).
ztx notify --wake

# Hand ztx the exact transcript path.
ztx notify --transcript ~/.claude/projects/-home-user-myproject/session.jsonl
```

---

## `ztx sessions`

List all running ztx sessions. Prints one line per session: PID, socket path,
and working directory.

```sh
ztx sessions
```

No options. Example output:

```
12345  /tmp/ztx/abc123.sock  /home/user/myproject
67890  /tmp/ztx/def456.sock  /home/user/otherproject
```

---

## `ztx setup zed`

Generate and merge a ztx task and keybinding into the Zed configuration. Run
once after installing ztx. Prompts for confirmation and creates a backup before
writing any files.

```sh
ztx setup zed [OPTIONS]
```

| Flag | Values | Default | Description |
|------|--------|---------|-------------|
| `--yes` | — | off | Apply changes without asking for confirmation. |
| `--preview` | — | off | Show the changes that would be made without writing any files. |
| `--scope` | `global` \| `project` | `global` | Where to write the Zed configuration. `global` writes to `~/.config/zed/`. `project` writes to `<worktree>/.zed/` (rooted at `ZED_WORKTREE_ROOT`, else the current directory); because Zed has no project-local keymap, project scope writes only the task and prints the keybinding for manual addition. |

### Examples

```sh
# Interactive installation into the global Zed config.
ztx setup zed

# Preview what would change without writing anything.
ztx setup zed --preview

# Non-interactive install into the current project's .zed directory.
ztx setup zed --scope project --yes
```
