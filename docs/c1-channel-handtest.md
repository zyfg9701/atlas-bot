# C1 频道最小可用 · 短手测

> 日期：2026-09-17 · 切片 **C1**（PC 优先 · 单平台 Slack stub）  
> 基线：`zyfg9701/atlas-bot` · **零新 `bot.*` / 零新 command 名**  
> **非真连 Slack** · **非多平台** · 非上架签名 · 非 GA2/YOLO/desktop 集群 · 非强制移动 · 非密钥托管产品化

## 后端范围

| 后端 | C1 `connectChannel` / `disconnectChannel` / `refreshChannel` / `getAgentChannels` |
|------|----------------------------------------------------------------------------------|
| **InMemory**（Hub-only stub） | **已跟** |
| **Box** | **已跟**（同形；token 仍仅进程内存） |
| **CLI** / **OpenAI** | **未跟** — 仍 `UnknownMethod` / `gateway/unknown-method`（闭集，不装成功） |

## Token 边界（硬）

| 阶段 | 行为 |
|------|------|
| **进入** | 仅 `connectChannel` args.`token`（PC password 框粘贴） |
| **存储** | gateway **进程内存**；进程重启丢失（不装「已持久托管」） |
| **外出** | reply / `listAgents` / 日志 info **永不回显** token；`ChannelConnection` 无 token 字段 |
| **清除** | `disconnectChannel` 删除该 `(id, platform)` 键 |
| **非目标** | 钥匙串 / 加密落盘 / 云端托管 |

## stub 语义（钉死）

- **platform：** 仅 `slack`（非法 platform → `InvalidArgs`）
- **status：** 连接成功后钉 **`connected`**（本地 stub，非真连）
- **refreshChannel：** 重读内存并规范化 status/`detail`；**禁止**假装探测外网
- **群 agent：** 默认允许挂频道（不改 `isGroup`）
- **UI 文案：** 必须标明 **本地 stub、未出网**；禁止写「已连接到真实 Slack 工作区」

## 手测表

| 编号 | 内容 | 期望 |
|------|------|------|
| **C1-P1** | InMemory `getAgentChannels` | `SandChannelsView`；≥1 `slack` stub manifest |
| **C1-P2** | `connectChannel` | connections 可见；status=`connected`；**JSON 无 token** |
| **C1-P3** | `disconnectChannel` | 清除后 get 无该连接；内存 token 清除 |
| **C1-P4** | `refreshChannel` | 与上表 stub 语义一致；不假装出网探活 |
| **C1-P5** | 非法路径 | 空 token / 非 slack platform / 未知 id → `InvalidArgs`；无假成功 |
| **C1-P6** | PC Connect | 选 agent → Channels → 粘贴 → Connect；stub 文案可见 |
| **C1-P7** | PC 断/刷新 | Disconnect / Refresh 可达；状态一致 |
| **C1-P8** | G1 / 对话回归 | 建群路径 + Subscribe/Send 不回归 |
| **C1-P9** | 未跟后端 | CLI/OpenAI 频道命令仍 Unknown |
| **C1-P10** | 文档 | 本页 + `pc-user-guide` §4.6；声明非真连/非多平台/非上架/非 GA2·YOLO·集群 |
| **C1-P11** | Box | 同形（本 PR 已跟） |

## 单测入口

```bash
cargo test -p atlas-bot-gateway --lib channel
cargo test -p atlas-bot-gateway --lib box_create_group
```

## 已知限制（验收 §9）

- **入口落点：** PC 侧栏 **Channels (C1 · local stub)**（选中 agent 后主路径）。  
- **stub status：** `connected`。  
- **Box：** 已跟。**CLI / OpenAI：** 未跟 + 闭集。  
- **refresh：** 内存重读 + 规范化 detail；无外网探测。  
- **群挂频道：** 默认允许。  
- **manifest 文案：** blurb / connectGuide / credentialLabel 含「local stub / no egress」。  
- **不做：** 真出网、多平台、新 wire、密钥托管、移动强制、GA2/YOLO/集群。
