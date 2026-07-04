# Glossary

| Term | Definition |
|------|------------|
| **Agent CLI** | A terminal-based AI coding agent such as Claude Code (`claude`) or antigravity-cli (`agy`). The process zediator wraps. |
| **Terminal Thread** | A terminal session hosted in Zed's agent panel (also called a terminal session). Runs the real CLI; its sidebar name follows the terminal's OSC title. |
| **Wrapper** | The `zediator run` process: a PTY proxy sitting between the parent terminal (Zed) and the agent CLI. |
| **PTY proxy** | The core passthrough: zediator owns a pseudo-terminal, runs the child inside it, and relays bytes both ways while observing them. |
| **Adapter** | A per-CLI plugin (`--adapter`) that raises feature quality using CLI-specific knowledge: session titles, native transcripts. One exists for Claude Code and one for antigravity-cli. |
| **Fallback quality** | Behavior when no adapter applies: titles pass through from the child, and exports use the PTY capture. Every feature keeps working. |
| **Session log** | What the session did. Two sources: the CLI's **native transcript** (e.g. Claude Code's session JSONL) and zediator's own **scrollback capture**. |
| **Scrollback** | zediator's ANSI-stripped, line-oriented record of child output on the primary screen. Bounded ring buffer with a temp-file spill. |
| **Tap** | The observer parsing child output (via VTE) into the scrollback and screen-state flags without modifying the passthrough stream. |
| **OSC title** | The `OSC 0/2` escape sequence setting a terminal title. Zed shows it as the Terminal Thread's session name. |
| **Title mode** | How zediator treats the child's OSC titles: `passthrough` (forward), `managed` (suppress; zediator emits adapter-driven titles), `prefix` (rewrite with a prefix). |
| **Prefix key** | `ctrl-]` by default. All zediator key bindings live behind it so the wrapped CLI keeps its own keymap. |
| **Hint mode** | `ctrl-] f`: an overlay labeling file paths found in the scrollback; typing a label opens that path in the editor (tmux-thumbs style). |
| **Export** | Converting the session log to Markdown and opening it in the editor (`ctrl-] e` or `zediator export`). |
| **IPC socket** | Per-wrapper Unix socket (`<pid>.sock`) accepting messages from `zediator send`, injected into the child as a bracketed paste. |
| **`latest.sock`** | Symlink to the most recently started wrapper's socket; the default target for `zediator send`. |
| **Bracketed paste** | Terminal convention (`ESC[200~ … ESC[201~`) marking pasted text, so multi-line injections arrive as a single paste. |
| **Alternate screen** | The full-screen terminal buffer used by TUIs (vim, less). Not part of the scrollback capture; exports note its absence. |
