# 内容索引控制：实现契约与验收指南

本阶段在当前源码中实现；不代表已发布的 `v0.1.0-beta.3` 安装包已包含这些功能。
上一阶段的扫描协调与跨进程写锁参见 [扫描协调验证](./scan-coordination-validation.md)。
本阶段由主代理直接实现、检查实际 diff 并验证，未另行委派。

## 1. 用户行为

入口：桌面 **设置 → 内容索引**，或以下 CLI。默认保持开启以兼容既有行为。
控制仅针对本机项目的源码/文档内容索引，不修改远程服务器，也不删除源文件、项目、
工具使用记录、会话或偏好配置。关闭内容索引不会停止 AI 工具会话元数据的项目扫描。

| CLI（桌面对应操作） | 成功后的状态 | 后续扫描行为 |
| --- | --- | --- |
| `content-index status --json`（刷新状态） | 只读查询；缺少索引时 `indexExists=false`，不创建目录或数据库 | 无 |
| `content-index disable`（关闭并清理） | `enabled=false`、内容词项/预览/指纹/统计清空 | 不读取项目源码/文档 |
| `content-index enable`（开启） | `enabled=true`；不会立即遍历项目文件 | 下次扫描允许增量索引内容 |
| `content-index clear`（清理索引） | 内容数据清空；开关不变 | 若开启，下次扫描重新生成 |
| `content-index rebuild`（立即重建） | 已开启时完整重建；不复用旧指纹 | 后续恢复增量索引 |

所有命令前缀为 `sessionatlas`；未安装时使用 `cargo run -p sessionatlas-cli -- ...`。
省略 action 等同 `status`。所有操作支持 `--json`，成功输出一个 JSON 对象；操作失败
退出非零并在 stderr 提示如何查询实际状态，不在 stdout 输出伪成功对象。
关闭时调用 rebuild 被拒绝，不隐式重新开启。

缺少项目索引时只允许状态查询，其他操作要求先扫描。这样不会因为打开设置或点击控制
创建一个空索引，误使首次启动流程跳过扫描。这个设计不提供“首次扫描前关闭”的设置；
若需要从第一次扫描起显式 opt-in，应单独设计持久化策略，不能通过更改 config.json 绕过。

状态字段：`indexExists`、`enabled`、`phase`、`searchable`、`indexedFiles`、`indexedBytes`。
`indexedBytes` 为被索引源内容的字节数，不是 SQLite 实际磁盘占用。
`searchable=true` 表示允许执行内容搜索，不保证一定存在命中（空索引也可为 true）。

## 2. 开发实现与失败边界

- 核心入口：`crates/sessionatlas-core/src/store/content_control.rs`。
  开关 `content_index_enabled` 与生命周期 `content_index_phase` 存在 `sessionatlas_meta`。
  缺少这两个键的旧索引按 enabled/ready 兼容，仍必须通过已有 sanitizer/schema 安全门。
  键值异常时内容查询关闭，名称/路径查询仍保留；状态查询报告异常。
- 写入入口持有上一阶段的 `index.db.lock`，与 CLI、Tauri 扫描互斥。
  refresh 在持锁后读取开关，不使用扫描前缓存的选择。普通查询连接继续只读。
- CLI 与桌面搜索在同一个只读事务快照中检查内容策略、读取 FTS/预览并组装项目结果，
  防止检查后发生的并发重建将部分内容混入结果；后续查询读取最新已提交的策略。
- disable 先在事务中提交 `enabled=false` 与 `cleanupPending`；clear 只提交后者。
  gate 事务失败则保持旧状态并返回错误；不得谎称已关闭。
- gate 成功后，开启 `secure_delete`，在事务中执行 FTS `delete-all` 并删除内容文件/统计；
  再执行 VACUUM 与 WAL TRUNCATE checkpoint。使用 `delete-all` 是为了清除 FTS 内部段，
  避免只留下删除标记。仅全部成功后才设为 `ready`。
- 删除事务、VACUUM、checkpoint 或最后状态写入失败均报告错误；若 gate 已提交，
  `cleanupPending` 保留并禁止内容搜索。读者占用 WAL 时清理可能超时，关闭读取事务后
  重试 clear。不能把“逻辑记录已删”直接当成“物理清理已完成”。
- rebuild 先清理，再标记 `rebuildPending`，逐项目写入，最后才恢复 `ready`。
  中途失败可能已有一部分项目提交，但所有内容结果均隐藏。重新打开后的下一次扫描
  会丢弃部分数据和指纹后重试；关闭状态只补完清理，不读取源码。
- Tauri 通过后台线程调用同一 core 服务，action 是严格枚举，前端不能传数据库路径。
  前端有确认、单次执行保护、请求序号防止旧状态覆盖新结果；失败后重新查询真实状态，
  查询也失败则禁用操作，只保留刷新入口。切换页面后仍保持忙状态，完成后刷新搜索结果。
- 浏览器演示只显示说明并禁用控制，不伪造实际索引变更。

这些是应用层协作保证，不是安全擦盘承诺：外部备份、文件系统快照、SSD 历史块、
旧 SQLite 读取事务及已返回/显示的数据不能撤回。旧版本 CLI/桌面和直接操作 SQLite
的工具不遵守新开关或写锁；整体验收必须使用本次源码构建的两个程序。
普通增量索引沿用原先的逐项目提交语义，不在本阶段改为全索引的最后成功快照。

## 3. 自动化验证与通过条件

所有测试使用合成内容、临时数据目录或模拟 Tauri IPC，不使用真实会话凭据。
禁止用日常 `~/.sessionatlas` 做清理测试，也不需要 GitHub Actions。

先 staging 静态资源，再执行 Rust lint/test（干净 checkout 没有 `frontend/dist`）：

```powershell
npm --prefix frontend run build:static
npm --prefix frontend run check
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
npm --prefix frontend test
git diff --check
```

PowerShell 应逐条检查 `$LASTEXITCODE`，上一步非零即记录失败，不能仅凭最后一条成功放行。
若受沙箱限制出现 Node `spawn EPERM`，按执行权限流程批准本地浏览器进程后重跑，
不要视作业务测试通过。

| 层级 | 本阶段验证的关键通过条件 | 证据源码 |
| --- | --- | --- |
| core，13 项新增 | 缺失索引零创建、状态 bytes/mtime 不变、关闭持久化与源文件/会话/代际不变、clear 后重读、禁用时拒绝 rebuild | `crates/sessionatlas-core/src/store/content_control/tests.rs` |
| core 故障注入（包含在 13 项内） | gate 回滚、删除失败后 fail-closed、真实 WAL 读取锁阻塞及重试、FTS 合成词项不残留在 DB/WAL/journal、部分重建隐藏及恢复、相同 mtime/size 强制重建、异常键值、未来 schema、写锁串行化 | 同上 |
| CLI，4 项新增 | 注入扫描器遵守开关；真实子进程全生命周期、缺失与无效 action 无副作用、真实写锁超时非零退出 | `scan_config_commands_tests.rs`、`tests/content_index_e2e.rs` |
| Tauri，2 项新增 | 真实 readonly 搜索连接和桌面扫描遵守 core 开关；重建恢复文件摘要；枚举拒绝非法输入且状态字段 camelCase | `src-tauri/src/content_controls/tests.rs` |
| 浏览器，12 项新增 | 中英文完整生命周期、取消、忙态、错误文本转义、失败后真实状态、无效响应、缺失索引、旧搜索移除、演示边界、旧请求竞态、状态失败、键盘重试重建 | `frontend/tests/browser/content-index.spec.js` |

针对性复跑：

```powershell
cargo test -p sessionatlas-core store::content_control
cargo test -p sessionatlas-cli --test content_index_e2e
cargo test -p sessionatlas-cli cli_scan_honors_content_policy
cargo test -p sessionatlas-tauri content_controls
npm --prefix frontend run test:browser -- tests/browser/content-index.spec.js
```

## 4. 已执行的本地证据（2026-09-08，Windows）

- Rust 全量 **587 PASS / 0 FAIL / 0 ignored**：CLI 105（含 3 项真实进程集成测试）、
  core 340、Tauri 142。包含上一阶段全部回归。
- 前端 **24 项 Node 单元 + 72 项 Playwright PASS**，其中 12 项为本阶段新增。
- staging、JS 语法、Rust fmt、Clippy `-D warnings`、diff 空白检查通过。
- 真实 CLI + 项目扫描解析器的隔离验收脚本通过：2 个项目、2 个内容文件，
  disable 后再次扫描不重建，enable 不立即重建，clear 保留开关，显式 rebuild 恢复，
  最终项目数量与源文件 SHA-256 不变，SQLite sidecar 为 0。
- 忽略目录日志：`target/content-index-workspace-tests.log`、
  `target/content-index-frontend-tests.log`、`target/content-index-fixture-validation.log`。
- 本机保留的隔离夹具：`artifacts/content-index-acceptance/d318f37c53d3449fab056f8bfa124d2e/`。
  其中 `fixture-manifest.json` 记录所有合成输入、校验和及 `verification.contentIndex`。
  这些数据不提交 Git。

16 张截图由浏览器测试生成于 `frontend/test-results/content-index-*/`；每次运行 Playwright
会重建该目录，需要长期保留时复制到指定验收目录。截图使用合成项目，并非原生 Tauri 验收。

| 图片名 | 场景 |
| --- | --- |
| `01-enabled.png`、`02-disabled.png` | 开启初始状态、关闭并清理（zh/en 各一份） |
| `03-reenabled-empty.png`、`04-rebuilt.png`、`05-cleared.png` | 开启但未重建、重建、清理（zh/en 各一份） |
| `06-busy.png` | 页面切换后仍不能重复操作 |
| `07-cleanup-failed.png`、`08-status-failed.png` | 清理部分失败、状态读取失败 |
| `09-missing.png`、`10-browser-demo.png` | 无项目索引、浏览器演示边界 |
| `11-rebuild-pending.png` | 中断重建和重试入口 |

## 5. 后续原生桌面整体验收（待用户执行）

可以复用本机上述夹具，或重新生成：

```powershell
cargo build -p sessionatlas-cli
$contentAcceptanceRoot = Join-Path $env:TEMP ('SessionAtlas-Content-Acceptance-' + [guid]::NewGuid().ToString('N'))
.\scripts\New-AcceptanceFixture.ps1 -OutputRoot $contentAcceptanceRoot -ScannerPath .\target\debug\sessionatlas.exe -VerifyContentIndex
```

预期出现 `CONTENT_INDEX_CONTROLS=PASS`、`CONTENT_INDEX_FILES=2`、`LIST_PROJECTS=2`、
`INDEX_SIDECARS=0`。脚本拒绝复用非空目录，恢复进程环境变量，不删除旧目录或安装应用。

先正常退出日常 SessionAtlas（包括托盘），再在独立 PowerShell 窗口中将
`SESSIONATLAS_HOME` 指向这个临时夹具并运行当前源码；该窗口中后续 CLI 也使用同一路径。

```powershell
$env:SESSIONATLAS_HOME = $contentAcceptanceRoot
cargo tauri dev
```

关闭此 PowerShell 窗口后该进程级环境覆盖即结束；不要把它设置为用户/系统环境变量。
检查以下各步并为原生应用截图，截图应只包含合成数据：

1. 初始有 `atlas-alpha`、`atlas-beta` 两个项目；内容查询 `Synthetic` 可命中。
2. 设置 → 内容索引：状态 enabled/ready，索引文件数为 2。
3. 取消“关闭并清理”确认：状态不变。再次确认关闭：文件数 0、disabled/ready。
4. 内容查询不再命中；名称 `atlas` 仍返回两个项目。执行项目重新扫描，内容文件仍为 0。
5. 完全退出应用并重新打开，状态仍关闭。CLI `content-index status --json` 与桌面一致。
6. 开启后仍为 0；点击立即重建后为 2，内容查询与摘要恢复。
7. 清理索引后为 0 但开关仍开；重新扫描恢复为 2。源文件仍可读取，哈希与夹具清单一致。
8. 切换中英文、键盘 Tab/Enter/Esc、最小窗口、深浅主题；确认文字不截断、按钮可达。
9. CLI 持锁期间测试桌面控制的等待/报错及再次点击恢复，不能把忙状态视为成功。
   不要对真实数据库手工制造 SQLite 锁；底层故障场景已有隔离自动化覆盖。

记录为 PASS/FAIL/BLOCKED，不以浏览器 mock 或 Rust 单元测试代替原生 IPC、WebView、
窗口/托盘交互验收。本轮 **未**重建/安装 MSI 或 NSIS、触发 UAC、执行 Unix cfg 分支、
运行 GitHub Actions、提交/推送/打标签/发布；这些仍是后续单独门禁。

下一阶段可继续首次启动/最后成功快照/终端恢复闭环；本次内容控制按本文件单独验收。
