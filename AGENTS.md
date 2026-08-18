# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## What this is

`SessionAtlas` aggregates projects that have been worked on by multiple AI CLI coding tools (Claude Code, Codex, Kimi, OpenCode, Aider, Pi Coding Agent). The repo is a **pure Rust workspace** with two cooperating components sharing one data directory (`~/.sessionatlas/`):

1. **`sessionatlas` CLI** (`crates/sessionatlas-cli`, Rust) — the canonical scanner. Walks each AI tool's data directory, deduplicates by normalized path, and writes a unified index to `~/.sessionatlas/index.db` (SQLite + FTS5). Also launches AI CLIs in project dirs via `sessionatlas open`.
2. **Tauri desktop console** (`src-tauri/` + `frontend/`, Rust + plain HTML/JS) — the **current primary GUI**. Opens the CLI's SQLite index read-only, browses/searches projects, and runs interactive AI-CLI terminals in-app via PTY. This is the actively-developed frontend.

Both binaries link the shared `crates/sessionatlas-core` library in-process. The Tauri console **does not own its data** — it opens a read-only view of the index the shared scanner maintains. On first launch, the frontend checks for a missing `index.db` and runs the in-process scan once before loading the project list. Existing indexes, including intentionally empty ones, are never rescanned automatically; failed first scans keep the index absent and show an actionable retry state.

**Identity contract:** the only supported identities are the `sessionatlas` CLI,
`SessionAtlas` product identifiers, Tauri crate `sessionatlas-tauri`,
identifier `com.sessionatlas.console`, `sessionatlas.*` localStorage keys,
`SESSIONATLAS_HOME`, and `~/.sessionatlas/`. There are no fallback aliases and
earlier data roots are not read or migrated automatically.

## Build & run

### `sessionatlas` CLI — from repository root
```bash
cargo run -p sessionatlas-cli -- scan         # scan all tools, atomically update the index
cargo run -p sessionatlas-cli --              # no args → list --interactive
cargo run -p sessionatlas-cli -- --help
cargo install --path crates/sessionatlas-cli --locked   # install to Cargo bin dir
```
`cargo test -p sessionatlas-cli` runs the CLI test suite.

### Tauri console — from repository root
```bash
cargo tauri dev            # run with hot frontend reload
cargo tauri build          # distributable bundle
cd src-tauri && cargo check   # type-check Rust only (fast)
```
No JS bundler — `frontend/` is plain static HTML/CSS/JS served directly.
`frontend/package.json` only defines the Node test and syntax-check commands.

## Architecture

### Workspace layout
```
Cargo workspace
├─ crates/sessionatlas-core   # shared library: adapter, model, path, scanner,
│                             # indexer, store, config, process/security, launcher
├─ crates/sessionatlas-cli    # `sessionatlas` executable (clap commands)
└─ src-tauri                  # Tauri 2 app, depends on sessionatlas-core
```
The core crate has no dependency on Tauri, the frontend, or CLI display
libraries. CLI and Tauri are I/O adapters over the same core.

### `sessionatlas-core` (`crates/sessionatlas-core`)
- **`src/adapter.rs`** — strict declarative adapter API v1, six compiled official
  manifests, immutable local-version registry, active-version selection, and
  safe new/resume argv construction. See `docs/tui-adapter-contract.md`.
- **`src/model.rs`** — `Project`, `ToolUsage`, `Session`, `ToolSource`; identity and defaults.
- **`src/path.rs`** — path normalization, root-path display, and same-or-child
  parent/child semantics (Windows case-insensitive, Unix byte-sensitive).
- **`src/scanner/`** — adapter-selected scanners: official `builtin.<id>`
  handlers bridge the mature per-tool parsers and extension adapters use the
  bounded `metadata-v1` parser; legacy `custom` entries remain compatible.
  `base.rs` holds the shared driver, `parsing.rs` the shared time/record parsers. A
  structured `ScanOutcome` keeps a trustworthy empty snapshot distinct from
  unavailable or failed input.
- **`src/indexer.rs`** — dedup/merge keyed by normalized path; same project
  touched by multiple tools collapses into one `Project` with multiple
  `ToolUsage` entries. Reads `.git/HEAD` for `GitBranch`.
- **`src/store.rs`** — `~/.sessionatlas/index.db`. Tables: `projects`,
  `tool_usages`, `sessions`, plus FTS5 `projects_fts`. Snapshot replacement,
  orphan cleanup, activity-time recomputation, and FTS rebuild happen in one
  SQLite transaction. Schema created/migrated idempotently.
- **`src/config.rs`** — `~/.sessionatlas/config.json` (adapter selections and
  active versions, legacy custom tools, per-path preferred tool, default terminal) with case-insensitive reads, a bounded
  cross-process lock, fingerprint conflict detection, and atomic replacement.
- **`src/process.rs`** — injectable process runner for git, SSH, and browser
  launch; keeps every external invocation as an argument array.
- **`src/security.rs`** — validation/quoting for tool/session IDs, SSH,
  remote paths, URLs, and opener templates.
- **`src/launcher.rs`** — builds `<tool>{sessionId}` command lines and opens a
  platform terminal for `open`.

### `sessionatlas-cli` (`crates/sessionatlas-cli`)
Clap commands (`scan`, `list`, `search`, `recent`, `open`, `config`, `tools`)
in `src/commands/`; rendering and safe selection in `src/render.rs` /
`src/select.rs`. User strings are rendered as plain text, never markup.

### Tauri console (`src-tauri/src/lib.rs` + `frontend/`)

**Data source**: opens the CLI's `~/.sessionatlas/index.db` read-only (see
`db_path()`); expected tables `projects`, `tool_usages`, `projects_fts` (FTS5).
`scan_projects` invokes the `sessionatlas-core` scan pipeline **in-process** on
`spawn_blocking` — it does not spawn a sidecar or subprocess — then returns
`COUNT(*)`. CLI and Tauri share the same `index.db`; reads are read-only and
successful scans replace snapshots atomically.

**Tauri commands**: registered in `run()`'s `generate_handler!`. Structs use
`#[serde(rename = "camelCase")]` so Rust snake_case arrives in JS as
`lastAccessedAt` etc. — keep this mapping when adding fields. Notable commands:
`list_projects`, `search_projects` (FTS5 `MATCH`), `list_tools`,
`scan_projects`, `pty_spawn`/`pty_attach`/`pty_write`/`pty_resize`/`pty_kill`,
the remote-SSH set (`test_remote_connection`/`add_remote_server`/`scan_remote_server`),
project ignores, opener prefs, groups, git info, and the tray-sync commands.

**TUI adapters**: every official and extension TUI resolves through the active
`AdapterRegistry`. The settings page can import a validated absolute-path
`adapter.json`, activate an already installed newer version, or point back to a
retained older version. Adapter files are immutable and contain no executable
code; v1 permits only fixed npm/uv package metadata and bounded scanner
handlers. Per-machine enablement still requires the executable to be detected,
and `platforms` / `supportsRemote` are enforced in the backend.

**In-app terminals (PTY)**: the right pane hosts multiple interactive terminal
tabs, one PTY each. `pty_spawn` creates and registers an unattached
pseudo-terminal; after the frontend has registered its tab, `pty_attach`
receives structured `toolKey` / `sessionId` metadata. Rust validates and
converts it to the optional tool command exactly once, then starts the output
reader. Natural exit, read failure, explicit close, and app exit all remove the
registry entry and reap the child. The registry uses a short map lock plus
separate reader/writer/resize/child locks. xterm.js + addon-fit are **vendored
locally** in `frontend/vendor/` — do NOT switch to a CDN.

**Remote SSH**: servers are added via the Settings drawer;
`test_remote_connection` pre-checks passwordless (key/agent) login *before*
persisting, and `classify_ssh_failure` turns ssh errors into actionable
bilingual hints. `BatchMode=yes` is enforced everywhere (pure passwordless).
User/host values reject option and shell syntax, `--` terminates SSH options,
identity files are canonical absolute regular files, and remote paths use
lossless POSIX quoting.

**Execution boundary**: `src-tauri/src/security.rs` owns validation and quoting
for PTY metadata, SSH, remote paths, URLs, and custom opener templates. Prefer
`ProcessSpec` / argument arrays for all local processes. Shell text is allowed
only where a terminal or SSH remote command requires it, and every inserted
value must pass the matching validator/quoting function. The complete contract
is in `docs/execution-security-contract.md`.

**Layout**: `.stage` is a two-column grid — `.stage__left` (project ledger) and `.stage__right` (terminal tabs). State lives in a single `state` object in `app.js`; mutations flow through `applyFilters()` → render fns. `reload()` is the single data-entry point; a 60s auto-refresh re-pulls (skipped while searching or scanning).

**Dual-mode frontend**: `app.js` keys off `window.__TAURI__` (`HAS_TAURI`). Under Tauri it calls Rust commands; in a plain browser it falls back to a bundled `SAMPLE` array. Preserve this duality when adding features.

**i18n**: `frontend/i18n.js` holds zh/en string tables; `lang-init.js` sets `<html lang>` before paint. ~270 keys wired through `tr()`; `applyLocalizedUI()` re-renders on language change. The OS tray menu follows language too (Rust `set_tray_language`).

**Icons**: `frontend/icons.js` inlines ~20 brand/functional SVGs (currentColor, theme-following). App icons in `src-tauri/icons/` are generated by `make_icons.py` (PIL) — re-run to regenerate, don't hand-edit.

**Capabilities**: `src-tauri/capabilities/default.json` grants `core:default` + `shell:allow-open` to `main`. Add new plugin permissions here or `invoke` calls get denied.

## Conventions

- Data dir for all components: `~/.sessionatlas/` (`index.db`, `config.json`, `adapters/<id>/<version>/adapter.json`, `prefs.db`). `prefs.db` also owns source-scoped project ignore rules; any path component starting with `.` is always hidden in the desktop project view. `*.db`/`*.db-journal`/`*.db-shm`/`*.db-wal` are gitignored.
- Tool keys are lowercase short strings. Official identities (`claude`, `codex`, `kimi`, `opencode`, `aider`, `pi`) come from `adapters/official/*.json`; do not reintroduce separate launch/install key maps. Unknown adapter keys use the frontend's generic icon/color fallback.
- Rust: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` must stay green; use `#![deny(...)]`/strict clippy where the code already does.
- Tauri frontend: keyboard is first-class (`/` search, `Esc` clear, `↑↓` nav, `Enter` launch) — don't break these. Match existing CSS tokens in `styles.css` rather than introducing new design primitives.
- `DESIGN.md` holds the current Rust architecture/data-flow/security design; this file is the shorter engineering reference.
- Migration from the retired implementation is recorded in `docs/rust-migration-plan.md`; it is the only document that may retain retired stack references.
