# 扫描并发与写锁阶段验收

日期：2026-09-08。基线：`e97b7d2` 之后的本地工作树。
本记录只覆盖扫描协调阶段，不代表安装包、跨平台原生交互或公开版本已经重新验收。

## 实现契约

1. 所有 `SqliteStore` 写连接持有同一数据库的持久 sibling lock：`index.db.lock`。
   SQLite 连接先关闭，随后释放文件锁；锁文件不删除，避免出现同名文件对应不同锁的情况。
   锁等待默认最多 5 秒，竞争返回 Busy，文件系统错误单独返回；进程退出后由操作系统释放锁。
2. CLI/Tauri 在扫描前读取 `scan_generation`，扫描得到可信结果后取得写锁，再复核代际。
   如果其他扫描已提交，则持锁重扫一次；若此后代际仍变化，拒绝覆盖当前索引。
3. 快照、FTS 与代际递增在同一 SQLite 事务中提交。事务失败、无效代际和代际溢出均回滚。
   缺失旧代际标记按 0 处理；读取错误不能伪装成索引不存在。
4. Tauri 每批并发请求最多执行首轮加一轮补扫。补扫期间的新请求共享这一批结果，
   不继续追加扫描；panic 通知等待者并允许下一批重试；递归调用立即失败，避免自锁。
5. 首轮零可信工具结果在获取写锁前返回，不创建索引、SQLite sidecar 或锁文件。
   可信的空快照仍可清理该工具的旧数据；失败/不可用工具保留旧快照。
6. CLI `open` 的查询、选择、取消与启动失败均使用只读路径。终端启动成功后才取得写锁记录会话；
   如果记录失败，错误文案明确区分“终端已经启动”和“会话记录失败”。
7. 桌面的既有只读连接在扫描提交后能观察到新快照。内容索引仍是快照提交后刷新的独立缓存，
   缓存失败不把已经提交的项目快照误报为失败。

## 验证入口

| 目标 | 回归测试或检查 | 通过条件 |
| --- | --- | --- |
| 补扫请求有界 | `scan_coordinator::tests::requests_during_follow_up_do_not_extend_the_batch` | 8 个调用者只执行 2 次，结果一致，下一批仍可运行 |
| 跨进程互斥及异常退出 | `scan_coordinator::tests::another_process_holds_the_lock_until_it_exits` | 独立子进程持锁时 Busy；终止子进程后立即可获取同一锁 |
| panic 与递归恢复 | `scan_coordinator::tests` 中的 panic/recursive 测试 | 等待者不取旧结果，后续请求可重试 |
| 慢扫描不覆盖新数据 | `local_scan_tests::delayed_scan_cannot_overwrite_a_newer_committed_snapshot` | B 先提交，A 被释放后重扫，最终仅保留新快照且代际为 2 |
| 快照与代际原子性 | `store_contract` 的 generation 测试 | 成功递增；触发器失败、无效值、溢出均保留旧快照与 FTS |
| 版本和 guard 门禁 | `store_contract` 的 unsupported / wrong database 测试 | 拒绝过新或负数版本前不建锁；错误 guard 不创建第二个数据库 |
| 零成功不写盘 | CLI scan 与 Tauri local scan 测试 | 缺失索引不新建数据目录；现有索引和 sidecar 保持原状态 |
| CLI 取消/启动失败 | `open_commands_tests` | 数据库、锁与 sidecar 的字节/mtime 不变；直接路径失败不创建索引 |
| 缓存读连接刷新 | `local_scan_tests::existing_index_stays_queryable_after_scan` | 同一个扫描前打开的只读连接可读取扫描后的数据 |
| 全量回归 | 下列本地命令 | 各命令退出 0 |

可复现命令（仓库根目录）：

```powershell
npm --prefix frontend run build:static
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
npm --prefix frontend run check
npm --prefix frontend test
git diff --check
```

首次启动 Rust lint/test 前必须完成 `build:static`，因为 `frontend/dist` 不进入 Git。
本机 Node 测试首次被沙箱的子进程限制阻止（`spawn EPERM`），随后经执行权限审批在沙箱外运行通过。
测试仅使用临时夹具、注入的启动器和本地静态服务器，不以真实 AI CLI/SSH 业务操作作为通过条件。

## 本次结果

- 前端静态资源 staging、语法检查通过；24 项 Node 单元测试、60 项 Playwright 浏览器测试通过。
- 最终 Rust 全量测试 568 项通过，0 失败、0 忽略：CLI 101 项、core 327 项、Tauri 140 项。
  其中 core 存储契约 50 项、扫描协调器 8 项均通过。
- `cargo clippy --workspace --all-targets -- -D warnings`、Rust 格式检查与 `git diff --check` 通过。
  最终 Rust 测试摘要保存在忽略目录下的 `target/scan-coordination-workspace-tests.log`。
- core 契约测试覆盖 Unix 的 `0700`/`0600` 权限规则，但本次 Windows 执行不覆盖 `cfg(unix)` 分支。
- 未调用 GitHub Actions；未重新构建或安装 MSI/NSIS；未提交、推送、打标签或发布。

## 复核与后续边界

此前 Luna 子代理 `implement_scan_coordinator` 提供实现及 CLI 测试候选，
`store_lock_contracts` 补充 core 契约测试。主代理检查实际 diff，修正无限追加补扫、错误分类、
递归等待、失败路径验证与文档契约，并独立执行本地全量验证。
子代理曾将锁文件从通用指纹比较中排除；主代理已把该例外限缩为“迁移事务失败”测试，
普通只读和版本拒绝测试继续保留完整目录指纹检查。

这把锁是协作锁：仅能协调遵守相同协议的写者。升级验证应同时使用本阶段构建的 CLI 和桌面程序，
不能把旧版本写者或外部 SQLite 工具纳入相同并发保证。

内容索引开关与清理闭环的后续实现和独立验证已记录在
[`content-index-controls.md`](./content-index-controls.md)：持久化选择、关闭后停止读取内容、
清理已有词项/预览、重新开启后的重建，以及清理失败时的状态恢复。
后续再处理首次启动/最后成功快照/终端恢复和本地发布证据。
