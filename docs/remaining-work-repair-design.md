# SessionAtlas 剩余已知问题修复设计（历史存档）

> 本文保留 2026-08-03 建立的历史修复设计。此后仓库已从退役的 C# 桌面实现迁移为
> 纯 Rust 工作区（迁移过程与每任务验收见
> [`rust-migration-plan.md`](./rust-migration-plan.md)），且下述工作包均已通过
> 各自的自动化回归。本文件不再作为操作手册；当前剩余风险与 R14 门禁统一以
> [`remaining-issues.md`](./remaining-issues.md) 和
> [`test-baseline.md`](./test-baseline.md) 为准。

**日期：** 2026-08-03（历史基线）；归档整理 2026-08-15

**状态：** 所有工作包完成，仅参加最终回归

## 1. 设计时覆盖的工作包与现状

| 工作包 | 问题摘要 | 现状 |
| --- | --- | --- |
| WP-01 | 分组/指派/排序多步写入不原子；筛选后提交不完整顺序 | 已完成：group revision、事务、完整顺序校验、回滚测试；frontend mutation 队列与 catalog/search 分离 |
| WP-02 | Tauri 以读写/WAL 模式打开 CLI 所有的 `index.db` | 已完成：只读打开、`query_only`、写入拒绝、无 sidecar 文件 |
| WP-03 | rusqlite 行解析错误被 `.flatten()` 静默丢弃 | 已完成：坏行返回带上下文错误，不再静默丢行 |
| WP-13 | 多个远程扫描根中部分 `find` 失败可被后续成功掩盖 | 已完成：单根失败 fail-closed、stderr 脱敏/截断、批量 partial 结果 |
| WP-04 | 搜索词进入 `innerHTML`，可注入标记和样式 | 已完成：查询按文本节点渲染，真实 DOM 浏览器测试覆盖中英与恶意载荷 |
| WP-05 | xterm URL link provider 参数签名错误 | 已完成：vendored WebLinks addon、Ctrl+点击仅开 HTTP(S) |
| WP-06 | 后端写入失败后前端仍更新状态/清空表单/显示成功 | 已完成：显式 try/catch/return，成功后才有状态变更 |
| WP-07 | 全量/自动/搜索及远程失败发布规则不一致 | 已完成：full/auto/search 分离 gate、last-known-good |
| WP-08 | 文档/目录树/预览请求可跨项目覆盖较新界面 | 已完成：surface token、gate、反向完成测试 |
| WP-09 | 根目录路径规范化和项目名称处理不一致 | 已完成：统一 Windows/Unix flavor helper；Windows/Ubuntu 托管 CI 证据已取得 |
| WP-11 | CLI 标记转义、字符串选择、负数限制和工具键大小写不一致 | 已完成：typed choice、markup 字面转义、NOCASE 索引/查询 |
| WP-12 | `config.json` 直接覆盖写，崩溃或并发保存时可能损坏 | 已完成：原子写、fingerprint 冲突、有界锁、stale temp 清理、100 次并发更新 |

## 2. 保留的实质性风险与验收门禁

以下不变量在最终回归中必须继续保持（R13 之后仍是发布门禁）：

1. `index.db` 由 `sessionatlas` CLI 所有；Tauri 对它只能只读，写入拒绝且不创建
   WAL/SHM/journal。用户偏好只写 `prefs.db`。
2. 不把搜索结果或工具筛选结果当成完整项目目录；排序切换不能丢项目。
3. 后端错误必须保留原状态；前端不得在失败后伪造成功。
4. 搜索文本始终作为文本节点渲染，不能创建任何元素或属性。
5. 远程扫描单根失败必须 fail-closed，保留旧快照；批量结果显式报告 partial。
6. 本地进程保持「程序 + 参数数组」模型；工具/session ID、SSH、URL、打开器模板
   必须通过 `security.rs` 的 validator/quoting（完整契约见
   [`execution-security-contract.md`](./execution-security-contract.md)）。
7. 测试一律使用临时 `SESSIONATLAS_HOME` 与合成数据，不读取真实
   `~/.sessionatlas/`，不启动真实 AI CLI / SSH。

## 3. 当前验证命令

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
npm --prefix frontend run check
npm --prefix frontend test
```

R14 完整发布门禁与隔离扫描验收见 `rust-migration-plan.md` 的 R14 与
`remaining-issues.md` 第 4 节。
