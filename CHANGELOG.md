# Changelog

All notable changes to SessionAtlas will be documented here. The project follows [Semantic Versioning](https://semver.org/) once a stable compatibility promise is published.

## [Unreleased]

### Added

- Apache-2.0 licensing, third-party notices, community health files, and bilingual project documentation.
- Reproducible Windows release automation with checksums, SBOM generation, and build provenance.
- A self-contained `sessionatlas` CLI built and tested alongside the Tauri desktop installers.
- Declarative TUI adapter API v1 with six bundled official manifests, validated
  local extension import, per-machine selection, independent activation, and
  non-destructive adapter rollback.

### Changed

- Updated Rust and vendored frontend dependencies to audited, traceable releases.
- Removed runtime Google Fonts requests so the desktop UI uses local system fonts and assets.
- Corrected the OpenCode adapter resume command to use `--session <id>` with
  adapter version `1.0.1`.

### Security

- Claude task queues retain normal permission checks instead of passing a permission-bypass flag.
- Tauri scanning runs the shared Rust scanner in-process with the same adapter
  registry as the CLI; adapter manifests cannot load scripts or native code.

## [0.1.0-beta.2] - 2026-08-16

- Added Linux desktop CI coverage and fixed Linux compilation.
- Reused one SSH PTY per remote server and safely switched deterministic tmux sessions.

## [0.1.0-beta.1] - 2026-08-15

First public beta candidate: unified AI CLI project scanning, search, grouping, real PTY terminals, opener preferences, and passwordless remote SSH indexing.
