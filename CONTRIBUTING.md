# Contributing to SessionAtlas

Thanks for helping improve SessionAtlas. Small, focused changes with reproducible evidence are easiest to review.

## Before you start

1. Search existing issues and discussions before opening a duplicate.
2. Use an issue for significant features or behavior changes so scope can be agreed before implementation.
3. Never include real AI session data, credentials, SSH keys, access tokens, personal paths, or files from `~/.sessionatlas/` in an issue, test, screenshot, or commit.
4. Follow the [Code of Conduct](./CODE_OF_CONDUCT.md) and the execution rules in [`docs/execution-security-contract.md`](./docs/execution-security-contract.md).

## Development setup

Install stable Rust, Node.js 22+, and the Tauri 2 prerequisites for your operating system. From the repository root:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cd frontend
npm ci
npm run check
npm test
```

Run `cargo tauri dev` from the repository root for the desktop console. `scan_projects` runs the `sessionatlas-core` scan pipeline in-process on `spawn_blocking`; the installer does not bundle a separate scanner binary, and nothing requires a .NET runtime.

## Test isolation

Automated and manual tests must use a unique temporary `SESSIONATLAS_HOME`. Create synthetic projects and sanitized fixtures; do not copy a real index or tool history. Tests must not start a real AI CLI or connect to a real SSH server unless a maintainer has explicitly approved that exact acceptance run.

## Pull requests

- Keep each pull request focused and explain the user-visible outcome.
- Add or update tests for every behavior change.
- Update both Chinese and English user-facing text when applicable.
- Update `CHANGELOG.md` for user-visible changes.
- Run the relevant checks and report exact pass, fail, skip, and unverified results.
- Do not commit generated databases, build output, logs containing user data, or secrets.

By submitting a contribution, you agree that it is licensed under Apache-2.0, the repository's license.

## Reporting security issues

Do not open a public issue for a suspected vulnerability. Use GitHub's private vulnerability reporting once enabled for this repository. If that channel is unavailable, contact the repository owner privately through their GitHub profile and include only a minimal, redacted reproduction.
