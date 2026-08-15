# SessionAtlas 剩余问题与完成执行手册

**更新时间：** 2026-08-15

**文档状态：** 当前剩余工作的唯一入口

**适用范围：** 当前 `main` 与后续验收工作；本文件只整理设计和验收步骤，不代表未标记
为 `PASS` 的功能已经完成

## 1. 使用方式

1. 先阅读第 2 节确认当前状态，不要重复实施已经完成的 R0～R11。
2. 严格按第 4 节的顺序处理。每个步骤都要留下命令输出、截图或日志。
3. 任一步未达到“通过标准”时，将该项保持为 `BLOCKED` 或 `FAIL`，不得跳过后宣布整体完成。
4. 每完成一步，按第 5 节模板更新本文件和
   [`test-baseline.md`](./test-baseline.md)。
5. 原生交互场景的逐项动作以
   [`manual-acceptance-checklist.md`](./manual-acceptance-checklist.md) 为准；
   本文件说明开始这些场景之前还要准备什么，以及怎样判定整个工作包完成。

状态只允许使用：`TODO`、`IN PROGRESS`、`BLOCKED`、`PASS`、`FAIL`。

## 2. 当前结论

### 2.1 已完成，不再列为剩余缺陷

- R0～R11 的实现和自动化回归已经完成。
- 分组/排序已有事务、完整顺序校验、revision 冲突检查和回滚测试。
- Tauri 对 CLI `index.db` 的只读访问、数据库坏行错误传播、远程部分失败、
  搜索 HTML 安全、xterm 链接、前端 mutation/reload/surface race 已有回归测试。
- C# CLI、共享路径语义、配置原子写以及 legacy Avalonia 的 headless 修复已经完成。
- Avalonia 启动死锁已修复；真实窗口、标题和 accessibility tree 已确认可读取。
- Tauri release 构建及基础原生 smoke 已完成：启动、空索引错误、恶意搜索文本、
  Escape 清理和设置抽屉开关均已有证据。
- 主前端已升级为“项目列表 / 项目概览 / 终端工作区”三栏界面；
  第二轮已调整栏宽和可读性，并加入真实最新会话入口及单行状态栏；浏览器样例模式
  已完成视觉与无溢出检查。此项不替代 RI-02 的原生 Tauri 证据。
- 当前自动化证据为：C# `89/89`、Desktop `7/7`、frontend unit `16/16`、
  Playwright `24/24`、Rust `54/54`；Rust fmt、clippy 和两个 C# build 均通过。
- Windows 主机上的本地 Linux 容器等价检查已通过 39 个路径测试及两个 build；
  这只能作为补充证据，不能代替托管 CI。
- GitHub Actions `path-semantics` 已在提交
  `f2ce07c6245c0ee8fbf31bd84d9b9312beafb99c` 上通过：Windows 与 Ubuntu 各
  `39/39`、0 skipped，CLI/Desktop build 均为 0 warning、0 error。

### 2.2 仍未关闭的问题

| ID | 优先级 | 类型 | 当前状态 | 完成条件摘要 |
| --- | --- | --- | --- | --- |
| RI-01 | P1 | 测试隔离/实现 | IN PROGRESS | 路径、fixture 和 sidecar smoke 已完成；仍需原生前后哈希证据 |
| RI-02 | P1 | Tauri 原生验收 | BLOCKED by RI-01 | T1～T9 全部 PASS，并保存非生产 fixture 的证据 |
| RI-03 | P1 | Avalonia 原生验收 | BLOCKED by RI-01/桌面控制环境 | A1～A6 全部 PASS；当前只证明窗口可启动 |
| RI-04 | P1 | 托管 CI/发布 | PASS | Windows + Ubuntu 托管任务已在同一提交上通过 |
| RI-05 | P2 | 供应链检查 | PASS | Rust advisory scan 成功，无未处置漏洞 |
| RI-06 | P2 | 版本控制/交付 | BLOCKED by RI-01/RI-02/RI-03 | 首次 GitHub 交付已完成；剩余门禁通过后执行最终交付复核 |
| RI-07 | P3 | 测试残留处置 | BLOCKED by 用户授权 | 5 个可恢复数据库备份被归档或删除，结果有记录 |

当前已关闭 `2/7`：RI-04、RI-05。

### 2.3 已知现场状态

- 当前 `origin` 是私有 GitHub 仓库 `juliohuang/SessionAtlas`；旧 Git 历史已生成可恢复
  bundle，新仓库以单一根提交 `f2ce07c6245c0ee8fbf31bd84d9b9312beafb99c` 首次上传。
- 首次上传前确认 140 个受控文件与旧 HEAD Tree SHA 完全一致，未暂存数据库、`.env`、
  私钥或生成缓存；后续工作树状态仍以实时 `git status --short` 为准。
- 2026-08-15 使用 `cargo-audit 0.22.2` 扫描 526 个锁定 Rust crate，结果为 0 个漏洞、
  17 个已分析的上游 warning。warning 包括 GTK3 与 `rust-unic` 维护状态、
  `proc-macro-error` 维护状态以及未被本项目调用的 Linux `glib::VariantStrIter` API；
  处置边界记录在 `execution-security-contract.md`，RI-05 已标记 PASS。
- 2026-08-14 全量身份迁移开始时，只读快照确认支持的数据根与前代数据根均不存在；
  交接中列出的 5 个可恢复备份当前未找到，本轮没有删除或搬迁任何数据库。
- C# 与 Tauri 均支持 `SESSIONATLAS_HOME`。Tauri 的 `db_path`、`cli_config_path`、
  `prefs_db_path` 已统一调用单一解析函数，scan 子进程也显式传递该变量。
- Avalonia 原生窗口已经可启动，但自动桌面控制的鼠标和键盘操作返回
  `GetCursorPos access denied`，所以不能把 A1～A6 标成通过。

## 3. 每项问题的实施与检查设计

### RI-01：统一原生验收数据根

**目标不变量：** 设置 `SESSIONATLAS_HOME` 后，CLI、Tauri 和 Avalonia 都只读写
`$SESSIONATLAS_HOME/.sessionatlas/`；未设置时保持现有生产行为。原生验收开始前和结束后，
真实用户目录中的 `index.db`、`config.json`、`prefs.db` 及 sidecar 均不得变化。

**实施步骤：**

1. 在 Rust 中建立单一 home 解析函数，语义与
   `ScannerRegistry.GetHomeDirectory()` 对齐：非空 `SESSIONATLAS_HOME` 先转换为绝对路径，
   空值或未设置时才使用 OS home。
2. 让 `db_path()`、`cli_config_path()` 和 `prefs_db_path()` 都调用该函数，禁止三处各自解析。
3. 将“解析 override 值”和“读取进程环境”分开，使单元测试不必并发修改全局环境变量。
4. 增加 Rust 测试，覆盖：绝对 override、相对 override、空白 override、无 override、
   三个目标文件均位于同一个 `.sessionatlas` 目录。
5. 保留并复跑 C# home override 测试；增加一个跨组件契约说明，明确该环境变量代表
   “用户 home 根”，不是 `.sessionatlas` 目录本身。
6. 新增一个原生验收 fixture 脚本。脚本必须创建唯一临时 home、最小 `index.db`、
   两个虚构项目目录和可删除的偏好库；不得读取或复制真实索引、凭据、SSH 配置。
7. 原生启动器必须显式把 `SESSIONATLAS_HOME` 传给子进程，并在日志首行记录临时根，
   但不得记录任何凭据。

**当前实施进度：** 第 1～7 步的实现已完成；`scripts/New-AcceptanceFixture.ps1`
创建唯一临时 home、两个合成项目、最小索引、可删除偏好库与 SHA-256 清单。2026-08-15
打包 sidecar 在该环境中真实扫描出 `2` 个项目并退出 0；新增 Rust 契约测试 `5/5`，
C# Home/Config 定向测试 `11/11`。完整原生过程的真实数据根前后哈希尚未记录，因此
RI-01 仍保持 `IN PROGRESS`。

**定向检查：**

```powershell
Push-Location src-tauri
cargo test home_
cargo test application_files
Pop-Location
dotnet test SessionAtlas.Tests\SessionAtlas.Tests.csproj --nologo --filter "Home|Config"
```

如果最终测试名不同，执行者应把实际 filter 和命中的测试名记录到验收日志，不能接受
“0 个测试命中”的成功退出。

**原生隔离检查：**

```powershell
$realData = Join-Path $env:USERPROFILE '.sessionatlas'
$before = Get-ChildItem -LiteralPath $realData -File -ErrorAction SilentlyContinue |
  Where-Object Name -Match '^(index|config|prefs)' |
  Get-FileHash -Algorithm SHA256

$acceptanceHome = Join-Path $env:TEMP ("SessionAtlas-Acceptance-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $acceptanceHome | Out-Null
$env:SESSIONATLAS_HOME = $acceptanceHome
# 在此环境中运行 fixture 生成器以及 Tauri/Avalonia smoke。

$after = Get-ChildItem -LiteralPath $realData -File -ErrorAction SilentlyContinue |
  Where-Object Name -Match '^(index|config|prefs)' |
  Get-FileHash -Algorithm SHA256
Compare-Object $before $after -Property Path,Hash
Get-ChildItem -LiteralPath (Join-Path $acceptanceHome '.sessionatlas') -Force
```

**通过标准：** Rust/C# 定向测试通过；三条 Rust 路径都使用 override；
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
Push-Location frontend
npm ci
npm run check
npm test
Pop-Location
Push-Location src-tauri
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo tauri build
Pop-Location
```

**通过标准：** `16` 个 unit、`23` 个 browser 和 `48` 个 Rust 测试至少保持通过；
T1～T9 全部为 PASS；没有真实凭据、外部 SSH 或 AI CLI；真实数据目录哈希未变化。

**失败处置：** 将失败场景标成 FAIL，记录最小复现、截图/日志、候选程序哈希和临时 home；
为缺陷补自动化回归后，从该场景开始重测，并最终重跑 T1～T9 全部矩阵。

### RI-03：完成 Avalonia 原生交互矩阵

**前置条件：** RI-01 为 PASS；Desktop tests/build 通过；使用无害 CLI shim，
其参数和生命周期可记录但不会访问网络或生产数据。

**实施步骤：**

1. 用临时索引准备 A、B 两个项目、精确历史 session ID、Windows 根路径；
   Unix 根路径场景只在对应平台执行，不能在 Windows 伪造通过。
2. 启动 Avalonia，确认标题、窗口响应和临时数据根；这一步只证明 A0 启动条件，
   不能代替 A1～A6。
3. 使用人工交互或可工作的桌面自动化环境，执行 A1～A3：双 tab 切换、单 tab 关闭、
   整窗关闭；由 shim 的进程日志证明 PTY 生命周期。
4. 执行 A4，用 shim 参数日志核对完整 session ID，禁止仅凭 UI 文本判断。
5. 执行 A5，使用可控延迟让较早查询较晚完成，确认只发布最后查询结果。
6. 执行 A6，写入、查回并显示根目录项目；按实际 OS 标记适用场景。
7. 当前 `GetCursorPos access denied` 的控制环境若仍失败，状态保持 BLOCKED；
   改由有权限的交互式 Windows 会话执行，不能把 accessibility tree 截图当成功证据。

**自动检查：**

```powershell
dotnet test SessionAtlas.Desktop.Tests\SessionAtlas.Desktop.Tests.csproj --nologo
dotnet build SessionAtlas.Desktop --nologo
dotnet test SessionAtlas.Tests\SessionAtlas.Tests.csproj --nologo --filter "Path|Root|Session|Dispatcher"
```

**通过标准：** Desktop `7/7` 至少保持通过、build 为 0 warning/0 error；A1～A6
所有适用项 PASS；N/A 有平台理由和替代自动化证据；真实数据根哈希未变化。

**失败处置：** 若只是桌面控制权限失败，记录为环境 BLOCKED；若实际 UI 行为错误，记录为
FAIL，补回归测试和最小修复后重跑 Desktop 全套及 A1～A6。

### RI-04：取得 Windows/Ubuntu 托管 CI 证据

**目标不变量：** 同一 commit 必须在 Windows 和 Ubuntu 的托管执行器上运行相同的路径、
快照、FTS 测试以及 CLI/Desktop build。开发机和本地容器结果不能替代该门禁。

**实施步骤：**

1. 由仓库所有者选择唯一执行路线：
   - 将分支镜像到允许 GitHub Actions 的仓库，直接使用
     `.github/workflows/path-semantics.yml`；或
   - 在 Aliyun Codeup 中建立等价的 Windows/Ubuntu pipeline。
2. 在任何远端操作前记录目标仓库、可见性、分支保护和费用影响；需要用户明确授权后
   才能新增 remote、push 或修改平台设置。
3. 确认两个 job 都执行以下命令，且测试 filter 实际命中 39 个当前聚焦测试：

```powershell
dotnet test SessionAtlas.Tests/SessionAtlas.Tests.csproj --nologo --filter "Path|Root|Snapshot|Fts"
dotnet build --nologo
dotnet build SessionAtlas.Desktop --nologo
```

4. 在同一 commit 上触发 pipeline，保存 Windows/Ubuntu job URL、commit SHA、测试数量和日志。
5. 将两条结果写入 `test-baseline.md`；若一端失败，先修复并让两端在新 commit 上重跑，
   不得拼接不同 commit 的结果。

**本地预检：**

```powershell
dotnet test SessionAtlas.Tests\SessionAtlas.Tests.csproj --nologo --filter "Path|Root|Snapshot|Fts"
dotnet build --nologo
dotnet build SessionAtlas.Desktop --nologo
```

**通过标准：** Windows 和 Ubuntu 均成功；两条记录指向同一 commit；无 skipped/failure；
仓库的必需检查或发布清单引用这两个 job。

**执行结果（PASS，2026-08-14）：** 私有仓库
[`juliohuang/SessionAtlas`](https://github.com/juliohuang/SessionAtlas) 的
[`path-semantics` run 31812198177](https://github.com/juliohuang/SessionAtlas/actions/runs/31812198177)
在提交 `f2ce07c6245c0ee8fbf31bd84d9b9312beafb99c` 上完成。Windows job
`94805258666` 与 Ubuntu job `94805258752` 均为 `39/39`、0 skipped；两端 CLI 和
Desktop build 均为 0 warning、0 error。仅有 Actions Node 20 迁移至 Node 24 的平台弃用
提示，不影响本次结果。

**失败处置：** 平台未选或未授权时保持 BLOCKED；CI 失败时保存完整日志，按失败平台复现，
修复后重新触发完整矩阵。不得删除或绕过失败 job。

### RI-05：完成 Rust 依赖漏洞检查

**实施步骤：**

1. 记录 Rust toolchain、`src-tauri/Cargo.lock` 哈希和 advisory database 更新时间。
2. 在隔离的开发工具环境安装 `cargo-audit`；安装本身不修改项目依赖。
3. 对锁文件执行审计并保存完整输出。
4. 对每个 advisory 判断：可升级、无可用修复、误报或不在可达路径；不能只看退出码。
5. 如需升级依赖，作为独立修复提交，重跑 Rust、frontend 和 Tauri build 全套。

**检查命令：**

```powershell
cargo install cargo-audit --locked
Push-Location src-tauri
cargo audit
cargo test
cargo clippy --all-targets -- -D warnings
Pop-Location
```

**通过标准：** `cargo audit` 退出 0，或每个未修复 advisory 都有风险说明、影响范围、
临时缓解和明确批准；不能把“工具未安装”记录为通过。

**失败处置：** 不自动执行大版本升级；保留审计输出，建立独立修复项并重新评估 Tauri
兼容性。无网络导致数据库无法更新时标记 BLOCKED，并记录数据库日期。

### RI-06：审阅、提交和远端交付

**前置条件：** RI-01～RI-05 均为 PASS，或有维护者签字接受的例外；用户明确授权本次
stage/commit/push 的目标和范围。

**实施步骤：**

1. 用 `git status --short` 和 `git diff --stat` 建立文件清单，确认没有 `.env`、数据库、
   测试报告、私钥或生成缓存。
2. 逐个阅读所有改动文件；重点复核 secrets、外部命令、路径、SQLite 所有权、超时、
   重试、幂等、错误传播和前端 HTML sink。
3. 运行第 4 节最终门禁，并把实际数量写入验收记录。
4. 由维护者确定提交拆分；建议至少分为“业务修复与测试”“验收隔离”“文档与 CI”。
5. 获得授权后才 stage 和 commit；再次检查 staged diff 和文件名单。
6. 获得 push 授权并确认远端/分支后才推送；记录 commit SHA 和远端检查链接。

**检查命令：**

```powershell
git status --short
git diff --check
git diff --stat
git diff --name-only
git diff --cached --check
git status --short
```

**通过标准：** 所有文件都在预期范围；无敏感或生成文件；最终门禁通过；commit 可追溯；
若执行 push，远端 commit 与本地 SHA 一致。

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

**只读检查：**

```powershell
$dataRoot = Join-Path $env:USERPROFILE '.sessionatlas'
Get-ChildItem -LiteralPath $dataRoot -File |
  Where-Object Name -Like 'index.db.native-*-20260809' |
  Select-Object FullName,Length,LastWriteTime,
    @{Name='SHA256';Expression={(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash}}
```

**通过标准：** 5 个文件各自有处置记录；若保留，记录位置和用途；若移动，目标可访问且
哈希一致；若删除，用户已明确授权且复查不再存在。`prefs.db*` 和任何活动索引不在范围内。

**失败处置：** 路径、数量或哈希不符就停止，不移动、不删除，并让用户确认实际范围。

## 4. 推荐执行顺序与逐步门禁

| 步骤 | 工作 | 开始条件 | 本步检查 | 完成后进度 |
| --- | --- | --- | --- | --- |
| S1 | 实施 RI-01 数据根统一 | 当前自动化基线可运行 | Rust/C# 定向测试 + 两个原生 smoke + 真实 home 哈希不变 | 1/7 |
| S2 | 执行 RI-02 Tauri 矩阵 | S1 PASS | 自动化全套 + T1～T9 + fixture/EXE 哈希 | 2/7 |
| S3 | 执行 RI-03 Avalonia 矩阵 | S1 PASS，有交互式 Windows 会话 | Desktop 全套/build + A1～A6 | 3/7 |
| S4 | 执行 RI-04 托管 CI | 用户选定并授权平台 | 同一 SHA 的 Windows/Ubuntu job | 4/7 |
| S5 | 执行 RI-05 Rust 审计 | 可访问 advisory database | `cargo audit` + Rust 回归 | 5/7 |
| S6 | 最终总门禁和 RI-06 交付 | S1～S5 PASS/有批准例外 | 下列完整命令 + diff/敏感文件检查 | 6/7 |
| S7 | 执行 RI-07 备份处置 | 用户明确选择 | 路径/数量/哈希前后对比 | 7/7 |

S2、S3 在 S1 后可以并行；S4、S5 可以独立准备，但 S6 必须等待所有发布门禁结论。

### 最终总门禁

```powershell
dotnet test SessionAtlas.Tests\SessionAtlas.Tests.csproj --nologo
dotnet test SessionAtlas.Desktop.Tests\SessionAtlas.Desktop.Tests.csproj --nologo
dotnet build --nologo
dotnet build SessionAtlas.Desktop --nologo

Push-Location frontend
npm ci
npm run check
npm test
Pop-Location

Push-Location src-tauri
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo audit
cargo tauri build
Pop-Location

git diff --check
git status --short
```

检查实际数量至少不低于当前基线：C# 89、Desktop 7、frontend unit 16、browser 24、
Rust 54。新增测试后数量应上升；测试数量下降、跳过或 filter 命中 0 均视为失败。

## 5. 每步汇报模板

每完成或阻塞一个步骤，立即追加以下记录：

```text
步骤/问题 ID：
状态：TODO / IN PROGRESS / BLOCKED / PASS / FAIL
开始与结束时间：
执行平台与版本：
代码/构建 SHA：
本步改动文件：
执行命令或手工场景：
测试数量与结果：
原生证据路径：
真实数据根前后哈希：
发现的问题：
失败处置或回滚：
下一步：
需要用户决定的事项：
```

进度只按本文件的 7 项计算。例如 RI-01、RI-05 完成时报告“2/7”，不能把自动化测试
数量或历史 R0～R11 混入剩余项进度。

## 6. 整体完成定义

只有同时满足以下条件，才能宣布“剩余问题全部关闭”：

1. RI-01～RI-07 均为 PASS；任何例外都有维护者、原因、影响和后续期限。
2. T1～T9、A1～A6 全部有可审计记录，不能用 headless 测试或启动截图替代交互结果。
3. 同一 commit 的 Windows/Ubuntu 托管 CI 均通过。
4. 最终总门禁无失败、无跳过、无新增 warning，测试数量不低于记录基线。
5. 原生测试未启动真实 AI CLI、未连接真实 SSH、未读取或改变真实索引和凭据。
6. Rust advisory scan 已执行并处置结果。
7. 工作树、提交和远端状态与用户授权一致；数据库备份处置有明确记录。

历史问题的设计理由和回滚边界仍保留在
[`remaining-work-repair-design.md`](./remaining-work-repair-design.md)，但后续执行、进度和
“还剩什么”的判断均以本文件为准。

## 7. SessionAtlas 全量身份迁移（2026-08-14，不计 RI 进度）

**状态：PASS。** 用户已明确批准取消旧标识兼容。当前唯一身份映射为：
`sessionatlas` CLI、`SessionAtlas.*` C# 项目/命名空间、`sessionatlas-tauri` crate、
`com.sessionatlas.console` Tauri 标识、`sessionatlas.*` localStorage 键、
`SESSIONATLAS_HOME` 和 `~/.sessionatlas/` 数据目录。代码不读取旧环境变量、旧持久化键或
旧数据目录，也不自动迁移仓库外数据。

已完成 C# 根项目、Desktop、两个测试项目的文件/目录重命名和 71 个源码/项目文件更新；
Tauri crate、lib target、扫描命令、SSH 探针、三条数据库路径和前端持久化键也已迁移。
定向结果为 C# Home/Config `11/11`、CLI correctness `13/13`、Rust 新路径契约 `5/5`、
Playwright smoke `5/5`；全套结果为 C# `89/89`、Desktop `7/7`、frontend unit `16/16`、
Playwright `24/24`、Rust `53/53`。两个 C# build、frontend syntax、Rust fmt、严格 clippy
和 Tauri release build 均通过。

release 生成 `sessionatlas-tauri.exe`、`SessionAtlas_0.1.0_x64_en-US.msi` 和
`SessionAtlas_0.1.0_x64-setup.exe`。概念图内的产品、项目、路径和终端包名已全部统一，
尺寸为 1586×992。排除构建缓存、依赖和二进制后，全仓旧项目标识静态审计为 0 命中。
测试前后只读快照一致，支持的数据根与前代数据根均保持不存在。

失败与回滚：一次 Playwright 定向命令使用 Windows 反斜杠后命中 0 项并退出失败，改用
正斜杠路径后 `5/5` 通过，未修改断言。Rust 因 crate 改名首次重新链接耗时约 3 分钟，
没有源码失败。两次旧缓存删除尝试被安全策略拒绝，均未产生删除；随后只把 5 个旧顶层
release 文件可恢复移动到 Codex quarantine。深层忽略编译缓存不参与源码、打包或发布审计。

RI-01 已从 `TODO` 推进为 `IN PROGRESS`；本轮自动化前后快照已相等，下一项仍是完成隔离
fixture 和原生 smoke，再取得该原生过程自己的前后哈希证据，之后才能解除 RI-02/RI-03 阻塞。
