# Your first session

This walkthrough shows what ztx does step by step. It assumes you have
[installed ztx](installation.md) and have Claude Code (`claude`) available.

## Step 1: Start a wrapped session

Open a terminal — inside Zed's agent panel or anywhere else — and run:

```sh
ztx run -- claude
```

ztx launches Claude Code inside a PTY proxy. The output looks identical to
running `claude` directly. Every key you press goes through to Claude; every
byte Claude writes comes back to your terminal unchanged. ztx is transparent
by default.

To wrap a different agent, replace `claude` with its command. The adapter is
auto-detected from the command name:

```sh
ztx run -- agy          # antigravity-cli
```

For anything else — a shell, a custom script — pass `--adapter none`:

```sh
ztx run --adapter none -- bash
```

All ztx key bindings and the export feature still work; only the
adapter-specific session-name enrichment is absent.

## Step 2: Watch the session name change

If you are running inside a Zed Terminal Thread, look at the thread's name in
the agent panel sidebar. As Claude transitions between states — thinking,
waiting for your reply, idle — ztx updates the name with a status emoji
(`🔄` busy, `⏳` idle, `🔔` waiting for input).

The name itself comes from your project: typically the worktree or branch name
derived from the current directory. See [Session names](../guide/session-names.md)
for the full details and configuration options.

## Step 3: Open a file from the log

After Claude has produced some output, press:

    ctrl-] f

Hint labels appear on every file path visible on screen. Type the label for a
path (one or two characters) to open that file in Zed at the indicated line.
Press `Escape` to dismiss without opening anything.

Zed's built-in cmd+click also detects `path:line:col` patterns and opens them
directly — no ztx action needed for those.

See [Open files from the log](../guide/open-files.md) for more detail on hint
mode and its fallback behaviour.

## Step 4: Export the session log

Press:

    ctrl-] e

ztx converts the session transcript to Markdown and opens it in your editor.
When the Claude adapter can locate Claude's native transcript, that is used
(full fidelity). Otherwise the export falls back to ztx's PTY capture of the
primary screen.

You can also run `ztx export` from another terminal in the same project to
export the most recent session for that directory.

See [Export the session log](../guide/export-log.md) for more.

## What's next

- [Zed setup](zed-setup.md) — wrap every Terminal Thread automatically with
  `terminal_init_command`.
- [Key bindings](../reference/key-bindings.md) — the full list of `ctrl-]`
  actions and how to change the prefix key.
