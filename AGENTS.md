# AGENTS.md — 洛书 Luoshu

> 本仓库由 cran-code 会话接管开发（工作目录即本仓库根目录）。读完本文件 + `README.md` + `docs/` 三份 ADR 即可开工。

## 当前状态（2026-09-07）

MVP 已完成并推送：`crates/luoshu-bridge`（loopback MCP 设备桥，28 测试绿）+ `src-tauri`（Tauri v2 壳骨架）+ `shell/`（原生壳 UI）。

## v2 任务清单（全部要做，按依赖排序）

1. **反向隧道（最关键）**：云端 cran-code 服务器无法触达用户本地 loopback。实现：洛书作为 WS 客户端主动连接 `wss://crys.tt2.li`（带 token），云端把 MCP tools/call 经隧道下发，洛书本地执行后回传。要求：断线重连、指数退避、token 鉴权、云端侧的设备注册/在线状态端点（需要 cran-code 侧配合改动，见第 5 条）。
2. **Computer Use 工具面扩展**：`browser_eval` 返回值（Linux 上 Tauri eval 是 fire-and-forget——调研用 JS 回写 loopback HTTP 的方式拿结果）、`luoshu_screenshot` 返回内联图片、tab 管理（多 webview 或单 webview + 历史）。
3. **壳体验**：窗口/图标/启动画面/设置页完善；洛洛人格露出（assets/ 里的素材）；暗色模式跟随系统。
4. **观猹登录（公开客户端 + PKCE）**：Tauri 内嵌授权流，loopback 回调 `http://127.0.0.1:<port>/oauth/callback`。申请材料见 cran-code 仓库 `docs/dev/guancha-application.md`（用户已启动申请流程；client_id 到位前把流程代码留可配置）。
5. **cran-code 侧配合改动**（在 /root/workspace/crys 提交，注意那个仓库的 AGENTS.md 与 docs/dev/ 规范）：
   - `devices` 表 + `GET/POST /api/v2/devices`（注册/心跳/列表，require_user）
   - WS 隧道端点 `/api/v2/devices/{id}/tunnel`（洛书侧连入）
   - 会话工具路由：用户启动洛书设备绑定的会话时，把 MCP 调用经隧道下发（可先做成"设备绑定会话"的最小链路：会话元数据记 device_id，worker 的 MCP 客户端连云端中继而非 loopback）

## 纪律

- commit 身份必须 `NLPark-Cran <crina@tt2.li>`（repo-local 已配置，勿改）。
- Rust：bridge 逻辑必须在无 GUI 环境可测（`cargo test -p luoshu-bridge`）；Tauri GUI 部分缺 webkit 依赖时记录即可。
- 不提交任何密钥；bridge token 永远 per-install 随机。
- 素材只用 `assets/` 里的特工洛洛图。
