# SessionAtlas 剩余已知问题修复设计

> 本文保留历史设计和回滚边界；命令、项目路径与命名空间已统一为当前
> SessionAtlas 身份，不提供旧标识兼容。

**日期：** 2026-08-03

**状态：** In progress（本文档同时作为剩余工作操作手册；本次仅更新文档，不修改业务代码）

**基线：** 当前 `main` 工作树；以实际源码和测试结果为准，不以历史审计结论代替复核

**目标：** 将剩余已知问题拆成可实施、可回滚、每一步都有检查方法和通过标准的工作包。

> 本文保留历史问题、设计理由和工作包明细。2026-08-09 之后的实际剩余项、
> 执行顺序、检查标准和进度以
> [`remaining-issues.md`](./remaining-issues.md) 为唯一入口。

## 1. 范围与原则

### 1.1 已完成，不在本设计中重复实施

- 远程扫描已改为保存 Git 工作树根目录，而不是 `.git` 元数据目录。
- 远程扫描已覆盖普通仓库和 `.git` 文件形式的 worktree，并对重叠根目录结果去重。
- 远程项目时间已改为真实 Git/工具活动时间，旧远程项目 ID 的工具历史可迁移。
- 扫描快照、PTY 生命周期、外部命令参数边界和远程 SSH 输入验证已有独立契约与测试。

完成项仍参加最终回归，但不得在后续工作中重新设计或无故改写。

### 1.2 本设计覆盖的剩余问题

| 工作包 | 优先级 | 问题摘要 | 主要区域 |
| --- | --- | --- | --- |
| WP-01 | P2 | 分组、指派、排序多步写入不原子；筛选后会提交不完整顺序 | Rust + frontend |
| WP-02 | P2 | Tauri 以读写/WAL 模式打开 CLI 所有的 `index.db` | Rust |
| WP-03 | P2 | rusqlite 行解析错误被 `.flatten()` 静默丢弃 | Rust |
| WP-13 | P2 | 多个远程扫描根中部分 `find` 失败可被后续成功掩盖，并覆盖完整快照 | Rust + frontend |
| WP-04 | P2 | 搜索词进入 `innerHTML`，可注入标记和样式 | frontend |
| WP-05 | P2 | xterm URL link provider 参数签名错误，链接功能失效 | frontend |
| WP-06 | P2 | 后端写入失败后前端仍更新状态、清空表单或显示成功 | frontend |
| WP-07 | P2 | 全量刷新、自动刷新、搜索及远程失败的发布规则不一致 | frontend |
| WP-08 | P2 | 文档、目录树和预览请求可跨项目覆盖较新的界面 | frontend |
| WP-09 | P2 | 根目录路径规范化和项目名称处理不一致 | Core + Models |
| WP-10 | P2/P3 | legacy Avalonia 精确查找、工具可用性、PTY 和 UI 线程存在缺陷 | Avalonia |
| WP-11 | P3 | CLI 标记转义、字符串选择、负数限制和工具键大小写不一致 | CLI + Store |
| WP-12 | P3 | `config.json` 直接覆盖写，崩溃或并发保存时可能损坏 | Core/Config |

当前没有新的已确认 P1。若实施中发现数据丢失、任意命令执行或整次扫描不可用的新证据，应停止当前工作包，重新分级并先补最小复现测试。

### 1.3 实施约束

1. 每个工作包开始前先运行基线；基线失败时不得把既有失败归因于当前改动。
2. 先写会失败的回归测试或故障注入，再修改生产代码。
3. `index.db` 由 C# CLI 所有；Tauri 对它只能读。用户偏好只写 `prefs.db`。
4. 不把搜索结果、工具筛选结果或时间筛选结果当成完整项目目录。
5. 后端错误必须保留原状态；前端不得在失败后伪造成功。
6. legacy Avalonia 与当前 Tauri 主界面分开交付，避免共享 `Core/Models` 变更被遗漏验证。
7. 每个工作包都应能独立回滚；跨层协议变更必须在同一提交或同一发布单元内完成。

### 1.4 原始源码锚点与历史基线

以下内容是本文档首次建立时的只读审计快照，用于说明问题来源；其中 WP-01/WP-02/WP-03/WP-04/WP-13 的部分源码已经改变，不能再把本表当成当前实现结论。实施时必须按函数名重新定位，并以 1.5 节状态表和实际测试为准。

| 主题 | 原始源码锚点 | 原始复核结论 |
| --- | --- | --- |
| `index.db` 连接 | `src-tauri/src/lib.rs:181-213` | 读索引仍复用写侧 WAL 配置 |
| rusqlite 静默丢行 | `src-tauri/src/lib.rs:461,519,544,603,761,1449,1616,1693,1798,2444,2594,2626,2683` | 13 处数据库行结果使用 `.flatten()` |
| 分组写入 | `src-tauri/src/lib.rs:1510-1679` | delete、assign、order 均非显式事务；order 写后才解析组键 |
| 远程扫描 shell | `src-tauri/src/lib.rs:2116-2129` | 每个 `find` 的 stderr 被丢弃，循环最终状态只反映最后一轮 |
| 批量远程扫描 | `src-tauri/src/lib.rs:2436-2455` | 单服务器失败只写 stderr，调用者仍收到成功总数 |
| 搜索 HTML | `frontend/app.js:357,417` | 两条路径都把含查询词的翻译写入 `innerHTML` |
| xterm URL | `frontend/app.js:1802` 附近 | 把 1-based 行号误当 buffer line，且返回范围 y 固定为 1 |
| 设置写入 | `frontend/app.js:2762-2950` 附近 | 多条写路径 catch 后继续发布成功状态 |
| 刷新协调 | `frontend/app.js:96,159,3058,3473` 附近 | auto 路径已保留远程旧值；full 路径和 gate 优先级仍有缺口 |
| C# 路径 | `Core/Store/SqliteStore.cs:288,375,534`、`Models/Project.cs:18` | 快照预校验、根路径裁剪、FTS 名称和显示名称语义不一致 |

首次只读审计观察到的测试基线为：C# `47/47`、frontend `9/9`、Rust library `31/31`。这些历史数字只用于对照；当前数字见 1.5 节。frontend 仍缺真实浏览器层，因此 unit 通过不能证明 `app.js` 的 DOM、Tauri 调用和异步发布路径正确。

### 1.5 2026-08-03 实施快照（后续执行以此为准）

原始问题清单保留用于审计，但“剩余工作”应以本表为准。状态不得仅凭代码存在改成完成，必须同时满足本表的检查证据。

| 工作包 | 当前状态 | 已有证据 | 仍需完成 |
| --- | --- | --- | --- |
| WP-02 | 已完成 | 只读 flag、`query_only`、写入拒绝、无 sidecar 测试 | 只参加最终回归 |
| WP-03 | 已完成 | 数据库坏行返回错误；数据库 `MappedRows` 不再静默 flatten | 只参加最终回归 |
| WP-13 | 已完成 | 单根失败封闭、stderr 脱敏/截断、批量 partial 结果；Rust 全套通过；前端消费 partial/LKG | 只参加最终回归 |
| WP-01 | 已完成 | group revision、事务、完整顺序校验、非法键/重复 ID/缺失成员校验、回滚测试；frontend mutation 队列和 catalog/search 分离 | 只参加最终回归 |
| WP-04 | 已完成 | 查询安全渲染的真实 Playwright DOM 测试覆盖中英、恶意标签和零结果；frontend unit/browser 全通过 | 只参加最终回归 |
| WP-05～WP-08 | 已完成 | xterm vendoring、mutation/LKG/reload、surface request gate、目录 pending/关闭竞态均有定向浏览器测试 | 只参加最终回归 |
| WP-09 | 本机完成，CI 待证 | 统一 Windows/Unix flavor helper、Store/FTS/root round-trip；C# 全套通过；已加入 Windows/Ubuntu CI 矩阵 | CI 两平台实际通过后才可发布路径修复 |
| WP-11 | 已完成 | typed recent choice、markup escape、CLI/Store/Rust limit、NOCASE index/query；C# 84、Rust 48 | 只参加最终回归 |
| WP-12 | 已完成 | 原子写、fingerprint conflict、bounded lock、replace failure、stale temp、100 次并发更新；C# 89 | 只参加最终回归 |
| WP-10 | 本机 headless 完成，GUI 手工待证 | 独立 Desktop tests 7/7、exact path/tool/session、PTY close state、dispatcher/search generation、初始化 dispatcher 回归；Desktop build 无警告 | 交互式 Avalonia 手工矩阵 |

当前最终本机证据：C# `89/89`、Desktop `7/7`、frontend unit `16/16`、Playwright browser `23/23`、Rust `48/48`；Rust `fmt` 和 `clippy -D warnings` 通过。数字只用于识别回归，不能代替新增场景的定向断言；Windows/Ubuntu CI 与交互式 GUI 仍按最终验收记录单独保留。

## 2. 总体实施顺序

```text
阶段 0：冻结基线和契约                                      [已完成]
  ├─ 阶段 1：数据边界与远程失败封闭（WP-02、WP-03、WP-13） [已完成]
  ├─ 阶段 2：排序一致性（WP-01）                            [后端核心完成，R2 剩余]
  ├─ 阶段 3：前端安全与状态（WP-04～WP-08）                 [已完成]
  ├─ 阶段 4：Core/CLI 可靠性（WP-09、WP-11、WP-12）          [本机完成，WP-09 CI 待证]
  └─ 阶段 5：legacy Avalonia（WP-10）                        [headless 完成，GUI 手工待证]
阶段 6：完整回归、迁移检查和发布验收                         [本机自动化完成]
```

依赖规则：

- WP-03 应在 WP-07 之前完成。行解析错误开始向上传播后，前端必须能区分“失败”和“空结果”。
- WP-13 应在 WP-07 之前完成。WP-13 先定义单服务器和批量扫描的显式失败结果，WP-07 再负责在界面上发布完整、部分成功或陈旧状态。
- WP-01 的 Rust 命令签名与前端调用必须同批交付，不能留下新旧参数不匹配的中间版本。
- “完整 catalog 与 search/view 分离”的状态基础由 WP-01 先落地，以保证排序正确；WP-07 只能复用并扩展这套模型，不得再造第二套 catalog。
- WP-09 先于 WP-10/WP-11。Avalonia 精确查找和 CLI 路径选择都应复用统一路径语义。
- WP-12 可独立实施，但最终必须同时运行 CLI 与 Avalonia 构建，因为二者共享 Core。

阶段门禁：

| 阶段 | 可进入条件 | 必须产出的证据 | 可进入下一阶段的条件 |
| --- | --- | --- | --- |
| 0 | 当前工作树已分类 | 工具链版本、测试数量、失败/跳过清单 | 所有既有失败都有归属，不存在真实 HOME/凭据访问 |
| 1 | 阶段 0 稳定 | readonly sidecar 对比、坏行 Err、远程 partial-failure 快照 | 三个 WP 的定向测试和 Rust 全套均通过 |
| 2 | 新错误语义已稳定 | group revision/事务/完整序列及 JS intent 测试 | Rust/JS 新协议同批可用，旧协议兼容保护生效 |
| 3 | catalog 模型已由 WP-01 建立 | 注入、URL、mutation、gate、surface race 证据 | frontend 自动化与 Tauri 手工矩阵通过 |
| 4 | 主 GUI 无协议悬空 | 路径 migration、CLI exact session、atomic config 故障注入 | CLI tests/build 与共享 Core 回归通过 |
| 5 | WP-09 已完成 | Desktop 独立 tests、PTY/UI dispatcher 状态机 | Desktop tests/build 与 Windows 手工验收通过 |
| 6 | 以上阶段全部通过 | 完整命令输出、无凭据集成、发布矩阵 | 没有未解释失败、warning、临时数据或凭据 |

### 2.1 剩余工作逐步操作清单

本节是执行入口，后续章节是每个工作包的详细契约。执行者必须严格按 `R0 → R12` 顺序操作；每个步骤未达到“通过标准”时停止，不得开始下一步。

#### R0：建立本次执行检查点

操作：

1. 从仓库根目录运行 `git status --short`，把已有修改按“本轮已有实现、用户修改、未跟踪证据文档”分类；禁止清理或覆盖无法归属的修改。
2. 运行 `git diff --check`，若只有已知行尾提示可记录；若有尾随空格、冲突标记或 patch 错误先修文档/格式。
3. 运行当前三套基线，记录测试名称和数量，不只记录退出码。

检查命令：

```powershell
git status --short
git diff --check
dotnet test SessionAtlas.Tests\SessionAtlas.Tests.csproj --nologo
Push-Location frontend
npm run check
npm test
Pop-Location
Push-Location src-tauri
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
Pop-Location
```

通过标准：C# 不少于 `47/47`、frontend unit 不少于 `10/10`、Rust 不少于 `43/43`，且无未知失败。若数量减少，即使命令退出码为 0 也视为失败并先解释测试为何消失。

#### R1：建立 frontend 真实浏览器检查层

操作：

1. 按 0.3 节固定 Playwright 版本、生成锁文件，并增加 `test:unit`、`test:browser`、`test` 三层脚本。
2. 增加只服务 `frontend/` 的本地静态服务器 fixture；不得访问 CDN，也不得读取真实 HOME。
3. 在页面脚本执行前注入 fake Tauri `invoke/listen`，支持成功、拒绝、延迟和反向完成。
4. 先写 demo 启动和 mocked-Tauri 启动两个 smoke test，保证后续所有 WP 都能检查真实 `app.js`。

检查命令：

```powershell
Push-Location frontend
npm ci
npx playwright install chromium
npm run check
npm run test:unit
npm run test:browser
Pop-Location
```

通过标准：两种启动模式均成功；浏览器控制台无未处理 rejection、资源 404 或 CDN 请求；关闭页面后没有残留 server 进程。失败时只修测试基础，不夹带业务修复。

#### R2：完成 WP-01 前端 catalog 与语义排序协议

操作：

1. 先在 `frontend/core.js` 建立唯一 `catalog`、`searchResults` 和派生 `visibleProjects`；`reload` 只在完整加载成功后替换 catalog，搜索不得覆盖 catalog。
2. 在 Rust 增加语义移动命令：参数至少包含 project、源/目标 group、anchor、before/after、可见 ID 和 expected revision；命令从完整 catalog 与 prefs 历史顺序计算最终序列。
3. 同一 `BEGIN IMMEDIATE` 事务内校验 revision、分组存在性、anchor、完整成员集合，更新 assignment/sort/revision；冲突返回 typed conflict 和权威 revision。
4. 前端位置拖拽改调用语义命令；兼容期旧 `set_group_order` 只允许已证明完整的列表。分组 mutation 按实体串行，成功后以服务端返回快照发布，失败恢复调用前深快照。
5. 保留已有 trigger 回滚测试，再增加“隐藏成员不丢失”“旧 revision 冲突”“两次拖拽逆序完成”测试。

定向检查：

```powershell
Push-Location src-tauri
cargo test group_
Pop-Location
Push-Location frontend
npm run test:unit
npx playwright test --grep "group|catalog|reorder"
Pop-Location
```

手工检查：搜索或 recency/tool 筛选后拖动一个可见项目，清除筛选；隐藏成员仍存在且相对顺序不变。制造 stale revision 后，界面显示冲突并重新载入权威顺序，不能局部覆盖。

通过标准：Rust 回滚前后逻辑快照一致且 `PRAGMA integrity_check='ok'`；浏览器反向完成测试最终只显示最新操作；完整顺序不依赖 `state.all` 的可见子集。

#### R3：完成 WP-04 浏览器安全验收

操作：

1. 保留现有“可信翻译模板 + 查询 Text 节点”的实现，不把全局 `tr()` 改成统一 HTML 转义。
2. 添加表驱动浏览器测试，输入 `&<>\"'`、`<style>body{display:none}</style><iframe src=https://example.com>` 和带 `onerror` 的 `<img>`。
3. 分别覆盖有结果、零结果、中文和英文；审计所有含 `state.query` 的 sink。

检查命令：

```powershell
Push-Location frontend
rg -n "state\.query|innerHTML" app.js
npm run check
npx playwright test --grep "search.*safe|query.*text"
Pop-Location
```

通过标准：查询按字面显示；计数与空状态内不存在 `style/iframe/img/script` 或事件属性；`document.body` 样式未变化；源码审计中没有不可信查询直接进入 `innerHTML`。

#### R4：完成 WP-05 xterm URL 链接

操作：

1. 先检查 vendored xterm 版本；首选 vendored 完全匹配版本的 WebLinks addon，记录文件来源、版本和 SHA-256。不得改用 CDN。
2. 删除旧 provider，确保每个 terminal 只注册一个 provider/addon，并在 terminal dispose 时同步 dispose。
3. handler 只接受 HTTP(S)，普通点击保留文本选择，Ctrl+点击调用现有 `openWebTab()`。
4. 浏览器测试注入 PTY 行，覆盖第 1/7 行、中文、emoji、多 URL、折行和滚动缓冲区。

检查命令：

```powershell
Push-Location frontend
Get-FileHash vendor\*WebLinks* -Algorithm SHA256
rg -n "registerLinkProvider|WebLinks|https?" app.js index.html vendor
npm run check
npx playwright test --grep "terminal.*link|url.*link"
Pop-Location
```

通过标准：无网络资源请求；普通点击打开次数为 0；Ctrl+点击恰好为 1；非法协议不生成链接；反复开关 tab 后无重复回调。

#### R5：完成 WP-06 mutation 失败一致性

操作：

1. 用 `rg` 列出所有写命令及 `await invoke(...).catch(...)`，建立命令、草稿、commit point、rollback、错误文案清单。
2. 在 `core.js` 实现可注入的 mutation controller；create/delete/update/checkbox/拖拽分别声明 optimistic 或 pessimistic 策略。
3. 所有写路径改成显式 `try/catch/return`；只有后端成功后才清表单或显示成功。commit 成功但 reconciliation 失败标记 stale，不谎报写失败。
4. Rust 所有 UPDATE 检查 affected rows，0 行返回 NotFound；数据库错误继续传播。

检查命令：

```powershell
rg -n "invoke\(|\.catch\(show|showError" frontend/app.js
Push-Location frontend
npm run test:unit
npx playwright test --grep "mutation|save failure|rollback"
Pop-Location
Push-Location src-tauri
cargo test
Pop-Location
```

通过标准：每类 rejected invoke 都保留旧 canonical state 和用户草稿；没有 success toast/reset；删除失败保留原行；不存在 ID 明确返回 NotFound。

#### R6：完成 WP-07 请求发布与 last-known-good

操作：

1. 复用 R2 的 catalog，不创建第二套主数据；为 local、remote、tools、servers、groups、openers 分别保存 last-known-good 与 stale 标志。
2. full、auto、search 分离 gate；full 启动时 invalidate auto，full/scan/search 进行时不启动 auto。
3. fetch 失败与成功空数组采用不同结果；失败保留旧值，成功空数组才清空。
4. 消费 WP-13 batch result：成功服务器发布新值，失败服务器保留旧值并显示非阻断 partial warning。
5. 用 deferred Promise 枚举 full/auto/search 逆序完成组合。

检查命令：

```powershell
Push-Location frontend
npm run test:unit
npx playwright test --grep "reload|refresh|remote.*partial|last-known-good"
Pop-Location
```

通过标准：低信息请求不能覆盖 full；失败不会清空；成功空结果会清空；Q2 失败不显示 Q1 搜索结果；partial 警告准确列出失败服务器。

#### R7：完成 WP-08 docs/tree surface 竞态

操作：

1. entry docs、entry tree、doc modal、left tree 各自持有 gate，关闭/切项目/切模式时 invalidate。
2. 请求捕获 token、project identity 和 container；发布前同时校验三者以及 `isConnected`。
3. 每个目录节点合并重复 expand；collapse 立即改变状态，迟到响应只填数据，不重新展开。
4. 文件点击从 tree root identity 取项目，不读取全局 `selectedId`。

检查命令：

```powershell
Push-Location frontend
npm run test:unit
npx playwright test --grep "document race|tree race|surface"
Pop-Location
```

手工检查：A/B 项目快速切换 20 次；展开后立即折叠；关闭 modal 后再释放旧请求。标题、路径、内容和展开状态必须始终属于当前项目。

通过标准：所有反向完成测试中旧响应均不能发布；重复 expand 只调用一次后端；无 detached DOM 写入异常。

#### R8：完成 WP-09 跨平台路径语义

操作：

1. 先为 Windows/Unix flavor 写表驱动失败测试，再实现唯一 `NormalizeProjectPath` 和 display-name helper。
2. 将 Indexer、Store 的 upsert/query/session、snapshot duplicate validation 和 FTS 全部切到同一 helper。
3. 增加幂等 FTS rebuild；历史异常行只读检测后再迁移，有歧义只报告不猜测。
4. Windows 与 Ubuntu CI 都运行纯 flavor 测试及本机 filesystem round-trip。

检查命令：

```powershell
dotnet test SessionAtlas.Tests\SessionAtlas.Tests.csproj --nologo --filter "Path|Root|Snapshot|Fts"
dotnet test SessionAtlas.Tests\SessionAtlas.Tests.csproj --nologo
dotnet build --nologo
dotnet build SessionAtlas.Desktop --nologo
```

通过标准：`C:\`、UNC root、`/` 均 round-trip 且名称非空；Windows 大小写变体合并、Unix 大小写不合并；迁移失败时旧数据不变。缺少任一目标 OS 的 CI 证据时不得发布路径修复。

#### R9：完成 WP-11 CLI 正确性

操作：

1. 对 Recent/Scan/ProjectSelector 的所有 Spectre markup sink 做字面转义。
2. 选择模型改 typed choice，直接携带 exact project/tool/session；删除字符串包含反查。
3. CLI、Store、Rust 同时加入 limit/count 边界；tool filter 使用 NOCASE 比较和索引，不改 canonical key。
4. 先跑恶意字符串、同名/子串路径、选择旧 session、边界值和 query-plan 定向测试。

检查命令：

```powershell
dotnet test SessionAtlas.Tests\SessionAtlas.Tests.csproj --nologo --filter "Recent|Selector|Markup|Limit|ToolKey|Session"
dotnet test SessionAtlas.Tests\SessionAtlas.Tests.csproj --nologo
dotnet build --nologo
Push-Location src-tauri
cargo test
Pop-Location
```

通过标准：恶意 markup 只按字面显示；exact old session 进入 launcher argv；非法边界返回非零且不查询 SQLite；NOCASE 查询命中新索引。

#### R10：完成 WP-12 配置原子保存

操作：

1. 实现“跨进程锁内 reload → mutation → temp write/flush → atomic replace”，CLI 全部配置修改入口改用该 API。
2. 实例 Save 使用 fingerprint 检测冲突；锁等待有界，Conflict/Busy/IO 分开返回。
3. 注入 lock/create/write/flush/replace 失败；本次 temp 精确清理，24 小时 stale temp 只在持锁后按严格规则清理。
4. 两个 helper process 并发更新，第三个持续读取；再执行每进程 100 次压力测试。

检查命令：

```powershell
dotnet test SessionAtlas.Tests\SessionAtlas.Tests.csproj --nologo --filter "AppConfig|Atomic|Conflict|Concurrent"
dotnet test SessionAtlas.Tests\SessionAtlas.Tests.csproj --nologo
dotnet build --nologo
dotnet build SessionAtlas.Desktop --nologo
```

通过标准：任一失败点后只能读到完整旧 JSON；成功后读到完整新 JSON；并发成功 mutation 不丢失；Busy/Conflict 可见；输出不包含测试敏感占位字符串。

#### R11：完成 WP-10 legacy Avalonia

操作：

1. 先创建独立 Desktop tests 和 dispatcher/process/store fake，不把 Desktop 源重复 link 进 root tests。
2. 精确 path 查询复用 R8；工具可用性通过 `CliLauncher.IsToolAvailable`；无可用工具时不 fallback。
3. exact session ID 贯穿 tab/ViewModel/launcher；进程成功启动后才幂等记录 session。
4. tab Unloaded 与 Close 分离；窗口退出统一有界关闭全部 PTY。
5. 查询后台执行、SQLite 串行、UI publish 回 dispatcher；搜索用 generation 防止旧响应覆盖。

检查命令：

```powershell
dotnet test SessionAtlas.Tests\SessionAtlas.Tests.csproj --nologo
dotnet test SessionAtlas.Desktop.Tests\SessionAtlas.Desktop.Tests.csproj --nologo
dotnet build SessionAtlas.Desktop --nologo
```

手工检查：A/B tab 往返切换不杀进程；关闭 A 只终止 A；选择旧 session 启动 exact resume；无 CLI 环境显示可操作错误；快速搜索只发布最后查询。

通过标准：Desktop tests/build 无 warning；PTY close 幂等；所有 ObservableCollection/selected/status 发布均由 fake dispatcher 证明在 UI 上下文。

#### R12：最终集成与文档收口

操作：

1. 执行 17.1 的完整门禁，不使用 test filter；将命令、平台、测试数量和失败/跳过项写入 `docs/test-baseline.md` 或等价验收记录。若远端不是 GitHub Actions（当前 `origin` 为 Aliyun Codeup），必须明确记录这一事实，并用 Linux 容器等价门禁作为补充证据，不得声称 hosted CI 已通过。
2. 在临时 HOME 做 17.2 无凭据集成；检查 `index.db` 无 WAL/SHM/journal，且没有启动真实 AI CLI/SSH。
3. 执行 17.3 手工矩阵并记录证据；Windows/Unix 专属门禁分别记录，不能互相代替。
4. 运行工作树审计，确认未包含数据库、日志、临时文件、浏览器下载、凭据或真实路径 fixture。
5. 在 Windows 发布机安装并验证 Tauri CLI，执行 `cargo tauri info` 与
   `cargo tauri build`；构建产物只作为发布候选，未完成交互式矩阵前不得宣称
   原生 GUI 验收完成。

检查命令：

```powershell
git status --short
git diff --check
rg --files | rg "(?i)(\.db(-wal|-shm|-journal)?$|\.env$|playwright-report|test-results|\.tmp\.)"
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
cargo tauri info
cargo tauri build
Pop-Location
```

通过标准：第 17.4 节九项发布准入条件全部有证据；任何“不适用”都必须写明平台原因和替代证据，不允许留空。

### 2.2 每一步的统一检查记录格式

每完成 R1～R12 中的一步，立即追加以下记录；没有记录视为未完成：

```text
步骤：R?-<子步骤>
改动文件：
预期不变量：
定向检查命令与退出码：
新增/通过的测试名称：
完整回归命令与结果：
手工检查（如适用）：
失败注入/回滚证据：
遗留风险或阻塞：
结论：PASS / FAIL（FAIL 时不得进入下一步）
```

检查纪律：生产代码改动后先跑最小定向测试，再跑所属组件全套；跨 `Core/Models` 的改动同时 build CLI 与 Avalonia；跨 Rust/JS 协议的改动在同一步跑 Rust 与 frontend；任何失败都先保留完整错误输出并回到本步骤修复。

## 3. 阶段 0：冻结基线与验收证据

### 0.1 记录工作树和工具链

操作：

1. 记录 `git status --short` 和 `git diff --check`。
2. 记录 `dotnet --info`、`rustc --version`、`cargo --version`、`node --version`。
3. 将基线测试的实际数量更新到 `docs/test-baseline.md`；不能继续沿用过时的 44/7/20 数量。
4. 确认测试使用临时 HOME、临时数据库和虚构数据，不读取真实 `~/.sessionatlas`。

验证：

```powershell
git status --short
git diff --check
dotnet test SessionAtlas.Tests\SessionAtlas.Tests.csproj --nologo
Push-Location frontend
npm run check
npm test
Pop-Location
Push-Location src-tauri
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
Pop-Location
```

通过标准：所有既有测试通过，无跳过项；基线文档中的数量与命令输出一致。

### 0.2 建立每个工作包的证据记录

操作：

1. 每个工作包建立一条“失败前复现 → 修复后通过 → 完整回归”的记录。
2. 记录测试名称，不只记录命令退出码。
3. 手工测试只补充自动化无法覆盖的桌面交互，不代替自动化测试。

验证：审查记录中每个工作包必须同时存在负向用例、成功用例和回滚/保持旧状态用例。

回滚点：阶段 0 不修改生产逻辑；若基线不稳定，先修测试隔离，不进入后续阶段。

### 0.3 建立可执行的 frontend 浏览器测试层

现有 `node --test` 只加载 `core.js`，不能证明 `app.js` 的真实 DOM sink、Tauri invoke 绑定、vendored xterm 或 surface 生命周期正确。进入阶段 3 前先建立以下测试基础：

1. 在 frontend dev dependencies 固定 Playwright 精确版本并提交 `package-lock.json`；CI 使用 `npm ci`，再用锁定版本的 Playwright 安装 Chromium。该依赖不进入生产运行时，不引入 bundler。用测试内 Node 静态服务器只服务 `frontend/` 本地文件。
2. 测试在页面 module 执行前用 `addInitScript` 注入 `window.__TAURI__ = {core:{invoke},event:{listen},...}`；fake invoke 按命令返回值、延迟或拒绝，并记录调用。另保留完全不注入 Tauri 的 browser-demo 用例。
3. 浏览器测试加载真实 `index.html`、`app.js`、`i18n.js` 和 `frontend/vendor/` 中的 xterm/addon；通过用户可见 DOM 和 fake invoke 断言，不依赖只在测试存在的生产后门。
4. deferred fixture 可控制 full/auto/search、docs/tree 和 mutation 的反向完成顺序；每条测试结束检查无未处理 Promise rejection、console error 或重复 listener。
5. package scripts 明确分层：`test:unit` 运行 `node --test`，`test:browser` 运行 Playwright，`test` 顺序运行两者；CI 安装固定浏览器后执行完整 `npm test`。

验证：

```powershell
Push-Location frontend
npm ci
npx playwright install --with-deps chromium
npm run check
npm run test:unit
npm run test:browser
Pop-Location
```

通过标准：demo 与 mocked-Tauri 两种模式都能启动；恶意搜索真实 DOM、xterm Ctrl+点击、rejected invoke、reload gate 和 surface race 至少各有一个 `app.js` 集成用例。若 CI 无法安装浏览器，应明确标记基础设施阻塞，不能用 pure-core tests 代替发布门禁。

回滚点：Playwright、静态 server fixture 和 package scripts 可独立回滚；不得为方便测试改变生产双模式检测逻辑。

## 4. WP-02：将 `index.db` 改为真正只读

### 问题与目标

`src-tauri/src/lib.rs` 的 `open_index_db` 当前使用 `Connection::open`，随后调用会写入 journal/synchronous 状态的 `enable_wal`。这违反“CLI 所有、Tauri 只读”的架构契约，也会让只有读取权限的索引无法打开。

目标不变量：Tauri 对 `index.db` 的任何连接都不创建、删除或修改数据库、WAL、SHM 和 journal 文件；所有写 PRAGMA 仅用于 `prefs.db`。

### 实施步骤与逐步验证

1. **拆分连接配置。**
   - 将当前共用的 `enable_wal` 拆成 `configure_prefs_connection` 和 `configure_index_reader`。
   - `configure_prefs_connection` 保留 WAL、同步级别、外键和合理的 busy timeout。
   - `configure_index_reader` 只设置允许在只读连接上执行的读侧选项，并启用 `query_only`。
   - 验证：单元测试分别查询两个连接的 `PRAGMA query_only`、`journal_mode` 和 `foreign_keys`，确保配置没有串用。

2. **使用 SQLite 只读打开标志。**
   - `open_index_db` 改用 `Connection::open_with_flags`，只包含 `SQLITE_OPEN_READ_ONLY` 及必要的线程标志。
   - 不允许 `CREATE` 回退；索引缺失仍返回“运行 `sessionatlas scan`”错误。
   - 验证：临时创建索引后以只读连接执行 SELECT 成功，执行 `CREATE TABLE`/`UPDATE` 必须得到 readonly 错误。

3. **覆盖文件系统只读场景。**
   - 在临时目录生成最小索引，将文件设为只读，再调用索引读取函数。
   - Windows 和 Unix 的权限设置使用各自测试分支；无法可靠改变 ACL 的平台至少验证 SQLite 打开标志和 `query_only`。
   - 测试前记录目录文件名、文件大小、内容哈希和 mtime，测试后逐项比较。
   - 验证：读取成功；目录内不出现新的 `-wal`、`-shm` 或 `-journal` 文件，既有文件的大小、哈希和 mtime 不变。

4. **验证 CLI 写者与 Tauri 读者并存。**
   - 用临时数据库模拟 CLI 在事务中替换快照，同时建立 Tauri 只读连接。
   - 不使用 SQLite URI 的 `immutable=1`：CLI 仍会在应用运行期间更新该文件，immutable 会允许读者永久看见陈旧页。
   - 验证：写事务提交前，读者只看到旧完整快照；结束旧读事务并重新查询后，能看到新完整快照；任何时点都看不到半写入状态。

5. **验证缺失和权限错误的消息边界。**
   - 缺失文件仍返回“先运行 `sessionatlas scan`”；存在但不可读则返回权限/打开失败，不得误报为索引缺失。
   - 验证：两种 fixture 的错误类别和用户文案不同，且错误不包含用户目录之外的敏感上下文。

回滚：只需恢复 `open_index_db` 和连接配置函数；`prefs.db` schema 不发生迁移，回滚不涉及用户数据。

## 5. WP-03：停止静默丢弃 SQLite 行错误

### 问题与目标

`src-tauri/src/lib.rs` 多个 rusqlite `query_map` 结果使用 `.flatten()`，单行列类型错误或反序列化失败会被当作“没有这条记录”。注意：`Option::flatten` 和文件系统迭代器的 `flatten` 不属于本问题，不能机械替换。

目标不变量：数据库行无法解析时，命令整体返回带上下文的错误；绝不返回不完整但看似成功的列表。

### 实施步骤与逐步验证

1. **建立目标清单。**
   - 只标记 `rusqlite::MappedRows` 上的 `.flatten()`；当前 13 处属于 `fetch_usages_by_project`、本地项目列表/搜索、工具列表、opener、groups、sort、assignments、remote servers、批量扫描 ID、remote usages、远程项目列表/搜索。
   - 排除 PTY `Option::flatten`、`read_dir` 等语义不同的位置。
   - 验证：代码审查清单中的每个目标都能追溯到一条 SQL 和一个返回结构。

2. **统一收集方式。**
   - 使用 `collect::<rusqlite::Result<Vec<_>>>()` 或显式循环 `row?`。
   - 每个查询边界使用 `map_err` 增加“命令 + 表/查询用途”上下文，例如 `list_groups: decode project_groups row`，但不得包含路径、命令模板或其他敏感字段值。
   - 验证：现有正常数据库测试结果完全不变。

3. **修复不属于 `.flatten()`、但语义相同的错误吞噬。**
   - `create_group` 当前用 `if let Ok(existing) = query_row(...)`，会把“无记录”和“行损坏/数据库失败”都当作不存在。
   - 引入 `rusqlite::OptionalExtension`：只有 `QueryReturnedNoRows` 转为 `None`，其他错误带上下文返回。
   - 验证：同名组存在时返回原组；确实不存在时创建；损坏行或 SQL 失败时不执行 INSERT，数据库逻辑快照不变。

4. **增加损坏行测试。**
   - 以每个独立 row mapper/query shape 为粒度建立最小内存数据库，至少覆盖：local project、usage、tool、opener、group、sort、assignment、remote server、batch server ID、remote usage、remote project list 和 remote search；共享同一 mapper 的命令可共用 fixture，但必须在测试名中列出覆盖关系。
   - 在一个必需整数/文本列中放入 BLOB 等确定不兼容的 SQLite 值，调用对应内部查询函数。
   - 验证：命令返回 Err；不能返回少一行的 Ok(Vec)。

5. **检查前端错误语义。**
   - 确认上层不再把此类错误转换为 `[]`；该部分与 WP-07 联动。
   - 验证：模拟命令拒绝时，已有项目仍保留，并显示一次错误状态。

6. **增加目标静态门禁。**
   - 用轻量源码检查或 lint 规则只匹配 rusqlite `MappedRows` 的 `.flatten()`；上述 13 处修复后数量必须为 0，同时允许 `Option::flatten` 和 filesystem iterator 的合法用法。
   - 验证：向一个测试 fixture 人为恢复 `rows.flatten()` 时门禁失败；合法 `Option::flatten` 不误报。

回滚：该工作包不改 schema；若新错误暴露历史脏数据，应增加显式迁移/诊断，不能恢复静默丢弃。

## 6. WP-13：远程扫描部分失败必须 fail-closed

### 问题与目标

`build_remote_scan_command` 会依次执行多个扫描根，但每个 `find` 都把 stderr 丢到 `/dev/null`，循环退出状态通常只反映最后一个根。若前一个根不存在或无权限、后一个根成功，Rust 会把部分 stdout 当成完整快照，然后删除并重建该服务器的 `remote_projects`。

目标不变量：单服务器扫描只有在所有扫描根成功、输出协议完整时才允许替换快照；任一根失败时，即使 stdout 已包含部分项目，也必须返回失败并保留此前数据库状态。

### 实施步骤与逐步验证

1. **冻结远程命令退出协议。**
   - shell 循环为每个根记录 `find` 是否失败；继续检查其余根以收集诊断，但最终只要任一根失败就以非零退出。
   - 只在预期可忽略的 Git 元数据查询上定点抑制 stderr；不得再对整个 `find` 抑制 stderr。
   - NUL 三字段输出协议保持不变，失败根不能产生伪造的完整记录。
   - 验证：生成命令的单元测试断言包含失败累积和最终非零退出；不存在根 + 正常根的 POSIX 集成测试必须产生部分 stdout、非零 status 和有界 stderr。
   - 平台门禁：Windows 开发机运行命令字符串/runner fixture 测试；Linux/macOS CI 必须实际以 `sh` 执行坏根 + 好根。没有 Unix CI 时，该工作包不能只凭 Windows 字符串断言宣布完成。

2. **在 Rust 中先判退出状态，再解析、再写库。**
   - 抽取可注入 `ProcessRunner` 的内部扫描函数；生产命令仍使用当前 SSH runner，测试 runner 可返回指定 stdout/stderr/status。
   - 非零 status 直接返回“远程扫描未完成，旧快照已保留”，不得调用解析器或写事务。用户诊断最多保留 stderr 前 4096 bytes，移除 NUL/不可打印控制字符，把已知 remote home 和 identity path 替换为 `$HOME`/`<identity>`，截断时追加 `[truncated]`；stdout 从不进入错误文案。
   - SSH 认证/连接错误仍由 `classify_ssh_failure` 处理；远端 `find`/权限错误使用独立类别，不能误导为认证失败。
   - 验证：runner 返回“一个合法 NUL 记录 + 非零 status”时命令返回 Err，解析/写入 spy 均未调用。

3. **证明失败不会覆盖快照。**
   - 在临时 `prefs.db` 为服务器预置两个远程项目和 usage 历史。
   - 分别注入：非零退出且带部分合法 stdout、权限拒绝、零退出但 NUL 输出截断、零退出但非 UTF-8 路径、SSH 失败。
   - 验证：每种失败后，`remote_projects`、`remote_tool_usages` 排序后的逻辑快照与调用前相同，并执行 `PRAGMA integrity_check` 得到 `ok`。

4. **保留成功快照语义。**
   - 所有根成功且得到 0 项是合法空快照；只有该情况才允许删除旧 `remote_projects`。
   - 所有根成功且有重复仓库时继续按规范路径去重；既有 worktree、时间和 usage 迁移测试必须保留。
   - 不给 `remote_tool_usages` 增加指向 `remote_projects` 的级联外键：usage 按现有设计跨重新扫描持久化，项目重新出现时仍可恢复活动历史。
   - 验证：成功空结果会清空项目但保留按现行契约独立存储的 usage；成功非空结果原子替换并返回准确数量。

5. **让批量扫描显式报告部分成功。**
   - 将 `scan_all_remote_servers` 的纯数字结果替换为结构化结果：`totalCount`、`successCount`、`failureCount`、每台服务器的 `serverId/count/errorKind/message`。
   - 单台失败不撤销其他服务器已经成功的独立事务，但顶层结果必须带 `partial` 状态；不得只 `eprintln!` 后返回看似完整的总数。
   - frontend 显示“成功 N 台、失败 M 台”，保留失败服务器的旧快照并提供单台重试。
   - 验证：三台服务器中一台失败时，两个成功结果发布、失败服务器快照不变、批量结果为 partial；全部失败和全部成功各有独立用例。

回滚：shell 退出协议、runner seam、结构化批量结果和 frontend 消费端必须成组回滚；数据库 schema 不变，不需要数据降级。

## 7. WP-01：分组与排序的事务、完整顺序和回滚

### 问题与目标

现有 `delete_group`、`assign_project_to_group`、`set_group_order` 都包含多条 SQL，但使用裸连接执行。`set_group_order` 还会先删排序行，再验证 `group_key`。前端 `renderedGroupOrder` 只收集当前工具/时间/搜索条件下可见的成员，可能把完整手工顺序缩成一个子集。

目标不变量：一次用户动作要么完整更新分组指派、排序和 revision，要么三者都不变；完整顺序由后端从权威数据构造和校验，前端不再把“当前可见 ID”冒充完整顺序。

### 7.1 先定义 catalog、历史偏好和显示语义

操作：

1. 前端新增长期保存的 `catalog`，只由成功的完整加载/自动刷新更新；`searchResults` 和由 tool/recency/search 得到的 `visibleProjects` 只是派生视图，不得覆盖 `catalog`。该状态拆分由 WP-01 实施，WP-07 后续复用。
2. 后端的 active catalog 定义为：`index.db` 中全部本地项目 ID，加 `prefs.db.remote_projects` 中全部远程项目 ID。后端查询不得套用 ledger 的 `LIST_LIMIT=2000`。
   - 命令受理时先在一个短只读事务中物化全部 local IDs，结束读事务并释放 index lock；之后才进入 prefs transaction，读取 remote IDs 和 group state。全项目统一这一顺序，任何路径都不得持有 prefs lock 再等待 index lock。
   - 在 prefs transaction 内，把已捕获的 local IDs 和当前事务看到的 remote IDs 分别加 `local:`/`remote:` namespace、排序并做版本化确定性 digest，得到最终 `catalogFingerprint`；不能只 hash local IDs 后把它称为完整 catalog fingerprint。
   - 该组合快照是本次动作的 catalog 语义。若 CLI local scan 在“释放 index lock”和“提交 prefs”之间完成：新出现项目保持默认未分组/无手工 sort，并在下一次 snapshot 中按默认顺序补入；刚消失项目的偏好转为 historical 并完整保留。响应带组合 `catalogFingerprint`，frontend 若已观察到不同 catalog 则立即重读 group snapshot。
3. 若任何前端兼容路径仍要判断目录是否完整，列表 API 必须读取 `limit + 1` 并显式返回 `hasMore`；客户端自报 `knownProjectIds` 不能作为“完整”证明。
4. prefs 中存在、但当前 active catalog 不存在的 assignment/sort 行定义为“历史偏好”。默认保留这些行和相对位置，排序操作不得顺手清理；项目暂时消失再出现时应恢复原偏好。
5. 计数语义拆开：`activeMemberCount` 供 ledger 显示，`storedPreferenceCount` 用于诊断历史偏好。旧 `memberCount` 保留一个兼容周期并在前端切换后废弃，不能继续混用。

验证：

- 搜索、tool、recency、折叠只改变 `visibleProjects`，`catalog` 的 ID 集和顺序不变。
- 恰好达到 limit 时通过第 `limit + 1` 行返回 `hasMore=true`；不能仅凭返回数量等于 limit 猜测完整。
- 从 active catalog 暂时删除项目 H，拖动同组其他项目后，H 的 assignment/sort 仍存在；H 恢复后回到保存的位置。
- `activeMemberCount` 不计 H，`storedPreferenceCount` 仍计 H。
- 用 barrier 让 CLI local snapshot 替换分别发生在 local ID 读取前、读取后/prefs commit 前、commit 后；另让 remote snapshot 替换发生在 prefs transaction 取得前和 commit 后。断言 fingerprint 精确对应本次组合快照、新项目最终补入、消失项目变历史、成员不丢且两个 SQLite mutex 无死锁。

### 7.2 用语义移动协议替代可见子集覆盖

推荐新增单一写命令 `move_group_project`，请求至少包含：

```text
projectId
sourceGroupKey
targetGroupKey
anchorProjectId?       # 空表示目标组末尾/空组
placement              # before | after | end
visibleProjectIds      # 动作前可见子序列；仅用于同组槽位合并，不声明完整
expectedRevision
```

返回 `GroupMutationResult`：新 `revision`、起始 `catalogFingerprint`、权威 assignments、sort orders、`activeMemberCount` 和 `storedPreferenceCount`。具体步骤：

1. `groupKey` 只能是字面量 `ungrouped` 或无前导零的正整数；`placement` 使用枚举，禁止自由字符串。
2. 对 ID 数量、单个长度、空值、控制字符和重复值设上限；`projectId` 必须是 active 项目，anchor 必须属于动作前目标组，source 必须匹配当前 assignment。
3. `visibleProjectIds` 是动作前按界面顺序排列的可见子序列。它只允许是当前 active 源组的唯一子集，必须包含被移动项目；后端还要验证它等于“完整源序列按这些 ID 过滤”的结果。它绝不能决定被省略成员是否删除。
4. 后端读取源组/目标组的完整持久顺序，再把没有 sort row 的 active 成员按 catalog 默认顺序补到尾部，形成完整序列。
5. 同组筛选排序采用“可见槽位替换”：完整 `[A,B,C,D]`、可见 `[A,C]`，把 C 拖到 A 前，结果必须是 `[C,B,A,D]`。隐藏 active 成员和历史偏好都保留原槽位及相对顺序。
6. 跨组移动只从源完整序列删除 `projectId`，再相对目标 anchor 插入；目标的隐藏成员和历史偏好保持相对顺序。空组或 `placement=end` 明确追加到尾部。
7. 写入前验证两个完整结果序列无重复、无丢失、只有被移动项目改变组；除明确删除整个组外，历史偏好集合必须与写入前相同。

兼容策略：新命令上线前，搜索、筛选或 `hasMore=true` 时禁用旧 `set_group_order` 的位置拖拽。最终验收必须切到新语义命令并恢复筛选视图的安全拖拽；“永久禁用”不是最终修复。

验证：表驱动纯函数测试覆盖组内无筛选、三类筛选、跨组、空组、ungrouped、远程项目、历史偏好、无效 anchor、source 不匹配和重复 ID；每个成功结果同时断言成员集合、隐藏相对顺序和唯一 sort position。

### 7.3 把所有分组写入纳入同一 revision 和事务

操作：

1. 在 `prefs.db` 增加单例 group revision（例如 `prefs_revisions(scope PRIMARY KEY, revision)`，初始化 `groups=0`）。这是幂等 schema 增量，不修改既有分组数据。
2. 新增 `with_prefs_transaction(TransactionBehavior::Immediate, ...)`；内部 mutation helper 接受 `&rusqlite::Transaction`，不能只接受 `&Connection` 后依赖调用者自觉。
3. 每个事务按固定顺序执行：读取 revision → 比较 `expectedRevision` → 读取并验证全部输入/组/成员 → 计算完整结果 → 执行 DML → 检查 affected rows → revision `+1` → commit。
   - revision 检查优先于幂等/no-op 判断：携带过期 revision 的重复 create/rename/assign 一律返回 `Conflict`，不能因为“看起来已达到目标”掩盖客户端漏看的其他并发变化。
4. `delete_group` 在同一事务删除组、级联 assignment、清理该组 sort；不存在组返回 typed `NotFound`，不能成功空操作。
5. `assign_project_to_group` 在同一事务验证组、只更新该项目 assignment，并按目标是否已进入手工模式追加/清理该项目 sort；不能重写其他成员 assignment。
6. `move_group_project` 在同一事务重编号受影响完整序列为 10/20/30……，更新被移动项目一个 assignment，保留所有历史 preference row，并递增 revision。
7. `create_group`、`rename_group` 以及兼容期旧写命令也必须参与同一 revision。只有实际改变持久状态的成功操作递增；重复 create 返回既有组、rename 为完全相同规范名等 no-op 返回当前 revision，不制造冲突。
8. 任一 SQL 错误原样向上返回并由 transaction rollback；禁止 catch 后继续或在 commit 后伪造旧 revision。

验证：

- 两个客户端携带同一 revision 提交时，只允许第一个成功；第二个收到 `Conflict` 和当前 revision，数据库对应第一个完整动作。
- no-op create/rename/assign 不改变 revision；实际 mutation 恰好增加 1。
- stale revision + 语义 no-op 仍返回 Conflict；frontend 拉取权威 snapshot 后，若目标状态已经成立，可标记 reconciled 而不自动重复写，否则提示重试。
- 不存在组、非法 group key、重复/未知 ID、source/anchor 不匹配、revision 过期都在第一条 DML 前失败。
- 成功跨组移动后，源组不含 moved ID、目标组恰含一次、assignment 唯一、sort position 为严格递增的 10 倍数。

### 7.4 故障注入与回滚证明

操作：

1. 建立逻辑快照 helper，排序读取 `project_groups`、`project_group_assignments`、`project_sort` 和 group revision；不要比较 SQLite 文件逐字节，因为 WAL/header 在正确回滚后也可能变化。
2. 给 delete、assign、move 三条路径分别添加 trigger `RAISE(ABORT, ...)`，让第二条或第三条 DML 失败。
3. 每次失败后重新开连接读取逻辑快照，并运行 `PRAGMA integrity_check`。
4. 对所有“写前校验失败”用 SQLite trace/trigger 证明没有 DML，不能只看最终结果偶然相同。

验证标准：失败前后四类逻辑行完全相等、revision 不变、`integrity_check=ok`；成功路径 revision 恰好增加 1。delete、assign、同组 move、跨组 move 各自必须有独立回滚用例。

### 7.5 前端队列、权威发布和失败恢复

操作：

1. 所有 create/rename/delete/assign/move 进入一个全局 group mutation 队列；跨组动作不能只锁目标组。
2. 乐观预览前深拷贝 catalog 派生视图、assignments、sort orders、两类 member counts 和 revision。
3. 成功后直接发布 `GroupMutationResult` 的权威快照，不再并发调用 `list_groups`、`list_assignments`、`list_sort_orders` 后拼接可能不同时点的结果。
4. 后端 reject 时恢复完整预览快照并显示非破坏性错误；revision conflict 时读取最新权威 snapshot、撤销预览并提示重试。
5. 如果后端已经 commit、但前端发布/重读失败，标记“服务器已保存，界面可能陈旧”并重试读取；不得把已提交动作当作未提交回滚到服务器之前的 UI。

自动验证：用 deferred Promise 模拟两次拖拽逆序返回、revision 冲突、普通拒绝、commit 后 reconciliation 失败；最终 UI 必须与后端最新 revision 一致，ledger 不被错误卡覆盖。

手工验收：依次执行组内拖拽、跨组位置拖拽、标题拖放、tool/recency/search 筛选下拖拽、项目列表超过 limit、临时移除后恢复项目、并发快速拖拽、失败注入和刷新；清除筛选/重启应用后完整顺序与上述语义一致。

回滚：新 schema 表可保留不用；Rust 新命令、revision 返回和 frontend 消费端必须整体回滚。回滚不得删除历史 assignment/sort，也不得降低 `prefs.db` user data。

## 8. WP-04：搜索计数安全渲染

### 问题与目标

`frontend/app.js` 的搜索计数将 `state.query` 插入翻译字符串后写入 `ledgerCount.innerHTML`。当前 CSP 限制了脚本执行，但仍允许标记/样式注入和界面欺骗。

目标不变量：搜索文本始终作为文本节点渲染，不能创建任何元素或属性。

### 实施步骤与逐步验证

1. 将“可信翻译模板 + 不可信变量”改成结构化 render model；不要全局修改 `tr()` 自动转义，因为同一翻译函数同时服务 `textContent`、属性和 HTML，不同上下文不能共享一种转义。
   - 验证：中英文普通搜索计数文本与当前含义一致，翻译模板中的强调语义仍保留。
2. `renderCount()` 的所有分支，以及空搜索结果的第二条渲染路径，都用 `textContent`/`replaceChildren(Text)` 和程序创建的 `<strong>` 节点；不能只修“有结果”分支。
   - 验证：源码静态检查不再存在 `state.query` 经 `tr()` 后进入 `innerHTML` 的数据流。
3. 把纯 render model 放进 `core.js`，`app.js` 只负责创建 DOM；现有 Node test 不引入生产运行时依赖。
   - 验证：`&<>"'` 五类字符、英文单复数、中文和零结果都有表驱动测试。
4. 增加恶意查询测试：`<style>body{display:none}</style><iframe src=https://example.com>` 和带事件属性的 `<img>`。
   - 验证：除 pure render-model 用例外，Playwright 必须加载真实 `app.js`、在搜索框输入 payload，并断言查询字符串按字面显示；计数/空状态容器内没有 `style/iframe/img/script` 元素、事件属性或页面样式变化。
5. 审计所有 `innerHTML = tr(...)` 和模板中的 `${tr(...)}`，对每个动态变量标注输出上下文；本工作包只改存在不可信变量的路径。
   - 验证：审计清单逐项链接到 text、attribute 或 trusted-static-html 三类 sink。
6. 手工验证 `/` 搜索、Esc 清除、中英文切换、有结果和无结果状态；页面样式和布局不能被 payload 改变。

回滚：仅回滚计数渲染器和相应翻译键；不改搜索 API。

## 9. WP-05：修复 xterm URL link provider

### 问题与目标

`provideLinks` 的第一个参数实际是 1-based 行号，当前代码把它当 buffer line 调用 `translateToString`，异常被捕获后始终返回空链接。

目标不变量：任意缓冲区行中的 HTTP(S) URL 都能生成按终端 cell 计算的正确范围；普通点击保持选择，Ctrl+点击打开内置网页标签。

### 实施步骤与逐步验证

1. **首选方案：vendor 与现有 xterm 5.3.0 精确匹配的 WebLinks addon。**
   - 将 addon 文件放入 `frontend/vendor/`，记录上游版本和校验值；`index.html` 只加载本地文件，继续禁止 CDN。
   - handler 复用现有 `openWebTab()`；激活钩子明确要求 Ctrl，避免普通单击抢走文本选择。
   - 验证：离线启动时不发生网络资源请求，xterm 与 addon 版本一致且只注册一个 link provider。
2. **移除旧 provider，避免双重命中。**
   - 不能让旧自定义 provider 与 addon 同时存在；dispose terminal 时也 dispose addon/provider。
   - 验证：反复创建/关闭 terminal tab 后，一个 Ctrl+点击只调用一次 `openWebTab`，无累积 listener。
3. **为链接坐标建立独立契约测试。**
   - 覆盖第 1 行和第 7 行、ASCII 前缀、中文宽字符、emoji、同一行多个 URL、长 URL 折行、滚动后的 buffer、空行和已淘汰行。
   - 若采用官方 addon，不复制测试其内部私有算法；在 Playwright 中通过 fake `pty-data` listener 向真实 vendored xterm 写入这些行，按渲染 cell 移动鼠标并 Ctrl+点击，断言实际可点击区与视觉文本一致。
   - 若采用自定义 provider，另做 provider 返回值单元测试：`start.y/end.y` 等于真实行号，x 范围按 cell 而不是 JavaScript UTF-16 索引。
4. **验证协议和激活策略。**
   - 仅允许 `http:`/`https:`；`javascript:`、`file:` 和相似前缀不生成可激活链接。
   - 验证：普通单击不打开，Ctrl+点击恰好打开一次，同 URL 已存在时聚焦既有 web tab。
5. **仅在 addon 无法满足现有钩子时采用自定义 fallback。**
   - fallback 的签名必须是 `provideLinks(lineNumber, callback)`，通过 `getLine(lineNumber - 1)` 读取，并实现宽字符、emoji 和折行 cell 映射；异常/缺行只回调一次空数组。
   - 上述完整坐标测试未通过前，不得接受简单字符串索引实现。

手工验证：PTY 输出 `前缀 https://example.com/path?q=1` 和跨行长 URL；普通点击可选择文本，Ctrl+点击打开/聚焦一个 web tab。

回滚：vendored addon、`index.html` 加载和 provider 注册必须一起回滚；不影响 PTY 输入输出。

## 10. WP-06：后端写失败时前端不得伪成功

### 问题与目标

设置抽屉多处采用 `await invoke(...).catch(showError)`，catch 后流程继续更新本地对象、清空表单或设置成功状态；远程扫描失败还可能被显示为“扫描 0 项”。

目标不变量：只有后端确认成功后才发布新状态；失败时保留用户输入和最后一次确认状态。

### 实施步骤与逐步验证

1. **先拆分加载错误与动作错误。**
   - `showLoadError` 只用于主数据完全不可用，可显示 retry card；`showActionError` 用于设置/拖拽失败，只显示 toast/行内状态，不能用错误卡覆盖 ledger。
   - 验证：任意 mutation reject 后，现有项目 DOM 和 selected project 仍存在。
2. **建立完整 mutation 清单。**
   - 覆盖 opener enable/command/create/delete、group create/rename/delete/assign/order、remote add/delete/single scan/batch scan，以及所有 `invoke(...).catch(showError)` 写路径。
   - 验证：静态审计清单给出每条命令的 optimistic state、commit point、rollback state 和用户文案。
3. **统一显式控制流。**
   - 禁止 `await invoke(...).catch(showError)` 后继续；全部改为 `try/catch/return` 或通用 mutation controller。
   - canonical state 只在后端成功后更新；若做 optimistic preview，必须在调用前保存深快照。
   - 验证：拒绝 Promise 后，success toast、reset、成功 reload 和 canonical publish 都未调用。
4. **逐类定义失败行为。**
   - checkbox：失败恢复勾选状态。
   - command/rename：保留 DOM 草稿并标记“未保存”，canonical value 不变。
   - delete：成功后才移除/reload，失败保留原行。
   - create：成功后才 reset，失败保留字段、焦点和验证上下文。
   - remote add 成功、随后 scan 失败是显式 partial success：服务器保留并显示“已添加但扫描失败”，提供重试；不能显示“成功扫描 0 项”。
   - 验证：每一类都有 rejected invoke 用例，分别断言恢复/保留行为。
5. **防止 debounce 和乱序保存覆盖新值。**
   - opener command、group rename 等按实体 ID 串行化，或携带 request revision；旧请求晚返回时不能发布旧值。
   - 验证：连续输入 A→B、让 B 先完成 A 后完成，最终 canonical/DOM 都是 B。
6. **补后端 NotFound/affected-row 契约。**
   - `rename_group`、`set_opener_enabled`、`set_opener_command` 等 UPDATE 检查 affected rows；0 行返回 typed `NotFound`，不能 `Ok(())`。
   - create/read query 只把 `QueryReturnedNoRows` 当不存在；数据库解码/IO 错误必须传播。
   - 验证：不存在 ID 返回 NotFound，前端保留状态；正常更新恰好影响一行；数据库故障不被误报为不存在。
7. **处理 commit 后前端读取失败。**
   - 后端已成功、reconciliation 失败时显示“已保存但界面可能陈旧”并重试；不得回滚本地到旧值后告诉用户“保存失败”。
   - 验证：invoke 成功、reload reject 时状态标为 stale，后续成功重读收敛到后端值。

测试实现：把 mutation controller 放在无 DOM 的 `core.js`，以注入的 `invoke/publish/rollback/renderError` 和 deferred Promise 验证；`app.js` 保留事件绑定，另做最小手工 Tauri 验收。

回滚：按单个设置动作回滚；不得恢复“catch 后继续”的公共辅助函数。

## 11. WP-07：刷新、搜索和远程错误的一致发布规则

### 问题与目标

全量 reload 与 60 秒自动刷新共用一个 gate，信息较少的自动刷新可能使全量请求失效。自动刷新只比较数量和最大时间戳，分支/工具等变化可能保持陈旧。全量远程拉取失败返回 `[]`，会清空最后一次有效远程项目。

目标不变量：低信息请求不能覆盖高信息请求；失败不等于空集合；搜索结果不破坏完整项目目录。

### 实施步骤与逐步验证

1. **分离状态。**
   - 直接复用 WP-01 已建立的 catalog/search/view 分离；本工作包只补 remote last-known-good 和 metadata（tools/servers/groups/openers）的 last-known-good，不创建平行 catalog。
   - 验证：进入和退出搜索不需要重新构造或覆盖 catalog；状态对象中只有一个 catalog source of truth。
2. **分离请求所有权。**
   - full reload 与 auto refresh 使用不同 gate；full 开始时 invalidate auto 并设置 `fullReloadInFlight`，auto 在 full/scan/search 任一进行中都不启动。
   - 同类请求仍 latest-wins；auto 只能更新 catalog 数据字段，search 只能更新结果视图，二者都不能使 full token 失效。
   - 验证：用 deferred Promise 穷举“full 先发 auto 后触发”“auto 先发 full 后发且 auto 最晚回”“两个 full/search 逆序完成”，最终都由最新高优先级请求发布。
3. **取消脆弱的变化判断。**
   - 使用稳定 fingerprint，至少包含列表顺序、`id/source/path/name/lastAccessedAt/gitBranch/remoteServerId`，以及每条 usage 的 `toolKey/lastUsedAt/sessionCount/lastSessionId`；或总是发布后由渲染层做廉价 diff。
   - 验证：数量和最大时间相同但路径、分支、工具或 session 变化时 UI 会更新。
4. **将失败与空结果分开。**
   - fetch 函数统一返回 `{ok,value,error}` 或抛错；只有成功的空数组才清空状态。
   - local catalog 失败：全部旧状态保留并显示可重试主错误。remote/tools/servers/groups/openers 失败：各自保留 last-known-good，并显示非阻断 stale warning。
   - 搜索时远程失败显示“仅本地结果”，不得混入上一次查询的远程结果。
   - 验证：remote 成功 `[]` 会清空；remote reject 保留旧值；groups/openers reject 时状态保持调用前值且不得被清空；查询 Q2 的远程失败不会显示 Q1 远程结果。
5. **报告部分失败。**
   - 消费 WP-13 的结构化批量结果；本地成功、部分远程失败时显示非阻断警告，不能静默伪装成完整结果。
   - 验证：混合结果准确列出失败服务器，成功服务器数据仍发布，失败服务器保持 last-known-good。

手工验收：在 dev 模式模拟断开远程、触发自动刷新、立即手动刷新和连续搜索；检查 ledger 不闪空、不回退旧分支、不丢远程项目。

回滚：新状态模型应以单个功能开关切回旧 reload；回滚时不得修改数据库。

## 12. WP-08：文档和目录异步竞态

### 问题与目标

项目 A 的文档/目录请求等待期间切换到项目 B，doc modal 和左侧树共用容器，A 的旧响应可能覆盖 B。入口弹窗当前把旧节点写到已脱离 DOM 的容器，尚未复现直接污染 B，但请求仍会做无效工作。目录展开在等待期间折叠/再展开，还可能延迟压入过期 stack；文件点击依赖全局 `selectedId`，可能与树所属项目不一致。

目标不变量：异步响应只能更新发起时的同一 surface、同一项目和同一请求代次。

### 实施步骤与逐步验证

1. 为 entry docs、entry tree、doc modal、left-pane tree 分别建立 request gate；不要共用一个全局 token。entry 两条路径虽未复现串台，也要在关闭/换项目时取消旧请求。
   - 验证：不同 surface 可并行，同一 surface 只有最新请求能发布。
2. 请求开始时捕获 request ID、`project.id/path` 和容器实例；容器记录自己的 root project identity。完成时同时校验 token、身份和 `container.isConnected`。
   - 验证：项目 A 后返回时不能改变项目 B 的 DOM。
3. 所有 doc modal 关闭入口统一调用 `closeDocModal()`，在其中 invalidate；项目切换、entry 关闭和 view mode 切换也主动 invalidate，禁止散落的 `hidden=true` 绕过清理。
   - 验证：关闭后的响应不重新打开 modal，也不产生未处理异常。
4. 目录节点使用每节点 pending token/promise；同一节点重复 expand 合并请求。expand 立即写入 stack，collapse 立即删除；响应只填 children，不能再次改变展开状态或重复 push。
   - 验证：expand 后立即 collapse，响应回来时仍折叠且 stack 不含该路径；快速 expand 两次只发一个命令。
5. 文件点击从 tree container 获取 root project identity，不再从当前全局 `selectedId` 推断。
   - 验证：旧树上的文件事件不能用新项目路径打开文件。
6. 区分 loading、empty、error；旧错误不能覆盖新成功。
   - 验证：Playwright fake invoke 用反向完成的 deferred Promise 覆盖成功/失败组合，并检查真实标题、路径、内容和展开状态。
7. 手工快速切换至少 20 次项目和 docs/files 模式，并覆盖展开后立即折叠、关闭 modal 后响应完成，确认标题、路径、stack 和内容始终同源。

回滚：每个 surface gate 可独立回滚；不改变 Rust 文件读取命令。

## 13. WP-09：统一根目录路径语义

### 问题与目标

`SqliteStore.UpsertSnapshotProject` 直接 `TrimEnd`，会把 Windows `C:\` 变成 `C:`、Unix `/` 变成空串；`GetProjectByPath` 已使用保留根目录的 helper，两边因此不一致。`Project.Name` 对根目录也可能返回空名称。

目标不变量：写入、查找、去重和显示使用同一根目录安全规范化；Windows 比较仍不区分大小写。

### 实施步骤与逐步验证

1. **冻结规范化契约。**
   - 定义依赖中立的 `NormalizeProjectPath`：先验证/转换绝对路径，再移除非根尾分隔符；drive root、UNC share root 和 Unix `/` 保留根表示。Windows 比较不区分大小写，Unix 保持大小写敏感。
   - 将平台差异收口到内部 `IPathSemantics`/等价 adapter（root 解析、分隔符、comparison）；生产使用当前 OS 的 `System.IO.Path`，纯测试可显式选择 Windows/Unix flavor，避免在 Linux 上用 System.IO 假装验证 Windows 语义。
   - 明确空白、相对路径、不可解析路径和带 `.`/`..` 的行为；拒绝值不能进入数据库。
   - 验证：表驱动覆盖 `C:\`、`C:\repo\`、`C:\repo\.\`、UNC 根/子目录、`/`、`/repo/`、相对路径和空值。
2. **在快照预校验前规范化。**
   - `ValidateSnapshot` 当前仅 `GetFullPath` 后检查重复；改为先得到最终规范值，再基于最终比较器做 duplicate validation。
   - 验证：`repo` 与 `repo/`、Windows 大小写变体在任何写入前被识别为同一项目；失败快照不删除旧索引。
3. **统一所有 Store 入口。**
   - `UpsertSnapshotProject`、legacy `UpsertProject`、`RecordSession`、`GetProjectByPath`、Indexer 去重键和 exact query 全部调用同一 helper；不得各自 `TrimEnd`。
   - 验证：每个入口的根目录 round-trip、尾分隔符等价和重复写入测试均断言稳定 ID/`first_seen`。
4. **统一名称和 FTS。**
   - `Project.Name` 与 `SqliteStore` 写入 `projects_fts.name` 使用同一 root-safe display helper；根目录显示稳定根标识而非空串。
   - 对既有 FTS 内容执行显式幂等 rebuild/migration，不能只修未来 upsert。
   - 验证：根目录能按显示名搜索；重建前后普通项目结果相同；根项目 name 非空。
5. **处理历史错误行。**
   - 先只读检测 `C:`、空路径和规范化后冲突的行并输出诊断；迁移前备份逻辑行。
   - 自动修复只允许在可无歧义证明 `C:` 本意为当前平台 drive root 时执行；否则报告并跳过，不能猜测。
   - 验证：迁移幂等；冲突 fixture 返回可操作错误且旧行不变；迁移失败事务回滚。
6. **跨平台验证。**
   - Windows 断言 `C:\Repo`/`c:\repo\` 合并、UNC 正确；Unix 断言 `/Repo` 与 `/repo` 不合并。
   - CI 建立 `windows-latest` 与 `ubuntu-latest` 矩阵：两边都跑 flavor 纯测试；各自在本机 flavor 额外跑真实 filesystem/store round-trip。单一 OS 的通过不能替代另一平台发布门禁。

回滚：helper/入口/FTS rebuild 必须同批回滚；若已执行历史数据迁移，使用迁移前逻辑备份恢复，不能靠读取时猜测逆变换。

## 14. WP-11：CLI 输入、显示与选择正确性

### 14.1 Spectre 标记和 typed prompt

操作：

1. `RecentCommand` 表格中的工具名/路径使用 `Markup.Escape`；`ScanCommand` 进度中的 custom scanner `ToolName` 也必须转义。
2. 最近会话选择由 `SelectionPrompt<string>` 改为 `RecentSessionChoice` typed choice；取消项使用独立 typed sentinel 或 nullable 结果。
3. 删除 `choice.Contains(path)` 等字符串反查，直接读取 choice 中的 project path、tool key 和 `SessionIdFromTool`。
4. 增加明确的 resolved-open 接口，把用户选中的 exact session ID 传给 launcher；不能重新查询“最新会话”后恢复了另一条 session。
5. `ProjectSelector.UseConverter` 对名称、工具标签和截断路径分别转义；converter 的返回值按 Spectre markup sink 处理。

自动验证：

- 路径含 `[]`、两个路径互为子串、custom tool 名含 `[red]` 时，表格/进度不报错且只按字面显示。
- 两条同项目同工具但 session ID 不同的记录中选择旧记录，launcher argv 必须包含该旧 ID；不能落到最新 ID。
- 取消不调用 launcher；选择项中显示文本重复时仍按对象身份精确命中。

### 14.2 数量边界和工具键比较

操作：

1. CLI `--limit`/`--count` 在 Settings validation 阶段要求项目 `1..10000`、session `1..1000`；错误返回非零和可操作提示。
2. Store 层重复同样边界，防止未来非 CLI 调用绕过；Rust `list_projects(limit)` 在 i64 转 usize/SQL LIMIT 前也实施项目边界。
3. `SqliteStore.ListProjects(toolKey)` 使用显式 `COLLATE NOCASE`；增加与查询表达式匹配的 NOCASE index，避免正确性修复导致全表扫描。
4. ScannerRegistry、launcher 和 UI tool key 的规范值仍保持现有约定；只在比较边界大小写不敏感，不改持久化的 canonical key。

验证：

- 0、-1、项目 10001、session 1001 在 CLI、Store 和 Rust 边界分别被拒绝；SQLite 不再把 -1 当“无限”。
- 1、10000/1000 合法；空数据库返回成功空集合。
- `Codex`/`codex` 返回同一集合，结果中的 canonical tool key 未被改写。
- `EXPLAIN QUERY PLAN` 在 tool filter fixture 上命中新 NOCASE index；迁移重复运行不失败。

回滚：显示、exact-session 传播和输入边界可分别回滚；NOCASE index 可保留，若删除必须先确认无旧版本依赖。

## 15. WP-12：配置原子保存

### 问题与目标

`AppConfig.Save` 当前直接 `File.WriteAllText(config.json)`。进程崩溃、磁盘错误或并发写入可能留下截断 JSON。

目标不变量：保存成功后文件是完整新版本；保存失败后仍是完整旧版本；不同进程不能静默丢掉彼此的更新；崩溃遗留临时文件可安全回收。

### 实施步骤与逐步验证

1. **先定义跨进程修改入口。**
   - 新增 `AppConfig.Update(path, mutation)`/等价 API：取得跨进程锁后重新加载最新完整配置、应用一次 mutation、原子保存，再释放锁。CLI `add/remove/set` 必须走该入口，不能在锁外 load-modify-save。
   - 对仍保留的实例 `Save()` 记录 load 时内容 fingerprint；取得锁后若目标 fingerprint 已变化，返回 typed `Conflict`，不得覆盖较新文件。
   - 验证：两个过期实例先后 Save，第二个返回 Conflict；两个进程经 Update 分别修改不同字段，最终两项都存在。
2. **实现有界跨进程锁。**
   - 在 config 同目录使用固定 lock file 的独占 handle（`FileShare.None`/平台等价），以短退避最多等待 5 秒；进程崩溃由 OS 释放 handle，残留的空 lock 文件可复用，不能仅凭文件存在判死锁。
   - 同进程按规范化 config path 的锁复用该机制；不同 config path 互不阻塞。
   - 验证：两个 helper process 用 barrier 同时更新时串行完成；持锁进程被强制终止后，下一进程能在上限内取得锁；超时返回 Busy 而非破坏文件。
3. **在同目录写唯一临时文件并持久化。**
   - 文件名严格为 `config.json.tmp.<pid>.<guid>`，使用 create-new，写 UTF-8 JSON，调用 `FileStream.Flush(flushToDisk: true)`，关闭 handle 后才替换。
   - 验证：临时 HOME 下成功保存可被 `TryLoad` 完整读取，临时文件没有复用/跟随符号链接。
4. **同卷原子替换目标。**
   - Windows 与 Unix 分支明确替换 API、目标已存在/不存在和元数据行为；不能先删除目标再 move。
   - 验证：重复保存、首次保存、目标只读和跨平台分支均通过；任何可观察时点只能读到完整旧 JSON 或完整新 JSON。
5. **处理本次失败和崩溃遗留临时文件。**
   - finally 只删除本次精确 temp path。每次取得 config lock 后，可清理由同一严格前缀生成、位于同一目录、非 symlink/reparse point 且 mtime 超过 24 小时的 stale temp；不匹配或较新的文件一律不碰。
   - 验证：kill helper process 于 flush 后/replace 前会留下 temp；推进 fixture 时钟超过 24 小时后，下次 Update 只删除该 stale temp，不删除相似名称、其他目录、symlink 或新 temp。
6. **注入每个文件系统失败点。**
   - 为文件动作增加内部 seam，注入“取得锁失败”“创建 temp 失败”“写到一半失败”“flush 失败”“替换失败”。
   - 验证：已有配置保持完整旧版本；首次保存失败时目标仍不存在；本次临时文件被清理；原异常向调用者传播。
7. **修正 CLI 成功/失败发布。**
   - `ConfigCommand` 只在 Update/Save 成功后输出成功文案；Conflict/Busy/IO/权限错误使用不同可操作文案并返回非零。未知异常保持失败，不得转为成功。
   - 验证：只读目录/替换失败 fixture 中 CLI exit code 非零、没有 success 文案、旧配置仍可加载。
8. **审计敏感输出。**
   - 错误日志不输出 custom command、路径偏好或其他潜在敏感值，只报告目标配置路径和错误类别。
   - 验证：用带敏感占位字符串的配置触发失败，stdout/stderr 不包含该字符串。

跨进程压力验证：并行启动至少两个 helper process，各循环 100 次 Update，同时第三个进程持续 TryLoad；所有读取都必须是合法 JSON，最终配置包含每个成功 mutation，任何 Conflict/Busy 都被调用者明确观察而非静默成功。

回滚：实现保持原 JSON schema；可回滚保存算法而无需转换文件。

## 16. WP-10：legacy Avalonia 修复包

该 GUI 已被 Tauri 取代，因此安排在主界面和 Core 修复之后；但共享 Core 变更仍必须保证它可构建。

### 16.1 先建立独立测试边界

1. 新建 `SessionAtlas.Desktop.Tests`，只引用 Desktop 项目并测试 Desktop service/ViewModel/terminal adapter；不要把 Desktop 源再次 link 到现有 root tests，否则共享编译的 `Core/Models` 会产生重复类型身份。
2. 为 dispatcher、terminal process、launcher 和 store 定义最小 adapter；production wrapper 不改变行为，测试使用 fake。
3. 验证：新测试项目能单独运行，也能与 `SessionAtlas.Tests` 同一次 `dotnet test` 运行；无 duplicate type warning/compile error。

### 16.2 精确项目查找

1. `ProjectService.GetLastUsedTool/GetToolUsages` 改用 `SqliteStore.GetProjectByPath`，不再只加载最近一项。
2. 复用 WP-09 的路径比较语义。
3. 验证：目标项目不是最近项目、路径大小写/尾分隔符变化时仍能取得正确 usage；不存在时返回空。

### 16.3 工具可用性

1. `GuessDefaultTool` 使用 `CliLauncher.IsToolAvailable`，让 custom tool key 解析到配置的 `CliCommand`。
2. 若项目历史工具都不可用，返回显式“没有可用工具”，不能无条件 fallback 到 `claude`。
3. 验证：自定义 key 与可执行文件名不同仍能被选中；禁用/不存在工具不会选中；系统无任何 CLI 时不启动进程并显示可操作提示。

### 16.4 精确 resume 与会话记录时点

1. `ProjectListView` 当前取到了选定 session ID，但必须继续传给 tab/ViewModel；`AgentSessionManager` 的 session 参数必须真正进入 `CliLauncher` resume argv，不能被忽略或重新选择最新 session。
2. 不在创建 tab 后立即 `RecordSession`；由 terminal adapter 的“进程已成功启动”事件通知 ViewModel，再记录一次。
3. 记录与 tab launch attempt ID 绑定并做幂等；启动失败不得记录，重复 Loaded 不得重复记录。
4. 验证：选择非最新 session 时 argv 包含 exact ID；成功、启动异常、重复 Loaded 三种用例分别产生 1、0、1 条本地 session 记录。

### 16.5 PTY 生命周期

1. 区分 tab 内容暂时 Unloaded 与用户明确 Close；切换 tab 不能 Kill。
2. 进程终止只由 Close、窗口退出或实际错误路径触发，并保持幂等。
3. 主窗口 closing 统一调用 session manager 的 `CloseAllAsync`，等待有界优雅退出后再 kill；不能只依赖控件 Unloaded。
4. 验证：A/B tab 来回切换后两个 fake PID 仍存活；关闭 A 只终止 A；重复关闭无异常；应用退出恰好终止全部。

### 16.6 UI 线程、SQLite 串行和搜索代次

1. 后台线程只执行查询并返回不可变/普通列表；不得在 `Task.Run` 中修改 `ObservableCollection`。
2. 共享 SQLite store 的异步访问通过 `SemaphoreSlim` 或每操作连接串行化，不能让多个 `Task.Run` 并发使用同一 connection。
3. `ObservableCollection`、SelectedProject 和 StatusMessage 的发布统一回到注入的 Avalonia UI Dispatcher。
4. 搜索增加 debounce、generation/cancellation；旧查询即使不支持物理取消，也不能覆盖新查询。
5. 验证：两个搜索反向完成只发布新查询；fake dispatcher 断言所有 UI mutation 都在 UI 上下文；并发查询最大 in-flight 为 1；无跨线程集合异常。

验收命令：

```powershell
dotnet test SessionAtlas.Tests\SessionAtlas.Tests.csproj --nologo
dotnet test SessionAtlas.Desktop.Tests\SessionAtlas.Desktop.Tests.csproj --nologo
dotnet build SessionAtlas.Desktop --nologo
```

手工验收只在 Windows/Avalonia 环境执行；无头测试仍必须覆盖服务和 ViewModel 状态机。

回滚：Desktop adapter、ViewModel 状态机和 UI 绑定作为一个 Avalonia 单元回滚；独立测试项目可保留。该工作包不改数据库 schema，不得在回滚时删除已存在的 session 记录；共享 WP-09 的 Core 路径修复按其自身回滚方案处理。

## 17. 阶段 6：最终集成验收

### 17.1 完整自动化门禁

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
Pop-Location

git diff --check
```

若依赖源可访问且对应审计工具已安装，再执行；“网络/registry 不可达”和“`cargo-audit` 未安装”必须分别记录，不能都写成审计通过：

```powershell
dotnet list package --vulnerable --include-transitive
dotnet list SessionAtlas.Desktop package --vulnerable --include-transitive
Push-Location src-tauri
cargo audit
Pop-Location
```

### 17.2 无凭据安全集成

1. 临时 HOME 中创建最小 `index.db`、`prefs.db` 和 config。
2. 不配置 SSH key、AI CLI 凭据或真实项目数据。
3. 启动 Tauri 测试入口，验证项目列表读取、搜索、分组排序错误路径和设置失败路径。
4. 确认不会创建/修改临时 `index.db` 的 WAL/SHM，不会启动真实 AI CLI。

### 17.3 手工桌面验收矩阵

| 场景 | 预期证据 |
| --- | --- |
| 搜索恶意标记 | 作为文本显示，无新 DOM 元素 |
| 终端 URL | 正确高亮；Ctrl+点击打开；普通点击选择文本 |
| 后端设置写失败 | 表单和旧状态保留，只显示错误 |
| 远程读取瞬时失败 | 本地和最后有效远程项目保留 |
| 快速切项目/文档/文件树 | 旧响应不能覆盖新项目 |
| 筛选下组内拖拽 | 隐藏成员顺序保留 |
| 搜索或目录截断状态拖拽 | 新语义命令安全合并完整顺序；兼容期旧命令明确禁用 |
| 远程一个扫描根失败 | 单服务器旧快照保留；批量结果明确 partial |
| Avalonia 切换 tab | PTY 不退出；明确关闭才退出 |
| 根目录项目 | 可写入、查回、显示非空名称 |

### 17.4 发布准入条件

只有同时满足以下条件才可宣布剩余已知问题完成：

1. WP-01 至 WP-13 每项都有对应回归测试名称和通过输出。
2. 所有故障注入测试以排序后的逻辑快照和完整性检查证明失败后状态不变；不得把 SQLite 文件 byte equality 当作事务正确性的必要条件。
3. 完整测试、格式、lint、CLI build、Avalonia build 全部通过且无新增 warning。
4. `index.db` 只读证据包括 SQLite 写入拒绝和无 sidecar 文件。
5. 前端竞态测试使用反向完成顺序，而不是只测顺序完成。
6. 当前文档中的手工验收矩阵已执行并记录平台版本。
7. 工作树审查确认没有测试 fixture、数据库、凭据、日志或临时文件被提交。
8. `npm test` 同时包含 pure unit 与 Playwright browser tests；不能只以 `core.js` 测试通过替代 `app.js` 集成证据。
9. Unix shell fail-closed 和 Windows/Unix 路径 round-trip 已在对应 OS 门禁执行；跨进程 config 压力测试无损坏或静默丢更新。

## 18. 追踪矩阵

| 原始症状 | 工作包 | 必需自动化证据 |
| --- | --- | --- |
| 分组半更新、筛选后丢顺序 | WP-01 | 完整集合校验 + 三类 trigger rollback + JS 隐藏成员测试 |
| Tauri 修改 CLI 索引 | WP-02 | readonly flag/query_only + 写入拒绝 + 无 sidecar |
| 损坏行静默消失 | WP-03 | 每类查询的坏类型行返回 Err |
| 一个远程根失败却替换完整快照 | WP-13 | partial stdout + nonzero status + 旧逻辑快照不变 |
| 搜索计数注入标记 | WP-04 | 恶意字符串只产生 Text 节点 |
| URL 永远不可点击 | WP-05 | 行号/buffer/range/协议测试 |
| 保存失败仍显示成功 | WP-06 | rejected invoke 保持状态和表单 |
| 自动刷新覆盖全量、远程失败清空 | WP-07 | deferred Promise 逆序 + last-known-good |
| A 项目响应写入 B | WP-08 | surface token 逆序测试 |
| `C:\` 变 `C:`、`/` 变空 | WP-09 | 根目录 round-trip 测试 |
| legacy 查错项目/切 tab 杀进程 | WP-10 | 精确查找 + PTY 状态机 + UI 线程测试 |
| Spectre 路径报错/选择歧义/负 limit | WP-11 | 恶意标记 + typed choice + 边界值测试 |
| 配置截断 | WP-12 | 原子替换失败注入 + 并发保存测试 |

本设计不授权生产部署、真实 SSH 连接、真实 AI CLI 会话、账户变更或凭据操作；这些动作若需要执行，必须另行获得明确授权。
