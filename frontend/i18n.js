/* ============================================================
   SessionAtlas — i18n (zh / en)
   Zero-dependency: a flat key→string map per locale plus a tiny
   `t()` helper. Plurals are handled by key suffixes
   (`_one` / `_other`) resolved via Intl.PluralRules.

   NOT translated (kept verbatim everywhere):
     - brand name "SessionAtlas"
     - tool names from data (Claude Code, Codex CLI, …)
     - tool monograms (CL/CX/…)
     - shell command bodies (/help, git status, …)
     - internal keys ("ungrouped")
     - Rust backend Err strings (shown as technical diagnostics)

   `t("foo", { name })` interpolates `{name}` in the resolved string.
   `t("count.sessions", { count })` auto-picks `_one`/`_other` if the
   base key is absent but a suffixed variant exists.
   ============================================================ */

export const LANGS = ["en", "zh"];
const DEFAULT_LANG = "en";

const STRINGS = {
  en: {
    // ── generic ──
    "common.loading": "loading…",
    "common.close": "Close",
    "common.delete": "delete",

    // ── brand / workspace chrome ──
    "brand.subtitle": "AI CLI project workspace",
    "workspace.terminal": "TERMINAL WORKSPACE",
    "workspace.ready": "READY",

    // ── group names ──
    "group.ungrouped": "Ungrouped",
    "group.deleteTitle": "delete group",

    // ── search / top deck ──
    "search.placeholder": "query the archive — name, path, fragment…",
    "search.rescan": "Rescan all instruments",
    "search.rescanLabel": "RESCAN",
    "search.settingsTitle": "Openers & settings",
    "search.settingsLabel": "SETTINGS",
    "search.settingsAria": "Open settings",
    "search.themeAria": "Toggle theme",
    "theme.toDark": "Switch to dark theme",
    "theme.toLight": "Switch to light theme",

    // ── server rows (settings) ──
    "server.scan": "SCAN",
    "server.scanTitle": "Rescan this server",
    "server.hintKeyless": "Passwordless (key/agent) login by default — leave the key file blank to use <code>~/.ssh/id_*</code> / ssh-agent. The connection is verified before anything is saved.",

    // ── opener kind tag ──
    "opener.custom": "custom",

    // ── filter chips ──
    "filter.all": "ALL",
    "filter.toolsLabel": "Tools",
    "filter.recencyLabel": "Activity",
    "filter.recencyAll": "all",
    "filter.recency24h": "24h",
    "filter.recency7d": "7d",
    "filter.recency30d": "30d",
    "filter.byInstrument": "filter by instrument",
    "filter.byRecency": "filter by recency",

    // ── project overview ──
    "overview.aria": "Project overview",
    "overview.kicker": "PROJECT OVERVIEW",
    "overview.emptyTitle": "Select a project",
    "overview.emptyBody": "Project activity and launch actions appear here.",
    "overview.activity": "Tool activity",
    "overview.activityMeta_one": "{time} · {count} session",
    "overview.activityMeta_other": "{time} · {count} sessions",
    "overview.recentSessions": "Latest sessions",
    "overview.sessionMeta": "{id} · {time}",
    "overview.quickActions": "Quick actions",
    "overview.branch": "Branch",
    "overview.sessions": "Sessions",
    "overview.lastActive": "Last active",
    "overview.openTerminal": "Open",
    "overview.files": "Browse files",
    "overview.settings": "Project settings",

    // ── left pane: files view ──
    "files.kicker": "FILES",
    "files.backTitle": "Collapse last expanded directory",
    "files.emptyProject": "select a project first",
    "files.emptyDir": "empty directory",

    // ── right pane: sessions ──
    "terms.title": "SESSIONS",
    "terms.emptyGlyph": "⌘",
    "terms.emptyTitle": "No active session",
    "terms.emptyBody": "Pick a project on the left and open an instrument to start an interactive terminal here. Open several and switch between tabs.",
    "terms.common": "COMMON",
    "terms.shell": "SHELL",
    "terms.closeTab": "close",

    // ── footer ──
    "foot.gitBranchTitle": "click to switch branch",
    "foot.addRemote": "+ add remote",
    "foot.meta.instr": "instr",
    "foot.meta.proj": "proj",
    "foot.meta.sess": "sess",
    "foot.meta.last": "last",
    "foot.meta.sync": "sync",
    "foot.statusIdle": "idle",
    "foot.health": "Healthy",
    "foot.keys": "↑↓ nav · ⏎ open · Ctrl K or / search · g settings · esc clear",

    // ── modals ──
    "modal.projectTitle": "Project",
    "modal.docTitle": "Doc",

    // ── drawer chrome ──
    "drawer.kickerMenu": "Console",
    "drawer.kickerPage": "Settings",
    "drawer.titleMenu": "Settings",
    "drawer.backAria": "Back to settings",
    "drawer.backTitle": "Back",

    // ── ledger ──
    "ledger.count.matches_one": `<strong>{count}</strong> match for “{query}”`,
    "ledger.count.matches_other": `<strong>{count}</strong> matches for “{query}”`,
    "ledger.count.recency": `<strong>{shown}</strong> of {total} in last {label}`,
    "ledger.count.capped": `showing first <strong>{limit}</strong> of more — narrow with search`,
    "ledger.count.entries_one": `<strong>{count}</strong> entry indexed`,
    "ledger.count.entries_other": `<strong>{count}</strong> entries indexed`,
    "ledger.emptyTitle": "Archive empty",
    "ledger.emptyBodySearch": `No entries match “{query}”.`,
    "ledger.emptyBody": "Run a rescan to survey your instruments.",
    "ledger.errorTitle": "Archive unavailable",
    "ledger.retry": "RETRY",

    // ── entry / session cards ──
    "entry.collapseSessions": "Collapse sessions",
    "entry.expandSessions": "Expand sessions",
    "entry.showFileTree": "Show file tree",
    "entry.more": "more",
    "entry.remoteTooltip": "remote: {label}",
    "entry.noInstruments": "No recorded instruments.",
    "entry.noInstrumentsHint": `No recorded instruments. Use “+ plain shell” to start a fresh terminal.`,
    "entry.label.group": "Group",
    "entry.label.docs": "Docs",
    "entry.label.files": "Files",
    "entry.label.openSession": "Open session",
    "entry.label.openWith": "Open with",
    "entry.label.openSessionWith": "Open session with",
    "entry.label.otherSessions": "Other open sessions ({count})",
    "entry.noDocs": "No markdown files found.",
    "entry.docsRemote": "Unavailable for remote projects — browse via the terminal.",
    "entry.shellNew": "＋ plain shell",
    "entry.cliNew": "＋ CLI",
    "entry.newSession": "New session",
    "entry.resume": "Resume",
    "entry.sessionsCount_one": "{count} session",
    "entry.sessionsCount_other": "{count} sessions",
    "entry.sessionMarker_one": "{count} open session ({tools})",
    "entry.sessionMarker_other": "{count} open sessions ({tools})",
    "entry.sessAbbr": "sess",
    "entry.lastPrefix": "last {time}",
    "entry.openHint": "open",

    // ── right-pane launch panel ──
    "launch.openersBuiltIn": "built-in",

    // ── context menu (file tab) ──
    "ctx.sendToCommandLine": "Send to command line",
    "ctx.copyFilePath": "Copy file path",
    "ctx.copyAbsPath": "Copy absolute path",
    "ctx.selectLinesFirst": "select lines first",
    "ctx.noPtyTab": "no command-line tab open",

    // ── branch menu ──
    "branch.menuHead": "local branches",

    // ── git footer ──
    "git.detached": "(detached)",
    "git.notARepo": "not a git repo",
    "git.addRemoteLabel": "add remote",
    "git.addRemoteSubmit": "add",
    "git.addRemoteNamePh": "name (e.g. origin)",
    "git.addRemoteUrlPh": "https://github.com/you/repo.git",
    "git.addRemoteTitle_repo": "add a remote to this git repo",
    "git.addRemoteTitle_init": "init this directory as a git repo and add a remote",
    "git.remoteTooltip": "{name} → {url}  (click to open in browser)",

    // ── settings: menu + views ──
    "settings.remote.title": "Remote servers",
    "settings.remote.blurb": "SSH into dev boxes to discover their git projects. Sessions use the AI tools already installed on the remote.",
    "settings.groups.title": "Groups",
    "settings.groups.blurb": "Bucket ledger entries by project, workstream, or anything else. Drag entries between groups in the ledger itself.",
    "settings.openers.title": "Openers",
    "settings.openers.blurb": "External launchers that open a project in your editor, file manager, or terminal. {path} is replaced with the project dir.",
    "settings.language.title": "Language",
    "settings.language.blurb": "Switch the interface between English and Chinese. Follows your system language by default.",
    "settings.language.zh": "中文",
    "settings.language.en": "English",
    "settings.configuredServers": "Configured servers",
    "settings.addSshServer": "Add SSH server",
    "settings.groupsLabel": "Groups",
    "settings.addGroup": "Add group",
    "settings.openersLabel": "Openers",
    "settings.addCustomOpener": "Add custom opener",
    "settings.noRemote": "No remote servers yet. (Key-file auth only; default roots: ~, ~/projects, ~/code.)",
    "settings.noGroups": "No groups yet — add one below.",
    "settings.noOpeners": "No openers configured.",
    "settings.prefsUnavailable": "prefs unavailable: {error}",

    // ── form placeholders ──
    "form.labelPh": "Label (e.g. homelab)",
    "form.userPh": "user",
    "form.hostPh": "host / ip",
    "form.portPh": "22",
    "form.identityPh": "Identity file (optional, e.g. ~/.ssh/id_ed25519)",
    "form.groupNamePh": "Group name (e.g. Work)",
    "form.openerLabelPh": "Label (e.g. Sublime)",
    "form.openerCmdPh": "Command template (e.g. subl {path}) — {path} is replaced with the project dir",
    "form.submit.addScan": "ADD + SCAN",
    "form.submit.add": "ADD",

    // ── status messages (setStatus) ──
    "status.groupAssignFailed": "group assign failed: {err}",
    "status.opening": "opening {label} @ {path}…",
    "status.demoWouldOpen": "(browser demo) would open {label} at {path}",
    "status.opened": "opened {label} @ {path}",
    "status.openerFailed": "opener failed: {err}",
    "status.noActiveSession": "no active session",
    "status.writeFailed": "write failed: {err}",
    "status.noActivePty": "no active command-line tab — open a session first",
    "status.pasted": "pasted {ref} → {tool} (type to continue, Enter to send)",
    "status.copied": "copied {path}",
    "status.copyFailed": "copy failed: {err}",
    "status.sortFailed": "sort failed: {err}",
    "status.sortNeedsFullList": "clear search and load the complete group before reordering",
    "status.openingTool": "opening {tool} @ {name}…",
    "status.demoWouldOpenTerm": "(browser demo) would open an interactive terminal here",
    "status.termLoadFailed": "terminal library failed to load (offline?) — reload to retry",
    "status.sessionFailed": "session failed: {err}",
    "status.sessionOpen": "session open · {title}",
    "status.openedName": "opened {name}",
    "status.openedUrl": "opened {url}",
    "status.scanning": "scanning {name}…",
    "status.scanResult_one": "{name}: {count} project",
    "status.scanResult_other": "{name}: {count} projects",
    "status.scanningInstruments": "scanning instruments…",
    "status.scanComplete_other": "scan complete · {count} projects",
    "status.scanComplete_one": "scan complete · {count} project",
    "status.searchHits_one": "search · {count} hit",
    "status.searchHits_other": "search · {count} hits",
    "status.connected": "connected",
    "status.labelCmdRequired": "label and command required",
    "status.serverFieldsRequired": "server label, user, host are required",
    "status.portRange": "port must be 1-65535",
    "status.demoRemoteUnavailable": "(browser demo) remote servers unavailable",
    "status.probing": "testing passwordless connection to {name}…",
    "status.adding": "adding {name}…",
    "status.groupNameRequired": "group name required",
    "status.nameUrlRequired": "name and url are required",
    "status.addedRemote": "added remote {name}",
    "status.demoWouldSwitch": "(browser demo) would switch to {name}",
    "status.switched": "switched to {name}",
    "status.boot": "boot · tauri={tauri} term={term}",
    "status.demoWouldOpenUrl": "(browser demo) would open {url}",
    "status.openFailed": "open failed: {err}",
    "status.addRemoteFailed": "add remote failed: {err}",
    "status.checkoutFailed": "checkout failed: {err}",
    "status.scanFailed": "scan failed: {err}",
    "status.serverAddedScanFailed": "{name} was added, but its initial scan failed: {err}",
    "status.staleSources": "showing last-known-good data; stale: {sources}",
    "status.errorPrefix": "error: {text}",
    "status.cantEmbed": "Can't embed this page",
    "status.cantEmbedBody": `The site blocks being shown inside another app. <a href="{url}" target="_blank" rel="noopener noreferrer">Open in your browser →</a>`,
    "status.readFailed": "Failed to read {path}.",

    // ── terminal inline ──
    "term.sessionEnded": "[session ended]",
    "term.sessionEndedCode": "[session ended · exit {code}]",
    "term.streamFailed": "[terminal stream closed unexpectedly]",
    "term.eventsFailed": "terminal event channel is unavailable",
    "term.startFailed": "failed to start session: {err}",

    // ── Claude task queue ──
    "queue.panelLabel": "Task queue",
    "queue.placeholder": "One prompt per line. Each runs with `claude -p` to completion, then the next starts.",
    "queue.hint": "Runs unattended with Claude's normal permission policy; tasks that need approval may stop.",
    "queue.runQueue": "Run queue",
    "queue.addToQueue": "Add to queue",
    "queue.noPrompts": "Enter at least one prompt.",
    "queue.queueOpen": "Queue tab open: {idx}/{total} running.",
    "queue.banner": "Claude task queue · {name} · {count} task(s)",
    "queue.taskHeader": "Task {idx}/{total}",
    "queue.resumed": "Resumed — {count} more task(s) queued.",
    "queue.starting": "Starting queue of {count} task(s) for {name}…",
    "queue.startFailed": "queue failed to start: {err}",
    "queue.advanceFailed": "next task failed to start: {err}",
    "queue.running": "Running task {idx}/{total} for {name}…",
    "queue.appended": "Added {count} task(s); queue now has {total}.",
    "queue.allDone": "All {count} task(s) complete.",
    "queue.allDoneStatus": "Queue finished for {name}.",
    "queue.notifyTaskDone": "Task {name} step done",
    "queue.notifyTaskDoneBody": "Completed {idx} of {total}.",
    "queue.notifyAllDone": "Queue finished: {name}",
    "queue.notifyAllDoneBody": "All {count} task(s) complete.",

    // ── web tab ──
    "web.icon": "WB",

    // ── common command hints (terse labels) ──
    "hint.screen": "screen",
    "hint.where": "where",
    "hint.list": "list",
    "hint.git": "git",
    "hint.node": "node",
    "hint.rust": "rust",
    "hint.python": "python",
    "hint.help": "help",
    "hint.status": "status",
    "hint.ctx": "ctx",
    "hint.memory": "memory",
    "hint.agents": "agents",
    "hint.mcp": "mcp",
    "hint.resume": "resume",
    "hint.usage": "usage",
    "hint.model": "model",
    "hint.policy": "policy",
    "hint.diff": "diff",
    "hint.exit": "exit",
    "hint.tools": "tools",
    "hint.session": "session",
    "hint.config": "config",
    "hint.files": "files",
    "hint.undo": "undo",

    // ── member count (groups) ──
    "group.memberCount_one": "{count} member",
    "group.memberCount_other": "{count} members",

    // ── tray (also surfaced in Rust, but listed here for reference) ──
    "tray.show": "Show",
    "tray.quit": "Quit",
    "tray.tooltip": "SessionAtlas",

    // ── language sub-page ──
    "lang.rowTitle": "Interface language",
    "lang.followSystem": "Follow system",
  },

  zh: {
    // ── generic ──
    "common.loading": "加载中…",
    "common.close": "关闭",
    "common.delete": "删除",

    // ── 品牌 / 工作区 ──
    "brand.subtitle": "AI CLI 项目工作台",
    "workspace.terminal": "终端工作区",
    "workspace.ready": "就绪",

    // ── group names ──
    "group.ungrouped": "未分组",
    "group.deleteTitle": "删除分组",

    // ── search / top deck ──
    "search.placeholder": "查询归档 — 名称、路径、片段…",
    "search.rescan": "重新扫描所有工具",
    "search.rescanLabel": "重新扫描",
    "search.settingsTitle": "打开器与设置",
    "search.settingsLabel": "设置",
    "search.settingsAria": "打开设置",
    "search.themeAria": "切换主题",
    "theme.toDark": "切换到深色主题",
    "theme.toLight": "切换到浅色主题",

    // ── server rows (settings) ──
    "server.scan": "扫描",
    "server.scanTitle": "重新扫描该服务器",
    "server.hintKeyless": "默认免密（密钥/agent）登录 — 密钥文件留空即使用 <code>~/.ssh/id_*</code> 或 ssh-agent。保存前会先验证连接。",

    // ── opener kind tag ──
    "opener.custom": "自定义",

    // ── filter chips ──
    "filter.all": "全部",
    "filter.toolsLabel": "工具",
    "filter.recencyLabel": "活跃时间",
    "filter.recencyAll": "全部",
    "filter.recency24h": "24小时",
    "filter.recency7d": "7天",
    "filter.recency30d": "30天",
    "filter.byInstrument": "按工具筛选",
    "filter.byRecency": "按时间筛选",

    // ── 项目概览 ──
    "overview.aria": "项目概览",
    "overview.kicker": "项目概览",
    "overview.emptyTitle": "选择一个项目",
    "overview.emptyBody": "项目活动和启动操作会显示在这里。",
    "overview.activity": "工具活动",
    "overview.activityMeta_other": "{time} · {count} 个会话",
    "overview.recentSessions": "最近会话",
    "overview.sessionMeta": "{id} · {time}",
    "overview.quickActions": "快捷操作",
    "overview.branch": "分支",
    "overview.sessions": "会话",
    "overview.lastActive": "最近活跃",
    "overview.openTerminal": "打开",
    "overview.files": "浏览文件",
    "overview.settings": "项目设置",

    // ── left pane: files view ──
    "files.kicker": "文件",
    "files.backTitle": "收起最近展开的目录",
    "files.emptyProject": "请先选择一个项目",
    "files.emptyDir": "空目录",

    // ── right pane: sessions ──
    "terms.title": "会话",
    "terms.emptyGlyph": "⌘",
    "terms.emptyTitle": "暂无活动会话",
    "terms.emptyBody": "在左侧选择一个项目并打开一个工具，即可在此启动交互式终端。可同时打开多个并在标签间切换。",
    "terms.common": "常用命令",
    "terms.shell": "Shell",
    "terms.closeTab": "关闭",

    // ── footer ──
    "foot.gitBranchTitle": "点击切换分支",
    "foot.addRemote": "+ 添加远程仓库",
    "foot.meta.instr": "工具",
    "foot.meta.proj": "项目",
    "foot.meta.sess": "会话",
    "foot.meta.last": "最近",
    "foot.meta.sync": "同步",
    "foot.statusIdle": "空闲",
    "foot.health": "健康",
    "foot.keys": "↑↓ 导航 · ⏎ 打开 · Ctrl K 或 / 搜索 · g 设置 · esc 清除",

    // ── modals ──
    "modal.projectTitle": "项目",
    "modal.docTitle": "文档",

    // ── drawer chrome ──
    "drawer.kickerMenu": "控制台",
    "drawer.kickerPage": "设置",
    "drawer.titleMenu": "设置",
    "drawer.backAria": "返回设置",
    "drawer.backTitle": "返回",

    // ── ledger ──
    "ledger.count.matches_other": `<strong>{count}</strong> 条匹配“{query}”`,
    "ledger.count.recency": `最近 {label}：共 <strong>{shown}</strong> / {total} 个`,
    "ledger.count.capped": `仅显示前 <strong>{limit}</strong> 个，更多请用搜索缩小范围`,
    "ledger.count.entries_other": `已索引 <strong>{count}</strong> 个条目`,
    "ledger.emptyTitle": "归档为空",
    "ledger.emptyBodySearch": `没有匹配“{query}”的条目。`,
    "ledger.emptyBody": "运行一次重新扫描以盘点你的工具。",
    "ledger.errorTitle": "归档不可用",
    "ledger.retry": "重试",

    // ── entry / session cards ──
    "entry.collapseSessions": "收起会话",
    "entry.expandSessions": "展开会话",
    "entry.showFileTree": "显示文件树",
    "entry.more": "更多",
    "entry.remoteTooltip": "远程：{label}",
    "entry.noInstruments": "暂无已记录的工具。",
    "entry.noInstrumentsHint": "暂无已记录的工具。点击“+ 纯 Shell”启动一个新终端。",
    "entry.label.group": "分组",
    "entry.label.docs": "文档",
    "entry.label.files": "文件",
    "entry.label.openSession": "打开会话",
    "entry.label.openWith": "打开方式",
    "entry.label.openSessionWith": "使用以下工具打开会话",
    "entry.label.otherSessions": "其他打开的会话（{count}）",
    "entry.noDocs": "未找到 Markdown 文件。",
    "entry.docsRemote": "远程项目暂不支持 — 请通过终端浏览。",
    "entry.shellNew": "＋ 纯 Shell",
    "entry.cliNew": "＋ 命令行",
    "entry.newSession": "新会话",
    "entry.resume": "继续",
    "entry.sessionsCount_other": "{count} 个会话",
    "entry.sessionMarker_other": "{count} 个活动会话（{tools}）",
    "entry.sessAbbr": "会话",
    "entry.lastPrefix": "{time}前",
    "entry.openHint": "打开",

    // ── right-pane launch panel ──
    "launch.openersBuiltIn": "内置",

    // ── context menu (file tab) ──
    "ctx.sendToCommandLine": "发送到命令行",
    "ctx.copyFilePath": "复制文件路径",
    "ctx.copyAbsPath": "复制绝对路径",
    "ctx.selectLinesFirst": "请先选中行",
    "ctx.noPtyTab": "没有打开的命令行标签",

    // ── branch menu ──
    "branch.menuHead": "本地分支",

    // ── git footer ──
    "git.detached": "(游离头)",
    "git.notARepo": "非 git 仓库",
    "git.addRemoteLabel": "添加远程",
    "git.addRemoteSubmit": "添加",
    "git.addRemoteNamePh": "名称（如 origin）",
    "git.addRemoteUrlPh": "https://github.com/you/repo.git",
    "git.addRemoteTitle_repo": "为该 git 仓库添加远程",
    "git.addRemoteTitle_init": "将该目录初始化为 git 仓库并添加远程",
    "git.remoteTooltip": "{name} → {url}（点击在浏览器打开）",

    // ── settings: menu + views ──
    "settings.remote.title": "远程服务器",
    "settings.remote.blurb": "通过 SSH 连接开发机以发现其 git 项目。会话使用远程已安装的 AI 工具。",
    "settings.groups.title": "分组",
    "settings.groups.blurb": "按项目、工作流或任意维度对条目分桶。可在主列表中拖动条目在分组间移动。",
    "settings.openers.title": "打开器",
    "settings.openers.blurb": "在编辑器、文件管理器或终端中打开项目的外部启动器。{path} 会被替换为项目目录。",
    "settings.language.title": "语言",
    "settings.language.blurb": "在中文与英文之间切换界面。默认跟随系统语言。",
    "settings.language.zh": "中文",
    "settings.language.en": "English",
    "settings.configuredServers": "已配置的服务器",
    "settings.addSshServer": "添加 SSH 服务器",
    "settings.groupsLabel": "分组",
    "settings.addGroup": "添加分组",
    "settings.openersLabel": "打开器",
    "settings.addCustomOpener": "添加自定义打开器",
    "settings.noRemote": "暂无远程服务器。（仅支持密钥认证；默认根目录：~、~/projects、~/code。）",
    "settings.noGroups": "暂无分组 — 在下方添加一个。",
    "settings.noOpeners": "未配置任何打开器。",
    "settings.prefsUnavailable": "偏好数据库不可用：{error}",

    // ── form placeholders ──
    "form.labelPh": "标签（如 homelab）",
    "form.userPh": "用户名",
    "form.hostPh": "主机 / IP",
    "form.portPh": "22",
    "form.identityPh": "身份文件（可选，如 ~/.ssh/id_ed25519）",
    "form.groupNamePh": "分组名称（如 Work）",
    "form.openerLabelPh": "标签（如 Sublime）",
    "form.openerCmdPh": "命令模板（如 subl {path}）— {path} 会被替换为项目目录",
    "form.submit.addScan": "添加 + 扫描",
    "form.submit.add": "添加",

    // ── status messages (setStatus) ──
    "status.groupAssignFailed": "分组指派失败：{err}",
    "status.opening": "正在打开 {label} @ {path}…",
    "status.demoWouldOpen": "（浏览器演示）将在 {path} 打开 {label}",
    "status.opened": "已打开 {label} @ {path}",
    "status.openerFailed": "打开器失败：{err}",
    "status.noActiveSession": "没有活动会话",
    "status.writeFailed": "写入失败：{err}",
    "status.noActivePty": "没有活动的命令行标签 — 请先打开一个会话",
    "status.pasted": "已粘贴 {ref} → {tool}（继续输入，回车发送）",
    "status.copied": "已复制 {path}",
    "status.copyFailed": "复制失败：{err}",
    "status.sortFailed": "排序失败：{err}",
    "status.sortNeedsFullList": "请清除搜索并加载完整分组后再调整顺序",
    "status.openingTool": "正在打开 {tool} @ {name}…",
    "status.demoWouldOpenTerm": "（浏览器演示）将在此打开一个交互式终端",
    "status.termLoadFailed": "终端库加载失败（离线？）— 重新加载以重试",
    "status.sessionFailed": "会话失败：{err}",
    "status.sessionOpen": "会话已打开 · {title}",
    "status.openedName": "已打开 {name}",
    "status.openedUrl": "已打开 {url}",
    "status.scanning": "正在扫描 {name}…",
    "status.scanResult_other": "{name}：{count} 个项目",
    "status.scanningInstruments": "正在扫描工具…",
    "status.scanComplete_other": "扫描完成 · {count} 个项目",
    "status.searchHits_other": "搜索 · {count} 条命中",
    "status.connected": "已连接",
    "status.labelCmdRequired": "标签和命令均为必填",
    "status.serverFieldsRequired": "服务器标签、用户名、主机均为必填",
    "status.portRange": "端口必须在 1-65535 之间",
    "status.demoRemoteUnavailable": "（浏览器演示）远程服务器不可用",
    "status.probing": "正在检测到 {name} 的免密连接…",
    "status.adding": "正在添加 {name}…",
    "status.groupNameRequired": "分组名称为必填",
    "status.nameUrlRequired": "名称和 URL 均为必填",
    "status.addedRemote": "已添加远程 {name}",
    "status.demoWouldSwitch": "（浏览器演示）将切换到 {name}",
    "status.switched": "已切换到 {name}",
    "status.boot": "启动 · tauri={tauri} term={term}",
    "status.demoWouldOpenUrl": "（浏览器演示）将打开 {url}",
    "status.openFailed": "打开失败：{err}",
    "status.addRemoteFailed": "添加远程失败：{err}",
    "status.checkoutFailed": "切换失败：{err}",
    "status.scanFailed": "扫描失败：{err}",
    "status.serverAddedScanFailed": "已添加 {name}，但首次扫描失败：{err}",
    "status.staleSources": "正在显示上次有效数据；已过期：{sources}",
    "status.errorPrefix": "错误：{text}",
    "status.cantEmbed": "无法嵌入此页面",
    "status.cantEmbedBody": `该站点不允许在其它应用中展示。<a href="{url}" target="_blank" rel="noopener noreferrer">在浏览器中打开 →</a>`,
    "status.readFailed": "读取 {path} 失败。",

    // ── terminal inline ──
    "term.sessionEnded": "[会话已结束]",
    "term.sessionEndedCode": "[会话已结束 · 退出码 {code}]",
    "term.streamFailed": "[终端数据流意外关闭]",
    "term.eventsFailed": "终端事件通道不可用",
    "term.startFailed": "启动会话失败：{err}",

    // ── Claude 任务队列 ──
    "queue.panelLabel": "任务队列",
    "queue.placeholder": "每行一个任务。每个任务用 `claude -p` 执行完毕后，自动开始下一个。",
    "queue.hint": "无人值守运行，但保留 Claude 的正常权限策略；需要批准的任务可能会停止。",
    "queue.runQueue": "运行队列",
    "queue.addToQueue": "加入队列",
    "queue.noPrompts": "请至少输入一个任务。",
    "queue.queueOpen": "队列标签已打开：正在执行 {idx}/{total}。",
    "queue.banner": "Claude 任务队列 · {name} · 共 {count} 个任务",
    "queue.taskHeader": "任务 {idx}/{total}",
    "queue.resumed": "已恢复 — 队列中还有 {count} 个任务。",
    "queue.starting": "正在为 {name} 启动 {count} 个任务的队列…",
    "queue.startFailed": "队列启动失败：{err}",
    "queue.advanceFailed": "下一任务启动失败：{err}",
    "queue.running": "正在执行 {name} 的任务 {idx}/{total}…",
    "queue.appended": "已加入 {count} 个任务；队列共 {total} 个。",
    "queue.allDone": "全部 {count} 个任务已完成。",
    "queue.allDoneStatus": "{name} 的队列已执行完毕。",
    "queue.notifyTaskDone": "任务 {name} 一步完成",
    "queue.notifyTaskDoneBody": "已完成 {idx} / {total}。",
    "queue.notifyAllDone": "队列完成：{name}",
    "queue.notifyAllDoneBody": "全部 {count} 个任务已完成。",

    // ── web tab ──
    "web.icon": "WB",

    // ── common command hints ──
    "hint.screen": "清屏",
    "hint.where": "位置",
    "hint.list": "列表",
    "hint.git": "git",
    "hint.node": "node",
    "hint.rust": "rust",
    "hint.python": "python",
    "hint.help": "帮助",
    "hint.status": "状态",
    "hint.ctx": "上下文",
    "hint.memory": "记忆",
    "hint.agents": "代理",
    "hint.mcp": "mcp",
    "hint.resume": "继续",
    "hint.usage": "用量",
    "hint.model": "模型",
    "hint.policy": "策略",
    "hint.diff": "差异",
    "hint.exit": "退出",
    "hint.tools": "工具",
    "hint.session": "会话",
    "hint.config": "配置",
    "hint.files": "文件",
    "hint.undo": "撤销",

    // ── member count (groups) ──
    "group.memberCount_other": "{count} 个成员",

    // ── tray (surfaced in Rust, listed for reference) ──
    "tray.show": "显示",
    "tray.quit": "退出",
    "tray.tooltip": "SessionAtlas",

    // ── language sub-page ──
    "lang.rowTitle": "界面语言",
    "lang.followSystem": "跟随系统",
  },
};

// Per-locale BCP-47 tag used for Intl (plural rules, date formatting).
const LOCALE_TAG = { en: "en-US", zh: "zh-CN" };

// One PluralRules per locale (cheap, reused).
const PLURAL = {
  en: new Intl.PluralRules("en-US"),
  zh: new Intl.PluralRules("zh-CN"),
};

/// Current interface language ("en" | "zh"). Reads <html lang>, which is
/// set synchronously by lang-init.js before first paint.
export function currentLang() {
  const l = document.documentElement.lang;
  return l === "zh" ? "zh" : "en";
}

/// BCP-47 tag for the current language, for Intl date/number formatting.
export function currentLocaleTag() {
  return LOCALE_TAG[currentLang()] || LOCALE_TAG[DEFAULT_LANG];
}

/// Translate `key`, interpolating `{name}` placeholders from `vars`.
/// Plural keys are encoded as `key_one` / `key_other`; if the exact key is
/// missing but a suffixed variant exists, the variant is chosen via
/// Intl.PluralRules on `vars.count` (defaulting to `other` when absent).
/// Falls back to the key itself if nothing matches (makes missing keys
/// obvious during development without crashing).
export function t(key, vars) {
  const lang = currentLang();
  const table = STRINGS[lang] || STRINGS[DEFAULT_LANG];

  let s = table[key];
  // Plural resolution: only when the base key is absent but a suffixed
  // form exists. Requires a `count` in vars.
  if (s === undefined && vars && typeof vars.count === "number") {
    const category = (PLURAL[lang] || PLURAL[DEFAULT_LANG]).select(vars.count);
    s = table[`${key}_${category}`] ?? table[`${key}_other`];
  }
  // Last resort: try the default language, then the key verbatim.
  if (s === undefined) {
    s = STRINGS[DEFAULT_LANG][key];
  }
  if (s === undefined) return key;

  if (vars) {
    for (const [k, v] of Object.entries(vars)) {
      s = s.replace(new RegExp(`\\{${k}\\}`, "g"), String(v));
    }
  }
  return s;
}
