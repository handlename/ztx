# Requirements

## Background

Zed integrates AI agents two ways:

1. **External agents (ACP)** — Zed owns the UI; the CLI runs behind the Agent
   Client Protocol. Rich editor integration, but only generalized features
   survive the protocol boundary.
2. **Terminal sessions (Terminal Threads)** — the real CLI runs in a terminal
   inside the agent panel. Every native CLI feature works, but the editor
   integration that ACP sessions enjoy is missing.

ztx targets the second mode: keep 100% of the CLI's native features and
recover the missing integration.

## Goal

Provide a PTY-proxy wrapper CLI (Rust) that mediates between Zed terminal
sessions and AI agent CLIs, delivering the three features below for both
Claude Code and antigravity-cli.

## Features (all must-have for v1)

1. **Automatic session naming** — the Zed agent panel's session name reflects
   what the session is currently doing, without manual renaming.
2. **Open files from the session log** — file paths appearing in the log can
   be opened in Zed by mouse click and by a keyboard-only hint mode.
3. **Session log as Markdown** — equivalent of the ACP sessions' "Open thread
   as Markdown", for terminal sessions.

## Constraints

- **Architecture**: PTY proxy with a CLI-agnostic core. CLI-specific
  knowledge lives in adapters; without an adapter every feature must still
  work at PTY-recording quality (graceful degradation).
- **Zed configuration dependency is minimized**: prefer terminal-side
  solutions. ztx writes no Zed configuration of its own; the only Zed-side
  setting is the optional `terminal_init_command` the user chooses to set.
- **Experience over implementation cost.**
- **Never degrade the raw CLI experience**: raw mode, resize, signals, exit
  codes, and extended keyboard protocols must pass through unchanged; idle
  CPU stays below 1%.
- Implementation language: Rust. Documentation and code comments: English.

## Non-goals (v1)

- Replacing Zed's ACP/external-agent integration.
- Contributing to or forking Zed itself.
- Zed WASM extensions (the extension API cannot touch the terminal or UI).
- Guaranteed support for CLIs other than Claude Code and antigravity-cli
  (the design stays CLI-agnostic, but only these two are verified).
- **Sending editor selections into the session.** Zed's built-in
  `agent::AddSelectionToThread` (`cmd->`) has covered this for Terminal Threads
  since zed-industries/zed#57301, so ztx ships no equivalent and installs no
  Zed task or keybinding.
- **Injecting text into a session from outside Zed's agent panel** (another
  terminal, another editor, CI). The supported surface is a Terminal Thread in
  Zed's agent panel; the IPC socket carries `notify` control frames only.

## Feature lifecycle

ztx exists only because Zed's Terminal Threads lack conveniences its ACP
sessions have. Every feature is therefore a gap-filler, and the intended
trajectory is for ztx to shrink rather than grow: when Zed ships an equivalent
of a ztx feature, Zed's wins and ztx's is removed. Overlap is not a
competition to win — a second implementation of something Zed already does
costs users a dependency and costs this project maintenance, for no gain.

**What counts as equivalent.** The Zed feature has to cover the same need for
the same workflow. Partial overlap does not trigger a removal; both stay, and
the docs explain when to reach for which.

The one removal so far is the worked example: editor-selection sending
(`ztx send`, listed under Non-goals above) went away once Zed's
`agent::AddSelectionToThread` (`cmd->`) worked in Terminal Threads, because it
covered the whole need. Its `ztx setup zed` companion went with it, having
existed only to install that keybinding.

The current near-miss is Zed's built-in path detection (cmd+click) vs. hint
mode (`ctrl-] f`): cmd+click resolves one visible path, hint mode is
keyboard-only over the recent scrollback. Not equivalent, so both stay.

**How a removal happens**, following [Semantic Versioning](https://semver.org/):

- **Before 1.0** — a feature can be removed in any release. 0.x makes no
  compatibility promise.
- **1.0 onward** — the feature is marked deprecated first and keeps working
  unchanged, then is removed no earlier than the next major version.

## Acceptance criteria

The bar v1 was built against, exercised with Claude Code and antigravity-cli
inside Zed Terminal Threads:

- A wrapped CLI is indistinguishable from the bare CLI in daily use
  (raw mode, resize, signals, exit codes, Shift+Enter / kitty protocol).
- Feature 1: the session name follows the session's activity
  (adapter quality) or the child's own titles (fallback).
- Feature 2a: cmd+click opens logged file paths at the right line.
- Feature 2b: hint mode opens logged file paths without the mouse.
- Feature 3: one action exports the log as Markdown into the editor
  (structured via adapter transcript; capture fallback otherwise).
- Wrapping a CLI with no adapter (e.g. bash) keeps features 1–3 working
  at fallback quality.
- Terminal state is restored even when ztx panics.
- Idle CPU usage stays below 1%.

Interactive criteria are held up by the author's day-to-day use rather than by
an automated conformance suite; see the alpha-quality caveats in the README.

The decision history behind these requirements (interview transcript,
ambiguity scoring, ADR) lives in the author's private planning notes and is
not published.
