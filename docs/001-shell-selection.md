# 技术决策记录 001：壳选型（2026-09-07）

## 结论

**Tauri 壳 + ego-lite 的 driver/skill API 模式**；不采用 ego-lite 本体做壳（它是 agent 浏览器驱动，不是壳），不采用 Obscura（headless 引擎，适合服务端抓取，不适合可见渲染）。

## 调研摘要

| 候选 | 结论 |
|---|---|
| ego-lite (citrolabs) | MIT；agent 浏览器驱动的 snapshot/diff 模式值得借鉴为"设备桥 API"设计；二进制再分发权未公开 |
| Tauri | Rust + 系统 WebView，体积 MB 级，ACL 权限模型契合审批闸门；**选定** |
| Electron | 生态最成熟但 ~200MB；作为 WebKit 兼容性兜底 |
| Obscura (h4ckf0r0day) | Rust 无头浏览器（真实 V8 + CDP，~70MB 单文件）；性能声称为厂商数据未独立验证；headless 定位与洛书不符，可作为平台服务端抓取引擎候选 |

## 设备桥原则

- loopback HTTP/WS only，权限闸门复用猹询码审批体系
- 最小权限：文件/终端/系统信息/截图按能力逐项授权
