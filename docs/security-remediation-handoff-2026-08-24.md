# SessionAtlas 开源安全修复交接（2026-08-24）

本文最初用于交接 2026-08-24 未完成的安全修复；2026-08-26 已按“一次只处理一项”的要求完成 S1–S7。每项均由 Luna 子代理实现，Codex 检查实际 `git diff`、修正复核问题并独立运行针对性验证，最后统一执行完整本地门禁。

## 1. 当前边界与仓库状态

- 分支：`codex/open-source-readiness`
- 当前基线提交：`ad08fd7a9e94d41e74230c70db38b4f8d8652730`
- 本轮没有 commit、push、merge、修改仓库可见性或调用 GitHub Actions。
- 用户已明确要求不再调用 GitHub Actions；后续只执行本地验证，并如实保留 Linux、macOS 和 CodeQL 未托管验证的边界。
- 工作区中的未提交修改属于本轮安全修复，不得重置、覆盖或与无关改动混合。
- S1–S7 均已完成，当前工作区保留全部未提交修复、回归测试和本文档更新。

当前应有修改文件：

```text
crates/sessionatlas-core/src/adapter.rs
crates/sessionatlas-core/src/process.rs
crates/sessionatlas-core/src/scanner/aider.rs
crates/sessionatlas-core/src/scanner/base.rs
crates/sessionatlas-core/src/scanner/cache.rs
crates/sessionatlas-core/src/scanner/claude.rs
crates/sessionatlas-core/src/scanner/codex.rs
crates/sessionatlas-core/src/scanner/custom.rs
crates/sessionatlas-core/src/scanner/kimi.rs
crates/sessionatlas-core/src/scanner/mod.rs
crates/sessionatlas-core/src/scanner/opencode.rs
crates/sessionatlas-core/src/scanner/pi.rs
crates/sessionatlas-core/tests/scanner_resource_limits.rs
crates/sessionatlas-core/Cargo.toml
Cargo.lock
frontend/app.js
frontend/tests/browser/search-safety.spec.js
src-tauri/src/lib.rs
src-tauri/src/process.rs
src-tauri/src/security.rs
src-tauri/src/session_cleanup.rs
src-tauri/src/tui_tools.rs
docs/security-remediation-handoff-2026-08-24.md
```

## 2. 今天已完成并验收的任务

### S1：隔离被动 Git 操作中的仓库可执行配置 — PASS

实现结果：

- 被动 Git status、元数据读取和后台 fetch 使用单独的安全 `ProcessSpec`。
- 禁用仓库控制的 fsmonitor、hooks、credential helper、askpass、自定义/ext transport 和子模块递归；后台 fetch 只接受 HTTPS、SSH URI 和 SSH scp 风格地址。
- fetch 后重新读取 status，返回更新后的 ahead/behind，而不是 fetch 前快照。
- `git init`、`remote add`、`switch` 等明确用户操作使用独立构造器，保留正常 hook 和 Git 行为。

已验证：

- Luna：Tauri 112 项测试、Tauri Clippy、fmt、`git diff --check` 通过。
- Codex：`git_sync_tests` 9/9、`process::tests` 6/6 通过，并复核所有 `git_read_spec` / `git_user_operation_spec` 调用点。
- 临时仓库复现：普通 `git status` 会触发恶意 fsmonitor marker；安全参数下 marker 不生成；ext transport 被拒绝。

### S2：限制导入 Adapter 的可执行命令和自动探测 — PASS

实现结果：

- adapter `command` 只能是单个、无路径分隔符/盘符/空白的裸 PATH 命令名。
- Windows/Unix 绝对路径、UNC、相对路径和多组件路径均被拒绝。
- 本地与远程探测再次执行裸命令校验。
- 默认只探测 bundled adapter；扩展 adapter 必须明确启用后才允许探测。
- enable/install 的探测失败会恢复调用前的启用状态；远程偏好错误向上传播。

已验证：

- Luna：core 43 项、Tauri 116 项测试，两个 crate 的 Clippy、fmt、`git diff --check` 通过。
- Codex：manifest 路径拒绝 1/1、裸命令规则 2/2、adapter 选择 1/1、回滚 1/1、本地/远程拒绝探测 2/2 通过。

### S3：统一 session ID 校验并阻止参数注入 — PASS

实现结果：

- Tauri 复用 `sessionatlas-core` 的 session ID 校验规则。
- 以 ASCII `-` 开头、控制字符、Unicode 连字符形似字符和非法空白输入均不能进入 legacy `--resume` argv。
- 本地 PTY 和远程 tmux/SSH 共用同一校验边界；合法 provider ID 保持兼容。

已验证：

- Luna：core 43 项、Tauri 118 项测试，两个 crate 的 Clippy、fmt、`git diff --check` 通过。
- Codex：session ID 恶意语料 1/1、legacy 本地/远程阻断 1/1 通过，并确认 core 首字符规则与 Tauri 错误语义。

注意：S1–S7 合并后的完整 workspace 门禁已按第 4 节顺序统一重跑。

## 3. 已按顺序完成的任务

### S4：远程 SSH 扫描超时、输出和记录预算 — PASS

允许范围：`src-tauri/src/process.rs`、`src-tauri/src/lib.rs` 及紧邻测试。

安全不变量：

- 远程 home、项目和 session/tool discovery 都必须有专用硬超时。
- stdout、stderr 超限必须失败，不能把静默截断的数据当成完整协议。
- NUL/行协议解析后的记录数必须有合理上限。
- 超时、超限、记录不完整或解析失败时，上一份快照保持不变。

必须验证：

```powershell
cargo test -p sessionatlas-tauri process::tests -- --nocapture
cargo test -p sessionatlas-tauri remote_scan -- --nocapture
cargo fmt --all -- --check
cargo clippy -p sessionatlas-tauri --all-targets -- -D warnings
git diff --check
```

通过条件：永不退出的子进程被终止；stdout 和 stderr 分别超限时失败；边界内完整输出通过；记录数超限在写数据库前失败；旧快照保留测试通过。

实现与验收结果：远程 discovery 使用专用硬超时和 stdout/stderr 上限，解析记录数受限；超时、超限和协议失败均不会覆盖旧快照。针对性 process/remote scan 测试、Tauri Clippy、fmt 和 diff 检查通过。

### S5：为本地会话扫描增加共享资源预算 — PASS

允许范围：`crates/sessionatlas-core/src/scanner/`、必要的 scanner 测试；只有共享 API 确有需要时才修改相邻 core 文件。

安全不变量：

- 统一限制递归深度、文件数、单文件字节数、总字节数、单行长度、记录数和扫描持续时间/取消状态。
- 超限输入必须产生结构化诊断；不得静默返回可信空快照。
- 仍保持 symlink/junction 不跟随、成功扫描原子替换、失败保留旧索引。

必须覆盖：文件数超限、超大单文件、超长单行、许多小记录、总字节超限、正常 Claude/Codex/Kimi/OpenCode/Aider/Pi/metadata-v1 样本。

```powershell
cargo test -p sessionatlas-core scanner -- --nocapture
cargo test -p sessionatlas-cli --no-fail-fast
cargo fmt --all -- --check
cargo clippy -p sessionatlas-core --all-targets -- -D warnings
git diff --check
```

通过条件：所有恶意 fixture 在预算内结束并给出诊断；正常 fixture 结果不变；首次扫描超限不会生成误导性的空 `index.db`。

实现与验收结果：共享扫描预算覆盖递归深度、文件数、单文件/总字节数、单行长度、记录数与时限/取消；超限产生结构化诊断。新增 8 项资源上限回归测试，六个官方扫描器和 metadata-v1 正常样本保持通过。

### S6：会话清理顶层符号链接和移动前包含关系 — PASS

允许范围：`src-tauri/src/session_cleanup.rs` 及紧邻测试。

安全不变量：

- 用 `symlink_metadata` 拒绝顶层 symlink、junction/reparse point。
- 规范化批准根与候选路径，要求候选是根本身或其子路径。
- preview 后、实际 quarantine/restore 前再次校验，阻止 TOCTOU 目录替换。
- 正常目录的预览、回滚、trash 和 restore 行为不变。

```powershell
cargo test -p sessionatlas-tauri session_cleanup -- --nocapture
cargo fmt --all -- --check
cargo clippy -p sessionatlas-tauri --all-targets -- -D warnings
git diff --check
```

通过条件：Unix symlink 和 Windows junction/reparse fixture 均不能进入候选；preview 后替换根目录不能移动文件；正常 quarantine/restore 通过。

实现与验收结果：顶层和嵌套 symlink/junction/reparse point 被拒绝，quarantine/restore 在移动前重新验证规范化端点，manifest 的批次 ID、包含关系和端点冲突均被校验；Windows junction 与正常回滚/恢复测试 9/9 通过。Windows 本地无法执行 `cfg(unix)` fixture；完全消除检查与 `rename` 间的同用户微小竞态仍需平台原生 handle-relative API。

### S7：项目 ID 的 HTML 属性和 CSS selector 边界 — PASS

允许范围：`frontend/app.js`、相关前端测试；只有建立统一 ID 契约确有必要时才修改 `src-tauri/src/lib.rs`。

安全不变量：

- 所有进入 HTML 属性的项目 ID 必须进行属性编码，或改用 DOM `dataset` 赋值。
- 所有 CSS selector 中的 ID 必须统一使用 `CSS.escape`。
- Rust/JS 边界应验证本地 UUID/hash 与远程合成 ID 的合法格式，不得破坏已有索引兼容性。

```powershell
npm --prefix frontend run check
npm --prefix frontend run test:unit
npm --prefix frontend run test:browser
git diff --check
```

通过条件：包含引号、尖括号、方括号、反斜杠和 CSS 元字符的 ID 只能作为惰性数据；不会生成额外 DOM、selector 异常或错误选中；正常本地/远程项目选择和键盘导航通过。

实现与验收结果：HTML 属性统一编码，selector 统一使用 `CSS.escape`，项目 ID 映射改为无原型对象。浏览器测试使用可执行属性注入 PoC、CSS 元字符和 `__proto__` ID 验证 DOM、键盘导航、分组、opener 与托盘载荷；55/55 通过。

## 4. 全部修复后的完整本地门禁

顺序不可调整：Tauri Rust 检查前必须先生成 `frontend/dist`。

```powershell
npm --prefix frontend run build:static
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
npm --prefix frontend run check
npm --prefix frontend run test:unit
npm --prefix frontend run test:browser
npm --prefix frontend audit
cargo audit
cargo tauri build --ci
cargo build --locked -p sessionatlas-cli --release
git diff --check
git status --short --branch
```

2026-08-26 执行结果：上述命令依次通过。Workspace Rust 测试、前端单元测试 22/22、浏览器测试 55/55、npm audit（0 vulnerability）、Tauri MSI/NSIS 构建和 CLI locked release 构建均成功；`git diff --check` 无错误。

`cargo audit` 扫描 538 个依赖并以成功状态结束，但报告 17 个允许告警，主要来自 Tauri/GTK 传递依赖中的 unmaintained crate，以及 `glib 0.18.5` 的 unsound 告警。Tauri 成功构建时另有 `__TAURI_BUNDLE_TYPE` 未找到的 updater patch 告警；两项都不是本轮门禁失败，但需在发布依赖升级/自动更新验收中继续跟踪。

最终还要逐项复核：

1. 所有 7 个原安全触发均不能复现，合法控制样本保持有效。
2. `git diff` 只包含 S1–S7 的最小修复、回归测试和本文档。
3. 当前代码及所有 Git refs 再做一次高置信敏感信息扫描。
4. 不把本地通过描述为 Linux、macOS、CodeQL 或真实 SSH/安装器人工验收通过。
5. 不自动 commit、push、merge、删除远端分支或将仓库设为 Public。

## 5. 安全修复之后仍需用户决策的开源门禁

- 2026-08-26 通过 GitHub 只读接口核实 7 个当前远端分支，分支树中未发现 `.cs` 文件；旧交接记录中的“8 个远端旧分支含 C#”已不再成立。历史提交的高置信密钥格式扫描也为 0，但本机未安装专用 secret scanner，本次采用保守规则扫描。
- Draft PR #18 当前为 Open + Draft，GitHub 返回 `CONFLICTING`；需要先同步目标分支、解决冲突，再做最终 PR 审核/合并。
- 原生 Tauri T1–T9、干净环境安装、首次扫描和多平台真实终端仍是发布安装包前的人工门禁。
- 仓库改为 Public、提交、推送、合并和历史改写都需要单独授权。
