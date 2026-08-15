# Security Policy

## Supported versions

| Version | Security support |
| --- | --- |
| Latest tagged `0.1.x` beta | Supported |
| Older beta releases | Best effort only |
| Untagged development builds | No guaranteed support |

Before the first public beta tag exists, only the latest commit on `main` is
treated as the supported release candidate.

## Project security boundary

SessionAtlas is a local desktop application and CLI. It reads local AI-tool
history metadata, writes its index and preferences under `~/.sessionatlas/`,
and can start local terminals, AI CLIs, Git, platform openers, and SSH.

The authenticated local operating-system user and explicitly installed
third-party executables are trusted. Scanned files, paths, SQLite rows, config
values, remote SSH output, terminal output, URLs, and error messages are treated
as untrusted input.

Third-party AI CLIs, Git, SSH, WebView2, and their network or account behavior
remain separate security boundaries. A vulnerability in one of those products
is in scope here only when SessionAtlas makes it reachable or materially worse.

## Security invariants

- Process launches use a program and structured argument list whenever possible.
- Shell text is limited to PTY and SSH boundaries and must use dedicated
  validation and quoting.
- SessionAtlas must not add flags that bypass an AI tool's permission checks.
- The Tauri console opens the CLI-owned project index read-only.
- SSH private-key contents are never read or logged, and SSH uses batch mode.
- Credentials, session contents, real user databases, and private paths must not
  enter source control, fixtures, screenshots, logs, or release artifacts.
- Product data remains local by default; SessionAtlas contains no telemetry,
  cloud-sync service, or runtime CDN dependency.
- Tests and native acceptance use a disposable `SESSIONATLAS_HOME` and synthetic
  data.

The detailed execution contract is documented in
`docs/execution-security-contract.md`.

## Reporting a vulnerability

Do not open a public issue for a suspected vulnerability.

Use GitHub private vulnerability reporting:

https://github.com/juliohuang/SessionAtlas/security/advisories/new

If that channel is unavailable, contact the repository owner privately through
their GitHub profile. Do not send secrets, real session histories, private
databases, SSH keys, or unredacted personal paths.

A useful report contains:

- affected version or commit;
- operating system and affected component;
- security impact and realistic attack prerequisites;
- minimal, sanitized reproduction steps or proof of concept;
- suggested mitigation, if known.

## Response expectations

These are response targets, not a contractual service-level agreement:

- acknowledgement within 5 business days;
- initial validity and severity assessment within 10 business days;
- coordinated status updates for validated issues;
- disclosure after a fix or mitigation is available, unless earlier disclosure
  is necessary to protect users.

Please allow maintainers a reasonable opportunity to investigate and prepare a
release before public disclosure.

## Examples of reportable issues

- command, argument, shell, URL, or SSH injection;
- arbitrary file access or writes outside the intended data boundary;
- credential, session, key, or private-path disclosure;
- HTML or terminal content reaching privileged native IPC unexpectedly;
- bypass of read-only index access or permission checks;
- unsafe sidecar lookup, replacement, packaging, or update behavior;
- a dependency vulnerability with a reachable SessionAtlas attack path.

## Out of scope

- vulnerabilities entirely inside an unmodified third-party AI CLI, Git, SSH,
  WebView2, or operating-system component;
- attacks that already require arbitrary code execution as the same local user
  and do not gain additional capability through SessionAtlas;
- social engineering without a SessionAtlas security-boundary failure;
- maintenance-status notices without a demonstrated reachable vulnerability.

## Known upstream risk

No vulnerability advisory is silently accepted. Current non-vulnerability Rust
maintenance and soundness warnings, including their reachability analysis, are
tracked in `docs/execution-security-contract.md` and are re-evaluated when the
lockfile or Tauri dependency chain changes.
