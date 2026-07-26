# ztx

> [!WARNING]
> **Status: alpha — use at your own risk.**
>
> - **100% vibe-coded.** The entire codebase was produced by AI agents from
>   natural-language direction.
> - **Interfaces will change.** The author develops ztx while using it daily,
>   so flags, config keys, and key bindings can change without notice or a
>   deprecation period.
> - **Alpha quality.** Expect rough edges. There is no warranty and no
>   guarantee of fitness for any purpose; you use it at your own risk.
> - **Built for its author.** This is a personal tool published in case it is
>   useful to someone else. Issues are read, but a reply, a fix, or a merged
>   pull request is not promised.

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

## Installation

Download a binary from [releases](https://github.com/handlename/ztx/releases),
or build from source:

```sh
cargo install --path .
```

## Quick start

```sh
ztx run -- claude        # wrap an agent CLI (adapter auto-detected)
```

## Documentation

- **[User Manual (English)](https://handlename.github.io/ztx/)**
- **[ユーザーマニュアル (日本語)](https://handlename.github.io/ztx/ja/)**

The manual covers installation, every feature, the full CLI and configuration
reference, and troubleshooting.

For contributors: [DESIGN.md](DESIGN.md) (architecture),
[REQUIREMENTS.md](REQUIREMENTS.md) (requirements), and
[GLOSSARY.md](GLOSSARY.md) (terminology).

## License

[MIT](LICENSE)
