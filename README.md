# 洛书 Luoshu

> 猹询码（Cran Code）的本地陪伴客户端 —— 云端 Agent 与本地设备之间的入口桥。

![特工洛洛与伙伴们](assets/luoluo-trio-banner.webp)

## 定位

洛书是一个**本地定制浏览器壳 + 设备桥**：用户在本地打开洛书，即进入云端运行的猹询码 / 镜听空间 / tt2-cli 等项目的统一入口，并通过洛书授予的本地能力（文件、终端、系统信息、截图等）让云端 Agent 真正"触达"本地设备。

- **壳**：Tauri（Rust + 系统 WebView，体积极小；渲染兼容性不足时退 Electron）
- **桥**：loopback HTTP/WS + 权限闸门（参考 cran-code 的审批体系），Agent 经授权后调用本地能力
- **魂**：陪伴人格「洛洛」（喜欢苹果 🍎），跨会话记忆，观猹登录 + TokenPay 结算
- **API 设计参考**：citrolabs/ego-lite 的 driver/skill 模式（MIT）

## 素材

| 文件 | 用途 |
|---|---|
| `assets/luoluo-trio-banner.webp` | 横幅：洛洛与伙伴们 |
| `assets/luoluo-avatar-laptop.webp` | 洛洛工作头像（小） |
| `assets/luoluo-apple-avatar.jpg` | 洛洛苹果头像（大） |

## 文档

- `docs/` — 调研与决策记录（壳选型、记忆架构、观猹/TokenPay 接入）

## 状态

🚧 筹建中（2026-09）。技术决策记录见 docs/。
