# Release process

SessionAtlas public releases are built from an existing `v*` Git tag by `.github/workflows/release.yml` on a GitHub-hosted Windows runner.

## Release gate

Before creating the tag:

1. Update `CHANGELOG.md` and version fields.
2. Run the full Rust, frontend, dependency-audit, and installer checks.
3. Run native acceptance with `scripts/New-AcceptanceFixture.ps1`, passing the
   explicitly built release CLI as `-ScannerPath target/release/sessionatlas.exe`,
   with a unique temporary `SESSIONATLAS_HOME`; never use a real AI history or
   SSH server. The acceptance script reads the index back through that same
   release binary (project count and UTC times) and records the seeded projects,
   session IDs and timestamps in its manifest.
4. Confirm CI and Security workflows pass on the exact commit.
5. Review the complete diff and verify that no databases, logs, credentials, private paths, generated caches, or test artifacts are tracked.

The Windows release runner installs Playwright Chromium
(`npx playwright install chromium`) before `npm test`, and explicitly builds the
release CLI with `cargo build --locked -p sessionatlas-cli --release` before the
acceptance smoke test; it does not rely on `cargo tauri build` to produce the CLI
implicitly.

R14 local automation (2026-08-15) passed the format/lint/test/frontend/installer/
isolation gates on this machine. Manual and live gates remain open and must be
completed before tagging: sandbox install without any extra language runtime,
real-user read-only scans, cross-platform real `sessionatlas open`, native Tauri
matrix T1–T9, and hosted CI/security workflows (including the pinned
`cargo-audit` run, which local R14 did not rerun).

## Published evidence

The workflow publishes both MSI and NSIS installers together with:

- `SHA256SUMS.txt` for download verification;
- `sessionatlas.spdx.json`, an SPDX software bill of materials;
- a GitHub build-provenance attestation bound to the release artifacts;
- generated release notes.

The beta workflow does not currently perform Windows Authenticode signing. Users should expect an unknown-publisher warning and verify SHA-256 checksums and the GitHub attestation. Do not describe a build as signed until a protected signing identity has been configured and its signature verified in CI.

## Publishing

Create an annotated beta tag such as `v0.1.0-beta.1` only after the release gate passes, then push that exact tag. Tags containing `-` are published as GitHub prereleases. The workflow refuses a manual run that is not attached to a tag.

Repository visibility, private vulnerability reporting, secret scanning, branch protection, and release verification are GitHub settings and must be checked after the repository becomes public.
