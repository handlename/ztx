# Session names

Zed displays the OSC title of each Terminal Thread as its name in the agent
panel. ztx controls that title, so the thread name follows what the session is
actually doing.

## How it works

When `ztx run` starts, it suppresses the child's own OSC title sequences and
periodically emits its own instead (managed mode). The title it emits comes
from the active adapter. Without an adapter the child's titles pass through
unchanged, and all other terminal escape sequences are always forwarded
byte-for-byte.

## Adapter quality

The title shown depends on which adapter is active.

### Claude Code adapter (default for `claude`)

The Claude Code adapter reads Claude's per-process session registry at
`~/.claude/sessions/<pid>.json`. It builds the title as:

```
{status emoji} {worktree name}
```

The worktree name is derived from the session's working directory in this
order:

1. Worktree name — when the path follows the `.../worktrees/<repo>/<name>/<repo>`
   layout (git worktrees), the worktree name is used (e.g. `elder-reef`).
2. Git branch — the current branch of the repository.
3. Directory basename — the final path component as a last resort.

The status emoji changes as Claude works:

| Status | Default emoji | Meaning |
|--------|--------------|---------|
| `busy` | 🔄 | Claude is processing a request |
| `idle` | ⏳ | Claude is waiting for a prompt |
| `waiting` | 🔔 | Claude needs user input (permission prompt, choice menu) |

All three emojis are configurable in `config.toml` under `[status_emoji]`. Set
any of them to an empty string to omit the prefix for that state.

Without the Claude Code plugin, the adapter polls the session registry every
two seconds. The title's status emoji may therefore lag slightly behind the
actual state.

#### Naming the Claude Code session

In a git worktree checkout, `ztx run` also passes `-n <worktree name>` to
`claude`, so Claude Code names its own session the same way the thread is
labelled. The name appears in Claude Code's session picker, and
`claude --resume <worktree name>` resolves it.

The flag is inserted before the arguments you pass after `--`. Claude Code
takes the last `-n` on the command line, so `ztx run -- claude -n mine` still
names the session `mine`.

Outside a worktree layout the command is left untouched. The branch and
basename fallbacks work fine as a thread label, but a Claude Code session name
is a machine-wide resume handle, and every repository checked out on `main`
would claim the same one.

### antigravity-cli adapter (default for `agy`)

The antigravity-cli adapter looks up the current conversation title from
`~/.gemini/antigravity-cli/cache/last_conversations.json` and
`conversation_summaries.db` for the working directory. It shows the
conversation title exactly as antigravity-cli records it. Because the
conversation payload format is opaque, no status emoji is added.

### No adapter (`--adapter none`)

The child CLI's own OSC title sequences are forwarded unchanged. Every other
ztx feature (hint mode, export, send) still works at terminal-capture quality.

## Title modes

The `--title-mode` flag controls how ztx handles the child's title sequences:

| Mode | Behavior |
|------|----------|
| `managed` | Suppress the child's titles; ztx emits adapter-driven titles. Default when an adapter matches. |
| `passthrough` | Forward the child's titles unchanged. Default when no adapter matches. |
| `prefix` | Rewrite the child's titles with a fixed prefix (set via `--title-prefix`). |

```sh
# Use a custom prefix instead of adapter-driven titles
ztx run --title-mode prefix --title-prefix "myproject: " -- claude
```

## Claude Code plugin (instant title updates)

Without the plugin, title updates depend on the two-second polling interval.
Installing the bundled Claude Code plugin eliminates that lag: its hooks call
`ztx notify --from-hook` the moment Claude changes state, which wakes the
title thread immediately.

```sh
# In Claude Code:
/plugin marketplace add handlename/ztx
/plugin install ztx@ztx
```

The hooks are a silent no-op when no ztx session is running for the current
project, so they never interfere with a plain `claude` started outside ztx.
Polling remains the source of truth; the plugin only accelerates what already
works.

See [Configuration](../reference/configuration.md) for the full list of
`[status_emoji]` keys.

## Desktop notifications (macOS)

With the Claude Code plugin installed, ztx can also raise a macOS desktop
notification when Claude waits for input or finishes responding.

**Requirements:**

- macOS
- [`terminal-notifier`](https://github.com/julienXX/terminal-notifier) on your
  `PATH` (`brew install terminal-notifier`)
- Claude Code plugin installed and active

Each notification carries:

- **Title** — `<repo>/<worktree>` (e.g. `ztx/push-notification`), matching
  the thread name shown in Zed.
- **Subtitle** — the status emoji plus "Waiting for input" or "Finished".
- **Zed icon** — extracted from `Zed.app` and cached on first use.
- **Click action** — opens `zed <cwd>`, focusing that project's Zed workspace.

Notifications use a per-session group ID so a new event replaces the previous
banner for the same session rather than stacking.

The feature is strictly additive. When `terminal-notifier` is absent, or the
host is not macOS, or no ztx session is live, the desktop notification is
silently skipped and title refresh continues as normal.

Set the notification style to **Banner** in System Settings → Notifications →
terminal-notifier so banners auto-dismiss.

Configure or disable the feature in `config.toml`:

```toml
[notify]
desktop = true   # set to false to disable entirely
sound = "Glass"  # sound name from Sound Preferences; "" for silent
```

See [Configuration](../reference/configuration.md) for all `[notify]` keys.
