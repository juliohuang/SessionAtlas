# SessionAtlas 剩余问题与完成执行手册

**更新时间：** 2026-08-18

**文档状态：** 当前剩余工作的唯一入口。仓库已切换为纯 Rust 工作区
（见 [`rust-migration-plan.md`](./rust-migration-plan.md)）；本文件只整理 R13 之后
仍需关闭的问题与 R14 发布门禁。迁移历史与每任务的验收命令以
`rust-migration-plan.md` 为准。

状态只允许使用：`TODO`、`IN PROGRESS`、`BLOCKED`、`PASS`、`FAIL`。

## 1. 使用方式

1. 先阅读第 2 节确认当前状态，不要重复实施已经完成的 R00～R13。
2. 严格按第 3 节的顺序处理。每个步骤都要留下命令输出、截图或日志。
3. 任一步未达到“通过标准”时，将该项保持为 `BLOCKED` 或 `FAIL`，不得跳过后宣布整体完成。
4. 原生交互场景的逐项动作以
   [`manual-acceptance-checklist.md`](./manual-acceptance-checklist.md) 为准。

## 2. 当前结论

### 2.1 已完成，不再列为剩余缺陷

- R00～R13 实现与自动化回归已完成：Rust workspace（`sessionatlas-core`、
  `sessionatlas-cli`、`sessionatlas-tauri`）、全部扫描器迁移、SQLite 快照事务、
  CLI 命令、进程安全/启动器、Tauri 进程内扫描（`spawn_blocking`，无 sidecar）、
  以及退役源码/测试删除与文档切换。
- 分组/排序已有事务、完整顺序校验、revision 冲突检查和回滚测试。
- Tauri 对 CLI `index.db` 的只读访问、数据库坏行错误传播、远程部分失败、
  搜索 HTML 安全、xterm 链接、前端 mutation/reload/surface race 已有回归测试。
- R13 之前的本地 Rust 测试基线：CLI `96/96`、Core `238/238`（跨测试二进制）、
  Tauri `60/60`，Rust 合计 `394/394`；frontend 语法检查通过；R12 隔离 Rust CLI
  扫描对 2 个合成项目建索引并退出 0。
- 全仓不再存在退役源码/项目文件；现行文档（除迁移历史文档）不再包含退役实现的构建说明。

### 2.2 仍未关闭的问题

| ID | 优先级 | 类型 | 当前状态 | 完成条件摘要 |
| --- | --- | --- | --- | --- |
| RI-01 | P1 | 测试隔离/原生证据 | IN PROGRESS | 临时 home 与 fixture 已完成；仍需原生过程真实数据根前后哈希证据 |
| RI-02 | P1 | Tauri 原生验收 | BLOCKED by RI-01 | T1～T9 全部 PASS，并保存非生产 fixture 的证据 |
| RI-04 | P1 | 托管 CI/发布 | BLOCKED | 需同一 commit 的 Windows + Ubuntu 托管 job 证据；本地 R14 自动化不能替代托管执行器 |
| RI-05 | P2 | 供应链检查 | PASS | 2026-08-18 本地 `cargo audit 0.22.2` 扫描 538 个锁定 crate：0 个漏洞；17 个上游 informational warning 已记录风险与依赖路径 |
| RI-06 | P2 | 版本控制/交付 | BLOCKED by RI-01/RI-02 | 首次 GitHub 交付已完成；剩余门禁通过后执行最终交付复核 |
| RI-07 | P3 | 测试残留处置 | BLOCKED by 用户授权 | 5 个可恢复数据库备份被归档或删除，结果有记录 |

### 2.3 已知现场状态

- 当前 `origin` 是私有 GitHub 仓库 `juliohuang/SessionAtlas`。
- 2026-08-18 使用 `cargo-audit 0.22.2` 扫描 538 个锁定 Rust crate，结果为 0 个漏洞、
  17 个已分析的上游 informational warning；处置边界记录在
  `execution-security-contract.md`。本地 RI-05 判为 PASS；托管 Security workflow
  仍须在交付 commit 上成功运行，本地结果不能替代托管证据。
- R14 自动化门禁（fmt/clippy/394 测试/前端 check+test/`cargo tauri build --ci`/
  release CLI 构建/隔离验收/`git diff --check`）已于 2026-08-15 在本机全部通过；
  托管 CI 未运行，RI-04 不因本地结果改判为 PASS。
- 真实用户数据目录中的 `index.db`、`config.json`、`prefs.db` 不得被测试读写；
  所有自动化与原生验收必须使用临时 `SESSIONATLAS_HOME`。

## 3. 每项问题的实施与检查设计

### RI-01：统一原生验收数据根

**目标不变量：** 设置 `SESSIONATLAS_HOME` 后，CLI 和 Tauri 都只读写
`$SESSIONATLAS_HOME/.sessionatlas/`；未设置时保持现有生产行为。原生验收开始前和结束后，
真实用户目录中的 `index.db`、`config.json`、`prefs.db` 均不得变化。

**当前实施进度：** 单一 home 解析函数、`db_path()`/`cli_config_path()`/`prefs_db_path()`
统一调用、`scripts/New-AcceptanceFixture.ps1` 创建唯一临时 home 与两个合成项目并生成
SHA-256 清单均已完成；R12 隔离 CLI 扫描在该环境中识别 2 个合成项目并退出 0。
完整原生过程的真实数据根前后哈希尚未记录，因此 RI-01 仍保持 `IN PROGRESS`。

**定向检查：**

```powershell
cargo test -p sessionatlas-core path
cargo test -p sessionatlas-core config
cargo test -p sessionatlas-tauri
```

**原生隔离检查：**

```powershell
$realData = Join-Path $env:USERPROFILE '.sessionatlas'
$before = Get-ChildItem -LiteralPath $realData -File -ErrorAction SilentlyContinue |
  Where-Object Name -Match '^(index|config|prefs)' |
  Get-FileHash -Algorithm SHA256

$acceptanceHome = Join-Path $env:TEMP ("SessionAtlas-Acceptance-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $acceptanceHome | Out-Null
$env:SESSIONATLAS_HOME = $acceptanceHome
# 在此环境中运行 fixture 生成器以及 Tauri smoke。

$after = Get-ChildItem -LiteralPath $realData -File -ErrorAction SilentlyContinue |
  Where-Object Name -Match '^(index|config|prefs)' |
  Get-FileHash -Algorithm SHA256
Compare-Object $before $after -Property Path,Hash
Get-ChildItem -LiteralPath (Join-Path $acceptanceHome '.sessionatlas') -Force
```

**通过标准：** 定向测试通过；三条 Rust 路径都使用 override；
`Compare-Object` 无输出；临时根只出现 fixture 声明过的文件；无真实 AI CLI 或 SSH 进程。

**失败处置：** 立即停止原生矩阵，关闭本次启动的进程，保留临时根和文件哈希；
只回滚本工作包的路径 helper/fixture 改动，不删除真实用户目录中的任何文件。

### RI-02：完成 Tauri 原生交互矩阵

**前置条件：** RI-01 为 PASS；自动化全套通过；使用临时 home 和虚构项目；
准备一次性、本机隔离的 SSH 服务或等价离线故障注入，不连接真实服务器。

**实施步骤：**

1. 用 RI-01 的脚本创建至少两个项目、两个分组、隐藏成员、文档树、历史 session
   和一个无害终端命令；记录 fixture 清单和 SHA-256。
2. 构建候选程序并记录 commit/worktree 标识、EXE 哈希、Windows 和 WebView2 版本。
3. 按验收清单执行 T1～T5：快速项目切换、docs/files race、modal close race、
   恶意搜索和 HTTP(S) 终端链接。
4. 将临时 `prefs.db` 调整为可稳定复现的写失败，执行 T6；恢复权限后确认上次成功状态
   仍在。禁止通过破坏真实 prefs 数据制造失败。
5. 只连接一次性本机 SSH fixture，分别制造“一根成功一根失败”和“一台成功一台失败”，
   执行 T7、T9；确认 last-known-good 和 partial 状态。
6. 在有隐藏成员的搜索/tool/recency 筛选下执行 T8，清除筛选并重启，核对完整顺序。
7. 每个场景立即填写记录模板；失败时保存临时 home，不继续覆盖现场。

**构建与自动检查：**

```powershell
npm --prefix frontend run check
npm --prefix frontend test
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo tauri build
```

**通过标准：** Rust 与前端自动化全套通过；T1～T9 全部为 PASS；没有真实凭据、
外部 SSH 或 AI CLI；真实数据目录哈希未变化。

**失败处置：** 将失败场景标成 FAIL，记录最小复现、截图/日志、候选程序哈希和临时 home；
为缺陷补自动化回归后，从该场景开始重测，并最终重跑 T1～T9 全部矩阵。

### RI-04：取得 Windows/Ubuntu 托管 CI 证据

**目标不变量：** 同一 commit 必须在 Windows 和 Ubuntu 的托管执行器上运行相同的
Rust 测试与 CLI 构建。开发机和本地容器结果不能替代该门禁。

**实施步骤：**

1. 使用 `.github/workflows/ci.yml`，确认 `rust-core-cli` 与 `tauri` job 在同一 commit
   上运行 `cargo test`、`cargo clippy --workspace --all-targets -- -D warnings`、
   `cargo fmt --all -- --check`。
2. 在任何远端操作前记录目标仓库、可见性、分支保护和费用影响；需要用户明确授权后
   才能新增 remote、push 或修改平台设置。
3. 保存 Windows/Ubuntu job URL、commit SHA、测试数量和日志。
4. 将两条结果写入 `test-baseline.md`；若一端失败，先修复并让两端在新 commit 上重跑，
   不得拼接不同 commit 的结果。

**通过标准：** Windows 和 Ubuntu 均成功；两条记录指向同一 commit；无 skipped/failure。

**失败处置：** 平台未选或未授权时保持 BLOCKED；CI 失败时保存完整日志，按失败平台复现，
修复后重新触发完整矩阵。不得删除或绕过失败 job。

### RI-05：完成 Rust 依赖漏洞检查

**实施步骤：**

1. 记录 Rust toolchain、`Cargo.lock` 哈希和 advisory database 更新时间。
2. 在隔离的开发工具环境安装 `cargo-audit`；安装本身不修改项目依赖。
3. 对锁文件执行审计并保存完整输出。
4. 对每个 advisory 判断：可升级、无可用修复、误报或不在可达路径；不能只看退出码。
5. 如需升级依赖，作为独立修复提交，重跑 Rust、frontend 和 Tauri build 全套。

**检查命令：**

```powershell
cargo install cargo-audit --locked
cargo audit
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

**通过标准：** `cargo audit` 退出 0，或每个未修复 advisory 都有风险说明、影响范围、
临时缓解和明确批准；不能把“工具未安装”记录为通过。

**失败处置：** 不自动执行大版本升级；保留审计输出，建立独立修复项并重新评估 Tauri
兼容性。无网络导致数据库无法更新时标记 BLOCKED，并记录数据库日期。

### RI-06：审阅、提交和远端交付

**前置条件：** RI-01、RI-02、RI-04、RI-05 均为 PASS，或有维护者签字接受的例外；用户明确授权本次
stage/commit/push 的目标和范围。

**实施步骤：**

1. 用 `git status --short` 和 `git diff --stat` 建立文件清单，确认没有 `.env`、数据库、
   测试报告、私钥或生成缓存。
2. 逐个阅读所有改动文件；重点复核 secrets、外部命令、路径、SQLite 所有权、超时、
   重试、幂等、错误传播和前端 HTML sink。
3. 运行第 4 节最终门禁，并把实际数量写入验收记录。
4. 由维护者确定提交拆分。
5. 获得授权后才 stage 和 commit；再次检查 staged diff 和文件名单。
6. 获得 push 授权并确认远端/分支后才推送；记录 commit SHA 和远端检查链接。

**检查命令：**

```powershell
git status --short
git diff --check
git diff --stat
git diff --name-only
git diff --cached --check
```

**通过标准：** 所有文件都在预期范围；无敏感或生成文件；最终门禁通过；commit 可追溯。

**失败处置：** 未授权时保持 BLOCKED，不 stage、不 commit、不 push；发现异常文件时先停止，
只移除可证明为本次生成的文件，绝不使用 `git reset --hard` 或覆盖用户改动。

### RI-07：处置 5 个测试数据库备份

这些文件位于仓库外，可能具有恢复价值；本项不阻塞代码正确性，但发布收口时必须给出
明确决定。未经用户授权不得删除。

**实施步骤：**

1. 只枚举名称精确匹配 `index.db.native-*-20260809` 的 5 个文件，记录绝对路径、大小、
   修改时间和 SHA-256。
2. 由用户选择：保留原位、移动到用户指定归档目录，或永久删除。
3. 移动或删除前再次解析每个绝对路径，确认其父目录就是
   `%USERPROFILE%\.sessionatlas`，且目标不是活动 `index.db`。
4. 执行用户选择后重新枚举目录，记录结果和可恢复性。

**通过标准：** 5 个文件各自有处置记录；若保留，记录位置和用途；若移动，目标可访问且
哈希一致；若删除，用户已明确授权且复查不再存在。`prefs.db*` 和任何活动索引不在范围内。

**失败处置：** 路径、数量或哈希不符就停止，不移动、不删除，并让用户确认实际范围。

## 4. R14 最终总门禁

R00～R13 已完成，本门禁属于 R14（全量发布验收），详见
`rust-migration-plan.md` R14。必须逐条执行并记录实际退出码与数量：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
npm --prefix frontend run check
npm --prefix frontend test
cargo tauri build --ci
cargo build --locked -p sessionatlas-cli --release
$scanner=(Resolve-Path target/release/sessionatlas.exe)
./scripts/New-AcceptanceFixture.ps1 -ScannerPath $scanner
git diff --check
git status --short
```

**通过标准：** 所有自动化命令退出码为 0；隔离 home 生成 `index.db` 且同时包含
两个合成 Codex 项目、正确 session ID 和 UTC 时间；MSI 和 NSIS 安装包均生成；
`git diff --check` 无空白错误；`git status` 只包含预期改动。

**R14 本机执行记录（2026-08-15，Windows x64）：** 上述自动化命令全部退出 0；
`cargo test --workspace --no-fail-fast` 394/394 通过；`npm --prefix frontend test`
16 单元 + 24 浏览器测试通过；MSI
`SessionAtlas_0.1.0_x64_en-US.msi`（4,304,896 字节）与 NSIS
`SessionAtlas_0.1.0_x64-setup.exe`（3,128,120 字节）均生成（SHA-256 分别为
`cd53a5fe9862a10f351928de76818b3055df614b249ac906340c138743b89f03`、
`1506ccc28d6b20fa08deee922c0a0753f822b999afe6ff274e6830c4202c08f8`）；隔离 home 的
`index.db` 86016 字节并持久化两个会话 ID、SQLite sidecar 为 0，`list` 2 个项目，
`search` 读回两个 UTC 时间，
manifest schemaVersion=2 完整记录两个项目/会话 ID/UTC 时间；`git diff --check`
通过。修复了一处 kimi/opencode/parsing 测试共享 `ENV_LOCK` 的并发缺陷。

**R14 未通过/未执行（仍为发布门禁）：** 本机未重跑 `cargo audit`（托管 Security
workflow 仍是门禁）；托管 CI 未运行（RI-04 保持 BLOCKED）；第 10 节手工/真机
门禁与原生 UI T1～T9 全部未通过。

**2026-08-18 后续复核：** 已在当前工作区执行 `cargo audit 0.22.2`，扫描 538 个
锁定 crate，结果为 0 个漏洞、17 个已分析的上游 informational warning，RI-05
更新为 PASS。R14 当时未执行的历史记录保留不变；托管 Security workflow 仍需在
交付 commit 上通过。

## 5. 整体完成定义

只有同时满足以下条件，才能宣布“剩余问题全部关闭”：

1. RI-01、RI-02、RI-04～RI-07 均为 PASS；任何例外都有维护者、原因、影响和后续期限。
2. T1～T9 全部有可审计记录，不能用 headless 测试或启动截图替代交互结果。
3. 同一 commit 的 Windows/Ubuntu 托管 CI 均通过。
4. R14 最终门禁无失败、无跳过、无新增 warning，测试数量不低于记录基线。
5. 原生测试未启动真实 AI CLI、未连接真实 SSH、未读取或改变真实索引和凭据。
6. Rust advisory scan 已执行并处置结果。
7. 工作树、提交和远端状态与用户授权一致；数据库备份处置有明确记录。
