# Zed setup

## One-time setup

Run this once from any directory:

```sh
ztx setup zed
```

ztx merges two entries into your Zed configuration — a task and a keybinding
— then exits. A confirmation prompt appears for each file before anything is
written.

### What gets added

**`~/.config/zed/tasks.json`** — a task named `ztx: send selection`:

```json
{
  "label": "ztx: send selection",
  "command": "ztx",
  "args": ["send", "--from-zed-env"],
  "reveal": "never",
  "hide": "always"
}
```

**`~/.config/zed/keymap.json`** — a binding that triggers the task:

```json
{
  "context": "Editor",
  "bindings": {
    "cmd-alt-z": ["task::Spawn", { "task_name": "ztx: send selection" }]
  }
}
```

After setup, select any text in the Zed editor and press `cmd-alt-z`. ztx
injects a `file:line` reference and the selected text into the running session
for the same project. See [Send editor selections](../guide/send-selections.md)
for the full behaviour.

### Safety

- **Confirmation**: each file shows what will be added and asks `[y/N]` before
  writing. Pass `--yes` to skip the prompts.
- **Backup**: before modifying an existing file, ztx writes a copy next to it
  (e.g. `tasks.json.ztx.bak`).
- **Comments**: Zed's JSON files can contain comments. If a file cannot be
  parsed as plain JSON, ztx prints the entry for you to add manually and
  leaves the file unchanged.
- **Idempotent**: running `ztx setup zed` again when the entries are already
  present is a no-op.

## Flags

### `--preview`

Prints what would be added to each file without writing anything:

```sh
ztx setup zed --preview
```

### `--yes`

Applies all changes without asking for confirmation:

```sh
ztx setup zed --yes
```

### `--scope project`

Installs the task into the project-local `<worktree>/.zed/tasks.json` instead
of the global `~/.config/zed/tasks.json`. The worktree root is taken from the
`ZED_WORKTREE_ROOT` environment variable, or the current directory if that
variable is unset.

```sh
ztx setup zed --scope project
```

Because Zed has no project-local keymap, the keybinding is not written to a
file under project scope. Instead, ztx prints the keymap entry so you can add
it to `~/.config/zed/keymap.json` yourself (or run `ztx setup zed` without
`--scope project` once to write it globally).

## Wrapping every Terminal Thread automatically

Set `terminal_init_command` in Zed's `settings.json` to wrap every new
Terminal Thread in the agent panel without typing `ztx run` each time:

```json
{
  "agent": {
    "terminal_init_command": "ztx run -- claude"
  }
}
```

Open Zed's settings with `cmd-,`, search for `terminal_init_command`, and set
the value to the agent CLI you want to use.

## Zed's built-in selection shortcut

Zed also has a native action, `agent::AddSelectionToThread` (`cmd->`), that
sends the current editor selection into the focused agent thread. This works
with Terminal Threads and requires no ztx setup — it is entirely independent
of `ztx send` and `cmd-alt-z`. The two approaches complement each other:
`cmd->` targets the active thread directly, while `cmd-alt-z` routes to the
ztx session running in the same project regardless of which thread is focused.
