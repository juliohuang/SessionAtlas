# Roadmap

This roadmap describes direction, not a delivery promise. Priorities may change based on security findings and community feedback.

## Published beta status (`v0.1.0-beta.3`)

- `v0.1.0-beta.3` is public with MSI, NSIS, `SHA256SUMS.txt`, and `sessionatlas.spdx.json`. The 2026-08-31 local report at `target/installer-cycle-20260831-5bc6a8c/REPORT.md` covers a post-beta.3 candidate, not the exact published assets (generated evidence is not tracked).
- The prerelease provides clearly identified unsigned Windows artifacts, SHA-256 checksums, and an SPDX SBOM (`sessionatlas.spdx.json`). Beta.3 has no GitHub build-provenance attestation; Authenticode signing is not configured.
- Secret scanning and push protection are enabled. Private vulnerability reporting, Dependabot security updates, and branch protection still require repository-setting follow-up; no hosted Actions result is claimed for this cycle.
- Resolve or intentionally retain the project name after a trademark/confusion review.

The 2026-08-31 installer cycle is a post-beta.3 candidate check. Its MSI/NSIS
hashes differ from the published beta.3 assets; the exact values are recorded in
[`docs/test-baseline.md`](./docs/test-baseline.md). It does not prove the exact
published beta.3 assets or the full Tauri T1–T9 interaction matrix,
real-user scans for all six CLIs, cross-platform CLI launch, remote SSH
compatibility, or a canonical security-scan artifact.

## Beta hardening

- Improve first-run guidance and diagnostics when no tool history exists.
- Add upgrade and rollback coverage for `~/.sessionatlas/` schemas.
- Expand macOS and Linux CLI packaging evidence.
- Convert high-value manual desktop acceptance scenarios into reliable automation.

## Later exploration

- Plugin-style scanner interfaces with a narrow, documented trust boundary.
- Optional export/import that is local, explicit, encrypted where appropriate, and never enabled by default.
- Accessibility and localization improvements driven by real user feedback.

Community proposals are welcome through GitHub issues. Security, privacy, and local-first behavior take precedence over feature breadth.
