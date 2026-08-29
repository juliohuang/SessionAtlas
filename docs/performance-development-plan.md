# SessionAtlas 性能开发计划与验收记录

本文把性能优化拆成可回滚、可验证的阶段，并记录本轮实现状态。性能优化不改变
SessionAtlas 的数据所有权、安全执行边界或用户可恢复性。

## 1. 目标与非目标

目标：

- 让首次分析后的重复分析复用安全的增量缓存，避免重复读取未变化的大型 JSONL；
- 让会话整理在分析快照失效时拒绝操作，在安全复核后以可恢复隔离方式工作；
- 降低 Git、远程扫描、TUI 能力探测和 PTY 输出对 UI/网络/线程的阻塞；
- 让项目列表在 2,000 项目规模下保持有界 DOM、可用键盘导航、分组折叠和拖拽；
- 每个阶段都有针对性测试，最终用 Rust、前端语法、单元和浏览器测试复核。

非目标：

- 不删除会话、索引或其他真实用户数据，不在验证中执行 quarantine、push 或真实 SSH；
- 不改变 SSH `BatchMode`、参数数组、远程路径 quoting、PTY 生命周期或原子索引快照契约；
- 不以隐藏错误、跳过失败结果或降低功能来换取指标；
- 不把首次 Release 编译时间计入运行性能，也不把浏览器测试服务器的退出问题伪装成通过。

## 2. 基线与性能预算

起点为 `main` 的 `c59fa12`，工作区在开始时干净。基线问题来自代码审查：会话分析每次
重读 JSONL，Git 选择会重复启动本地进程，所有项目预渲染完整详情，远程探测可能形成
网络突发，PTY 小提示符可能等到下一次输出才显示。

预算（以现有测试环境测量；不同机器需重新记录）：

| 区域 | 预算/判定 |
| --- | --- |
| 2,000 项目初始 DOM | `< 30,000` 节点；本轮浏览器夹具收紧为 `< 6,000` 节点、`< 400` 按钮、`< 80` 项目行 |
| 单次筛选 | `< 300 ms`，测试记录 `performance.now()` |
| TUI/远程探测 | 全局最多 2 个并发；5 分钟 TTL，设置页强制刷新除外 |
| Git 选择 | 300 ms 防抖、单项目 single-flight、180 s TTL；本地快速快照与同步刷新分离 |
| PTY 输出 | 16 ms 批处理、单批最多 64 KiB；有界通道，退出前 flush |
| 远程 Git | SSH/远程 fetch 总体有界超时约 12 s；超时返回错误，不阻塞 UI |

本轮已获得的可复现实测数据：浏览器 2,000 项目夹具的首屏 compact 行高为 42 px、
分组标题为 31.5 px；冷估算调整为 42/32 px 后，滚到底部项目 `project-1999` 可见，
`scrollHeight` 两次读取稳定且按“2 个标题 + 2,000 行”比例门通过。首屏为 448 个 DOM
节点、60 个按钮、20 个项目行，筛选 `< 300 ms` 也在该夹具中验证。当前仓库 Git 快速
快照连续 5 次为 193.88/187.11/180.65/210.53/192.02 ms，平均 192.84 ms、每次 4 个
进程；相对初始 608.7 ms、9 个进程的基线，平均墙钟下降约 68.3%。未在真实 900 MB
会话目录上运行扫描，因此不虚构
冷/热墙钟数据；应在发布候选机上补充该 live gate。

## 3. 分阶段实施与依赖

### 阶段 A：会话整理 P0（已实现）

范围：`src-tauri/src/session_cleanup.rs`、`crates/sessionatlas-core/src/config.rs`。

- 使用版本化、按规范化来源路径/大小/mtime/解析器版本索引的 BTreeMap 缓存；损坏或
  缺失时安全重建，不把失败解析写成可信空结果；相同字节跳过替换；Windows 使用
  `ReplaceFileW`/`MoveFileExW` 等价原子替换；
- 分析返回 snapshot 世代标识；quarantine 先做库存指纹和保护规则复核，陈旧快照拒绝；
- 保留 current thread、parent/child、latest-for-project 保护，失败移动可回滚；前端清理
  后局部移除候选并标记需手动刷新，不自动完整分析。

验证：缓存命中/失效/损坏重建、连续两次保存第二版可读、删除条目 pruning、Unix 反斜杠
路径安全、陈旧快照、受保护项、移动失败回滚和恢复测试均通过。真实 quarantine 未执行。

### 阶段 B：Git P0（已实现）

范围：`src-tauri/src/lib.rs`、`src-tauri/src/process.rs`、`frontend/app.js` 及浏览器测试。

- `get_git_info` 改为异步 `spawn_blocking`；本地快速状态合并为有限的 4 个读取进程；
- fetch/status 同步与快速快照分离，fetch 使用文件重定向和有界超时，避免 stderr pipe 填满；
- 前端 300 ms 防抖、single-flight、TTL、token 门控和快速结果缓存；切换 A→B→A 可复用
  未完成的 provisional 结果；语言重绘只使用已有缓存；远端 Git 同样使用全程超时；
- add remote/checkout 先 invalidate，再强制刷新；Git 徽标只做局部更新。

验证：本地/远端 Git mock、dirty/ahead/behind/no-upstream、旧 token、延迟 fetch、回切项目、
请求调用次数和语法测试通过；不执行 push。

### 阶段 C：项目列表 P0/P1（已实现并通过浏览器退出门）

范围：`frontend/app.js`、`frontend/styles.css`、`frontend/tests/browser/performance.spec.js`。

- 详情卡片和工具按钮只在展开时生成；使用带前后 spacer 的测量窗口和事件委托；
- 分组、折叠、筛选、排序、键盘导航、选中滚动、远程/本地图标和拖拽均沿用同一行模型；
- `applyFilters()` 只构造一次 rows，组移动/重排路径重新构造 rows；
- 窗口签名不变时只改 transform/height，不替换节点，从而保留展开队列 textarea 的值、选择和焦点；
- 窗口回收期间关闭入场动画，拖拽期间不替换源节点。

验证：2,000 项目夹具验证 DOM、按钮、滚底 `project-1999`、scrollHeight 稳定、分组折叠/展开、
跨组移动、2000 条达到上限时拒绝手工重排、低于上限时分组移动与手工重排、队列输入/焦点
保留、筛选耗时。测试静态服务器改由 Playwright global setup 在 runner 进程内托管，避开 Windows
进程树回收驻留；性能用例 2/2、会话清理/Git/TUI 目标用例 4/4、完整浏览器套件 50/50 均以
退出码 0 正常结束。

### 阶段 D：普通索引 P1（已实现）

范围：`crates/sessionatlas-core/src/scanner/cache.rs`、scanner base/Codex/Claude。

- 按 path/size/mtime/parser-version 做持久增量缓存；缓存异常退化为完整扫描；
- 按工具 retain 本轮见到的路径，删除文件不会导致缓存永久增长；原子 snapshot 与
  successful/failed/unavailable 语义保持不变。

验证：cache hit、mtime/size 变化、删除、解析失败、Unavailable 保留旧快照及各扫描器测试通过。
真实 Release 冷/热目录基准仍是 live gate。

### 阶段 E：远程扫描与 TUI P1/P2（已实现）

范围：`src-tauri/src/lib.rs`、`frontend/app.js`、浏览器/单元测试。

- 重叠扫描根只去 exact duplicate，保留嵌套根发现深度；昂贵 branch/log 前远端按路径去重；
- `sort -z -u` 不可用时回退原始 NUL 流并检查 find/xargs 失败；临时文件 trap 立即安装；
- 服务器扫描使用稳定 id 顺序、最多 2 个 worker、逐服务器 partial 提交；
- TUI 探测延迟到使用点并使用 TTL/全局 2 并发，设置页刷新仍可强制执行。

验证：远程 shell quoting、空路径/换行/NUL、BSD sort 回退、稳定顺序、最大并发及浏览器
TUI 本地优先/跨三台最多 2 并发/TTL/强制刷新测试；不连接真实 SSH。

### 阶段 F：PTY P2（已实现）

范围：`src-tauri/src/pty.rs`、`src-tauri/src/lib.rs`。

- reader 向有界 `sync_channel` 写入 chunk，单一输出 worker 负责 UTF-8 decoder、16 ms
  batcher 和 emit；空闲时阻塞等待，不做 16 ms 空轮询；
- 批次保序、边界不拆 UTF-8，短提示符 16 ms 内可见，退出 flush 尾部；ResizeObserver 只对
  实际 cols/rows 改变调用 resize，IME composing 保护不变。

验证：批次大小/顺序、短块定时 flush、空 tick 不发空事件、退出 flush、`MAX-1 ASCII + 界`
边界和 resize 去重测试通过；不启动破坏性终端命令。

## 4. 验证矩阵与回滚

每个阶段先读现有测试，再实现、补测试、运行最小验证；任一失败先修复再进入下一阶段。
建议最终命令：

```text
npm --prefix frontend run check
npm --prefix frontend run test:unit
npx playwright test tests/browser/performance.spec.js --reporter=line
npx playwright test tests/browser/session-cleanup-git.spec.js tests/browser/tui-tools.spec.js --reporter=line
npm --prefix frontend run test:browser -- --reporter=line
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p sessionatlas-tauri
git diff --check
git status --short
```

浏览器测试必须从 `frontend` 工作目录使用 `playwright.config.js` 的 baseURL；每个 spec
都应设置合理的 test timeout，并在 Windows 上确认只清理本轮启动的 webServer/Node 进程。

回滚只撤回本轮代码和缓存格式版本，不删除真实数据；旧缓存版本视为缺失并安全重建。
索引写入继续使用原子 snapshot，远程扫描和 Git 超时失败保留旧快照/旧显示结果。

## 5. 最终验收和外部 live gate

代码静态/单元验收以命令输出为准；最终交付前还要在发布候选机完成：

1. Codex/Claude 真实目录的 Release 冷分析、热分析和缓存未变化复测（不得执行清理）；
2. 当前仓库 Git 5 次本地快照均值和有 upstream 的 fetch/status 超时观测（不得 push）；
3. 人工确认真实远程服务器/TUI/PTY/IME、托盘和用户数据恢复流程。

本轮未连接真实 SSH、未执行 quarantine/恢复、未 push/commit；这些是有意保留的外部门槛。
