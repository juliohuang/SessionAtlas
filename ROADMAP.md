# Roadmap

This roadmap describes direction, not a delivery promise. Priorities may change based on security findings and community feedback.

## Public beta

- Complete isolated native acceptance for the Windows installer.
- Publish signed or clearly identified unsigned Windows artifacts, SHA-256 checksums, SBOM, and build provenance.
- Enable private vulnerability reporting, secret scanning, dependency review, and branch protection on GitHub.
- Resolve or intentionally retain the project name after a trademark/confusion review.

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
