# SessionAtlas Rust migration execution progress

Updated: 2026-08-15

> 迁移计划、任务依赖、每个任务的允许修改范围与验证见
> [`rust-migration-plan.md`](./rust-migration-plan.md)。本文件只保留 R00～R14 的
> 执行摘要与可复核证据。当前仓库为纯 Rust 工作区，退役实现已删除。

| Step | Status | Main verification |
| --- | --- | --- |
| R00 | complete | CI 移除已删除桌面 GUI 项目的测试/构建步骤，基线可运行 |
| R01 | complete | Rust workspace 骨架：`sessionatlas-core`、`sessionatlas-cli`、`src-tauri` 三个 package 可编译可测试 |
| R02 | complete | `model.rs`/`path.rs` 迁移，路径与父子语义契约测试通过 |
| R03 | complete | `config.rs` 原子写、有界锁、fingerprint 冲突与并发测试通过 |
| R04 | complete | `scanner/` 框架、`ScanOutcome`、诊断与共享解析迁移完成 |
| R05A/B/C | complete | Claude/Codex、Kimi/OpenCode、Aider/自定义扫描器迁移完成 |
| R06 | complete | `indexer.rs` 跨工具合并、session ID 去重、Git 分支读取通过 |
| R07 | complete | `store.rs` SQLite 快照事务、FTS5、孤儿清理、只读异常行检查通过 |
| R08 | complete | CLI 只读命令 `list`/`search`/`recent` 与默认交互列表通过 |
| R09 | complete | CLI `scan`/`config` 只写临时 home，退出/保留语义有测试 |
| R10 | complete | `process.rs`/`security.rs`/`launcher.rs` 参数数组与拒绝规则通过 |
| R11 | complete | Tauri `scan_projects` 改为 `spawn_blocking` 进程内调用 `sessionatlas-core`，无 sidecar |
| R12 | complete | 移除旧扫描器捆绑构建、`externalBin` 和旧生态审计；CI 与发布链切换为 cargo |
| R13 | complete | 删除全部退役源码/测试，文档更新为纯 Rust 架构，issue/PR 模板更新 |
| R14 | complete（本地可执行门禁） | 格式/lint/测试/前端/安装包/隔离扫描全部在本机通过；托管依赖审计与手工/真机门禁未通过（见下方清单） |

## R14 可复核证据（本机自动化，2026-08-15）

- `cargo fmt --all -- --check` 退出 0。
- `cargo clippy --workspace --all-targets -- -D warnings` 退出 0。
- `cargo test --workspace --no-fail-fast` 退出 0，Rust 合计 394/394 通过，0 失败/忽略
  （CLI 96、Core 238 跨其测试二进制、Tauri 60）。
- `npm --prefix frontend run check` 退出 0。
- `npm --prefix frontend test` 退出 0（16 单元测试 + 24 Playwright 浏览器测试）。
- `cargo tauri build --ci` 退出 0，生成
  `target/release/bundle/msi/SessionAtlas_0.1.0_x64_en-US.msi`（4,304,896 字节）与
  `target/release/bundle/nsis/SessionAtlas_0.1.0_x64-setup.exe`（3,128,120 字节）；
  SHA-256 分别为 `cd53a5fe9862a10f351928de76818b3055df614b249ac906340c138743b89f03`
  与 `1506ccc28d6b20fa08deee922c0a0753f822b999afe6ff274e6830c4202c08f8`。
- `cargo build --locked -p sessionatlas-cli --release` 退出 0。
- `scripts/New-AcceptanceFixture.ps1 -ScannerPath <release CLI>` 退出 0：隔离 home 生成
  `index.db`（86016 字节），持久化两个会话 ID 且 SQLite sidecar 为 0，`list` 恰好
  2 个项目（atlas-alpha/atlas-beta），`search`
  读回两个 UTC 时间（2026-08-15 01:00 / 02:00）；manifest schemaVersion=2 完整记录
  两个合成项目路径、两个会话 ID（acceptance-alpha/acceptance-beta）与两个 UTC 时间。
- `git diff --check` 无空白错误；`git status --short` 仅包含预期的迁移与验收改动。
- 修复一处测试并发缺陷：kimi/opencode/parsing 三个测试模块各自持有独立的
  `ENV_LOCK`，并行运行时互相覆盖 `SESSIONATLAS_HOME`/`KIMI_CODE_HOME` 导致
  `kimi_scanner_home_resolution_follows_sessionatlas_then_kimi_code_home` 偶发失败；
  改为共享 parsing 模块级 `crate::scanner::parsing::ENV_LOCK` 后 6 轮全量 lib 测试稳定通过。

## Remaining release gates（R14 未执行项与手工/真机验收）

1. R14 本地未重跑 `cargo audit`（本机未安装，按约定不新增工具）；托管 Security
   workflow 固定安装 `cargo-audit 0.22.2` 并执行 `cargo audit`，仍是发布门禁。
2. 托管 CI 与托管 Security workflow 未在托管执行器上运行；Windows/Ubuntu 托管 job
   证据需在同一 commit 的托管执行器上产生。
3. 原生 Tauri 交互矩阵 T1～T9（见
   [`manual-acceptance-checklist.md`](./manual-acceptance-checklist.md)）。
4. 手工/真机门禁：无额外语言运行时的沙箱安装与首次扫描、真实工具数据目录只读扫描、
   `sessionatlas open` 在多个平台的真实启动、迁移前 `index.db` 兼容性验收
   （见 `rust-migration-plan.md` 第 10 节）。
