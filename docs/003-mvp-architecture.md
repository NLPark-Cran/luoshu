# 技术决策记录 003：MVP 架构（2026-09-07）

## 组件图

```
┌──────────────────────────── 用户设备 ────────────────────────────┐
│                                                                  │
│  洛书 App (Tauri v2, Rust)                                       │
│  ┌──────────────────────────────────────────────────────────┐    │
│  │ Window "shell"                                           │    │
│  │  ┌────────────────────────────────────────────────────┐  │    │
│  │  │ webview "shell" (48px 工具栏, shell/index.html)    │  │    │
│  │  │ back/forward/reload/home · 设备桥状态灯 ·          │  │    │
│  │  │ 复制 MCP 配置 · 审批对话框 · 关于页                 │  │    │
│  │  └────────────────────────────────────────────────────┘  │    │
│  │  ┌────────────────────────────────────────────────────┐  │    │
│  │  │ webview "main" → https://crys.tt2.li               │  │    │
│  │  │ (LUOSHU_TARGET_URL 可覆盖)                          │  │    │
│  │  └────────────────────────────────────────────────────┘  │    │
│  └──────────────┬───────────────────────────┬───────────────┘    │
│        IPC(nav/status/mcp_config/deny)      │ BrowserControl     │
│                 │                           │ (eval/navigate)    │
│  ┌──────────────▼───────────────────────────▼───────────────┐    │
│  │ luoshu-bridge (crates/luoshu-bridge, 独立可测)           │    │
│  │  axum server, bind 127.0.0.1:<random>                    │    │
│  │  POST /mcp  ← MCP Streamable HTTP (JSON-RPC 子集)        │    │
│  │  GET  /healthz                                           │    │
│  │  工具: sysinfo / fs_read / fs_write / shell /            │    │
│  │        screenshot / browser_eval / browser_navigate      │    │
│  └──────────────▲──────────────────────────────────────────┘    │
│                 │ Authorization: Bearer <per-run token>          │
│  ~/.luoshu/bridge.json (0600) {port, token, pid, started_at}     │
│  ~/.luoshu/allowed_roots.json (可选，覆盖默认沙箱根)              │
│  ~/.luoshu/screenshots/                                          │
└─────────────────┬────────────────────────────────────────────────┘
                  │ (云端 agent 所在服务器须有到用户 loopback 的通路,
                  │  或 agent 就跑在本机 —— 见「威胁模型」)
        云端 cran-code session (fastmcp client, transport=http)
```

## 关键决策

### MCP 传输：手写 JSON-RPC 子集，不上 SDK

MCP Streamable HTTP 的服务端最小面只有：`POST /mcp` 接 JSON-RPC 2.0，
实现 `initialize` / `ping` / `tools/list` / `tools/call`，notification 回 202。
手写约 100 行，避免引入尚不稳定的 Rust MCP SDK（rmcp 等）的版本漂移。
SSE 流式响应与 GET /mcp 长连接在 v1 不实现（我们的工具都是短请求-响应，
cran-code 侧 fastmcp 的 `transport="http"` 兼容纯 JSON 响应模式）。

### 鉴权

每次启动生成 64 位十六进制随机 token，与端口一起写入
`~/.luoshu/bridge.json`（0600）。所有 HTTP 请求（含 initialize）都要
`Authorization: Bearer <token>`，常量时间比较。per-run 而非 per-install：
token 不落第三方、重启即换，泄露窗口最小化；代价是每次重启要重新复制配置
（UI 一键复制，可接受）。

### 文件沙箱

- 默认根：用户家目录；`allowed_roots.json` 存在且非空则**替换**默认。
- 根以下的隐藏路径组件（`.xxx`）一律拒绝（`~/.ssh`、`~/.aws` 天然覆盖）。
- 敏感模式在任何位置都拒绝：`.ssh/.aws/.gnupg/.kube/.docker/.netrc/
  id_rsa/id_ed25519/credentials/bridge.json`、`.env*` 前缀、`*.pem/*.key/
  *.p12/*.pfx/*.kdbx` 后缀。
- 读取用 `canonicalize` 后判前缀（防 symlink/`..` 逃逸）；写入对最深已存在
  祖先做 canonicalize 再拼接验证。

### Shell 审批流（v1 简化版）

- 危险清单 = 词级 token 匹配（`shutdown`/`passwd`/`systemctl`…）+ 子串匹配
  （`rm -rf /`、`| sh`、`of=/dev/`…）。词级匹配避免 `cat /etc/passwd`
  误伤。
- 命中清单且无 `approval_token`：桥生成 6 位一次性 token（TTL 5 分钟，绑定
  命令串），通过 `BridgeEventSink` 推给壳 UI 弹窗；工具调用返回
  "approval required" 错误。用户**人工**把批准码转述给 Agent，Agent 带码
  重试；桥校验并消费（一次性）。
- 未命中清单的命令直接执行：30s 超时 kill，stdout/stderr 各截断 64KB。
- 已知弱点：清单覆盖不全（v1 接受），弹窗未做防伪造 UI（v1 接受，见下文
  deferred）。

### Screenshot / Browser 工具

- `luoshu_screenshot`：`screenshots` crate（X11/Wayland/mac/win），存 PNG
  到 `~/.luoshu/screenshots/`，只回路径。feature `screenshot` 可关
  （`--no-default-features`），无显示环境运行时报清晰错误。
- `luoshu_browser_*`：桥定义 `BrowserControl` trait，Tauri 侧用 webview
  的 `eval`/`navigate` 实现。Linux WebKitGTK 的 eval 不返回值，v1 返回
  `"null"`（文档注明）。这是 Computer-Use 的落点：云端 agent 借此驱动
  用户正在看的那个页面。

## 威胁模型

| 威胁 | 缓解 | 残余风险 |
|---|---|---|
| 恶意网站/进程调用桥 | loopback only + 每跑随机 token，token 仅在 0600 文件与 UI 复制中出现 | 本机任意进程若能读 `~/.luoshu/bridge.json`（同 UID）即可调桥 —— 与 SSH agent socket 同级信任模型 |
| Token 经桥自身泄露 | `bridge.json` 在敏感清单内，`~/.luoshu` 是隐藏目录默认不可读 | 用户显式把 `~/.luoshu` 加进 allowed_roots 后仍被 `bridge.json` 敏感规则拦截 |
| Agent 读敏感文件 | 沙箱（根 + 隐藏组件 + 敏感模式） | 清单外的敏感文件（如 `~/documents/工资.xlsx`）不防 —— 沙箱不是数据防泄漏 |
| Agent 跑危险命令 | 危险清单 + 一次性批准码人工转述 | 清单外的破坏性命令（如 `rm -rf ~/projects` 无空格变体已覆盖，但 `mv ~ /tmp` 之类不在）—— v1 明确接受 |
| CSP/供应链 | 壳 UI 无构建步骤、零 npm 依赖；桥依赖锁定 Cargo.lock | Tauri/axum 等上游 CVE 需跟进 |
| 云端侧 | token 出现在 `~/.cran/mcp.json`（云端服务器上） | **重要**：若 agent 跑在远端服务器，它连不到用户的 127.0.0.1。当前形态适用于 agent 本机运行（tt2-cli 场景）；云端 agent 触达需反连隧道（deferred） |

## Deferred（明确不做，后续迭代）

- **观猹 OAuth 登录**：壳内复用 crys 的登录态即可，原生侧暂不接。
- **TokenPay 结算归因**：桥调用计数/上报接口预留位未做。
- **自动更新**：tauri-plugin-updater 未接。
- ~~**反向隧道**~~：已完成，见 docs/004-reverse-tunnel.md。
- ~~**截图内联返回**（MCP image content）、**browser_eval 返回值**~~：已在 v2
  完成——eval 返回值走「JS 回写 loopback + 一次性 nonce」（桥 token 不进页面
  上下文），截图同时返回路径与内联 PNG。残余风险：https 页面 → http loopback
  的 fetch 在个别 WebKitGTK 版本可能被 mixed-content 策略拦截，此时 eval 自动
  降级为 fire-and-forget（`returned: false`）。
- **审批 UI 加固**（防 agent 自读批准码：token 永远不进 webview DOM 之外的
  通道 —— 当前已满足，但未做屏幕截图遮蔽）。
