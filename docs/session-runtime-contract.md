# Session runtime contract

This contract defines the lifecycle of an in-app PTY without requiring tests
to launch an interactive shell.

## Open and attach

1. `pty_spawn` validates the local directory (or the remote SSH/tmux metadata),
   clamps the requested terminal dimensions, acquires the master reader/writer
   handles, and only then spawns the child.
2. A spawned session is registered but unattached. Rust does not consume PTY
   output yet, so the first prompt remains buffered by the PTY.
3. The frontend registers the tab and its event listeners before invoking
   `pty_attach`.
4. For local sessions, `pty_attach` can take the reader only once. The frontend
   sends structured `toolKey` / `sessionId` metadata, never shell text. Rust
   validates those values, constructs and writes the optional initial tool
   command once, then starts the output reader; a repeated attach is a harmless
   no-op.
5. For remote sessions, `pty_spawn` receives the same structured metadata and
   derives a stable `sessionatlas-<tool>-<hash>` tmux name from the remote path
   and tool. A missing session is created with the tool as its one-time startup
   command; an existing session is attached without injecting another command.
   SessionAtlas uses an isolated tmux socket and fixed prefix. The SSH PTY
   explicitly advertises `TERM=xterm-256color` so tmux can attach reliably from
   Windows. `pty_attach` then starts only the buffered output bridge.
6. Concurrent opens for the same project and tool share one in-flight promise;
   an existing live tab is focused instead of duplicated.
7. Remote opens are additionally serialized per server. If that server already
   has a live SSH PTY, `pty_remote_switch` verifies the PTY/server binding,
   creates the deterministic target session if needed, and switches the current
   tmux client. No second SSH child or terminal tab is created.

## Active session

- The registry map is locked only long enough to clone a session handle.
  Reader, writer, resize, and child control use separate locks, so a blocked
  write cannot stall another terminal or prevent that same child being killed.
- Write payloads are capped at 1 MiB. Columns are clamped to `2..=1000` and
  rows to `1..=1000`.
- PTY output is decoded incrementally. A multi-byte UTF-8 character split
  between reads is emitted intact; malformed or incomplete bytes produce one
  replacement character.
- A dead tab does not accept new input and does not count as a live project
  session.
- Initial tool metadata is bounded and validated before it reaches the PTY.
  Shell metacharacters, control characters, and option-shaped tool keys are
  rejected.
- During a remote switch, frontend keystrokes are paused and the backend holds
  the PTY writer lock while it sends two independent tmux prompt commands. A
  duplicate-session error cannot suppress the following `switch-client`.

## Exit and cleanup

- EOF removes the session from the registry, waits for the child, and emits
  `pty-exit` with the exit code when available.
- A reader error removes the session and kills the child before waiting.
- Explicit close removes first, then kills and waits. Repeated close/exit
  paths are harmless.
- Application exit drains the registry and kills/waits every remaining child.
- Closing a remote tab or exiting the app reaps the local SSH child but leaves
  the remote tmux session running for the next reconnect.
- Closing a tab while attach is in flight is treated as cancellation; a late
  attach result cannot resurrect the UI or record a successful open.

## Failure behavior

- A spawn failure leaves no child, xterm instance, pane, or tab behind.
- An attach failure is shown on a registered dead tab; the backend session is
  already cleaned and can be closed again safely.
- A switch failure keeps the existing tab title, project, and tool association;
  it does not claim that the requested target became active.
- If tmux is absent remotely, the connection probe reports the missing
  capability and the terminal prints bilingual install guidance before exiting;
  SessionAtlas does not silently fall back to a non-persistent remote shell.
- If the Tauri event listeners cannot be registered, terminal opening is
  disabled while the rest of the project browser continues to boot.

The process, SSH, URL, and quoting rules used by this lifecycle are defined in
[`execution-security-contract.md`](./execution-security-contract.md).
