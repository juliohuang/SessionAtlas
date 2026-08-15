## Outcome

Describe the user-visible result and why this change is needed.

## Scope

- In scope:
- Out of scope:

## Verification

List exact commands and results. Mark failures, skips, and unverified behavior explicitly.

- [ ] Rust fmt/clippy/tests (`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`)
- [ ] Frontend syntax/unit/browser tests
- [ ] Native or packaging checks when affected
- [ ] User-facing Chinese and English text updated when affected

## Safety and privacy

- [ ] Tests use a temporary `SESSIONATLAS_HOME`.
- [ ] No real sessions, personal paths, credentials, keys, databases, or sensitive screenshots are included.
- [ ] Process-launch, URL, SSH, and shell-boundary changes follow the execution security contract.

## Screenshots

Include sanitized before/after images only when the UI changed.
