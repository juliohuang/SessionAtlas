# SessionAtlas

English | [简体中文](./README.md)

[![CI](https://github.com/juliohuang/SessionAtlas/actions/workflows/ci.yml/badge.svg)](https://github.com/juliohuang/SessionAtlas/actions/workflows/ci.yml)
[![Security](https://github.com/juliohuang/SessionAtlas/actions/workflows/security.yml/badge.svg)](https://github.com/juliohuang/SessionAtlas/actions/workflows/security.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](./LICENSE)
[![Release](https://img.shields.io/github/v/release/juliohuang/SessionAtlas?include_prereleases)](https://github.com/juliohuang/SessionAtlas/releases)

A searchable workspace map for projects and sessions scattered across Claude Code, Codex, Kimi, OpenCode, Aider, and Pi Coding Agent.

![SessionAtlas browser demo](./docs/images/sessionatlas-browser-demo.png)

SessionAtlas has two cooperating components:

- **Windows desktop console (the primary UI):** browse, search, and group projects, then resume AI CLI sessions in real multi-tab PTY terminals.
- **`sessionatlas` CLI (the canonical scanner):** scans local tool histories, deduplicates normalized project paths, and writes `~/.sessionatlas/index.db`.

> SessionAtlas is an independent open-source project. It is not affiliated with, endorsed by, or connected to `sessionatlas.nl` or its owners. Assess trademark risk before public release.

## Who it is for

SessionAtlas helps people who use several AI coding CLIs and regularly lose track of which tool or session was last used for a project. It does not purchase or authenticate any AI service for you. After explicit confirmation, desktop settings can install only the fixed npm/uv package declared by a validated active adapter; account login remains the user's responsibility.

## Install

### Windows desktop beta

Download the latest `.msi` or `-setup.exe` from [GitHub Releases](https://github.com/juliohuang/SessionAtlas/releases/latest). Scanning runs in-process inside the desktop app; the installer does not bundle a separate scanner binary or require an additional language runtime. When the index is missing, the first launch scans automatically; use **Rescan** for later refreshes.

Requirements:

- Windows 10/11 x64;
- WebView2 Runtime (normally included with Windows 11);
- a supported AI CLI must be installed, compatible with the adapter's platform/remote contract, enabled for that machine, and authenticated before SessionAtlas can launch its sessions. When an active adapter declares a fixed npm/uv package, it can be installed from **Settings → AI TUI tools** if that manager is available.

The first public release is a beta. Keep a copy of `~/.sessionatlas/` before upgrading and see [SUPPORT.md](./SUPPORT.md) when reporting a problem.

### Run the CLI from source

Install a stable Rust toolchain (Tauri 2 system prerequisites apply; see Development below):

```bash
cargo run -p sessionatlas-cli -- scan
cargo run -p sessionatlas-cli -- list
cargo run -p sessionatlas-cli -- search <query>
cargo run -p sessionatlas-cli -- open [path]
cargo run -p sessionatlas-cli -- recent
cargo run -p sessionatlas-cli -- config
```

Or install it into your Cargo bin directory and run `sessionatlas` directly:

```bash
cargo install --path crates/sessionatlas-cli --locked
sessionatlas scan
```

The CLI source is built and tested on Windows, macOS, and Linux. Automated desktop packages currently target Windows x64 only.

## Supported tools

| Tool | Scan source | Launch command |
| --- | --- | --- |
| Claude Code | `~/.claude/projects/**/*.jsonl` | `claude` |
| Codex CLI | `~/.codex/sessions/**/*.jsonl` | `codex` |
| Kimi CLI | `~/.kimi-code/sessions/**/state.json` | `kimi` |
| OpenCode | `~/.local/share/opencode/opencode.db` | `opencode` |
| Aider | `.aider.chat.history` in common development roots | `aider` |
| Pi Coding Agent | `~/.pi/agent/sessions/**/*.jsonl` | `pi` |

The six official tools and extensions now use the same declarative adapter
contract. Official adapters are bundled with the app; **Settings → AI TUI
tools** can import a validated `adapter.json`, select it per machine, and
activate, update, or non-destructively roll back adapter versions independently
from the app. API v1 loads no scripts, native libraries, or arbitrary installer
commands. See [`docs/tui-adapter-contract.md`](./docs/tui-adapter-contract.md).
The legacy `sessionatlas config add-tool` command remains for compatibility.

## Highlights

- Unified scanning, path deduplication, and SQLite FTS5 search;
- tool and recency filters, persistent groups, and drag ordering;
- real multi-tab PTY terminals and recent-session resume actions;
- configurable VS Code, file manager, terminal, and custom openers;
- configurable browser-based development tools such as DSH; each requires an absolute credential-free HTTP(S) connection URL and opens in a sandboxed in-app web tab;
- remote SSH indexing through passwordless key/agent authentication, with persistent tmux TUI sessions and one reused SSH terminal per server;
- one declarative TUI adapter registry for official and extension detection, scanning, launch/resume, installation, and updates, with explicit import, activation, and non-destructive rollback;
- per-machine local/remote TUI detection and selection; only installed adapters compatible with the machine and remote mode can be enabled, while fixed npm/uv packages support confirmed one-click installation;
- Chinese and English UI with keyboard-first navigation;
- browser demo mode backed by bundled sample data when Tauri is unavailable.

## Privacy and security

- **Local first:** indexes, preferences, and configuration stay in `~/.sessionatlas/`; the project contains no telemetry or cloud-sync service.
- **Read boundary:** scanners read supported tools' local data directories to extract project paths, timestamps, and session metadata. SessionAtlas does not upload this content to a SessionAtlas service.
- **Adapter boundary:** adapters are strictly validated JSON declarations. They load no JavaScript, native library, WASM, or shell script; an ID/version cannot be overwritten and older versions remain available for rollback. API v1 supports explicit local-manifest import, not an online marketplace or signature trust store.
- **Execution boundary:** a user action can launch local AI CLIs, terminals, Git, SSH, or a fixed npm/uv package installation declared by a validated adapter and allowed by the backend. SessionAtlas never accepts an installer command from the frontend; those third-party tools and package managers retain their own network and data policies.
- **Normal permission policy:** queued Claude tasks do not add a permission-bypass flag; tasks requiring approval may pause for the user.
- **Local assets:** xterm.js, highlighting, and fonts do not depend on a CDN. See [THIRD_PARTY_NOTICES.md](./THIRD_PARTY_NOTICES.md).

The execution boundary is documented in [`docs/execution-security-contract.md`](./docs/execution-security-contract.md). Do not disclose vulnerabilities in a public issue; the private reporting process will be documented in `SECURITY.md`.

## Development

Development requires stable Rust, Node.js 22+, and the Tauri 2 system prerequisites. On Ubuntu/Debian, install the desktop development libraries first:

```bash
sudo apt-get update
sudo apt-get install --no-install-recommends \
  build-essential \
  curl \
  file \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libssl-dev \
  libwebkit2gtk-4.1-dev \
  libxdo-dev \
  wget
```

```bash
# Rust CLI and tests
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Frontend
cd frontend
npm ci
npm run check
npm test

# Tauri
cd ../src-tauri
cargo test
cargo tauri build
```

The desktop `scan_projects` runs the `sessionatlas-core` scan pipeline in-process via `spawn_blocking`; no sidecar is bundled and no additional language runtime is required. Tests must use a temporary `SESSIONATLAS_HOME` and must not read or mutate the real `~/.sessionatlas/`.

See [AGENTS.md](./AGENTS.md) for architecture, [`docs/scan-contract.md`](./docs/scan-contract.md) for scanner semantics, and [`docs/test-baseline.md`](./docs/test-baseline.md) for the verification baseline.

## Project status

SessionAtlas is preparing its first public beta. See [ROADMAP.md](./ROADMAP.md) and [CHANGELOG.md](./CHANGELOG.md).

## Contributing

Bug reports, feature proposals, documentation, and code contributions are welcome. Read [CONTRIBUTING.md](./CONTRIBUTING.md) and [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md) first.

## License

Licensed under the [Apache License 2.0](./LICENSE). The license does not automatically grant rights to use project names or logos; see [NOTICE](./NOTICE).
