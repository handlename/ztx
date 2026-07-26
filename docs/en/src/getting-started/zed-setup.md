# Zed setup

ztx needs no Zed configuration to work: run `ztx run -- <cli>` inside a
Terminal Thread and every feature is active. The settings below are
conveniences.

## Wrapping every Terminal Thread automatically

Set `terminal_init_command` in Zed's `settings.json` to wrap every new
Terminal Thread in the agent panel without typing `ztx run` each time:

```json
{
  "agent": {
    "terminal_init_command": "ztx run -- claude"
  }
}
```

Open Zed's settings with `cmd-,`, search for `terminal_init_command`, and set
the value to the agent CLI you want to use.

## Sending editor selections

Zed's native `agent::AddSelectionToThread` action (`cmd->`) sends the current
editor selection into the active thread, Terminal Threads included. It needs no
ztx setup, and ztx deliberately ships no equivalent: select a range in a buffer,
press `cmd->`, and the reference lands in the session's prompt.
