# Release process

SessionAtlas public releases are built from an existing `v*` Git tag by `.github/workflows/release.yml` on a GitHub-hosted Windows runner.

## Release gate

Before creating the tag:

1. Update `CHANGELOG.md` and version fields.
2. Run the full C#, frontend, Rust, dependency-audit, and installer checks.
3. Run native acceptance with `scripts/New-AcceptanceFixture.ps1` and a unique temporary `SESSIONATLAS_HOME`; never use a real AI history or SSH server.
4. Confirm CI and Security workflows pass on the exact commit.
5. Review the complete diff and verify that no databases, logs, credentials, private paths, generated caches, or test artifacts are tracked.

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
