# 洛书 Luoshu

> 猹询码（Cran Code）的本地陪伴客户端 —— 云端 Agent 与本地设备之间的入口桥。

![特工洛洛与伙伴们](assets/luoluo-trio-banner.webp)

## 定位

洛书是一个**本地定制浏览器壳 + 设备桥**：用户在本地打开洛书，即进入云端运行的猹询码 / 镜听空间 / tt2-cli 等项目的统一入口，并通过洛书授予的本地能力（文件、终端、系统信息、截图等）让云端 Agent 真正"触达"本地设备。

- **壳**：Tauri v2（Rust + 系统 WebView）。窗口内嵌 https://crys.tt2.li（`LUOSHU_TARGET_URL` 可覆盖），顶部 48px 原生工具栏（导航 / 设备桥状态灯 / 复制 MCP 配置 / 审批弹窗 / 关于页）
- **桥**：`crates/luoshu-bridge` —— loopback（127.0.0.1，随机端口）MCP server，Agent 经 Bearer token 授权后调用本地能力
- **隧**：反向隧道（ADR 004）——登记设备后，洛书作为 WS 客户端主动连接 `wss://crys.tt2.li/api/v2/devices/{id}/tunnel`，云端把 MCP 调用经隧道下发、本地执行回传（断线重连 + 指数退避；4401 = 凭证失效即停）
- **魂**：陪伴人格「洛洛」（喜欢苹果 🍎），跨会话记忆，观猹登录 + TokenPay 结算（均 deferred）

## 构建

预编译包：GitHub Actions（`.github/workflows/release.yml`）在 Windows / macOS（arm64 + x86_64）/ Linux 上构建；推 `v*` tag 自动发布到 Releases，手动触发（workflow_dispatch）产出 workflow artifacts。**未签名**：macOS 首次打开需右键 → 打开（Gatekeeper），Windows 会弹 SmartScreen 提示。

```bash
# 系统依赖（Debian 13 / trixie）
sudo apt-get install build-essential pkg-config libglib2.0-dev libgtk-3-dev \
  libwebkit2gtk-4.1-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev \
  libxcb1-dev libxcb-randr0-dev libsoup-3.0-dev patchelf imagemagick

cargo build            # 全量（含 Tauri GUI）
cargo test -p luoshu-bridge --all-features   # 桥逻辑单测 + HTTP 集成测试（headless 可跑）

# 无显示/无 webkit 的环境：桥可独立编译与测试
cargo test -p luoshu-bridge --no-default-features   # 关闭截图 feature
```

> **环境说明**：本仓库在 Debian 13 上验证过完整 `cargo build`（webkit2gtk-4.1 已装）。
> 若目标机缺 `libwebkit2gtk-4.1-dev` 等 GUI 依赖，`src-tauri` 无法链接，但
> `crates/luoshu-bridge` 始终可独立编译测试（除可选的 `screenshot` feature，
> 它只额外需要 libxcb 开发包）。

## 运行

```bash
cargo run -p luoshu                                    # 默认打开 https://crys.tt2.li
LUOSHU_TARGET_URL=http://localhost:5496 cargo run -p luoshu   # 指向自建实例
```

启动后设备桥自动拉起，并把 `{port, token, pid, started_at}` 写入
`~/.luoshu/bridge.json`（0600，token 每次启动随机重生成）。

## 反向隧道：让云端 Agent 触达本机

云端 cran-code 的 worker 连不到你的 loopback，所以洛书登记设备后**主动外连**
建立 WebSocket 隧道（ADR 004）。登记（观猹登录到位前的过渡形态）：

1. 在 crys Web UI 登录后调用 `POST /api/v2/devices` 创建设备（响应里的
   `token` 只显示一次）；
2. 把凭证写入 `~/.luoshu/device.json`（0600）：

```json
{
  "cloud_url": "https://crys.tt2.li",
  "device_id": "<device uuid>",
  "device_token": "<一次性 token>",
  "name": "我的机器"
}
```

3. 重启洛书：隧道自动连接，工具栏状态灯显示 tunnel 状态（`tunnel-status` 事件）。

云端侧：会话创建时传 `device_id` 绑定设备；worker 的 MCP 客户端拿到的是
loopback 中继地址 + 每会话 relay token（设备 token 永不出云端主进程）。
吊销：`DELETE /api/v2/devices/{id}`，在线隧道即刻断开。

## 设备桥：给 cran-code 接入本地工具

工具栏点「复制 MCP 配置」，把内容**合并**进 `~/.cran/mcp.json`（cran-code
的全局 MCP 配置，fastmcp 格式），形如：

```json
{
  "mcpServers": {
    "luoshu": {
      "url": "http://127.0.0.1:<port>/mcp",
      "transport": "http",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

或等价 CLI：`cran-code mcp add --transport http luoshu http://127.0.0.1:<port>/mcp --header "Authorization: Bearer <token>"`。
注意 token 每次洛书启动都会变，重启后需重新复制。

### 工具清单

| 工具 | 说明 |
|---|---|
| `luoshu_sysinfo` | OS/内核/arch/CPU/内存/uptime |
| `luoshu_fs_read` / `luoshu_fs_write` | 沙箱内读写文本（默认家目录；隐藏目录与 `.ssh`/`.env`/`*.pem` 等敏感模式拒绝；`~/.luoshu/allowed_roots.json` 可自定义根） |
| `luoshu_shell` | 30s 超时 + 64KB 输出截断；危险命令需 UI 弹窗里的一次性 6 位批准码（5 分钟 TTL，绑定命令） |
| `luoshu_screenshot` | 截屏存 `~/.luoshu/screenshots/`，返回路径 + 内联 PNG（MCP image content，>12MB 降级为仅路径） |
| `luoshu_browser_eval` / `luoshu_browser_navigate` | 驱动内嵌 webview（Computer-Use 钩子；eval 经一次性 nonce 回写返回 Promise 感知完成值，无回写通道时降级 fire-and-forget） |

## 安全模型（摘要，详见 docs/003）

- 桥只监听 127.0.0.1；所有请求需 per-run 随机 Bearer token；token 只在
  `~/.luoshu/bridge.json`(0600) 与 UI 复制按钮中出现，`bridge.json` 本身
  被沙箱敏感规则保护。
- 文件沙箱：allowed roots + 隐藏组件拒绝 + 敏感模式拒绝（symlink/`..` 逃逸
  经 canonicalize 防护）。
- 危险 shell 命令走人工批准（token 由用户转述给 Agent，一次性、限时、绑定命令）。
- **信任边界**：同 UID 的本机进程若能读 `~/.luoshu/bridge.json` 即可调桥
  （与 SSH agent 同级）；云端（远端服务器）上的 agent 连不到你的 loopback ——
  当前形态面向本机 agent（tt2-cli），云端触达需后续做反连隧道。

## 文档

- `docs/001-shell-selection.md` — 壳选型
- `docs/002-memory-architecture.md` — 跨会话记忆
- `docs/003-mvp-architecture.md` — MVP 组件图、威胁模型、deferred 清单
- `docs/004-reverse-tunnel.md` — 反向隧道（v2 任务 1）

## 状态

🚧 v2 进行中（2026-09）：壳 + 设备桥 + 反向隧道可用；观猹登录 / TokenPay / 自动更新 deferred。
