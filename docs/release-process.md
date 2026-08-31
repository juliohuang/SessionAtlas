# Release process

The repository contains a hosted release workflow, but the published
`v0.1.0-beta.3` assets were not the subject of the 2026-08-31 local installer
cycle: that report covers a post-beta.3 candidate and did not invoke GitHub
Actions.
When Actions are unavailable or intentionally not used, the local release gate
and its evidence report are the source of truth; do not claim hosted checks or
provenance that were not actually produced.

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
4. If hosted checks are intentionally not used, record that explicitly and do
   not mark CI/Security as passed. Otherwise confirm CI and Security workflows
   pass on the exact commit.
5. Review the complete diff and verify that no databases, logs, credentials, private paths, generated caches, or test artifacts are tracked.

The Windows release runner installs Playwright Chromium
(`npx playwright install chromium`) before `npm test`, and explicitly builds the
release CLI with `cargo build --locked -p sessionatlas-cli --release` before the
acceptance smoke test; it does not rely on `cargo tauri build` to produce the CLI
implicitly.

R14/R16 local automation passed the format/lint/test/frontend/installer/
isolation gates on this machine. The 2026-08-31 post-beta.3 candidate report
additionally records real MSI and NSIS install, launch, screenshot, and
uninstall cycles; it is not exact-asset acceptance for the published beta.3.
Remaining gates are real-user read-only scans, cross-platform real
`sessionatlas open`, native Tauri matrix T1–T9, and a formal canonical
security-scan artifact. Hosted CI/security remains unverified because Actions
were not invoked.

## Published evidence

When the hosted workflow is used, it is designed to publish both MSI and NSIS
installers together with:

- `SHA256SUMS.txt` for download verification;
- `sessionatlas.spdx.json`, an SPDX software bill of materials;
- a GitHub build-provenance attestation bound to the release artifacts (only if
  the hosted workflow actually ran and produced it);
- generated release notes.

The beta workflow does not currently perform Windows Authenticode signing.
Users should expect an unknown-publisher warning and verify SHA-256 checksums.
The published beta.3 assets contain no GitHub build-provenance attestation, so
do not ask users to verify one. Do not describe a build as signed or attested
until the relevant signature or attestation was actually produced and verified.

## Publishing

`v0.1.0-beta.3` is already the published prerelease. For a future annotated
beta tag, pass the local gate first, bind every artifact and report to the same
commit, and publish only after the remaining gates or a maintainer-approved
exception are recorded. Tags containing `-` are GitHub prereleases. Do not use
the floating `/releases/latest` URL for prerelease download instructions; link
the explicit tag or the releases list.

Repository visibility, private vulnerability reporting, secret scanning, branch protection, and release verification are GitHub settings. The repository is already public; current setting gaps must be recorded before the next release.
