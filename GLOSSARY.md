# Glossary

| Term | Definition |
|------|------------|
| **Agent CLI** | A terminal-based AI coding agent such as Claude Code (`claude`) or antigravity-cli (`agy`). The process ztx wraps. |
| **Terminal Thread** | A terminal session hosted in Zed's agent panel (also called a terminal session). Runs the real CLI; its sidebar name follows the terminal's OSC title. |
| **Wrapper** | The `ztx run` process: a PTY proxy sitting between the parent terminal (Zed) and the agent CLI. |
| **PTY proxy** | The core passthrough: ztx owns a pseudo-terminal, runs the child inside it, and relays bytes both ways while observing them. |
| **Adapter** | A per-CLI plugin (`--adapter`) that raises feature quality using CLI-specific knowledge: session titles, native transcripts. One exists for Claude Code and one for antigravity-cli. |
| **Fallback quality** | Behavior when no adapter applies: titles pass through from the child, and exports use the PTY capture. Every feature keeps working. |
| **Session log** | What the session did. Two sources: the CLI's **native transcript** (e.g. Claude Code's session JSONL) and ztx's own **scrollback capture**. |
| **Scrollback** | ztx's ANSI-stripped, line-oriented record of child output on the primary screen. Bounded ring buffer with a temp-file spill. |
| **Tap** | The observer parsing child output (via VTE) into the scrollback and screen-state flags without modifying the passthrough stream. |
| **OSC title** | The `OSC 0/2` escape sequence setting a terminal title. Zed shows it as the Terminal Thread's session name. |
| **Title mode** | How ztx treats the child's OSC titles: `passthrough` (forward), `managed` (suppress; ztx emits adapter-driven titles), `prefix` (rewrite with a prefix). |
| **Prefix key** | `ctrl-]` by default (configurable in config.toml). All ztx key bindings live behind it so the wrapped CLI keeps its own keymap. |
| **Config file** | Optional `~/.config/ztx/config.toml` setting the prefix key, editor command, and Claude status-title emoji. Precedence: CLI argument > config.toml > built-in default; a missing or malformed file falls back to defaults. |
| **Hint mode** | `ctrl-] f`: an overlay labeling file paths found in the scrollback; typing a label opens that path in the editor (tmux-thumbs style). |
| **Export** | Converting the session log to Markdown and opening it in the editor (`ctrl-] e` or `ztx export`). |
| **IPC socket** | Per-wrapper Unix socket (`<hash>.sock`, named from the project directory) accepting control frames from `ztx notify`. |
| **Project socket** | Each session's socket is named by a hash of its project directory (`<hash>.sock`), so `ztx notify` finds it in O(1) from the project root. One session per project; a sibling `<hash>.info` records pid + cwd for `sessions`. |
| **Alternate screen** | The full-screen terminal buffer used by TUIs (vim, less). Not part of the scrollback capture; exports note its absence. |
