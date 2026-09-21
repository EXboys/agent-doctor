# 产品边界：个人版 vs 团队版

Agent Doctor 是同一套代码、共用运维内核。**个人**与**团队**以**两套安装包（edition）**分发，应用内不再互切。

构建时设置 `AGENT_DOCTOR_EDITION=personal|team`（默认 `personal`）。

## 用户是小白（写死）

桌面里看到的人，默认是第一次用 Agent 的小白：不看终端，不认识 MCP、路径、协议和命令。

功能设计和界面文案都按这个人来，不能按会读日志的人来。

1. 一句话说清「这是什么问题」，用他能对上的词（项目、工具、连不上、要填密钥）。
2. 按钮写这次点击之后他能看到的变化，例如「放到当前项目」。做不到的事不要写成「修复」。
3. 点完以后，就在他点的那一块写出结果。问题还在时，用一句人话说明为什么还在、下一步点哪里。
4. 命令、文件路径、英文报错不能当唯一说明。可以藏在「详情」里。

## 共同内核

| 能力 | 含义 |
|------|------|
| **doctor** | 发现 runtime、探测配置/网关漂移、解释故障 |
| **repair** | 备份 → 类型化 playbook 修复 → 复检 → 审计（运维修好，不是「切换管家」） |
| **workspace** | 项目隔离，避免记忆 / MCP / Skill 串味 |

## 个人版（TeamUps）

| | |
|--|--|
| **安装包** | `AGENT_DOCTOR_EDITION=personal`（默认） |
| **入口** | TeamUps / 消费端桌面 + CLI |
| **接线** | 仅个人 Provider（无 Evotown / 团队 tab） |
| **价值** | 坏了能查、能修、项目不串味 |
| **不做** | 不做个人中转市场 / 用量看板产品 |

个人 Provider = 接线 endpoint + key + model，验证后写入 runtime，并修好 schema/网关。**服务商 URL/模型模板**（DeepSeek、OpenRouter 等）可以保留，那是填表便利。

## 团队版（企业定制）

| | |
|--|--|
| **安装包** | `AGENT_DOCTOR_EDITION=team` |
| **入口** | 企业定制桌面 + CLI |
| **接线** | 仅 Evotown / 公司网关（无个人 Provider UI） |
| **价值** | 合规、基线、同步、派活、审计 |
| **增量** | `setup` / `sync` / `policy` / `connect` + 审计/合规导出 |
| **不做** | 不为团队合规依赖个人中转生态 |

Evotown 是控制面；Agent Doctor 仍是本机执行与修复工具。详见 [enterprise.md](../enterprise.md)。

## 接线流水线（edition 内）

LLM 接线仍走 **`apply_mode_switch`**：解析凭证 → 写 overlay → 投影各 runtime → effector → LLM probe。edition 门控会拒绝另一条通路。

## 桌面打包

```bash
cd desktop
npm run tauri:build:personal   # TeamUps 个人版 — com.agentdoctor.app → desktop/
npm run tauri:build:team       # 企业团队版 — com.agentdoctor.team → desktop-team/
```

两套包使用不同 bundle id 与更新通道，互不抢更新。详见 [desktop-auto-update.md](../desktop-auto-update.md)。
