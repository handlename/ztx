# Introduction

> [!WARNING]
> **Status: alpha — use at your own risk.**
>
> - **100% vibe-coded.** The entire codebase was produced by AI agents from
>   natural-language direction.
> - **Interfaces will change.** The author develops ztx while using it daily,
>   so flags, config keys, and key bindings can change without notice or a
>   deprecation period. This manual describes the current state, not a stable
>   contract.
> - **Alpha quality.** Expect rough edges. There is no warranty and no
>   guarantee of fitness for any purpose; you use it at your own risk.
> - **Built for its author.** This is a personal tool published in case it is
>   useful to someone else. Issues are read, but a reply, a fix, or a merged
>   pull request is not promised.

> 日本語版: [ztx マニュアル](ja/introduction.html)

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

## Where to go next

- New to ztx? Start with [Installation](getting-started/installation.md), then
  [Your first session](getting-started/first-session.md).
- Using Zed? [Zed setup](getting-started/zed-setup.md) covers wrapping every Terminal Thread automatically.
- Looking for a specific feature? See the [Guide](guide/session-names.md).
- Looking up a flag or a config key? See the [Reference](reference/subcommands.md).
- Something not working? See [Troubleshooting](troubleshooting.md).

## How it works, briefly

ztx is a **passive-tap PTY proxy**: it owns a pseudo-terminal, runs the agent
CLI inside it, and relays bytes both ways unchanged — the single exception is
OSC 0/2 title handling. A side channel observes those bytes to build the state
that the features read, so ztx never rewrites the live stream. See
[Architecture](appendix/architecture.md) for the full design.
