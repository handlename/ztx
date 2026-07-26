# AGENTS.md

Guidance for AI agents (and humans) working on this repository.

## Project

ztx — a Rust PTY-proxy wrapper improving the fit between Zed terminal
sessions and AI agent CLIs. Read `README.md` (overview), `DESIGN.md`
(architecture), `REQUIREMENTS.md` (scope), `GLOSSARY.md` (terms) first.

User-facing usage lives in the manual under `docs/` (mdBook; `docs/en` is the
source of truth, `docs/ja` follows), published at
<https://handlename.github.io/ztx/>. When a change alters user-visible
behavior — a flag, a config key, a key binding — update `docs/en` in the same
change; `docs/ja` may follow later.

## Build, test, lint

Rust is provisioned via mise (`mise.toml`); prefix commands with
`mise exec --` if cargo is not on PATH.

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings   # CI-enforced
cargo fmt --all --check                     # CI-enforced
```

All three must pass before a change is considered done.

For the manual:

```sh
mise run docs          # serve docs/en with live reload (mise run docs ja for JA)
```

This serves one book at `/`, so a Japanese page's link to its English
counterpart silently lands on the wrong page. Cross-language links and the 404
page only resolve under the `/ztx/` subpath Pages serves from, so check those on
the deployed site rather than locally.

## Version control

- One logical change per commit.
- Commit messages: English, Conventional Commits (`feat:`, `fix:`, `chore:`,
  `docs:`).
- Commits are GPG-signed. If signing fails, stop and ask the user rather than
  committing unsigned.
- Run `cargo fmt --all` before every commit — unformatted code fails the
  CI-enforced `cargo fmt --all --check`.

## Conventions

- Documentation and code comments are **English**.
- Comments explain constraints the code cannot show; no redundant narration.
- Module-level `//!` docs carry the design context for each module.
- Undocumented external state (Claude Code / agy files) must be read
  defensively: parse failures return `None`, never panic — the wrapper must
  keep running when adapters break.
- Never write to stdout/stderr while the child is running (it corrupts the
  child's screen). Use `tracing` (`ZTX_LOG=debug`, file-based).

## Testing notes

- `cargo test` covers the unit level. Interactive behavior (PTY passthrough,
  hint mode) has no committed harness — check it ad hoc with `/usr/bin/expect`
  driving the real binary. Beware Tcl's `\x` escape: `send "\x1de"` sends
  U+01DE, not `ctrl-]` + `e`; split into two `send` calls.
- `ZTX_RUNTIME_DIR`, `ZTX_ZED_CONFIG_DIR`, `ZTX_EDITOR`, and
  `ZTX_LOG_FILE` exist so tests never touch real user state.
- Verify features against real data when possible — e.g. a Claude Code
  transcript under `~/.claude/projects/` for the checkout you are working in,
  when one exists.

## Release

tagpr manages releases: merging the tagpr-generated PR tags a version and
`release.yml` builds and uploads binaries (macOS/Linux, x86_64/aarch64),
then publishes the draft release. Requires the `GH_PAT` repository secret.
