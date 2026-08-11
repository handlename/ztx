# Installation

## crates.io

ztx is published on [crates.io](https://crates.io/crates/ztx), so one command
is enough:

```sh
cargo install ztx
```

This compiles from the published crate, so it needs a Rust toolchain — 1.96 or
newer ([`rustup`](https://rustup.rs) is the recommended way to install one). No
checkout is involved. Re-run the same command to upgrade.

## Binary

Download a pre-built binary for your platform from the
[releases page](https://github.com/handlename/ztx/releases).
Unpack the archive and place `ztx` somewhere on your `PATH` (e.g. `/usr/local/bin`).

## Build from source

Requires a recent stable Rust toolchain
([`rustup`](https://rustup.rs) is the recommended way to install one).

```sh
git clone https://github.com/handlename/ztx.git
cd ztx
cargo install --path .
```

## Verify

```sh
ztx --version
```

This prints the version number and the commit hash it was built from
(e.g. `0.1.0 (a1b2c3d)`).

## Optional dependency

macOS desktop notifications require
[`terminal-notifier`](https://github.com/julienXX/terminal-notifier) on your `PATH`:

```sh
brew install terminal-notifier
```

This is only relevant when you have the Claude Code plugin installed; the plugin's
hooks call `ztx notify --from-hook`, which fires a notification when Claude finishes
or waits for input. Without `terminal-notifier` (or outside macOS), notifications are
silently skipped and nothing else is affected. See
[Configuration](../reference/configuration.md) for the `[notify]` settings.
