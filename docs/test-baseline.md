# SessionAtlas test baseline

This baseline exists so scanner, index, remote, and launcher fixes can be made
without reading real user data or starting real external programs.

The supported identity is consistently `SessionAtlas` / `sessionatlas` across
product, code, packaging and persistence. No legacy aliases are exercised by
the tests, and earlier data roots are not read or migrated automatically.

## Isolation guarantees

- C# scanner tests set `SESSIONATLAS_HOME` to a unique temporary directory.
- `SqliteStore` accepts an explicit database path; tests place it under a
  unique temporary directory with connection pooling disabled.
- Tests never call the parameterless `SqliteStore` constructor.
- Fixture paths, IDs, timestamps, versions, and content are invented.
- Frontend tests import `frontend/core.js`, which has no DOM, Tauri, storage,
  or network access.
- External commands cross an injectable boundary:
  - C#: `IProcessRunner` covers command discovery and terminal launching.
  - Rust: `ProcessRunner` covers git reads, SSH command execution, and browser
    launching. Tests use a recording runner.
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

## Test commands

From the repository root:

```powershell
dotnet test SessionAtlas.Tests\SessionAtlas.Tests.csproj --nologo
Push-Location frontend
npm test
npm run check
Pop-Location
Push-Location src-tauri
cargo test
Pop-Location
```

Expected baseline:

- C#: 44 tests
- Frontend: 7 tests
- Rust: 20 tests
- No skipped tests

## Contracts protected now

- ProjectIndexer merges tool observations, counts distinct native session IDs,
  and keeps the session ID belonging to the latest activity.
- A database can be created and disposed entirely under a temporary root.
- Scanner home resolution can be redirected without changing the process'
  real user profile.
- Fixtures match the sanitized format outlines and contain no current user
  home path.
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

## Final repair acceptance — 2026-08-03

Host evidence was collected on Windows with .NET SDK 10.0.302 targeting
net8.0, Node 24.15.0/npm 11.12.1, and Rust 1.95.0. The frontend rows were
refreshed on 2026-08-14 after the three-column workspace UI update; the other
rows retain their 2026-08-03 evidence date.

| Gate | Result |
| --- | --- |
| `dotnet test SessionAtlas.Tests\SessionAtlas.Tests.csproj --nologo` | 89 passed, 0 failed, 0 skipped |
| `dotnet test SessionAtlas.Desktop.Tests\SessionAtlas.Desktop.Tests.csproj --nologo` | 7 passed, 0 failed, 0 skipped |
| CLI build | succeeded, 0 warnings, 0 errors |
| legacy Desktop build | succeeded, 0 warnings, 0 errors |
| `npm ci` | succeeded from lockfile |
| frontend syntax check | succeeded |
| frontend unit tests | 16 passed, 0 failed/skipped |
| Playwright Chromium | 24 passed, 0 failed/skipped |
| `cargo fmt -- --check` | succeeded |
| `cargo clippy --all-targets -- -D warnings` | succeeded |
| Rust tests | 48 passed, 0 failed/skipped |
| `git diff --check` | succeeded; only Windows LF→CRLF notices |

The 2026-08-14 frontend refresh added one Playwright regression covering the
persistent project overview, selection updates, tool-activity and latest-session
rows, the single-row status bar, terminal workspace region, and browser-demo
launch fallback. The focused case passed
`1/1`; the component suites then passed frontend unit `16/16` and Playwright
Chromium `24/24`. Browser sample-mode screenshots are visual evidence only and
do not close the native Tauri interaction matrix.

The isolated integration used a unique temporary `SESSIONATLAS_HOME`. `sessionatlas
scan` exited 0, created only `.sessionatlas/index.db`, produced no database
sidecars, and did not launch an AI CLI or SSH process. The temporary directory
was removed after its resolved path and prefix were checked.

Trackable-file audit found no database, sidecar, `.env`, Playwright report,
test-result, or generated config-temp file. A focused private-key/AWS/OpenAI
token pattern scan found no matches.

Known non-blocking warnings:

- Playwright reports that `NO_COLOR` is ignored because `FORCE_COLOR` is set by
  the environment; tests are unaffected.
- MSVC prints import-library creation text during Rust test and release links;
  Rust exposes it as a `linker_messages` warning. Strict clippy still passes
  with `-D warnings`, so this is toolchain output rather than a source lint.
- `cargo audit 0.22.2` reports zero vulnerabilities and 17 allowed upstream
  maintenance/unsoundness warnings. Their exact reachability and upgrade
  boundary are recorded in `execution-security-contract.md`.

Hosted CI evidence and remaining native release checks:

- PASS: GitHub Actions
  [`path-semantics` run 31812198177](https://github.com/juliohuang/SessionAtlas/actions/runs/31812198177)
  ran against exact commit `f2ce07c6245c0ee8fbf31bd84d9b9312beafb99c`.
  `windows-latest` job `94805258666` and `ubuntu-latest` job `94805258752`
  each passed 39/39 focused tests with 0 failed and 0 skipped; CLI and Desktop
  builds succeeded on both runners with 0 warnings and 0 errors.
- The earlier `mcr.microsoft.com/dotnet/sdk:8.0` run remains supplemental local
  Linux evidence: 39 focused tests and both builds passed. It is no longer being
  used as a substitute for hosted CI.
- A native Tauri smoke run was completed with an empty index (startup,
  malicious-search text safety, Escape clearing, and settings open/close).
  The remaining Tauri interaction matrix and the Avalonia visual/manual matrix
  still require interactive desktop evidence; browser/headless tests do not
  replace that final acceptance.
- `cargo tauri info` and `cargo tauri build` passed locally after installing
  Tauri CLI 2.11.4. The build produced the release EXE, MSI, and NSIS installer;
  the native smoke ran against the rebuilt EXE, while the full interaction
  matrix remains open.

The 2026-08-03 release-candidate artifacts predate the full identity migration.
Their checksums are intentionally omitted from the current release table; they
must not be presented as evidence for the renamed executable or installers.

Native Tauri smoke evidence (2026-08-09, Windows 10/WebView2) predates the full
identity migration. That build displayed the empty-index error without starting
an AI CLI or SSH process, rendered malicious search input as text, and after
Escape cleared both the query and the previous “matches” count. Settings
drawer open/close also completed. The window was closed and the exact release
process was terminated after the check; no user index was created.

Avalonia startup follow-up (2026-08-09): the initial attempt exposed a startup
deadlock risk in `MainWindowViewModel.LoadProjects`; it synchronously waited on
the UI dispatcher during framework initialization. The implementation now
publishes initial data directly on the initialization thread and uses
`ConfigureAwait(false)` in the service query path, with regression test
`InitialLoadPublishesOnTheCurrentUiThreadWithoutDispatcherWait`. A controlled
launch produced a responsive window handle (`264098`) under the then-current
identity; Desktop tests were 7/7 and the build was
warning-free. A later window enumeration captured the real Avalonia window and
its accessibility tree (search box, scan/refresh buttons, project list and
close button). Both mouse and keyboard actions were then attempted, but the
desktop-control layer returned `GetCursorPos` access denied; therefore search,
tab and close behavior are not claimed as passed. Each test-created `index.db`
was moved to five recoverable backups under the predecessor data root. At the
start of the full identity migration, read-only snapshots found neither the
supported nor predecessor data root; no database was deleted or moved.

## SessionAtlas full-identity migration verification (2026-08-14)

Status: **PASS**. The only supported identifiers are `sessionatlas`,
`SessionAtlas.*`, `sessionatlas-tauri`, `com.sessionatlas.console`,
`sessionatlas.*`, `SESSIONATLAS_HOME`, and `~/.sessionatlas/`.

| Gate | Current result |
| --- | --- |
| C# Home/Config focused tests | 11 passed, 0 failed/skipped |
| C# CLI correctness focused tests | 13 passed, 0 failed/skipped |
| C# full tests | 89 passed, 0 failed/skipped |
| Desktop tests | 7 passed, 0 failed/skipped |
| CLI build | passed, 0 warnings/errors; output `sessionatlas.dll` |
| Desktop build | passed, 0 warnings/errors |
| Rust home/data-root focused tests | 5 passed, 0 failed/ignored |
| `cargo metadata --no-deps --format-version 1` | crate and targets parsed with SessionAtlas identifiers |
| `npm run check` | passed |
| Playwright smoke file | 5 passed, 0 failed/skipped |
| frontend unit tests | 16 passed, 0 failed/skipped |
| Playwright full suite | 24 passed, 0 failed/skipped |
| `cargo fmt -- --check` | passed |
| Rust full tests | 54 passed, 0 failed/ignored |
| `cargo clippy --all-targets -- -D warnings` | passed |
| `cargo tauri build` from repository root | passed; EXE, MSI and NSIS produced |
| tracked-text old-identity scan | 0 matches |
| public README screenshot | 1440×900, synthetic names only; SHA-256 `F3E4696060C1D8B1C199A2A91FE51E6E54A2A749EF9E68BF1D7B48D2F05786AB` |
| real-data snapshot before/after | identical; both data roots absent |

Release artifacts from the current Windows beta build (2026-08-15):

| Artifact | Size | SHA-256 |
| --- | ---: | --- |
| `src-tauri/target/release/sessionatlas-tauri.exe` | 8,419,840 bytes | `225F47676434D43C3D7187C4698B3CC2F4BBB757633B1380EDFFFE17EA798772` |
| bundled `src-tauri/target/release/sessionatlas.exe` sidecar | 70,798,190 bytes | `C94DD1A31D9FD06EF58D600C623F503EF67BD1B506BB5AEDADEDF873E15CFC50` |
| `src-tauri/target/release/bundle/msi/SessionAtlas_0.1.0_x64_en-US.msi` | 35,266,560 bytes | `0A916EA4FAAC00F85465881C4DE5517E26F41EEA0730879FD5E0DCC9A83722C5` |
| `src-tauri/target/release/bundle/nsis/SessionAtlas_0.1.0_x64-setup.exe` | 26,360,383 bytes | `4630BDD21F75D929CEFA8A74FA69BC57FD9C013D6FE9D02E6AB163F20CBA7641` |

The initial and final real-data snapshots both reported the supported and
predecessor data roots absent. No native app, real AI CLI, SSH connection,
credential, or production index was used. Five obsolete top-level release files
were moved, not deleted, to the local Codex quarantine after deletion was denied
by the safety policy. This identity migration does not close the pending native
Tauri or Avalonia interaction matrices.

## Open-source beta readiness verification (2026-08-15)

| Gate | Result |
| --- | --- |
| C# full tests | 89 passed, 0 failed/skipped |
| Desktop tests | 7 passed, 0 failed/skipped |
| C# Release builds | CLI and Desktop passed, 0 warnings/errors |
| Frontend syntax | passed, including sidecar/vendor scripts |
| Frontend unit/browser | 16/16 and 24/24 passed, 0 skipped |
| Rust fmt / strict clippy | passed |
| Rust full tests | 54 passed, 0 failed/ignored |
| Rust advisory scan | 0 vulnerabilities; 17 analyzed upstream warnings |
| NuGet advisory scan | machine-readable audit passed for all 4 projects |
| npm advisory scan | install-time registry audit and cached offline recheck reported 0 vulnerabilities |
| Tauri release build | EXE, MSI and NSIS passed; both installers include `sessionatlas.exe` |
| Bundled sidecar smoke | isolated fixture scanned 2 synthetic projects and exited 0 |
| Runtime network boundary | Playwright observed no requests outside the local fixture server |
| Public sample privacy | synthetic names only; legacy concept image removed |
| Native UI interaction | BLOCKED: the unique release window launched, but Windows returned `GetCursorPos access denied` on both allowed capture attempts |
| GitHub publication | BLOCKED: `gh` is not installed/authenticated; no commit, push, visibility, settings, tag, or release action was performed |

The working tree and current content scan contain no tracked database/key/env
filenames, no private-key/token-pattern matches, and no known private workspace
paths. The two existing historical commits still contain the former sample and
icon-source paths. The repository must therefore be republished from a sanitized
root commit (or equivalently rewritten) before it is made public; a normal new
commit does not remove those paths from Git history.
