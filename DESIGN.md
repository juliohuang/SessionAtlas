# SessionAtlas 设计说明（当前 Rust 架构）

> 本文描述当前全部基于 Rust 的实现：共享核心库 `sessionatlas-core`、Rust CLI
> `sessionatlas`、Tauri 桌面控制台。早期实现的迁移历史记录在
> [`docs/rust-migration-plan.md`](./docs/rust-migration-plan.md)。

## 一、定位与核心模型

**一句话定位**：扫描本机多个 AI CLI（Claude Code、Codex、Kimi、OpenCode、Aider、Pi Coding Agent）
留下的工作目录，按规范化路径去重，建立统一 SQLite 索引，并在桌面控制台里一键续接
AI 会话。

**核心实体**（`crates/sessionatlas-core/src/model.rs`）：

- `Project`：一个工作目录，唯一身份是归一化后的绝对路径。
- `ToolUsage`：某个工具在该项目上的使用记录，含 `session_count`、`last_used_at`、
  `last_session_id`。
- `Session`：一次工具原生 session ID 的记录。
- `ToolSource`：工具来源（内置 key 或自定义工具）。

**身份契约**：唯一支持的身份是 `sessionatlas` CLI、`SessionAtlas` 产品标识、
Tauri crate `sessionatlas-tauri`、标识 `com.sessionatlas.console`、
`sessionatlas.*` localStorage 键、`SESSIONATLAS_HOME` 与 `~/.sessionatlas/`。
不提供旧别名回退，也不自动迁移旧数据根。

## 二、工作区结构

```text
Cargo workspace
├─ crates/sessionatlas-core   # 共享库：model / path / scanner / indexer /
│                             # store / config / process / security / launcher
├─ crates/sessionatlas-cli    # `sessionatlas` 可执行文件（clap 命令）
└─ src-tauri                  # Tauri 2 桌面应用，依赖 sessionatlas-core
```

- 核心 crate 不依赖 Tauri、前端或 CLI 显示库。
- CLI 与 Tauri 都是同一核心的输入输出适配层；两者链接同一个 `sessionatlas-core`。
- 数据目录统一为 `~/.sessionatlas/`：`index.db`（CLI 所有、Tauri 只读）、
  `config.json`、`prefs.db`（Tauri 所有：分组、打开器、远程项目和项目忽略规则）。

## 三、扫描与索引数据流

```text
各 AI 工具数据目录 → scanner/（每工具一个 Scanner + custom）→ ScanOutcome
→ indexer（规范化路径去重/合并，读 .git/HEAD 取分支）
→ store（单 SQLite 事务快照替换 + 项目 FTS5 重建）
→ content_index（限额遍历 + mtime/size 增量判定 + 内容 FTS5/压缩摘要）
```

- `ScanOutcome` 区分可信的空快照（`Succeeded`）、不可用（`Unavailable`）与失败
  （`Failed`）；只有 `Succeeded` 可以替换对应工具快照，其余保留旧数据。
- 路径语义在 `path.rs`：Windows 大小写不敏感、Unix 字节敏感；根路径不归一化为空。
- `Project.path_missing` 不写入数据库，而是在索引构建和每次读取时根据目录现状计算；
  因此扫描后目录被删除也会立即显示缺失，权限或瞬时 I/O 错误不会被误报为缺失。
- 快照替换、孤儿项目清理、活动时间重算与 FTS 重建在同一事务内原子完成。
- `prefs.db` 由 Tauri 管理，本地扫描不改其中的分组/排序/打开器/远程项目/忽略规则数据。

## 四、SQLite 存储

`index.db` 表：`projects`、`tool_usages`、`sessions`、`project_content_files`、
`project_content_status`，以及 FTS5 `projects_fts`、contentless
`project_content_fts`。正文原文不写入普通表；FTS 只保留词项，结果字幕来自
LZ4 压缩的 32 KiB 预览。
schema 幂等创建/迁移；`store.rs` 同时负责旧数据去重迁移与只读异常行检查。
Tauri 侧对 `index.db` 的打开使用只读标志并启用 `query_only`，任何连接都不创建或
修改 WAL/SHM/journal 文件。

## 五、配置与原子写

`config.json` 由 `config.rs` 管理：大小写不敏感读取、跨进程有界锁、fingerprint
冲突检测、临时文件精确清理与原子替换。CLI 的 `config` 命令与 Tauri 共用该实现。

## 六、进程安全与执行边界

- 本地进程一律使用「可执行文件 + 参数数组 + 工作目录」模型（`process.rs` /
  `ProcessSpec`）；shell 文本只用于互动终端或 SSH 远端命令的固有场景。
- `security.rs`（core 与 src-tauri 各自一份）校验/引用工具 key、session ID、
  SSH 用户/主机/身份文件/远端路径、URL 与打开器模板；所有输入先过对应的
  validator/quoting 再插入命令。
- `launcher.rs` 构造工具对应的恢复命令（Codex 使用 `resume <id>`，Pi 使用 `--session <id>`，其余工具保留各自参数），
  恢复参数与 session ID 由可信后端追加
  并打开平台终端。
- 远程 SSH：`BatchMode=yes` 全量强制，纯免密；连接探测同时报告 tmux 能力。
  每个「远程项目 + 工具」映射到确定性的 tmux 会话，首次打开创建并启动工具，
  后续打开只重连已有 TUI。同一服务器只保留一个前端 SSH PTY；选择该服务器上的
  其他项目或工具时，通过后端 `pty_remote_switch` 在现有连接内创建（如需要）并
  `switch-client`，不再启动额外 SSH 子进程。缺少 tmux 时给出安装提示，不回退到
  易丢失的直连 Shell。
  `classify_ssh_failure` 将 ssh 错误转为可操作的中英双语提示。完整契约见
  [`docs/execution-security-contract.md`](./docs/execution-security-contract.md)。

## 七、Tauri 桌面控制台

- **数据源**：查询时只读打开共享扫描核心维护的 `index.db`；首次启动检测到文件缺失时，
  前端先触发一次进程内扫描再加载项目。已有索引（包括可信的空索引）不自动重复扫描；
  首次扫描失败时不创建伪成功索引，并展示可重试引导。
- **进程内扫描**：`scan_projects` 通过 `spawn_blocking` 调用 `sessionatlas-core`
  的扫描管线，返回 `COUNT(*)`；不启动 sidecar 或子进程，不阻塞 Tauri async 线程。
- **远程扫描**：连接预检和 `scan_remote_server` / `scan_all_remote_servers` 的 SSH、
  `find` 与 SQLite 工作也运行在 `spawn_blocking` worker。新增服务器保存后，前端立即恢复
  表单并自动在后台完成首次扫描；按服务器去重扫描请求，完成后再刷新项目列表。每次成功
  写入远程项目快照时，会在同一事务内记录 `last_scanned_at`，服务器列表显示该时间；失败
  不覆盖上一次成功扫描时间。
- **项目可见性**：本机与远程的列表、搜索均隐藏路径中任一以 `.` 开头的目录及其后代；
  用户还可把任意项目目录树加入 `prefs.db.project_ignores`。本机规则按路径生效，远程规则
  额外按服务器 ID 隔离。手动忽略只在查询层过滤，不删除扫描数据，因此移除规则后无需
  重扫即可恢复；远程扫描返回的项目数也只统计最终可见行。
- **命令层**：`list_projects`、`search_projects`（FTS5 `MATCH`）、`list_tools`、
  `scan_projects`、PTY 一组（`pty_spawn/attach/write/remote_switch/resize/kill`）、远程 SSH 一组、
  项目忽略、打开器偏好、分组、Git 信息与托盘同步命令。结构体用 `#[serde(rename="camelCase")]`
  使 Rust snake_case 以 `lastAccessedAt` 形式到达 JS。
- **PTY 终端**：右栏多标签，每标签一个伪终端；本地会话由 `pty_attach` 在监听器
  就绪后恰一次启动工具。远程会话由 `pty_spawn` 把结构化工具元数据转换为安全的
  tmux 首次启动命令，`pty_attach` 只开始输出桥接，避免重连时向已有 TUI 重复注入
  启动命令；同服务器的后续项目由 `pty_remote_switch` 复用现有 SSH PTY，并将已
  绑定服务器 ID 与请求 ID 匹配后切换确定性 tmux 会话。自然退出/读失败/显式关闭/应用退出都会移除本地注册项并回收 SSH
  子进程，但远程 tmux 会话继续存在。xterm.js + addon-fit 本地 vendored 在
  `frontend/vendor/`。
- **前端**：`.stage` 双栏网格；单一 `state` 对象，改动经 `applyFilters()` → 渲染。项目账本
  和选中项目概览始终显示来源徽标：本机为“本地”，SSH 项目为“远程 · 机器名”。
  `app.js` 按 `window.__TAURI__` 双模式运行（Tauri 调 Rust 命令，浏览器用内置
  `SAMPLE` 数据）。`frontend/i18n.js` 提供中英文案，`lang-init.js` 在绘制前设置
  `<html lang>`；OS 托盘菜单语言随 `set_tray_language` 变化。
- **能力**：`src-tauri/capabilities/default.json` 对 `main` 授予 `core:default` +
  `shell:allow-open`；新增插件权限必须在此登记。

## 八、数据流与并发

- CLI `scan` 是唯一写者，原子替换快照；Tauri 只读读者始终看到完整旧快照或完整
  新快照，绝无半写状态。
- Tauri 扫描在 `spawn_blocking` 上运行，不与前端命令争抢 async 线程。
- `prefs.db` 的分组/排序写入使用事务与 revision 冲突检查；frontend 按 mutation
  队列串行化写操作，成功后发布服务端权威快照。
- 60 秒自动刷新在没有搜索词时静默重拉；full/auto/search 使用独立 gate，避免
  低信息请求覆盖完整加载（last-known-good 语义）。

## 九、安全模型要点

- 本地优先：索引/偏好/配置只保存在 `~/.sessionatlas/`，无遥测或云同步。
- 工具记录扫描器只取路径、时间、session ID 与必要 Git 元数据；项目内容索引器
  在本机限额读取源码/文档，排除敏感形态文件并按行脱敏，只持久化 FTS 词项和
  LZ4 压缩摘要，不持久化完整正文、提示词、消息、密钥或认证内容。
- 所有外部进程为参数数组；shell 元字符、控制字符、选项形 tool key 一律拒绝。
- 搜索词与用户字符串按文本节点渲染，绝不进入 `innerHTML` 执行标记。
- xterm/高亮/字体全部本地加载，不依赖 CDN。

## 十、测试与验收

- 根命令：`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets
  -- -D warnings`、`cargo test --workspace`、`npm --prefix frontend run check` /
  `npm --prefix frontend test`。
- 测试一律使用临时 `SESSIONATLAS_HOME` 与合成数据；不读取真实 `~/.sessionatlas/`，
  不启动真实 AI CLI / SSH。
- 自动化证据与剩余手工/发布门禁见
  [`docs/test-baseline.md`](./docs/test-baseline.md) 与
  [`docs/remaining-issues.md`](./docs/remaining-issues.md)。
- 发布序列（格式/lint/测试/前端/安装包/隔离扫描）在
  [`docs/rust-migration-plan.md`](./docs/rust-migration-plan.md) 的 R14 中定义，
  本地可执行门禁已通过；托管依赖审计与原生交互矩阵仍属待验收门禁。
