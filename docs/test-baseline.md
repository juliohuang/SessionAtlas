# SessionAtlas test baseline

This baseline exists so scanner, index, remote, and launcher fixes can be made
without reading real user data or starting real external programs. It
**distinguishes verified local automation from unverified manual/live gates**;
evidence is recorded with the exact command that produced it, and nothing is
invented that was not actually rerun.

The supported identity is consistently `SessionAtlas` / `sessionatlas` across
product, code, packaging and persistence. No legacy aliases are exercised by
the tests, and earlier data roots are not read or migrated automatically.

## Isolation guarantees

- Rust scanner tests set `SESSIONATLAS_HOME` to a unique temporary directory.
- `Store` accepts an explicit database path; tests place it under a unique
  temporary directory with connection pooling disabled.
- Tests never read or mutate the real `~/.sessionatlas/`.
- Fixture paths, IDs, timestamps, versions, and content are invented.
- Frontend tests import `frontend/core.js`, which has no DOM, Tauri, storage,
  or network access.
- External commands cross an injectable boundary: `ProcessRunner` covers git
  reads, SSH command execution, and browser launching. Tests use a recording
  runner.
- PTY tests do not start an interactive shell. The session-runtime suite
  exercises the registry, lifecycle primitives, size/input bounds, and
  streaming UTF-8 decoder without touching the user's terminal.

## Recorded format outlines

The fixture shapes were compared with locally installed tool formats on
2026-07-30. Only property names and schemas were inspected; prompt and message
content was not copied.

| Tool | Recorded outline |
| --- | --- |
| Claude Code | project bucket containing JSONL records with `cwd`, `sessionId`, and `timestamp` |
| Codex | date-nested JSONL whose first `session_meta.payload` contains `id`, `cwd`, and `timestamp` |
| Kimi Code | `~/.kimi-code/sessions/<worktree-key>/<session-id>/state.json` |
| OpenCode | SQLite `project` and `session` tables in `opencode.db` |
| Aider | project-local `.aider.chat.history` marker |

Fixtures deliberately describe the current formats even when a production
scanner does not support that format yet. Parser behavior for those fixtures
is added in the scanner-repair phase.

## Test commands (current Rust + frontend)

From the repository root:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
npm --prefix frontend run check
npm --prefix frontend test
git diff --check
```

Focused suites:

```powershell
cargo test -p sessionatlas-core       # model/path/scanner/indexer/store/config/security/launcher/process contracts
cargo test -p sessionatlas-cli        # read-only, scan/config, and open command tests
cargo test -p sessionatlas-tauri      # Tauri read-only index, in-process scan, PTY/registry, SSH validation
```

## Verified local automation (pre-R13 / R13)

Counts are the proven local results recorded before R13 and confirmed again by
the R13 verification reruns; they are not reruns of installer-build or
browser-test steps, which are left to R14:

| Gate | Result |
| --- | --- |
| `cargo test -p sessionatlas-cli` | 96 passed, 0 failed/ignored |
| `cargo test -p sessionatlas-core` | 238 passed across its test binaries, 0 failed/ignored |
| `cargo test -p sessionatlas-tauri` | 60 passed, 0 failed/ignored |
| Rust total | 394 passed, 0 failed/ignored |
| `cargo fmt --all -- --check` | passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | passed |
| frontend syntax check (`npm --prefix frontend run check`) | passed |
| R12 isolated Rust CLI scan | isolated `SESSIONATLAS_HOME`; `cargo run -p sessionatlas-cli -- scan` indexed 2 synthetic projects and exited 0, creating only `index.db` with no database sidecars |
| `git diff --check` | passed; only Windows LF→CRLF notices |

The R13 isolation scan used a unique temporary `SESSIONATLAS_HOME`, did not
launch an AI CLI or SSH process, and the temporary directory was removed after
checking its resolved path. Trackable-file audit found no database, `.env`,
test-report, or generated config-temp file.

## Verified local automation (R14, 2026-08-15, Windows x64)

R14 reran every locally available command listed below on the current tree.
Evidence is from the commands actually executed; nothing is reused from earlier
phases. The unavailable local dependency audit is listed separately below.

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | passed (exit 0) |
| `cargo clippy --workspace --all-targets -- -D warnings` | passed (exit 0) |
| `cargo test --workspace --no-fail-fast` | 394 passed, 0 failed/ignored (exit 0) |
| `npm --prefix frontend run check` | passed (exit 0) |
| `npm --prefix frontend test` | 16 unit + 24 Playwright browser tests passed (exit 0) |
| `cargo tauri build --ci` | passed (exit 0); MSI + NSIS produced |
| `cargo build --locked -p sessionatlas-cli --release` | passed (exit 0) |
| isolated acceptance on the release CLI | passed (exit 0); `index.db` 86016 bytes, `list` 2 projects, `search` shows UTC `2026-08-15 01:00`/`02:00` |
| `git diff --check` | passed (only Windows LF→CRLF notices) |

Release CLI isolation evidence (unique root under the repository's git-ignored
`.verify/`, removed after recording):

- Scanner exited 0; `index.db` created (86016 bytes), persisted both synthetic
  session IDs, and left no `-journal`/`-wal`/`-shm` sidecars.
- `sessionatlas list` returned exactly 2 rows containing `atlas-alpha` and `atlas-beta`.
- `sessionatlas search atlas` printed both projects with absolute UTC times
  `2026-08-15 01:00` and `2026-08-15 02:00`.
- `fixture-manifest.json` (schemaVersion 2) recorded both synthetic project
  paths, both session IDs (`acceptance-alpha`, `acceptance-beta`) and both UTC
  timestamps, plus SHA-256 of every file.

The acceptance fixture was enhanced for R14: it now reads the index back through
the same release binary (`list` for the project count, `search` for the UTC
times), verifies both session-ID markers in the newly created database, and
records the seeded projects/IDs/timestamps in the manifest. It stays
PowerShell 5 compatible, uses no `sqlite3`/Python, starts no real AI CLI or SSH
process, and verifies without any new user-facing CLI command.

The workflow gap was closed: `release.yml` now installs Playwright Chromium
before `npm test`, and both `ci.yml` (windows-desktop) and `release.yml` build
`cargo build --locked -p sessionatlas-cli --release` explicitly and pass
`target/release/sessionatlas.exe` to the acceptance script instead of relying on
`cargo tauri build` to produce the CLI implicitly.

### Contracts protected now

- `ProjectIndexer` merges tool observations, counts distinct native session
  IDs, and keeps the session ID belonging to the latest activity.
- A database can be created and disposed entirely under a temporary root.
- Scanner home resolution can be redirected without changing the process' real
  user profile.
- Fixtures match the sanitized format outlines and contain no current user home
  path.
- Frontend filtering, grouping, ordering, and structured PTY attach metadata
  are exercised as pure functions.
- Frontend terminal deduplication distinguishes live/dead tabs and coalesces
  concurrent opens by a stable project/tool key.
- Git, SSH, terminal, command-discovery, and browser process requests can be
  inspected without executing the real program.
- SQLite snapshot tests cover stable project identity, exact usage
  replacement, partial and successful-empty scans, orphan/FTS cleanup,
  migration of duplicate legacy usages, and transaction rollback.
- Current Claude, Codex, Kimi Code, OpenCode, and Aider formats are parsed from
  sanitized temporary sources. Missing, malformed, unreadable-shape, and
  successful-empty sources exercise distinct scanner outcomes.
- Configuration tests verify that malformed custom-tool configuration emits a
  diagnostic and that HOME overrides do not leak through a static path cache.
- PTY registry tests cover bounded dimensions and writes, wrap-safe IDs,
  idempotent removal/draining, and UTF-8 characters split across read chunks.
- Adversarial execution tests cover malicious tool/session IDs, shell
  templates, SSH users/hosts, remote paths containing apostrophes, URL schemes
  and credentials, and project paths containing spaces or shell punctuation.

The normative behavior shared by all phases lives in
[`scan-contract.md`](./scan-contract.md).
The terminal lifecycle and failure behavior live in
[`session-runtime-contract.md`](./session-runtime-contract.md).
External process and untrusted-input rules live in
[`execution-security-contract.md`](./execution-security-contract.md).

## Unverified manual/live gates (remaining)

These are **not** claimed to have passed by local automation. They remain
release gates and must be executed before release:

- Real-user read-only scan of actually installed Claude/Codex/Kimi/OpenCode/
  Aider data directories with project/session counts checked against the tool.
- Cross-platform real terminal launch of `sessionatlas open` (Windows
  Terminal/cmd, macOS Terminal, and at least one Linux terminal).
- Native Tauri interaction matrix T1–T9 (rapid A/B switching, docs/files close
  races, remote partial failure, group reorder, terminal link activation, and
  settings write-failure state) — recorded in
  [`manual-acceptance-checklist.md`](./manual-acceptance-checklist.md).
- Windows/Ubuntu hosted CI on the current commit (including the hosted Security
  workflow). Local R14 does not substitute for a hosted runner; RI-04 stays
  BLOCKED until such evidence exists on a shared commit.
- `cargo audit` rerun: R14 local did **not** rerun it (the tool is not installed
  locally and was not installed per the R14 boundary). The hosted Security
  workflow still pins `cargo-audit 0.22.2` and runs `cargo audit`; it remains a
  release gate. RI-05's earlier scan evidence is historical, not R14 evidence.
- Install in a sandbox without any extra language runtime and first scan.

Earlier implementation baselines are archived only in
[`rust-migration-plan.md`](./rust-migration-plan.md). They are not part of the
current baseline and must not be presented as evidence for the Rust-only
release.
