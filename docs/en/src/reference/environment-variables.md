# Environment variables

These variables influence ztx's behavior at runtime. For persistent
preferences, prefer [`config.toml`](configuration.md); environment variables
are useful for per-invocation overrides and for enabling diagnostics without
modifying any file.

## Reference

| Variable | Description |
|----------|-------------|
| `ZTX_EDITOR` | Editor command for Export and Hint mode. Ignored when `editor` is set in `config.toml`. See [Editor resolution order](#editor-resolution-order) below. |
| `ZTX_LOG` | Tracing filter directive (e.g. `debug`, `ztx=trace`). Logging is completely disabled when the variable is unset or empty. Logs never go to the terminal; see `ZTX_LOG_FILE`. |
| `ZTX_LOG_FILE` | Path to the log file. Default: `$XDG_STATE_HOME/ztx/ztx.log`, falling back to `~/.local/state/ztx/ztx.log`. |
| `ZTX_RUNTIME_DIR` | Directory for ztx's IPC socket files. Overrides the default runtime directory chosen by ztx. |

## Editor resolution order

When ztx needs to open a file — for Export (`ctrl-] e` / `ztx export`) or
Hint mode (`ctrl-] f`) — it resolves the editor command through the following
chain, stopping at the first non-empty result:

1. **`editor` in `config.toml`** — the user's persistent, explicit preference.
2. **`$ZTX_EDITOR`** — a per-session or per-invocation override.
3. **`zed`** — the built-in default, used when `zed` is found on `$PATH`.
4. **`$EDITOR`** — the standard Unix fallback.

If none of the above yields a command, the export or open action fails with an
error message suggesting that `ZTX_EDITOR` or `EDITOR` be set.

> **Note:** ztx spawns the editor detached from the terminal (stdin, stdout,
> and stderr are all redirected to null) so it does not interfere with the
> wrapped TUI. GUI editors such as Zed work correctly; terminal editors opened
> via `$EDITOR` will launch but will not be visible inside the wrapped session.

## Logging

ztx never writes log output to the terminal because any stray bytes would
corrupt the wrapped CLI's screen. All diagnostic output goes to a file.

To enable logging:

```sh
ZTX_LOG=debug ztx run -- claude
```

To log only ztx's own traces (excluding library noise):

```sh
ZTX_LOG=ztx=trace ztx run -- claude
```

To write logs to a custom path:

```sh
ZTX_LOG=debug ZTX_LOG_FILE=/tmp/my-ztx.log ztx run -- claude
```

The default log path (`~/.local/state/ztx/ztx.log`) honors `$XDG_STATE_HOME`
when set. The directory is created automatically if it does not exist.

Inside a running session, pressing `ctrl-] d` writes a state dump to
`$TMPDIR/ztx/state-<pid>-<n>.txt`, which can be useful alongside the log for
diagnosing title or hint-mode issues.
