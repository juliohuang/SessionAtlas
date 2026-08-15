# AI CLI 项目聚合工具设计方案

## 一、开源项目调研结论

经过全面搜索，**目前没有完全匹配需求的开源聚合实现**。现有相关项目分为以下几类，各有明显局限：

| 项目 | 定位 | 与需求的匹配度 | 说明 |
|------|------|-------------|------|
| **claude-code-history-viewer (CCHV)** | 统一历史查看器 | ❌ 不匹配 | 支持 25+ 个 AI 助手的历史记录查看/搜索，但**只读历史，不做项目管理，也不能打开 CLI** |
| **Superconductor** | Agent 聚合器 | ❌ 不匹配 | 闭源 macOS 应用，聚合 Claude Code / Codex / Gemini 等，支持多 Tab 并行，但**无项目发现/索引功能** |
| **CC Switch** | 配置统一管理平台 | ❌ 不匹配 | Tauri 桌面应用，管理 API Key / 模型 / 端点配置，**不涉及项目目录发现** |
| **Coder-AI-Ops** | 开发流程管理 | ⚠️ 部分相关 | 解决 Codex CLI 和 Claude Code 的流程管理不足，参考云效，但**侧重流程而非项目发现** |
| **acpx (OpenClaw)** | Agent 网关 | ❌ 不匹配 | 给其他 Agent 调用 Claude Code / Codex 的协议网关，**不是人用的项目管理工具** |
| **claude-code-switcher** | Claude 配置切换 | ❌ 不匹配 | 仅支持 Claude Code 的 Profile / MCP 配置切换 |
| **moa-x** | 多 Agent 协作 | ❌ 不匹配 | 利用多个 CLI 并行生成实现计划，**不是项目浏览器** |

**结论**：需求是一个全新的工具类别——需要从零设计实现。

---

## 二、工具定位

**名称**：`SessionAtlas`（命令行为 `sessionatlas`，数据目录为 `~/.sessionatlas/`）

**一句话定位**：扫描所有 AI CLI 工具的工作目录，建立统一项目索引，一键打开任意 CLI 进入指定项目。

**核心解决的问题**：
- 开发者同时使用 Claude Code、Codex、Kimi、OpenCode 等多个 CLI 工具
- 各工具散落在不同目录（`~/.claude/projects/`、`~/.codex/sessions/`、`~/.kimi-code/` 等）
- 想找回"上周用 Claude Code 改过的那个项目"非常困难
- 需要手动 `cd` 到项目目录再手动启动对应 CLI

---

## 三、功能设计

### 3.1 核心功能

```
┌─────────────────────────────────────────────────────────────┐
│  sessionatlas <command>                                           │
├─────────────────────────────────────────────────────────────┤
│  scan    → 扫描所有已安装 AI CLI 工具的数据目录             │
│  list    → 列出已索引的项目（支持过滤/排序）                │
│  search  → 模糊搜索项目名称或路径                           │
│  open    → 交互式选择项目 + CLI 工具，一键打开终端           │
│  recent  → 列出最近访问的项目（跨工具）                     │
│  config  → 管理工具配置、添加自定义扫描规则               │
└─────────────────────────────────────────────────────────────┘
```

### 3.2 详细功能

| 功能 | 说明 |
|------|------|
| **自动发现** | 首次运行自动检测系统已安装的 AI CLI 工具（通过 PATH 和已知安装路径） |
| **增量扫描** | 只扫描新增/修改的项目，已有项目缓存加速 |
| **多维度索引** | 项目路径、工具来源、最后访问时间、Git 分支、仓库信息 |
| **模糊搜索** | 支持拼音、路径片段、仓库名多维度搜索 |
| **跨工具项目去重** | 同一项目被多个工具编辑过，合并为一个项目，显示多标签 |
| **一键打开** | 选择项目后，选择工具（Claude Code / Codex / Kimi 等），自动 `cd` 并启动 CLI |
| **终端适配** | 支持 Windows Terminal、iTerm2、GNOME Terminal、VS Code 终端等 |
| **会话恢复** | 支持 `--resume` 直接进入上次工作的工具和项目 |

---

## 四、架构设计

### 4.1 模块结构

```
SessionAtlas/
├── CLI Layer          (Spectre.Console - TUI 交互)
│   ├── Commands/      (scan, list, search, open, recent, config)
│   ├── Prompts/       (交互式选择器、搜索框、确认框)
│   └── Renderers/     (表格、树形、面板渲染)
├── Core Layer
│   ├── Scanner/       (各工具数据目录扫描器)
│   ├── Indexer/       (项目索引构建与去重)
│   ├── Store/         (SQLite 本地存储 + 轻量缓存)
│   ├── Launcher/      (终端进程启动与命令行构建)
│   └── Config/        (用户配置与自定义扫描规则)
└── Models/
    ├── Project.cs      (项目实体)
    ├── ToolSource.cs   (AI CLI 工具来源定义)
    └── Session.cs      (会话记录)
```

### 4.2 数据流

```
┌──────────────┐    ┌──────────────┐    ┌──────────────┐    ┌──────────────┐
│  AI CLI 工具  │───→│   Scanner    │───→│   Indexer    │───→│   SQLite     │
│ 数据目录     │    │ (按工具解析) │    │ (去重/合并)  │    │  项目索引库   │
└──────────────┘    └──────────────┘    └──────────────┘    └──────────────┘
                                                                    │
                              ┌──────────────┐                     │
                              │   Launcher   │←────────────────────┘
                              │ (启动终端)   │    用户选择项目+工具
                              └──────────────┘
```

---

## 五、支持工具矩阵

| 工具 | 状态 | 数据目录 | 项目路径提取方式 |
|------|------|---------|-----------------|
| **Claude Code** | ✅ 优先支持 | `~/.claude/projects/` | 目录名 = 项目路径 |
| **Codex CLI** | ✅ 优先支持 | `~/.codex/sessions/` | session 元数据中的 `cwd` |
| **Gemini CLI** | ✅ 优先支持 | `~/.gemini/history/` | 历史文件中的 `cwd` |
| **Kimi CLI** | ✅ 优先支持 | `~/.kimi-code/sessions/` | `state.json` 中的 `workDir` |
| **OpenCode** | ✅ 优先支持 | `~/.local/share/opencode/opencode.db` | `project` / `session` 表 |
| **Aider** | ✅ 支持 | 项目目录内 `.aider.chat.history` | 扫描最近修改的项目目录 |
| **Cline** | ✅ 支持 | VS Code `globalStorage/<ext>/tasks/` | task 元数据中的 workspace |
| **Cursor** | ✅ 支持 | `~/.cursor/projects/` 或 Composer 数据 | 项目索引文件 |
| **Goose** | ✅ 支持 | `~/.config/goose/sessions/` | `sessions.db` 中的 `working_dir` |
| **Continue.dev** | ✅ 支持 | `~/.continue/sessions/` | 按 `workspace` 字段分组 |
| **Pi Agent** | ✅ 支持 | `~/.pi/` 或配置目录 | 已知项目路径索引 |
| **Amazon Q CLI** | 计划 | `~/.aws/amazonq/` | SQLite 数据 |
| **Trae** | 计划 | 应用数据目录 | 项目索引 |
| **自定义** | ✅ 支持 | 用户配置 | 正则/JSONPath 提取 |

---

## 六、界面设计（TUI）

### 6.1 主界面：`sessionatlas` 或 `sessionatlas list`

```
╭───────────────────────── AI Project Hub ──────────────────────────╮
│                                                                    │
│  #  工具          项目名称              路径                      最后访问    │
│  ────────────────────────────────────────────────────────────────  │
│  1  claude       my-api-service       ~/work/api-service         2h ago     │
│  2  codex   ┐    legacy-migration     ~/work/migration           1d ago     │
│  3  kimi    ┘    legacy-migration     ~/work/migration           3h ago     │
│  4  opencode     cli-tool             ~/projects/cli-tool         5d ago     │
│  5  aider        dotfiles             ~/.dotfiles                 1w ago     │
│  6  cursor       landing-page         ~/work/landing              3d ago     │
│                                                                    │
│  [↑/↓] 选择  [Enter] 打开  [/] 搜索  [?] 帮助  [q] 退出           │
╰────────────────────────────────────────────────────────────────────╯
```

### 6.2 打开项目交互

```
选中项目: legacy-migration (~/work/migration)

┌─────────────────────────────────────┐
│  选择要使用的 CLI 工具:             │
│                                     │
│  > Claude Code  (上次使用)          │
│    Codex CLI                        │
│    Kimi CLI                         │
│    OpenCode                         │
│    Aider                            │
│                                     │
│  [Enter] 启动  [Esc] 取消           │
└─────────────────────────────────────┘
```

### 6.3 搜索界面：`sessionatlas search api`

```
搜索: api

  结果 1: my-api-service    (claude, codex)    ~/work/api-service
  结果 2: api-gateway       (kimi)             ~/work/gateway
  结果 3: internal-api      (opencode)         ~/projects/internal-api
```

---

## 七、技术选型

| 层面 | 选型 | 理由 |
|------|------|------|
| 语言 | C# / .NET 8 | 用户技术栈熟悉，跨平台单文件发布，性能优秀 |
| CLI 框架 | Spectre.Console | .NET 生态最佳 TUI 库，支持表格、树、进度条、输入、选择器 |
| 数据存储 | SQLite | 零配置、轻量、支持全文搜索 (FTS5) |
| 配置 | JSON / YAML | 用户自定义扫描规则，易于手写 |
| 打包 | `dotnet publish -r` | 单文件自包含可执行文件，跨平台 |

---

## 八、实现阶段规划

| 阶段 | 目标 | 周期 |
|------|------|------|
| **MVP** | 支持 Claude Code / Kimi CLI 扫描 + list/search/open + SQLite 存储 | 1 周 |
| **V0.2** | 增加 Codex / Gemini / OpenCode / Aider 支持 + 去重合并 | 3 天 |
| **V0.3** | 增加 Cursor / Cline / Goose / Continue.dev + 终端适配器 | 3 天 |
| **V0.4** | 自定义扫描规则 + 会话恢复 + 配置管理 | 2 天 |
| **V1.0** | 稳定版 + 安装脚本 + 完整文档 | 2 天 |

---

## 九、使用示例

```bash
# 首次运行，扫描所有已安装工具
sessionatlas scan

# 列出所有项目（交互式 TUI）
sessionatlas list

# 搜索项目
sessionatlas search "api"

# 直接打开最近项目（上次使用的工具）
sessionatlas open --recent

# 指定项目和工具打开
sessionatlas open ~/work/my-api --tool claude

# 查看配置
sessionatlas config

# 添加自定义工具扫描规则
sessionatlas config add-tool --name my-custom-agent --path ~/.my-agent/history --pattern "cwd: (.*)"
```

---

## 十、与现有工具的关系

- **不替代**任何 AI CLI 工具，只做"项目浏览器 + 启动器"
- **可配合** CCHV 使用：CCHV 看历史，`sessionatlas` 打开项目
- **可配合** CC Switch 使用：CC Switch 管配置，`sessionatlas` 管项目
- **灵感来源** Superconductor 的聚合理念，但做成开源、跨平台、项目为中心的 CLI 工具
