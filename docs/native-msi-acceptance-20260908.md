# 本机原生桌面与 MSI 验收（2026-09-08）

对象：`codex/open-source-readiness` 当前未提交工作区，包括扫描协调与内容索引控制。
不代表已公开 Beta 资产，不调用 GitHub Actions，不提交或推送。

## 范围与隔离

- 当前主机无已安装的 SessionAtlas，无运行中的同名进程。
- 原安装包备份到 `artifacts/native-msi-20260908/previous-20260831.msi`。
- 安装、应用数据、WebView2 缓存和证据各用独立目录；合成数据只含两个项目。
- 日常 `~/.sessionatlas` 的 DB/config/lock/sidecar 在操作前后比较大小、mtime 和 SHA-256，
  不记录正文或凭据。安装注册项和快捷方式按前置快照核对恢复。
- computer-use 用于真实窗口观察、截图及允许的交互，不以浏览器 mock 冒充原生结果。
  该技能不允许代操作隐私开关和 UAC；内容索引状态通过隔离 CLI 切换，原生界面负责
  验证实际查询结果与状态更新。隐私控制按钮的原生点击确认不计作已验收。

## 执行记录

完整本机证据目录：`artifacts/native-msi-20260908/`（被 Git 忽略）。

| 门禁 | 状态 | 证据/说明 |
| --- | --- | --- |
| 操作前基线 | PASS | `before.json`；没有已有 MSI/NSIS 安装项 |
| 合成夹具与 CLI 控制流程 | PASS | `fixture-prepare.log`；两个项目、两个内容文件、SQLite sidecar 为 0 |
| 最新源码 MSI 构建 | 待完成 | `build-msi.log` |
| MSI 元数据、哈希与载荷 | 待完成 | 不用旧包或裸 EXE 的哈希替代安装载荷 |
| 管理员安装、注册项、快捷方式 | 待完成 | 使用独立 INSTALLDIR，UAC 由用户确认 |
| 已安装版原生交互与截图 | 待完成 | 覆盖当前内容索引、搜索、导航与文档场景 |
| MSI 卸载与环境恢复 | 待完成 | 只卸载本轮安装的精确 ProductCode |
| 日常数据前后比对 | 待完成 | `before.json` 与最终快照比对 |

本文件将在实际执行后更新 PASS / FAIL / BLOCKED 及证据，不提前宣称验收完成。
