# SessionAtlas execution security contract

This contract covers every boundary where indexed, configured, or UI-provided
data can cause an external process to start.

## Default rule

- Local processes are represented as a program plus an argument array.
- Project paths use `WorkingDirectory` or a dedicated argument. They are never
  concatenated into `cmd /c`, `start`, PowerShell, or another shell command.
- Shell text is used only when an interactive terminal or SSH remote command
  inherently requires it. Each inserted value must first pass its dedicated
  validator or lossless quoting function.
- Validation happens in the backend even when the frontend also constrains an
  input.

## Tool and session launch

- The frontend sends optional `toolKey` and `sessionId` fields to `pty_attach`
  for local sessions and to `pty_spawn` / `pty_remote_switch` for remote tmux
  sessions; it never sends a ready-made command or tmux target.
- Tool keys are bounded identifiers and cannot begin with `-` or contain
  whitespace, control characters, or shell punctuation.
- Session IDs are bounded identifiers. The tool-specific resume selector and
  value are appended by trusted backend code (`codex resume <id>` for Codex,
  `pi --session <id>` for Pi Coding Agent).
- The shared `crates/sessionatlas-core` `security.rs` and the Tauri
  `src-tauri/src/security.rs` apply the same rules before recording a session
  or launching a CLI.
- Only built-in identities or enabled custom tools from config may be launched;
  unknown keys and attempts to override a built-in key are rejected.
- Custom CLI commands may contain a program and ordinary quoted arguments, but
  not shell metacharacters, shell/script executables, or unbalanced quotes.

## SSH and remote paths

- The SSH settings field accepts a command-like convenience syntax, but the
  frontend only parses an optional leading `ssh`, one `user@host` destination,
  and optional `-p` / `-i` values. It rejects extra remote-command text and
  sends structured fields to Tauri; the supplied text is never executed as a
  shell command.
- SSH user and host fields are bounded and reject leading options, whitespace,
  control characters, destinations containing `@`, and shell syntax.
- Ports must be in `1..=65535`. `--` is placed before the destination so a
  future parsing change cannot turn it into an SSH option.
- Identity-file paths may use `~`, but must resolve to an existing absolute
  regular file. SessionAtlas checks metadata and canonicalizes the path; it never
  reads or logs key contents.
- All connections enforce `BatchMode=yes`.
- Connection probes and remote scans wait for SSH only on blocking workers;
  saving a server starts its initial project scan in the background. The UI
  prevents duplicate scans for the same server and disables deletion until the
  active scan completes.
- Remote terminal commands first verify `command -v tmux`. Session names are
  backend-generated from a validated tool key plus a deterministic path hash;
  callers cannot supply a tmux target or shell fragment. Existing sessions are
  attached without replaying the tool launch command.
- SessionAtlas uses its own `sessionatlas-v1` tmux socket with a fixed `C-b`
  prefix. One SSH PTY is reused for all projects on a server. A switch request
  is accepted only when its server ID matches the immutable server ID stored on
  that PTY; the backend separately types a validated create command and a
  backend-generated `switch-client` command into the tmux prompt. Tmux argument
  quoting also doubles caller-controlled `#` characters so paths and custom
  tool arguments cannot become `#{...}` or `#(...)` format expansions.
- Remote PTY children receive the fixed `TERM=xterm-256color` capability value;
  it is not caller-controlled and is forwarded by OpenSSH to tmux.
- Remote paths reject control characters and use POSIX single-quote escaping
  that preserves apostrophes. A bare `~` or `~/...` keeps home expansion;
  `~other-user` is rejected.
- The built-in scan roots treat absent `~/projects` or `~/code` directories as
  optional, while any `find` failure for an existing built-in root and every
  failure in a custom root list remains fail-closed and preserves the previous
  remote snapshot.

## Per-machine TUI capabilities and installation

- The desktop probes the six built-in TUI commands separately on the local
  machine and on each configured SSH server. A tool can be enabled only after
  the backend has detected its executable; explicit disable preferences are
  enforced again by local attach, external launch, remote spawn, and remote
  tmux-switch commands.
- Capability and installation work runs on blocking workers. Remote probes use
  one fixed script that emits bounded, prefixed records; no frontend-provided
  shell text is inserted into it.
- The frontend sends only a built-in `toolKey` plus an optional server ID to
  `install_tui`. The backend maps that enum-like key to one fixed npm package
  package (including Pi's fixed official package) or the fixed Aider uv package.
  Unknown keys are rejected before any process
  starts.
- Local installers are represented as `ProcessSpec` program/argument arrays.
  Remote installers are selected from fixed backend strings and travel through
  the same validated SSH command builder. Installer output included in an error
  is reduced to one bounded, control-character-free line.
- Installation requires an explicit UI confirmation, never supplies service
  credentials, and re-probes the machine before auto-enabling the tool. A
  missing npm or uv prerequisite is reported instead of falling back to an
  arbitrary shell bootstrapper.

## URLs and openers

- External URLs must be absolute `http` or `https` URLs with a host and without
  embedded credentials. They are passed directly to the platform opener.
- A custom opener must contain `{path}` as one complete argument. The backend
  replaces that token with one process argument, preserving spaces and shell
  punctuation as data.
- Shells and script wrappers (`cmd`, PowerShell, `sh`, `.bat`, `.cmd`, `.ps1`,
  and equivalents) are not valid custom openers.

## Display and dependency safety

- User strings originating in paths, config, scans, or errors are rendered as
  plain text, never as markup: the CLI renders through `render.rs` /
  `select.rs` (plain-text output, no markup library), and the frontend inserts
  query/result text through `textContent`/text nodes rather than HTML sinks.
- Dependency auditing covers the Rust workspace lockfile and the frontend
  lockfile. Obsolete ecosystem audit scripts were removed during migration;
  the history is recorded in [`rust-migration-plan.md`](./rust-migration-plan.md).
- The workspace lockfile uses `anyhow >=1.0.103`, `plist >=1.10.0` /
  `quick-xml >=0.41.0`, and `portable-pty >=0.9.0`; these remove the actionable
  soundness, XML denial-of-service, and abandoned `serial` dependency findings
  present in the previous lockfile.
- On 2026-08-16, `cargo audit 0.22.2` scanned all 542 locked Rust crates and
  reported zero vulnerabilities. It emitted 17 allowed upstream warnings:
  ten GTK3 maintenance notices, five `rust-unic` maintenance notices, one
  `proc-macro-error` maintenance notice, and the Linux-only
  `glib::VariantStrIter` soundness advisory. SessionAtlas does not call the
  affected `VariantStrIter` API; current Tauri/wry releases still own these
  transitive GTK3, URL-pattern, and macro dependency chains. Treat the warnings
  as tracked upstream risk, not as a clean bill of health, and re-audit them on
  every Tauri or lockfile upgrade.

## Verification

Security tests use recording process runners and synthetic paths. They assert
the complete program/argument shape without starting a browser, SSH client,
terminal, git process, or AI CLI. The full isolated commands and expected test
counts are listed in [`test-baseline.md`](./test-baseline.md).
