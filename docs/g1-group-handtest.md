# G1 群组最小可用 · 短手测

> 日期：2026-09-17 · 切片 **G1**（PC 优先）  
> 基线：`zyfg9701/atlas-bot` · 零新 `bot.*` / 零新 command 名  
> 频道已另刀 **C1**（`docs/c1-channel-handtest.md`）· 非上架签名 · 非 GA2/YOLO/desktop 集群 · 非强制移动 · 非群套群 / fan-out / 删群

## 后端范围

| 后端 | G1 `createGroup` / `setGroupMembers` |
|------|--------------------------------------|
| **InMemory**（Hub-only stub） | **已跟** |
| **Box** | **已跟**（同形） |
| **CLI** / **OpenAI** | **未跟** — 仍 `UnknownMethod` / `gateway/unknown-method`（闭集，不装成功） |

## 手测表

| 编号 | 内容 | 期望 |
|------|------|------|
| **G1-P1** | InMemory `createGroup` | 成功；`agent.isGroup===true`；`memberIds` 集合对（顺序可规范化） |
| **G1-P2** | `listAgents` | 新建群可见且可辨（`isGroup` / `memberIds`） |
| **G1-P3** | `setGroupMembers` | 成员更新；再 list 一致 |
| **G1-P4** | 非法路径 | 空名 / 空成员 / 未知 id / 群套群 → `InvalidArgs`（或闭集等价）；无假成功 |
| **G1-P5** | PC 建群 | 侧栏 Create group → 列表可选中（`[group]`） |
| **G1-P6** | PC 改成员 | Change members 可达；刷新后成员对 |
| **G1-P7** | 群上对话 | Subscribe / Send / transcript 不回归（单 transcript） |
| **G1-P8** | 频道 | 本刀交付时仍 Unknown；**现由 C1 覆盖** — 见 `docs/c1-channel-handtest.md` |
| **G1-P9** | 文档 | 本页 + `pc-user-guide` §4.5；声明非频道/非上架/非 GA2·YOLO·集群 |
| **G1-P10** | Box | 同形（本 PR 已跟） |

## 单测入口

```bash
cargo test -p atlas-bot-gateway --lib create_group
cargo test -p atlas-bot-gateway --lib box_create_group
```

## 已知限制（§9）

- **入口落点：** PC 侧栏 **Create group** + **Change members**（主路径）；调试区仅文案提示。  
- **Box：** 已跟。**CLI / OpenAI：** 未跟 + 闭集。  
- **群上 Send：** 单 transcript stub/CLI echo；**非** fan-out。  
- **description：** 允许空；**成员顺序：** gateway 规范化（sort + dedupe）。  
- **不做：** 频道族、删群、群套群、移动强制、新 wire。
