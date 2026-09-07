# 技术决策记录 004：反向隧道（2026-09-07）

> 落地 AGENTS.md v2 任务 1 与 ADR 003 的 deferred 项「反向隧道」。
> 不推翻 ADR 003 的任何决策；loopback 桥原样保留，隧道是其上的新增传输面。

## 问题

云端 cran-code 的 worker 跑在服务器上，连不到用户设备的 127.0.0.1
（ADR 003 威胁模型末节已点明）。需要一个用户设备**主动外连**的通道，
让云端把 MCP 调用下发到本地执行并回传。

## 结论

```
用户设备                                      云端 crys.tt2.li
┌──────────────────────┐                     ┌─────────────────────────────┐
│ 洛书 App             │                     │ FastAPI                     │
│  luoshu-bridge       │  WSS (设备主动外连)  │  WS /api/v2/devices/{id}/   │
│  ┌────────────────┐  │ ◀══════════════════ │        tunnel?token=…       │
│  │ TunnelClient   │  │  JSON-RPC 逐字透传   │  TunnelRegistry(内存)        │
│  │ 重连+指数退避   │  │                     │         ▲                   │
│  └───────┬────────┘  │                     │  POST /api/v2/devices/{id}/ │
│          │ 直接调用   │                     │        mcp（中继, loopback+  │
│  mcp::handle_request │                     │   每会话 relay token）      │
│  (与 HTTP 桥同一代码) │                     │         ▲                   │
└──────────────────────┘                     │  worker fastmcp client      │
                                             │  (CRAN_DEVICE_MCP_CONFIG 注入)│
                                             └─────────────────────────────┘
```

### 消息面

WS 上**逐字透传 MCP JSON-RPC**（cloud→device 为请求，device→cloud 为响应），
即 ADR 003 手写 JSON-RPC 子集原样复用，`mcp::handle_request` 是唯一执行入口
——隧道与 HTTP 桥共享同一套工具实现与沙箱/审批闸门。

中继侧对请求 `id` 做**重写**：云端并发会话可能撞 id（fastmcp 客户端各自
从 1 计数），中继把外发 id 替换为隧道唯一值（`t-<n>`），响应回来后映射回
原 id。设备侧无感知。

### 鉴权（三层，各自独立）

| 跳 | 凭证 | 存储 |
|---|---|---|
| 设备注册 REST `POST /api/v2/devices` | 用户 JWT（`require_user`） | — |
| 隧道 WS | per-device 随机 token（注册时**只返回一次**） | 云端只存 SHA-256 hash；设备侧 `~/.luoshu/device.json`(0600) |
| 中继 HTTP `POST …/mcp` | per-session relay token（会话创建时生成，内存态，随会话销毁） | 云端内存注册表；worker 经 `_build_worker_env` 注入 |

设备 token 绝不进 worker、不进会话元数据；worker 只拿到 loopback 中继 URL +
一次性 relay token。这堵死了「设备 token 出现在云端 mcp.json / worker env /
LLM 上下文」的泄露面（ADR 003 威胁模型最后一行的云端侧风险由此收敛）。

### 断线重连（设备侧）

指数退避 1s→2s→…→60s 封顶，±25% 抖动；连接成功即重置。
云端 close code 4401（token 无效/设备吊销）→ **不再重试**，写事件到
BridgeEventSink 提醒用户重新登记；其他断线一律重试。
云端侧 last_seen_at 在 connect 与每条消息时触碰；在线状态 = TunnelRegistry
在册（内存，进程重启即全员离线，等设备重连即可，不持久化）。

### 设备登记（观猹登录到位前的过渡形态）

用户在 crys Web UI（已登录态）创建设备 → 云端返回一次性 token →
用户写入 `~/.luoshu/device.json`：`{device_id, device_token, cloud_url, name}`。
任务 3（设置页）做 UI 化；任务 4（观猹 PKCE）到位后改为壳内自动登记。
device.json 已加入桥沙箱敏感名单（与 bridge.json 同级）。

## 威胁模型增量

| 威胁 | 缓解 | 残余风险 |
|---|---|---|
| 设备 token 泄露 | 只存 hash；只在创建时返回一次；隧道端点常量时间比较；可吊销（DELETE） | 用户设备上 `~/.luoshu/device.json` 同 UID 可读 —— 与 bridge.json 同级信任模型 |
| 中继端点被外部调用 | 仅接受 loopback 连接 + per-session relay token | 云端主机上同机进程可仿 worker 调用 —— 与 worker root 信任模型一致（crys 既有假设） |
| 隧道被云端其他用户劫持 | WS 校验 token 归属 device.user_id；中继校验会话 owner == device.user_id | — |
| 单设备多连接 | 后连踢先连（同设备新隧道替换旧的），防僵死连接占位 | 拍脑袋式抖动下互相踢 —— 退避抖动已缓解 |
| 放大攻击（云端大请求打满设备） | WS 消息 4MiB 上限（与 HTTP 桥一致）；中继 120s 超时 | 接受 |

## Deferred

- 多 webview tab 管理、截图内联（任务 2）。
- 隧道流量加密依赖 WSS（TLS）本身，不做应用层加密封套。
- 设备-会话绑定的 UI（当前 create_session API 传 `device_id`）。
