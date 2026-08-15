# SessionAtlas C# CLI 全量迁移到 Rust 实施手册

> 状态：R00–R13 已完成；R14 本地可执行门禁已通过（托管依赖审计与手工/真机门禁未通过）  
> 制定日期：2026-08-15  
> 目标：在不改变用户数据、CLI 身份和扫描语义的前提下，用 Rust 替换全部 C# CLI/核心实现，并从开发、CI 和安装包中移除 .NET 依赖。

> 进度：R00–R13 均已实施并通过各自验证，C# 源码/测试已删除，文档已切换为
> 当前全部 Rust 架构。R14 的格式/lint/测试/前端/安装包/隔离验收自动化门禁已于
> 2026-08-15 在本机执行通过（详见 R14 小节）。第 10 节的手工/真机验收（无运行时
> 沙箱安装、真实用户扫描、跨平台真实终端、原生 UI T1–T9、托管 CI）仍保持未通过。
> 本文档是唯一允许保留显式 C# 参考的迁移历史文档。

## 1. 完成定义

只有同时满足以下条件，迁移才算完成：

1. `sessionatlas scan/list/search/open/recent/config` 仍可用，公开命令名、关键参数和退出码语义不变。
2. Claude Code、Codex、Kimi、OpenCode、Aider 和自定义工具的扫描行为符合 [`scan-contract.md`](./scan-contract.md)。
3. 现有 `~/.sessionatlas/index.db` 和 `config.json` 可直接继续使用，不要求用户删库或手工迁移。
4. Tauri 直接调用 Rust 扫描核心，不再启动 C# sidecar。
5. 仓库中不再存在 C# 源码、`.csproj`、.NET 测试、`dotnet` 构建步骤或 C# sidecar 准备脚本。
6. Rust 测试覆盖现有 89 个 C# 用例所表达的行为；不要求测试数量恰好相等，但契约不得减少。
7. Windows 安装包可以在未安装 .NET Runtime/SDK 的隔离环境中完成首次扫描。
8. 格式、lint、Rust 全量测试、前端测试、隔离验收和安装包构建全部通过。

## 2. 不在本次范围

- 不更改 `~/.sessionatlas/` 身份、文件名或环境变量 `SESSIONATLAS_HOME`。
- 不将 `index.db` 与 Tauri 自有的 `prefs.db` 合并。
- 不重写前端，不调整 UI 交互，不增加新工具格式。
- 不修改远程 SSH 项目的数据模型。
- 不为了迁移而修改已公开数据的语义。
- 不提交、推送、切换分支或创建 PR，除非用户另行授权。

## 3. 不可破坏的契约

### 3.1 数据与路径

- 项目唯一身份是归一化后的绝对路径。
- Windows 路径大小写不敏感，Linux 大小写敏感；根路径不得归一化成空字符串。
- 重复扫描相同输入时，项目 ID、`first_seen_at`、会话计数和 FTS 内容保持稳定。
- `session_count` 是工具原生 session ID 去重后的数量，不是扫描次数。
- 只有 `Succeeded` 扫描结果可以替换对应工具快照。`Failed` 和 `Unavailable` 必须保留旧数据。
- 快照、孤儿项目清理、活动时间重算和 FTS 重建必须在同一 SQLite 事务内完成。

### 3.2 隐私与执行

- 扫描器只取得路径、时间、会话 ID 和必要的 Git 元数据；不持久化提示词、消息、密钥或认证内容。
- 测试和验收必须显式设置临时 `SESSIONATLAS_HOME`，不得读写真实用户数据。
- 外部进程使用“可执行文件 + 参数数组 + 工作目录”模型。只有互动终端或 SSH 固有需要时才构造 shell 文本。
- 工具 key、session ID、自定义命令和展示文本必须继续遵守 [`execution-security-contract.md`](./execution-security-contract.md)。

### 3.3 数据库兼容

Rust 必须继续支持以下表与 FTS5 虚拟表：

- `projects`
- `tool_usages`
- `sessions`
- `projects_fts`

`prefs.db` 仍由 Tauri 管理。本地扫描不得修改其中的分组、排序、opener 和远程项目数据。

## 4. 目标架构

```text
Cargo workspace
├─ crates/sessionatlas-core
│  ├─ model / path
│  ├─ scanner
│  ├─ indexer / store
│  ├─ config
│  └─ launch / process security
├─ crates/sessionatlas-cli
│  └─ sessionatlas 可执行文件
└─ src-tauri
   └─ 直接依赖 sessionatlas-core
```

核心 crate 不能依赖 Tauri、前端或 CLI 显示库。CLI 和 Tauri 只做输入输出适配。

## 5. OpenCode 执行规则

1. 一次 `opencode run` 只能接收一个任务 ID。
2. 提示中必须写明：目标、允许修改的文件、禁止范围、验证命令和通过条件。
3. 不同 OpenCode 实例不得修改同一文件。共享 `Cargo.toml`、`Cargo.lock`、`mod.rs`、`lib.rs`、CI 和文档的任务必须串行。
4. 并行上限为 3；仅本文档标记“可并行”的任务可以同时运行。
5. OpenCode 不得切换分支、提交、推送、删除用户数据或读取真实 `~/.sessionatlas/`。
6. OpenCode 结束时必须返回：修改文件、实现摘要、实际执行的验证、结果和剩余风险。
7. Codex 在每个任务后检查实际 `git diff`，然后自行重跑验证；OpenCode 的文字报告不等于验收。
8. 任务失败时不进入依赖它的下一任务。

OpenCode 通用提示模板：

```text
执行 docs/rust-migration-plan.md 中的单一任务 <TASK-ID>。
只修改任务列出的允许文件，保留当前工作树其他改动。
不提交、不推送、不切分支，不读写真实 ~/.sessionatlas/。
完成代码和本任务验证后，返回修改文件、验证命令、结果和剩余风险。
```

## 6. 任务依赖与并行波次

```text
R00 → R01 → R02 ─┬─→ R04 → [R05A, R05B, R05C] → R06 → R07
                  └─→ R03 ──────────────────┘          │
                                                                    ├→ R08 → R09
                                                                    └→ R10
R09 + R10 → R11 → R12 → R13 → R14
```

- 并行波次 A：R03 与 R04 只有在 R01 已经创建完整模块声明、且两者文件不重叠时才能并行。
- 并行波次 B：R05A、R05B、R05C 可三路并行。
- 其他任务默认串行。

## 7. 分任务实施与验收

### R00：修复 Avalonia 删除后的基线 CI

**目标**：移除 CI 中已删除 `SessionAtlas.Desktop*` 的测试和构建步骤，使 Rust 迁移从可验证基线开始。

**允许修改**：`.github/workflows/ci.yml`  
**禁止**：其他文件。

**验证**：

```powershell
rg -n "SessionAtlas\.Desktop|Avalonia|legacy desktop" .github/workflows/ci.yml
dotnet test SessionAtlas.Tests/SessionAtlas.Tests.csproj --configuration Release --nologo
```

**通过条件**：第一条无命中；.NET 基线测试全部通过。

### R01：建立 Rust workspace 和模块骨架

**目标**：建立 `sessionatlas-core`、`sessionatlas-cli` 和现有 Tauri crate 的 workspace；仅提供可编译骨架，不切换生产扫描。

**允许修改**：根 `Cargo.toml`、根 `Cargo.lock`、`.gitignore`中的根 `target/` 规则、`src-tauri/Cargo.toml`、删除或迁移 `src-tauri/Cargo.lock`、`crates/sessionatlas-core/**`、`crates/sessionatlas-cli/**`。  
**禁止**：`src-tauri/src/**`、C# 源码、前端、CI。

**验证**：

```powershell
cargo metadata --format-version 1 --no-deps
cargo check --workspace
cargo test --workspace --no-fail-fast
```

**通过条件**：三个 package 都出现在 metadata 中；workspace 可编译和测试。

### R02：迁移数据模型和路径语义

**目标**：迁移 `Project`、`ToolUsage`、`Session`、`ToolSource`、路径归一化、根路径显示和父子关系判定。

**允许修改**：`crates/sessionatlas-core/src/model.rs`、`path.rs`、对应独立测试文件。  
**禁止**：`lib.rs`、`Cargo.toml`、其他模块。

**必须覆盖**：Windows/POSIX 根路径、`.`/`..`、非绝对路径拒绝、大小写规则、同路径/子路径、显示名不为空。

**验证**：

```powershell
cargo test -p sessionatlas-core path -- --nocapture
cargo clippy -p sessionatlas-core --all-targets -- -D warnings
```

### R03：迁移配置读写

**目标**：兼容现有 `config.json`，实现大小写不敏感读取、跨进程限时锁、指纹冲突检测、原子替换和严格的旧临时文件清理。

**允许修改**：`crates/sessionatlas-core/src/config.rs`、对应独立测试。  
**禁止**：其他文件。

**验证**：

```powershell
cargo test -p sessionatlas-core config -- --nocapture
```

**通过条件**：至少覆盖空配置、无效 JSON、并发更新、忙锁超时、过期对象冲突、替换失败保留旧文件、不删除不匹配临时文件。

### R04：扫描器框架、诊断和共享解析

**目标**：建立 `Scanner`、`ScanOutcome`、`ScanStatus`、`ScanDiagnostic`、时间解析、安全绝对路径验证、可用性与数据可发现性分离。

**允许修改**：`crates/sessionatlas-core/src/scanner/mod.rs`、`scanner/parsing.rs`、`scanner/base.rs`、对应测试。  
**禁止**：具体工具扫描器文件、其他模块。

**验证**：

```powershell
cargo test -p sessionatlas-core scanner:: -- --nocapture
```

### R05A：迁移 Codex 和 Claude Code 扫描器（可并行）

**允许修改**：`scanner/codex.rs`、`scanner/claude.rs`、两者专属 fixture/测试。  
**必须覆盖**：递归 JSONL、最小字段提取、坏行容错、时间回退、Codex 缺失 session ID 失败、Claude 文件名 ID 回退。

```powershell
cargo test -p sessionatlas-core codex -- --nocapture
cargo test -p sessionatlas-core claude -- --nocapture
```

### R05B：迁移 Kimi 和 OpenCode 扫描器（可并行）

**允许修改**：`scanner/kimi.rs`、`scanner/opencode.rs`、两者专属 fixture/测试。  
**必须覆盖**：Kimi `state.json` 形状和时间回退；OpenCode 数据库只读打开、备选路径、schema 失配不得冒充空成功。

```powershell
cargo test -p sessionatlas-core kimi -- --nocapture
cargo test -p sessionatlas-core opencode_scanner -- --nocapture
```

### R05C：迁移 Aider 和自定义工具扫描器（可并行）

**允许修改**：`scanner/aider.rs`、`scanner/custom.rs`、两者专属 fixture/测试。  
**必须覆盖**：Aider 只检查 `.aider.chat.history` 元数据、不读取内容；自定义 `metadata.json` 的 `project_path/cwd/session_id`、`~` 展开和坏 JSON 降级。

```powershell
cargo test -p sessionatlas-core aider -- --nocapture
cargo test -p sessionatlas-core custom -- --nocapture
```

### R06：迁移项目索引器

**目标**：按原生路径语义合并扫描结果，对同一 `(project, tool)` 的 session ID 去重，保留最新 session ID，读取 Git 分支与 remote。

**允许修改**：`crates/sessionatlas-core/src/indexer.rs`、对应测试。

```powershell
cargo test -p sessionatlas-core indexer -- --nocapture
```

**通过条件**：覆盖跨工具合并、同 ID 去重、无原生 ID 计数为 0、最新时间/ID 一致。

### R07：迁移 SQLite 唯一写入器

**目标**：用 `rusqlite` 实现建表、旧数据迁移、路径身份、快照替换、FTS5、列表/搜索/精确查找和会话记录。

**允许修改**：`crates/sessionatlas-core/src/store.rs`、对应 SQLite 测试。  
**禁止**：Tauri 现有 `prefs.db` 实现。

```powershell
cargo test -p sessionatlas-core store -- --nocapture
```

**通过条件**：覆盖重复快照幂等、部分扫描、空成功清理、失败回滚、旧重复 usage 合并、大小写路径、FTS 特殊符号、根路径、隔离 DB 不创建其他文件。

### R08：实现 Rust CLI 的只读命令

**目标**：实现 `list`、`search`、`recent`、无参数默认交互列表、数量参数验证和终端输出安全。

**允许修改**：`crates/sessionatlas-cli/src/**`，不包含 `open`/`scan`/`config` 的实现。

```powershell
cargo test -p sessionatlas-cli
cargo run -p sessionatlas-cli -- --help
cargo run -p sessionatlas-cli -- list --limit 0
```

**通过条件**：测试通过；帮助列出公开命令；非法 limit 非零退出。

### R09：实现 Rust CLI 的扫描和配置命令

**目标**：实现 `scan [--tool]`、`config show/add-tool/set-default-terminal`，将扫描结果仅交给 R07 快照事务。

**允许修改**：`crates/sessionatlas-cli/src/**`和 CLI 专属测试。

```powershell
$oldHome=$env:SESSIONATLAS_HOME
$env:SESSIONATLAS_HOME=Join-Path $env:TEMP ("sessionatlas-rust-scan-"+[guid]::NewGuid().ToString("N"))
try {
  cargo run -p sessionatlas-cli -- scan --tool codex
  cargo run -p sessionatlas-cli -- config show
} finally { $env:SESSIONATLAS_HOME=$oldHome }
cargo test -p sessionatlas-cli
```

**通过条件**：命令只写入临时 home；未指定工具、未知工具、成功空扫描和扫描失败的退出/保留语义有测试。

### R10：迁移进程安全、终端启动和 `open`

**目标**：实现可注入 process runner、安全命令解析、工具/session ID 验证、可执行文件解析、跨平台终端启动、`open --recent`、模糊项目匹配和会话记录。

**允许修改**：`crates/sessionatlas-core/src/process.rs`、`security.rs`、`launcher.rs`及测试；`crates/sessionatlas-cli/src/**` 中仅 `open` 相关代码。

```powershell
cargo test -p sessionatlas-core security -- --nocapture
cargo test -p sessionatlas-core launcher -- --nocapture
cargo test -p sessionatlas-cli open -- --nocapture
```

**通过条件**：测试不启动真实终端或 AI CLI；完整断言 program/argument/working-directory；拒绝 shell 元字符、控制字符、未知工具和选项形 tool key。

### R11：Tauri 切换为进程内 Rust 扫描

**目标**：`scan_projects` 通过 `spawn_blocking` 调用 `sessionatlas-core`，保留 Tauri 命令名和返回值，删除 C# sidecar 调用代码。

**允许修改**：`src-tauri/Cargo.toml`、`src-tauri/src/lib.rs`、Tauri 专属测试、workspace lockfile。  
**禁止**：前端 invoke 名、`prefs.db` schema、远程扫描。

```powershell
cargo fmt --all -- --check
cargo clippy -p sessionatlas-tauri --all-targets -- -D warnings
cargo test -p sessionatlas-tauri
```

**通过条件**：`scan_projects` 不存在 sidecar/process spawn；扫描不阻塞 Tauri async 执行线程；索引不存在时可创建；旧索引扫描后可继续查询。

### R12：移除 .NET sidecar 构建和更新发布链

**目标**：移除 `prepare-sidecar.mjs`、`externalBin`、`setup-dotnet`、NuGet 审计和 sidecar 冒烟路径；安装包仍执行 Rust 扫描验收。

**允许修改**：`frontend/package.json`、`frontend/scripts/prepare-sidecar.mjs`（删除）、`src-tauri/tauri.conf.json`、`.github/workflows/*.yml`、`.github/dependabot.yml`（移除 NuGet 生态、cargo 目录改为 workspace 根）、`scripts/New-AcceptanceFixture.ps1`、依赖审计脚本，以及删除 `src-tauri/binaries/` 下被忽略的过期 sidecar 产物（`sessionatlas-x86_64-pc-windows-msvc.exe`，gitignore 已忽略）。

```powershell
rg -n -i "dotnet|\.csproj|prepare:sidecar|externalBin|binaries/sessionatlas|bundled CLI" .github frontend src-tauri scripts -g "!**/gen/**"
npm --prefix frontend run check
cargo test --workspace
```

**通过条件**：首条对生产/构建链无命中（迁移历史文档可保留明确的过去时描述）；前端和 Rust 测试通过。

### R13：删除 C# 代码并更新用户/开发文档

**目标**：删除 `Program.cs`、`CLI/`、`Core/`、`Models/`、`SessionAtlas.csproj`、`SessionAtlas.Tests/`，将 README、AGENTS、CONTRIBUTING、SUPPORT、DESIGN 和测试/安全文档更新为 Rust 架构。

**前置门槛**：R02–R12 全部通过，Rust 契约测试已替代 C# 测试。

**状态**：已完成（2026-08-15）。C# 源码/测试已删除；README、AGENTS、CLAUDE、
CONTRIBUTING、SUPPORT、DESIGN 与 docs/ 下执行/测试/安全/发布文档已更新为全部
Rust 架构；issue/PR 模板已移除 C#/.NET/Avalonia 选项；`src-tauri/src/lib.rs`
只改动测试注释中的过期声明，无功能代码改动。本任务不执行 R14 打包或实时 UI 验收。

```powershell
rg --files -g "*.cs" -g "*.csproj"
rg -n -i "C# CLI|dotnet|\.csproj|Microsoft\.Data\.Sqlite|Spectre\.Console|Avalonia" . -g "!docs/rust-migration-plan.md" -g "!.git/**" -g "!**/target/**" -g "!frontend/vendor/**" -g "!frontend/dist/**"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
npm --prefix frontend run check
git diff --check
```

**通过条件**：不再有 C# 文件或现行 C# 构建说明；上述两条 rg 均无现行命中；
Rust 全量测试、前端检查与 diff 检查通过。

### R14：全量发布验收

**目标**：从干净依赖视角验证格式、lint、测试、前端、隔离扫描、Tauri 安装包和依赖安全。

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

**通过条件**：

- 所有自动化命令退出码为 0。
- 隔离 home 生成 `index.db`，且同时包含两个合成 Codex 项目、正确 session ID 和 UTC 时间。
- MSI 和 NSIS 安装包均生成。
- `git diff --check` 无空白错误。
- `git status` 只包含本次 Avalonia 清理和 Rust 迁移的预期改动。

**R14 本机执行结果（2026-08-15，Windows x64）**：

| 门禁 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 退出 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 退出 0 |
| `cargo test --workspace --no-fail-fast` | 退出 0，394/394 通过，0 失败/忽略 |
| `npm --prefix frontend run check` | 退出 0 |
| `npm --prefix frontend test` | 退出 0（16 单元 + 24 Playwright 浏览器测试通过） |
| `cargo tauri build --ci` | 退出 0，生成 MSI 与 NSIS 安装包 |
| `cargo build --locked -p sessionatlas-cli --release` | 退出 0，生成 `target/release/sessionatlas.exe` |
| `scripts/New-AcceptanceFixture.ps1 -ScannerPath <release CLI>` | 退出 0（隔离 home：`index.db` 86016 字节且持久化两个会话 ID、SQLite sidecar 为 0、`list` 2 个项目、`search` 命中两个 UTC 时间；manifest schemaVersion=2 记录两个项目/会话 ID/UTC 时间） |
| `git diff --check` | 无空白错误 |
| `git status --short` | 仅含预期的 Rust 迁移与验收改动 |

最终干净重打包产物（当前工作树，2026-08-15）：

| 产物 | 字节 | SHA-256 |
| --- | ---: | --- |
| `target/release/sessionatlas.exe` | 2,662,912 | `a6f78709f97cd7efeda0973e826e41f0a071a255dae9aefa16895d54adb714fb` |
| `target/release/bundle/msi/SessionAtlas_0.1.0_x64_en-US.msi` | 4,304,896 | `cd53a5fe9862a10f351928de76818b3055df614b249ac906340c138743b89f03` |
| `target/release/bundle/nsis/SessionAtlas_0.1.0_x64-setup.exe` | 3,128,120 | `1506ccc28d6b20fa08deee922c0a0753f822b999afe6ff274e6830c4202c08f8` |

**本地未执行的 R14 门禁（仍为发布门禁）**：

- 本机未重跑 `cargo audit`（本机未安装 cargo-audit，按任务约定不新增工具）；托管
  Security workflow 固定安装 `cargo-audit 0.22.2` 并执行 `cargo audit`，仍是发布门禁。
- 托管 CI / 托管 Security workflow 未在本机之外运行；Windows/Ubuntu 托管 job 证据
  需在同一 commit 的托管执行器上产生。
- 第 10 节全部手工/真机验收项仍保持未通过。

## 8. 每任务复核清单

Codex 在任务转为“已完成”前必须检查：

- [ ] OpenCode 只修改了允许文件。
- [ ] 没有覆盖任务开始前的用户/既有改动。
- [ ] 错误不是被吞掉，失败状态不冒充空成功。
- [ ] 新测试使用合成数据和临时 home/DB。
- [ ] 没有读取或写入真实 `~/.sessionatlas/`。
- [ ] 本任务命令由 Codex 重跑并通过。
- [ ] `git diff --check` 通过。
- [ ] 文档中的验收命令仍与实际结构一致。

## 9. 中止与回退点

- R00–R10 期间 C# 仍是生产扫描器；Rust 失败可直接修正或放弃新 crate，不影响当前应用。
- R11 是第一个生产切换点。如果隔离扫描或 Tauri 查询回归，恢复 `scan_projects` sidecar 路径，不继续 R12。
- R12 后构建链不再产生 C# sidecar；在 R14 完成前不删除 C# 源码。
- R13 仅在 Rust 替代测试已完整通过后才执行。
- `index.db` 是派生索引，但不能以“可重建”为理由在迁移中自动删除用户的数据文件。

## 10. 手工/真机验收门槛

以下项目无法仅靠合成测试完成，发布前需要在用户授权的真实环境单独验收：

1. 安装包在没有 .NET Runtime/SDK 的 Windows 沙箱中安装和首次扫描。
2. 对用户实际安装的 Claude/Codex/Kimi/OpenCode/Aider 数据目录做只读扫描，核对项目数和最近会话。
3. `sessionatlas open` 在 Windows Terminal/cmd、macOS Terminal 和至少一个 Linux 终端中的真实启动。
4. Tauri 点击“重新扫描”时界面不卡死，扫描完成后项目、工具标签和搜索结果刷新。
5. 使用一份迁移前的 `index.db` 备份做兼容性验收，对比扫描前后逻辑数据，不在原文件上直接试验。
