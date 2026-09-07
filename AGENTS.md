# AGENTS.md — 洛书 Luoshu

> 本仓库由 cran-code 会话接管开发（工作目录即本仓库根目录）。读完本文件 + `README.md` + `docs/` 四份 ADR 即可开工。

## 当前状态（2026-09-07）

MVP 已完成并推送：`crates/luoshu-bridge`（loopback MCP 设备桥）+ `src-tauri`（Tauri v2 壳骨架）+ `shell/`（原生壳 UI）。
**任务 1/5 反向隧道已完成**（docs/004，crys 侧提交 620da51c + 09d9dbc0，已部署生产）：洛书 WS 隧道客户端（重连 + 指数退避 + 4401 停连）+ 云端设备注册/WS 隧道/中继/会话绑定。
**任务 2 Computer Use 工具面已完成**：browser_eval 返回值（nonce 回写）+ 截图内联 + 浏览历史。bridge 40 测试绿。

## v2 任务清单（全部要做，按依赖排序）

1. ~~**反向隧道（最关键）**~~ ✅ 已完成（docs/004）。
2. ~~**Computer Use 工具面扩展**~~ ✅ 已完成：`browser_eval` 返回值（JS 回写 loopback + 一次性 nonce，桥 token 不进页面上下文；拦截/超时自动降级 fire-and-forget）、`luoshu_screenshot` 内联 PNG（>12MB 降级仅路径）、tab 管理走「单 webview + 历史」（`luoshu_browser_history` + `on_navigation` 钩子，上限 500 条）。多 webview tab 条留作任务 3 可选增强。
3. ~~**壳体验**~~ ✅ 已完成：设置页（设备登记/注销 UI，替代手写 device.json）+ 隧道状态灯 + 暗色模式跟随系统 + 洛洛素材（MVP 已有）。窗口/图标 MVP 已具备；启动画面有意不做（保持零构建轻壳）。
4. **观猹登录（公开客户端 + PKCE）**：Tauri 内嵌授权流，loopback 回调 `http://127.0.0.1:<port>/oauth/callback`。申请材料见 cran-code 仓库 `docs/dev/guancha-application.md`（用户已启动申请流程；client_id 到位前把流程代码留可配置）。
5. ~~**cran-code 侧配合改动**~~ ✅ 已完成（crys main: 620da51c + 09d9dbc0）：
   - `devices` 表 + `POST/GET/DELETE /api/v2/devices`（注册/列表/吊销，require_user；在线状态 = 隧道在册，connect + 消息即心跳，故无独立心跳端点）
   - WS 隧道端点 `/api/v2/devices/{id}/tunnel`（设备 token 鉴权，库存 SHA-256 hash）
   - 会话绑定：`create_session(device_id)` 校验归属后记 `SessionState.device_id`；worker 经 `_build_worker_env` 注入 `CRAN_DEVICE_MCP_CONFIG`（loopback 中继 + 每会话 relay token，设备 token 不出云端主进程）

## 纪律

- **构建内存纪律（OOM 教训 2026-09-07）**：生产机仅 4GB RAM。任何 `npm run build` 必须 `NODE_OPTIONS="--max-old-space-size=1024"`，且同一时间只跑一个构建。违规曾一夜触发 3 次 OOM 杀死 cran-code 服务（全线 WS 断连、后台任务丢失）。
- commit 身份必须 `NLPark-Cran <crina@tt2.li>`（repo-local 已配置，勿改）。
- Rust：bridge 逻辑必须在无 GUI 环境可测（`cargo test -p luoshu-bridge`）；Tauri GUI 部分缺 webkit 依赖时记录即可。
- 不提交任何密钥；bridge token 永远 per-install 随机。
- 素材只用 `assets/` 里的特工洛洛图。
