# Changelog

All notable changes to SessionAtlas will be documented here. The project follows [Semantic Versioning](https://semver.org/) once a stable compatibility promise is published.

## [Unreleased]

### Local 0.1.1 candidate (2026-10-15)

- Per-project session history with bounded title/task previews, tool filters,
  recovery checks and exact historical-session terminal selection.
- Read-only staged/unstaged Git diffs and changed-file previews.
- Windows MSI/NSIS candidate packaging uses version 0.1.1; this is a local
  candidate and does not declare a public release. Native results are recorded
  separately from browser tests.

This section tracks work after the published `v0.1.0-beta.3` prerelease. The
beta.3 installers are available from the [explicit prerelease page](https://github.com/juliohuang/SessionAtlas/releases/tag/v0.1.0-beta.3).

### Fixed

- Stabilized virtualized ledger rendering after the beta.3 source baseline.

### Verification

- Added local post-beta.3 MSI/NSIS install, launch, screenshot, and uninstall
  evidence for the candidate commit `5bc6a8c`; this evidence is not evidence for
  the exact published beta.3 assets.

### Documentation

- Corrected public release-status, asset, and Windows UI documentation.
- Clarified that the beta.3 release contains MSI, NSIS, `SHA256SUMS.txt`, and
  `sessionatlas.spdx.json`; no GitHub build-provenance attestation was
  published with beta.3.

## [0.1.0-beta.3] - 2026-08-29

Published Windows x64 prerelease with MSI, NSIS, `SHA256SUMS.txt`, and
`sessionatlas.spdx.json`. The local installer report in the Unreleased section
covers a post-beta.3 candidate and is not evidence for these exact published
assets.

### Added

- Apache-2.0 licensing, third-party notices, community health files, and
  bilingual project documentation.
- A self-contained `sessionatlas` CLI built and tested alongside the Tauri
  desktop installers.
- Declarative TUI adapter API v1 with six bundled official manifests, validated
  local extension import, per-machine selection, independent activation, and
  non-destructive adapter rollback.

### Changed

- Updated Rust and vendored frontend dependencies to audited, traceable
  releases.
- Removed runtime Google Fonts requests so the desktop UI uses local system
  fonts and assets.
- Corrected the OpenCode adapter resume command to use `--session <id>` with
  adapter version `1.0.1`.

### Security

- Claude task queues retain normal permission checks instead of passing a
  permission-bypass flag.
- Tauri scanning runs the shared Rust scanner in-process with the same adapter
  registry as the CLI; adapter manifests cannot load scripts or native code.

### Release evidence

- Published MSI, NSIS, `SHA256SUMS.txt`, and `sessionatlas.spdx.json` with the
  prerelease. Beta.3 has no GitHub build-provenance attestation or formal
  canonical security-scan artifact.
- The MSI, NSIS installer, and executable are intentionally unsigned; users may
  see an unknown-publisher warning and should verify SHA-256 checksums.
- Beta.3 does not claim the post-beta.3 candidate's isolated installer-cycle
  evidence, real-user compatibility across all six CLIs, cross-platform desktop
  acceptance, hosted CI/security evidence, or a formal canonical security-scan
  artifact.

## [0.1.0-beta.2] - 2026-08-16

- Added Linux desktop CI coverage and fixed Linux compilation.
- Reused one SSH PTY per remote server and safely switched deterministic tmux sessions.

## [0.1.0-beta.1] - 2026-08-15

First public beta candidate: unified AI CLI project scanning, search, grouping, real PTY terminals, opener preferences, and passwordless remote SSH indexing.
