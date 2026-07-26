# Architecture

ztx is a **passive-tap PTY proxy**. It owns a pseudo-terminal, runs the agent
CLI as a child inside it, and forwards bytes in both directions unchanged — the
single exception is OSC 0/2 title handling. A side channel (the tap) parses the
child's output into a scrollback buffer and screen-state flags, and the features
read that state. Features never rewrite the live stream; interactive UI such as
hint mode is drawn only on demand, on the alternate screen, with the output pump
paused.

The design documents live in the repository rather than in this manual, because
they target contributors rather than users:

- **[DESIGN.md](https://github.com/handlename/ztx/blob/main/DESIGN.md)** —
  the approach, the module structure, and the alternatives that were rejected
  (for example, rewriting the stream inline to inject OSC 8 hyperlinks).
- **[REQUIREMENTS.md](https://github.com/handlename/ztx/blob/main/REQUIREMENTS.md)** —
  the requirements this tool answers.

Per-module details are carried by the module docs in `src/`.
