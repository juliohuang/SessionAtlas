# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## What this is

`SessionAtlas` aggregates projects that have been worked on by multiple AI CLI coding tools (Claude Code, Codex, Kimi, OpenCode, Aider). The repo contains **three cooperating components** sharing one data directory (`~/.sessionatlas/`):

1. **`sessionatlas` CLI** (C# / .NET 8, repo root `*.csproj`) — the canonical scanner. Walks each AI tool's data directory, deduplicates by normalized path, and writes a unified index to `~/.sessionatlas/index.db` (SQLite + FTS5). Also launches AI CLIs in project dirs via `sessionatlas open`.
2. **Tauri desktop console** (`src-tauri/` + `frontend/`, Rust + plain HTML/JS) — the **current primary GUI**. Reads the CLI's SQLite index, browses/searches projects, and runs interactive AI-CLI terminals in-app via PTY. This is the actively-developed frontend.
3. **Avalonia desktop GUI** (`SessionAtlas.Desktop/`, C# / Avalonia 12) — an earlier desktop frontend that re-uses the same `Core/`/`Models/` source. Retained for reference; the Tauri console supersedes it as the main UI.

The Tauri console **does not own its data** — it opens a read-only view of the index the C# CLI maintains. If the DB is missing, every Tauri command returns *"run `sessionatlas scan` first"*.

**Identity contract:** the only supported identities are the `sessionatlas` CLI,
`SessionAtlas.*` C# namespaces/project paths, Tauri crate `sessionatlas-tauri`,
identifier `com.sessionatlas.console`, `sessionatlas.*` localStorage keys,
`SESSIONATLAS_HOME`, and `~/.sessionatlas/`. There are no fallback aliases and
earlier data roots are not read or migrated automatically.

## Build & run

### C# CLI (`sessionatlas`) — from repo root
```bash
dotnet build
dotnet run -- scan         # scan all tools, (re)build the index
dotnet run                 # no args → list --interactive
dotnet publish -c Release -r win-x64 --self-contained true   # single-file publish
```
Target framework `net8.0`. No `.sln`, no test project.

### Tauri console — from repo root
```bash
cargo tauri dev            # run with hot frontend reload
cargo tauri build          # distributable bundle
cd src-tauri && cargo check   # type-check Rust only (fast)
```
No `package.json` / no JS bundler — `frontend/` is plain static HTML/CSS/JS served directly.

### Avalonia GUI (legacy)
```bash
dotnet build SessionAtlas.Desktop
dotnet run --project SessionAtlas.Desktop
```

## Architecture

### C# CLI pipeline (`scan` is the canonical example)
```
ScannerRegistry.Available  →  each IProjectScanner.Scan()  →  ProjectIndexer.BuildIndex()  →  SqliteStore.UpsertProject()
```
- **`Core/Scanner/`** — one `IProjectScanner` per AI tool; each knows its tool's data dir and how to extract path + last-accessed time. `ScannerRegistry` also loads `AppConfig.CustomTools` (runtime-configured tools via `sessionatlas config add-tool`, wrapped in `CustomToolScanner`).
- **`Core/Indexer/ProjectIndexer.cs`** — dedup/merge keyed by `NormalizePath`; same project touched by multiple tools collapses into one `Project` with multiple `ToolUsage` entries. Reads `.git/HEAD` for `GitBranch`.
- **`Core/Store/SqliteStore.cs`** — `~/.sessionatlas/index.db`. Tables: `projects`, `tool_usages`, `sessions`, plus FTS5 `projects_fts`. Schema created idempotently on construction.
- **`Core/Config/AppConfig.cs`** — `~/.sessionatlas/config.json` (custom tools, per-path preferred tool, default terminal).
- **`Core/Launcher/CliLauncher.cs`** — builds `cd "{path}" && <tool>{sessionId}` and spawns a platform terminal. `{sessionId}` expands to ` --resume "<id>"` when given.
- **`CLI/`** — Spectre.Console.Cli commands (`scan`, `list`, `search`, `open`, `recent`, `config`) registered in `Program.cs`. `EscapeMarkup` user strings before rendering in Spectre tables.

The two C# apps (CLI + Avalonia Desktop) share `Core/`/`Models/` via `<Compile Include>` globs, **not** a project reference — there is no `.sln`. Any change to `Core/`/`Models/` compiles into both. Do **not** add CLI-only (Spectre) or Desktop-only (Avalonia) dependencies into `Core/`/`Models/`; keep those layers dependency-free.

### Tauri console (`src-tauri/src/lib.rs` + `frontend/`)

**Data source**: opens the CLI's `~/.sessionatlas/index.db` read-only (see `db_path()`). Expected tables: `projects`, `tool_usages`, `projects_fts` (FTS5). `scan_projects` shells out to `sessionatlas scan`, then returns `COUNT(*)` — the console only refreshes its view of an index the CLI maintains.

**Tauri commands**: registered in `run()`'s `generate_handler!`. Structs use `#[serde(rename = "camelCase")]` so Rust snake_case arrives in JS as `lastAccessedAt` etc. — keep this mapping when adding fields. Notable commands: `list_projects`, `search_projects` (FTS5 `MATCH`), `list_tools`, `scan_projects`, `pty_spawn`/`pty_write`/`pty_resize`/`pty_kill`, the remote-SSH set (`test_remote_connection`/`add_remote_server`/`scan_remote_server`), opener prefs, groups, git info, and the tray-sync commands.

**In-app terminals (PTY)**: the right pane hosts multiple interactive terminal tabs, one PTY each. `pty_spawn` opens a real pseudo-terminal via `portable-pty`, runs the user's shell in the project dir, returns a session id; a reader thread pumps output as `pty-data` events. On first output the tab writes its tool command (`<toolKey> --resume <sessionId>`) to auto-launch the AI session. xterm.js + addon-fit are **vendored locally** in `frontend/vendor/` — do NOT switch to a CDN.

**Remote SSH**: servers are added via the Settings drawer; `test_remote_connection` pre-checks passwordless (key/agent) login *before* persisting, and `classify_ssh_failure` turns ssh errors into actionable bilingual hints. `BatchMode=yes` is enforced everywhere (pure passwordless).

**Layout**: `.stage` is a two-column grid — `.stage__left` (project ledger) and `.stage__right` (terminal tabs). State lives in a single `state` object in `app.js`; mutations flow through `applyFilters()` → render fns. `reload()` is the single data-entry point; a 60s auto-refresh re-pulls (skipped while searching or scanning).

**Dual-mode frontend**: `app.js` keys off `window.__TAURI__` (`HAS_TAURI`). Under Tauri it calls Rust commands; in a plain browser it falls back to a bundled `SAMPLE` array. Preserve this duality when adding features.

**i18n**: `frontend/i18n.js` holds zh/en string tables; `lang-init.js` sets `<html lang>` before paint. ~270 keys wired through `tr()`; `applyLocalizedUI()` re-renders on language change. The OS tray menu follows language too (Rust `set_tray_language`).

**Icons**: `frontend/icons.js` inlines ~20 brand/functional SVGs (currentColor, theme-following). App icons in `src-tauri/icons/` are generated by `make_icons.py` (PIL) — re-run to regenerate, don't hand-edit.

**Capabilities**: `src-tauri/capabilities/default.json` grants `core:default` + `shell:allow-open` to `main`. Add new plugin permissions here or `invoke` calls get denied.

## Conventions

- Data dir for all components: `~/.sessionatlas/` (`index.db`, `config.json`, `prefs.db`). `*.db`/`*.db-journal`/`*.db-shm`/`*.db-wal` are gitignored.
- Tool keys are lowercase short strings (`Codex`, `codex`, `kimi`, `opencode`, `aider`) — keep consistent across CLI scanners, launcher templates, config, and the Tauri `TOOL_COLOR`/`TOOL_DOT` maps.
- C#: Nullable reference types enabled. Implicit usings enabled in both csprojs (shared Core/Models files rely on them).
- Tauri frontend: keyboard is first-class (`/` search, `Esc` clear, `↑↓` nav, `Enter` launch) — don't break these. Match existing CSS tokens in `styles.css` rather than introducing new design primitives.
- `DESIGN.md` holds the original C# design (tool matrix, TUI mockups, roadmap); this file is the shorter engineering reference.
