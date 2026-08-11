# Introduction

> [!WARNING]
> **Status: alpha — use at your own risk.**
>
> - **100% vibe-coded.** The entire codebase was produced by AI agents from
>   natural-language direction.
> - **Interfaces will change.** The author develops ztx while using it daily,
>   so while ztx is pre-1.0, flags, config keys, and key bindings can change
>   without notice or a deprecation period. This manual describes the current
>   state, not a stable contract. See [Feature lifecycle](#feature-lifecycle).
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

## Feature lifecycle

Every feature above is a gap-filler for something Zed's Terminal Threads do
not do. When Zed ships an equivalent, the Zed one wins and ztx's version is
removed: expect this manual to lose pages over time rather than gain them.

That already cost this manual a page: editor-selection sending was dropped
once Zed's `agent::AddSelectionToThread` (`cmd->`) worked in Terminal Threads,
and the `ztx setup zed` command that bound it went too.

Partial overlap is not equivalence, though. Where a Zed feature covers only
part of the need, both stay and this manual explains the difference — see
[Open files from the log](guide/open-files.md), where cmd+click and hint mode
(`ctrl-] f`) solve neighbouring problems.

Removals follow [Semantic Versioning](https://semver.org/):

- **Before 1.0** — a feature can be removed in any release; 0.x makes no
  compatibility promise.
- **1.0 onward** — the feature is marked deprecated in this manual and in
  `ztx --help` first and keeps working, then is removed no earlier than the
  next major version.
