# Design

Deliberately terse; module docs in `src/` carry the details.

## Approach

zediator is a **passive-tap PTY proxy**: child output is forwarded unchanged
(the single exception is OSC 0/2 title handling), while a side channel
observes the bytes to build state that the features read. Features never
rewrite the live stream; interactive UI (hint mode) is drawn only on demand
on the alternate screen, with the output pump paused.

The alternative — rewriting the stream inline (e.g. injecting OSC 8
hyperlinks everywhere) — was rejected for v1: split-escape parsing plus
interference management during TUI redraws costs more than it returns while
Zed's native path detection already covers clicking.

## Structure

```
zediator run -- <agent-cli>
┌────────────────────────────────────────────────────────┐
│ pty        portable-pty child; raw mode; SIGWINCH;     │
│            signal forwarding; exit-code propagation    │
│ term       VTE tap → Scrollback (line buffer) +        │
│            alt-screen flag + last child title          │
│ title      OSC 0/2 filter (buffer until terminator) +  │
│            managed-title thread (adapter polling)      │
│ input      prefix-key (ctrl-]) filter on stdin         │
│ hint       path extraction + in-place labels           │
│            (modal list fallback on primary screen)     │
│ export     transcript→Markdown / capture→Markdown      │
│ ipc        per-project Unix socket (cwd-hash name);    │
│            `send` client; one session per project      │
│ adapter    trait + ClaudeCode / Antigravity / fallback │
│ setup      Zed tasks.json / keymap.json merging        │
└────────────────────────────────────────────────────────┘
```

Threads: stdin pump (owns the input filter and runs prefix actions), output
pump (owns the title filter and tap), title thread (managed mode only,
2-second adapter polls), signal thread, IPC accept loop. Two writers share
the parent terminal — the output pump and the title thread — serialized by a
mutex ("stdout gate"). Hint mode holds the gate for its whole interaction, so
the child cannot repaint over the overlay (PTY backpressure holds its
output).

## Key decisions

- **No cell grid.** A real screen model is a terminal emulator. The line
  buffer (with CR-overwrite and erase-in-line handling) is enough for path
  extraction and readable exports. If precision ever falls short, swap in
  `alacritty_terminal::Term` behind the same tap interface.
- **Adapters are best-effort readers of undocumented state** (Claude Code's
  `~/.claude/sessions/*.json` and transcript JSONL; agy's
  `conversation_summaries.db` and `last_conversations.json`). Schema drift
  must degrade to fallback behavior, never break the wrapper.
- **Terminal restoration is non-negotiable.** Raw mode is guarded by RAII
  plus a panic hook; the managed title is cleared on exit.
- **Zed config is opt-in.** `setup zed` shows the change, asks, and backs up;
  files with comments are never rewritten automatically.
- **Logs never touch the terminal** (`ZEDIATOR_LOG` writes to a file); stray
  output would corrupt the child's screen.

## Security notes

- IPC sockets live in a 0700 per-user directory; anything writable there can
  inject prompt text into sessions — same trust boundary as the user's shell.
- Exports may contain conversation content; they are written 0600 into the
  user's temp directory.
- Adapters open external CLI state strictly read-only.
