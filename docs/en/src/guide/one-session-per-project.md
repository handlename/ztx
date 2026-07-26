# One session per project

ztx enforces one active session per project directory. This constraint is what
makes `ztx send` work without configuration: both the editor task and the
running session agree on a single socket path derived from the project root,
so injection is always O(1) with no scanning.

## Project socket

When `ztx run` starts, it computes a deterministic socket path from the
canonical project directory and binds a Unix socket there:

```
<runtime-dir>/<fnv1a-hash-of-cwd>.sock
```

The runtime directory resolves in this order:

1. `$ZTX_RUNTIME_DIR` (if set and non-empty)
2. `$XDG_RUNTIME_DIR/ztx` (if `$XDG_RUNTIME_DIR` is set)
3. `$TMPDIR/ztx-run` (macOS per-user default)

Both the session and any `ztx send` call hash the same canonical path
(symlinks resolved) using FNV-1a 64-bit, so they always arrive at the same
socket name without a registry lookup.

A sibling `<hash>.info` file records the wrapper's pid and working directory
on two lines. It is used only for display (`ztx sessions`) and collision
reporting; socket resolution never reads it.

## Project root

The project root that drives socket naming is:

1. `ZED_WORKTREE_ROOT` environment variable — set by Zed for every task and
   terminal thread.
2. Current working directory — used when `ZED_WORKTREE_ROOT` is absent or
   empty.

The path is canonicalized before hashing so that symlink differences
(e.g. macOS `/tmp` vs. `/private/tmp`) do not produce distinct sockets for
the same directory.

## Starting a second session in the same project

If `ztx run` finds a live session already bound to the project socket, it
reports the existing session:

```
a ztx session is already running in this project (/path/to/project)
  pid    12345
  socket /private/tmp/ztx-run/a3f8c2d1e4b56789.sock
  cwd    /path/to/project
```

When run interactively (attached to a terminal), ztx offers to terminate the
existing session and start fresh in the current terminal. This is useful for
reclaiming a session that was orphaned by an editor restart — the process is
still listening but no longer attached to any Zed thread.

ztx sends SIGTERM to the existing wrapper and waits up to two seconds for the
socket to be released. If the socket is still owned after the grace period,
ztx escalates to SIGKILL.

A stale socket file whose owner has already exited is detected automatically
(the connect attempt fails) and taken over without prompting.

## Listing sessions

```sh
ztx sessions
```

Prints the pid, socket path, and working directory for every session that
currently owns a `.sock` file in the runtime directory. Sessions whose socket
file exists but whose owner has exited are shown as not alive.

## Routing sends explicitly

A bare `ztx send` routes to the project session. To address a specific session
regardless of the current directory:

```sh
ztx send --socket /private/tmp/ztx-run/a3f8c2d1e4b56789.sock --file foo.rs --line 1
```

This is useful when working across multiple projects simultaneously or when
scripting session interaction outside of Zed.

See [Send editor selections](send-selections.md) for the full `ztx send`
reference.
