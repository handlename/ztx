# Export the session log

Press `ctrl-] e` inside a wrapped session, or run `ztx export` from any
terminal in the same project directory, to convert the session log to Markdown
and open it in the editor.

```sh
# Open as Markdown in the editor
ztx export

# Write Markdown to stdout instead
ztx export --stdout
```

The `--adapter` flag selects which adapter to use when locating the native
transcript (default: auto-detected from the project):

```sh
ztx export --adapter claude
ztx export --adapter none   # force PTY-capture fallback
```

## Native transcript vs. terminal capture

Export quality depends on whether an adapter can locate the CLI's native
transcript.

### Native transcript (Claude Code adapter)

When the Claude Code adapter finds the active session's JSONL transcript at
`~/.claude/projects/<slug>/<sessionId>.jsonl`, it converts it to structured
Markdown:

- Each `user` turn becomes a `## User` section.
- Each `assistant` turn becomes an `## Assistant` section.
- Tool calls are rendered with their name and JSON input in a fenced block.
- Tool results are summarised as `*(tool result)*` to avoid enormous output.
- Unknown message types and malformed lines are silently skipped.

The Claude Code plugin (`ztx notify --from-hook`) hands ztx the exact
transcript path at the moment Claude starts a session, removing any need to
guess it. Without the plugin, the adapter derives the path from the session
registry and the project slug.

### PTY-capture fallback

When no native transcript is available — because no adapter matches, or the
adapter could not locate the file — ztx falls back to the ANSI-stripped
terminal scrollback captured during the session.

The fallback produces readable output in session order, but differs from the
native transcript in two ways:

- There is no role separation between user and assistant turns.
- Content shown in the **alternate screen** (full-screen TUI dialogs) is not
  included in the scrollback. If the child is currently in a full-screen mode,
  ztx appends a `## Current screen` section with a snapshot of the visible
  frame as a best-effort supplement.

The fallback header includes a note explaining the capture method so readers
know what they are looking at.

## Export file location

Exported files are written to `$TMPDIR/ztx/` with names of the form
`session-<random>.md`. The directory is created with owner-only permissions
(`0700`) so conversation content is not world-readable.

Files are left in place so the editor can keep them open after the export
command exits. ztx prunes exports older than seven days each time `ztx run`
starts (best-effort; errors are logged and never delay launch). The OS also
cleans `$TMPDIR` periodically.

## Editor resolution

The editor used to open the export follows this precedence:

1. `editor` key in `~/.config/ztx/config.toml`
2. `$ZTX_EDITOR` environment variable
3. `zed` if found on `PATH`
4. `$EDITOR` environment variable

See [Configuration](../reference/configuration.md) for the `editor` config key.
