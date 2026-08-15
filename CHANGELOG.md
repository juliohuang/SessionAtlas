# Changelog

All notable changes to SessionAtlas will be documented here. The project follows [Semantic Versioning](https://semver.org/) once a stable compatibility promise is published.

## [Unreleased]

### Added

- Apache-2.0 licensing, third-party notices, community health files, and bilingual project documentation.
- Reproducible Windows release automation with checksums, SBOM generation, and build provenance.
- A self-contained `sessionatlas` CLI bundled with the Tauri desktop installer.

### Changed

- Updated Rust and vendored frontend dependencies to audited, traceable releases.
- Removed runtime Google Fonts requests so the desktop UI uses local system fonts and assets.

### Security

- Claude task queues retain normal permission checks instead of passing a permission-bypass flag.
- Tauri scanning invokes the bundled sidecar with structured arguments and an explicit `SESSIONATLAS_HOME`.

## [0.1.0-beta.1] - 2026-08-15

First public beta candidate: unified AI CLI project scanning, search, grouping, real PTY terminals, opener preferences, and passwordless remote SSH indexing.
