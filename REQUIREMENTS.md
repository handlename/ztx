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
sessions and AI agent CLIs, delivering the four features below for both
Claude Code and antigravity-cli.

## Features (all must-have for v1)

1. **Automatic session naming** — the Zed agent panel's session name reflects
   what the session is currently doing, without manual renaming.
2. **Open files from the session log** — file paths appearing in the log can
   be opened in Zed by mouse click and by a keyboard-only hint mode.
3. **Session log as Markdown** — equivalent of the ACP sessions' "Open thread
   as Markdown", for terminal sessions.
4. **Pull editor selections into the session** — equivalent of
   `agent::AddSelectionToThread`, one action from selection to prompt.

## Constraints

- **Architecture**: PTY proxy with a CLI-agnostic core. CLI-specific
  knowledge lives in adapters; without an adapter every feature must still
  work at PTY-recording quality (graceful degradation).
- **Zed configuration dependency is minimized**: prefer terminal-side
  solutions; ship Zed config only where it clearly improves the experience,
  via an explicit `setup` command with confirmation and backups.
- **Experience over implementation cost.**
- **Never degrade the raw CLI experience**: raw mode, resize, signals, exit
  codes, and extended keyboard protocols must pass through unchanged; idle
  CPU stays below 1%.
- Implementation language: Rust. Documentation and code comments: English.
- Version control: git, one commit per implementation step, GPG-signed.

## Non-goals (v1)

- Replacing Zed's ACP/external-agent integration.
- Contributing to or forking Zed itself.
- Zed WASM extensions (the extension API cannot touch the terminal or UI).
- Guaranteed support for CLIs other than Claude Code and antigravity-cli
  (the design stays CLI-agnostic, but only these two are verified).

## Acceptance criteria

Verified with Claude Code and antigravity-cli inside Zed Terminal Threads:

- [ ] A wrapped CLI is indistinguishable from the bare CLI in daily use
      (raw mode, resize, signals, exit codes, Shift+Enter / kitty protocol).
- [ ] Feature 1: the session name follows the session's activity
      (adapter quality) or the child's own titles (fallback).
- [ ] Feature 2a: cmd+click opens logged file paths at the right line.
- [ ] Feature 2b: hint mode opens logged file paths without the mouse.
- [ ] Feature 3: one action exports the log as Markdown into the editor
      (structured via adapter transcript; capture fallback otherwise).
- [ ] Feature 4: one action sends the current editor selection into the
      session.
- [ ] Wrapping a CLI with no adapter (e.g. bash) keeps features 1–3 working
      at fallback quality.
- [ ] Terminal state is restored even when ztx panics.
- [ ] Idle CPU usage stays below 1%.

The full decision history (interview transcript, ambiguity scoring, ADR) is
kept in the planning documents outside this repository.
