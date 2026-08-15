# SessionAtlas remaining-work execution progress

Updated: 2026-08-09

> 最新剩余问题、依赖顺序、逐步操作和检查标准统一见
> [`remaining-issues.md`](./remaining-issues.md)。本文件保留 R0～R12 的历史执行摘要。
> 2026-08-14 全量身份迁移后，历史步骤中的命令和路径已统一按当前
> `SessionAtlas.*` / `sessionatlas` 标识表达，不代表保留旧别名。

| Step | Status | Main verification |
| --- | --- | --- |
| R0 | complete | Baselines captured: C# 47, frontend 10, Rust 43 |
| R1 | complete | Reproducible Playwright harness and mocked Tauri smoke tests |
| R2 | complete | Transactional group move/order, revision and rollback tests |
| R3 | complete | English/Chinese hostile-query browser tests |
| R4 | complete | Vendored xterm web-links provenance/hash and coordinate tests |
| R5 | complete | Mutation rollback/partial-success browser tests; Rust NotFound tests |
| R6 | complete | Independent reload ownership, LKG and stale-source tests |
| R7 | complete | Surface inverse-completion, close-late-response and collapse tests |
| R8 | implementation complete | 39 focused path tests; full C# 71 at step; Windows/Ubuntu CI gate added |
| R9 | complete | 25 focused CLI tests; full C# 84 and Rust 48 |
| R10 | complete | 7 atomic config tests including 100 concurrent updates; full C# 89 |
| R11 | complete | Independent Desktop project, 6 tests, 0-warning build |
| R12 | local automation, Tauri smoke, and Avalonia startup fix complete | C# 89, Desktop 7, frontend 39 (16 unit + 23 browser), Rust 48; Tauri smoke passed and Avalonia window/accessibility tree was captured |

## Remaining release checks

1. PASS on 2026-08-14: the project was reinitialized and uploaded to the private
   GitHub repository `juliohuang/SessionAtlas`. The Windows/Ubuntu
   `path-semantics` workflow passed on exact commit
   `f2ce07c6245c0ee8fbf31bd84d9b9312beafb99c`; each runner passed 39/39 focused
   tests and both CLI/Desktop builds with 0 warnings and 0 errors.
2. Complete the remaining native Tauri manual matrix against the built app:
   rapid A/B switching, docs/files close races, remote partial failure, group
   reorder and terminal link activation. Startup, malicious-search text safety,
   Escape clearing, and settings open/close smoke are already recorded.
3. Run the Avalonia manual matrix: A/B tab switching must keep both PTYs alive,
   explicit close kills only its tab, window exit closes all, and exact old
   session resume reaches the CLI. Startup deadlock is fixed and a responsive
   window handle was verified; full control-layer capture remains pending.

The executable procedures and evidence fields are in
`docs/manual-acceptance-checklist.md`.

At the original R12 checkpoint, no commit, push, deployment, real credential
use, SSH connection, or real AI CLI launch was performed. On 2026-08-14 the
repository history was intentionally reinitialized with user authorization and
uploaded to the private GitHub repository `juliohuang/SessionAtlas`. The old
`main` history remains recoverable from the recorded Git bundle. Native tests
continue to use isolated data and the full interaction matrix remains open.
